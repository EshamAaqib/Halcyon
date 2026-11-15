//! The interface every protocol family implements, and the data it takes.
//!
//! A protocol family is the wire format a group of keyboards share (often the same MCU and vendor
//! firmware). Keyboards themselves are JSON definitions in `devices/` that name their protocol; most new
//! keyboards need only a definition. To add a family, write a module like `hfd` and list it in
//! [`transports`], [`open`] and the `validate_*` functions below.

pub mod hfd;

use std::collections::BTreeMap;
use std::ffi::{CStr, CString};
use std::sync::Arc;

use hidapi::HidDevice;
use image::RgbImage;
use serde_json::{Value, json};

use crate::error::{Error, Result, bad};
use crate::hid::Hid;
use crate::json::{int, truthy};

/// An LED frame for a matrix display: one RGB value per LED, row by row.
pub type Frame = Vec<[u8; 3]>;
/// Colour per LED index for per-key lighting.
pub type PerKey = BTreeMap<u32, [u8; 3]>;
/// Function per key index.
pub type Keymap = BTreeMap<u32, Action>;
pub type Progress<'a> = &'a mut dyn FnMut(f64);

#[derive(Clone)]
pub struct LcdFrame {
    pub image: RgbImage,
    pub delay_ms: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightingState {
    pub effect: u8,
    pub color: [u8; 3],
    pub rainbow: bool,
    pub brightness: u8,
    pub speed: u8,
    pub direction: u8,
}

impl Default for LightingState {
    fn default() -> Self {
        LightingState {
            effect: 1,
            color: [0, 255, 255],
            rainbow: false,
            brightness: 5,
            speed: 3,
            direction: 0,
        }
    }
}

impl LightingState {
    /// Read a state sent by the UI, clamped to what the keyboard's definition allows.
    pub fn from_value(v: &Value, lighting: &Value) -> Result<Self> {
        let effect = int(v.get("effect").unwrap_or(&json!(1)), "effect")?;
        let known = lighting["effects"]
            .as_array()
            .map(|a| a.iter().any(|e| e["id"].as_i64() == Some(effect)));
        if known != Some(true) || !(0..=255).contains(&effect) {
            return bad(format!("effect {effect} is not defined for this keyboard"));
        }
        let clamp = |key: &str, default: i64, hi: i64| -> Result<u8> {
            Ok(int(v.get(key).unwrap_or(&json!(default)), key)?.clamp(0, hi.clamp(0, 255)) as u8)
        };
        let mut color = [0u8, 255, 255];
        if let Some(c) = v.get("color") {
            let parts = c
                .as_array()
                .filter(|a| a.len() == 3)
                .ok_or_else(|| Error::Bad("color must be [r, g, b]".into()))?;
            for (slot, p) in color.iter_mut().zip(parts) {
                *slot = int(p, "color")?.clamp(0, 255) as u8;
            }
        }
        let max_dir = lighting["directions"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|d| d["value"].as_i64())
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        Ok(LightingState {
            effect: effect as u8,
            color,
            rainbow: v.get("rainbow").map(truthy).unwrap_or(false),
            brightness: clamp(
                "brightness",
                5,
                lighting["brightness_max"].as_i64().unwrap_or(5),
            )?,
            speed: clamp("speed", 3, lighting["speed_max"].as_i64().unwrap_or(5))?,
            direction: clamp("direction", 0, max_dir)?,
        })
    }

    pub fn to_value(self) -> Value {
        json!({"effect": self.effect, "color": self.color, "rainbow": self.rainbow,
               "brightness": self.brightness, "speed": self.speed, "direction": self.direction})
    }
}

/// What a key does. `Default` returns it to its factory function.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Default,
    Disabled,
    Key { hid: u8, mods: u8 },
    Consumer(u16),
    Mouse(u8),
    Scroll(i8),
    Macro { slot: u8, mode: u8, count: u8 },
    Multi([u8; 3]),
}

impl Action {
    /// Read an action as the UI stores it, e.g. `{"type": "key", "hid": 5, "mods": 1}`. A macro action must
    /// already carry its `slot` (the registry resolves macro ids to slots).
    pub fn from_value(v: &Value) -> Result<Action> {
        let field = |k: &str| {
            v.get(k)
                .ok_or_else(|| Error::Bad(format!("key action needs '{k}'")))
        };
        Ok(
            match v.get("type").and_then(Value::as_str).unwrap_or("default") {
                "default" => Action::Default,
                "disabled" => Action::Disabled,
                "key" => Action::Key {
                    hid: (int(v.get("hid").unwrap_or(&json!(0)), "hid")? & 0xFF) as u8,
                    mods: (int(v.get("mods").unwrap_or(&json!(0)), "mods")? & 0xFF) as u8,
                },
                "consumer" => Action::Consumer((int(field("usage")?, "usage")? & 0xFFFF) as u16),
                "mouse" => Action::Mouse((int(field("buttons")?, "buttons")? & 0xFF) as u8),
                "scroll" => Action::Scroll(if int(field("delta")?, "delta")? > 0 {
                    1
                } else {
                    -1
                }),
                "macro" => {
                    let mode = int(v.get("mode").unwrap_or(&json!(0)), "mode")?;
                    if !(0..=255).contains(&mode) {
                        return bad("macro mode must be between 0 and 255");
                    }
                    let count = if mode == 1 {
                        int(v.get("count").unwrap_or(&json!(1)), "count")?.clamp(1, 255)
                    } else {
                        0
                    };
                    Action::Macro {
                        slot: (int(field("slot")?, "slot")? & 0xFF) as u8,
                        mode: mode as u8,
                        count: count as u8,
                    }
                }
                "multi" => {
                    let keys = field("keys")?
                        .as_array()
                        .ok_or_else(|| Error::Bad("keys must be a list".into()))?;
                    let mut out = [0u8; 3];
                    for (slot, k) in out.iter_mut().zip(keys) {
                        *slot = (int(k, "keys")? & 0xFF) as u8;
                    }
                    Action::Multi(out)
                }
                other => return bad(format!("unknown key action '{other}'")),
            },
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MacroEvent {
    Down(u8),
    Up(u8),
    MouseDown(u8),
    MouseUp(u8),
    DelayMs(u16),
}

impl MacroEvent {
    /// `["down", 4]`, `["up", 4]`, `["mouse_down", 1]`, `["mouse_up", 1]` or `["delay", 50]`.
    pub fn from_value(v: &Value) -> Result<MacroEvent> {
        let pair = v
            .as_array()
            .filter(|a| a.len() == 2)
            .ok_or_else(|| Error::Bad("a macro step is [kind, value]".into()))?;
        let n = int(&pair[1], "macro step")?;
        let byte = (n & 0xFF) as u8;
        Ok(match pair[0].as_str().unwrap_or("") {
            "down" => MacroEvent::Down(byte),
            "up" => MacroEvent::Up(byte),
            "mouse_down" => MacroEvent::MouseDown(byte),
            "mouse_up" => MacroEvent::MouseUp(byte),
            "delay" => MacroEvent::DelayMs(n.clamp(0, 0xFFFF) as u16),
            other => return bad(format!("unknown macro event '{other}'")),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    pub game_mode: bool,
    pub disable_alt_tab: bool,
    pub disable_alt_f4: bool,
    pub disable_win: bool,
    pub fn_switch: bool,
    pub sleep: u8,
    pub key_response: u8,
}

impl Settings {
    pub fn from_value(v: &Value) -> Result<Settings> {
        if !v.is_object() {
            return bad("settings must be an object");
        }
        let flag = |k: &str| v.get(k).map(truthy).unwrap_or(false);
        let sleep = int(v.get("sleep").unwrap_or(&json!(0)), "sleep")?;
        Ok(Settings {
            game_mode: flag("game_mode"),
            disable_alt_tab: flag("disable_alt_tab"),
            disable_alt_f4: flag("disable_alt_f4"),
            disable_win: flag("disable_win"),
            fn_switch: flag("fn_switch"),
            sleep: if (0..=3).contains(&sleep) {
                sleep as u8
            } else {
                0
            },
            key_response: (int(v.get("key_response").unwrap_or(&json!(0)), "key_response")? & 0xFF)
                as u8,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MatrixOptions {
    pub speed: u8,
    pub boot: bool,
    pub brightness: u8,
}

/// One open connection to one keyboard. Only `status` and `set_lighting` are required; the rest is called
/// only when the definition's `features` lists it, and fails with a clear message otherwise.
pub trait Transport: Send {
    fn connection(&self) -> &Value;
    /// The HID path this handle was opened on (`None` for the demo stand-in).
    fn path(&self) -> Option<&CStr>;
    /// At least `{"linked": bool}`; may add `battery` (percent) and `firmware`.
    fn status(&mut self) -> Result<Value>;
    /// Apply a lighting state; `Ok(false)` when the keyboard did not acknowledge it.
    fn set_lighting(&mut self, state: &LightingState) -> Result<bool>;

    fn set_perkey(&mut self, _colors: &PerKey, _brightness: u8) -> Result<bool> {
        unsupported("per-key colours")
    }
    fn set_keymap(&mut self, _keymap: &Keymap) -> Result<bool> {
        unsupported("key remapping")
    }
    fn set_settings(&mut self, _settings: &Settings) -> Result<bool> {
        unsupported("keyboard settings")
    }
    fn sync_clock(&mut self, _slot: u8) -> Result<bool> {
        unsupported("clock sync")
    }
    fn upload_macros(&mut self, _macros: &[Vec<MacroEvent>]) -> Result<bool> {
        unsupported("macros")
    }
    fn upload_matrix(
        &mut self,
        _frames: &[Frame],
        _opts: MatrixOptions,
        _progress: Progress,
    ) -> Result<bool> {
        unsupported("the LED matrix")
    }
    fn upload_lcd(&mut self, _frames: &[LcdFrame], _slot: u8, _progress: Progress) -> Result<bool> {
        unsupported("the screen")
    }
}

fn unsupported<T>(what: &str) -> Result<T> {
    Err(Error::Device(format!(
        "this connection does not support {what}"
    )))
}

/// The connection types a protocol family can talk over, or `None` for an unknown family.
pub fn transports(protocol: &str) -> Option<&'static [&'static str]> {
    match protocol {
        "hfd" => Some(&["dongle", "wired"]),
        _ => None,
    }
}

pub fn known_protocols() -> &'static [&'static str] {
    &["hfd"]
}

pub struct OpenArgs {
    pub connection: Value,
    pub device: HidDevice,
    pub path: CString,
    pub hid: Arc<Hid>,
}

pub fn open(protocol: &str, kind: &str, args: OpenArgs) -> Result<Box<dyn Transport>> {
    match (protocol, kind) {
        ("hfd", "dongle") => Ok(Box::new(hfd::Dongle::new(args))),
        ("hfd", "wired") => Ok(Box::new(hfd::Wired::new(args))),
        _ => Err(Error::Device(format!(
            "protocol '{protocol}' has no '{kind}' transport"
        ))),
    }
}

/// Checks run before anything is stored or sent; each fails with `Error::Bad` for input the keyboard cannot take.
pub fn validate_keymap(protocol: &str, keymap: &Keymap) -> Result<()> {
    match protocol {
        "hfd" => hfd::keymap_table(keymap).map(|_| ()),
        _ => Ok(()),
    }
}

pub fn validate_macros(protocol: &str, macros: &[Vec<MacroEvent>]) -> Result<()> {
    match protocol {
        "hfd" => hfd::encode_macros(macros).map(|_| ()),
        _ => Ok(()),
    }
}
