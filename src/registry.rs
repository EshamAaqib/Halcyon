//! Device definitions, saved state and connected-device management.
//!
//! Definitions come from two places; a user definition with the same id replaces the built-in one:
//!   * `devices/*.json`, compiled into the app
//!   * `~/.halcyon/devices/*.json`, the user's own (drop a file there to add a keyboard, no rebuild)
//!
//! Everything the user changes (effect names, lighting, keymap, macros, per-key colours, settings) lives
//! in `~/.halcyon/state.json`, so it survives app updates.

use std::collections::{BTreeMap, HashMap};
use std::ffi::CString;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock, TryLockError};
use std::thread::sleep;
use std::time::Duration;

use include_dir::{Dir, include_dir};
use serde_json::{Map, Value, json};

use crate::error::{Error, Result, bad};
use crate::hid::Hid;
use crate::json::{int, usb_id};
use crate::paths;
use crate::protocol::{
    self, Action, Frame, Keymap, LcdFrame, LightingState, MacroEvent, MatrixOptions, OpenArgs,
    PerKey, Progress, Settings, Transport,
};

static BUILTIN: Dir = include_dir!("$CARGO_MANIFEST_DIR/devices");
const REQUIRED: [&str; 6] = [
    "id",
    "vendor",
    "name",
    "protocol",
    "connections",
    "lighting",
];
/// width x height x max_frames for one display: bounds the memory a prepared upload can take (the AMG65's
/// screen is 240 x 135 x 130, about 4.2 million).
const MAX_DISPLAY_PIXELS: u64 = 16_000_000;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner()) // a panic elsewhere must not lock every later request out
}

/// Fail on anything the rest of the app would otherwise trip over on every request.
pub fn check_definition(d: &Value) -> Result<()> {
    let obj = d
        .as_object()
        .ok_or_else(|| Error::Bad("not a JSON object".into()))?;
    let missing: Vec<&str> = REQUIRED
        .iter()
        .copied()
        .filter(|k| !obj.contains_key(*k))
        .collect();
    if !missing.is_empty() {
        return bad(format!("missing {}", missing.join(", ")));
    }
    let id = d["id"].as_str().unwrap_or("");
    let ok_id = id
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        && paths::is_plain_name(id); // ids appear in URLs and folder names
    if !ok_id {
        return bad(
            "id may only contain letters, digits, '.', '_' and '-', and must be usable as a folder name",
        );
    }
    let proto = d["protocol"].as_str().unwrap_or("");
    let Some(kinds) = protocol::transports(proto) else {
        return bad(format!(
            "unknown protocol '{proto}' (known: {})",
            protocol::known_protocols().join(", ")
        ));
    };
    let conns = d["connections"]
        .as_array()
        .filter(|c| !c.is_empty())
        .ok_or_else(|| Error::Bad("connections must be a non-empty list".into()))?;
    for c in conns {
        let kind = c["type"].as_str().unwrap_or("");
        if !kinds.contains(&kind) {
            return bad(format!("protocol '{proto}' has no '{kind}' transport"));
        }
        for key in ["vid", "pid", "usage_page", "usage"] {
            if key == "vid" || key == "pid" || c.get(key).is_some() {
                usb_id(&c[key], key)?;
            }
        }
    }
    let effects = d["lighting"]["effects"]
        .as_array()
        .ok_or_else(|| Error::Bad("lighting.effects must be a list".into()))?;
    for e in effects {
        // the UI and the lighting code compare ids as JSON numbers, and the firmware takes one byte
        if !e["id"].as_i64().is_some_and(|n| (0..=255).contains(&n)) {
            return bad("every effect id must be a number between 0 and 255");
        }
    }
    for display in ["matrix", "lcd"] {
        let f = &d["features"][display];
        if f.is_null() {
            continue;
        }
        let dim = |k: &str, hi: u64| f[k].as_u64().filter(|n| (1..=hi).contains(n));
        let (Some(w), Some(h), Some(frames)) = (
            dim("width", 1024),
            dim("height", 1024),
            dim("max_frames", 1000),
        ) else {
            return bad(format!(
                "features.{display} needs a width and height (1..1024) and max_frames (1..1000)"
            ));
        };
        if w * h * frames > MAX_DISPLAY_PIXELS {
            return bad(format!("features.{display} is too large"));
        }
    }
    for (name, o) in d["originals"].as_object().into_iter().flatten() {
        // definitions are untrusted: these names must not reach outside Halcyon's and the vendor's folders
        let file_ok = o["file"].as_str().is_none_or(paths::is_plain_name);
        let path_ok = o["path"].as_str().is_none_or(paths::is_safe_relative);
        if !o.is_object() || !file_ok || !path_ok {
            return bad(format!(
                "originals.{name}: 'file' must be a plain file name and 'path' a relative path without '..'"
            ));
        }
    }
    let keys = d["layout"]["keys"]
        .as_array()
        .ok_or_else(|| Error::Bad("layout.keys must be a list".into()))?;
    for k in keys {
        for f in ["x", "y", "w", "code", "key_index", "led"] {
            if !k[f].is_number() {
                return bad(format!("every layout key needs a numeric '{f}'"));
            }
        }
    }
    Ok(())
}

pub struct Definitions {
    pub map: BTreeMap<String, Value>,
    pub problems: Vec<Value>,
}

/// All definitions, plus a list of problems, so a broken community file never stops the app.
pub fn load_definitions() -> Definitions {
    let mut map = BTreeMap::new();
    let mut problems = Vec::new();
    let mut add = |source: &str, name: String, text: std::result::Result<String, String>| {
        let parsed =
            text.and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()));
        match parsed.and_then(|d| check_definition(&d).map(|_| d).map_err(|e| e.to_string())) {
            Ok(mut d) => {
                d["source"] = json!(source);
                d["file"] = json!(name);
                map.insert(d["id"].as_str().unwrap_or_default().to_string(), d);
            }
            Err(e) => problems.push(json!({"file": name, "error": e})),
        }
    };
    let mut builtin: Vec<_> = BUILTIN
        .files()
        .filter(|f| f.path().extension().is_some_and(|e| e == "json"))
        .collect();
    builtin.sort_by_key(|f| f.path());
    for f in builtin {
        let text = f
            .contents_utf8()
            .map(str::to_string)
            .ok_or_else(|| "not UTF-8".to_string());
        add("built-in", f.path().display().to_string(), text);
    }
    let dir = paths::user_devices();
    let dir_problem = fs::create_dir_all(&dir)
        .err()
        .map(|e| json!({"file": dir.display().to_string(), "error": e.to_string()}));
    let mut user: Vec<_> = fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    user.sort();
    for p in user {
        add(
            "user",
            p.display().to_string(),
            fs::read_to_string(&p).map_err(|e| e.to_string()),
        );
    }
    problems.extend(dir_problem);
    Definitions { map, problems }
}

/// The user's saved values, one object per keyboard, written atomically.
pub struct State {
    data: Mutex<Map<String, Value>>,
    readonly: AtomicBool,
}

impl State {
    fn load(readonly: bool) -> State {
        let file = paths::state_file();
        let data = match fs::read(&file) {
            Err(_) => Map::new(), // first run
            Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                Ok(Value::Object(m)) => m,
                _ => {
                    // The next save would overwrite it: keep the damaged file so nothing is lost silently.
                    if !readonly {
                        let _ = fs::rename(&file, file.with_extension("json.damaged"));
                    }
                    eprintln!(
                        "halcyon: {} could not be read; starting with default settings",
                        file.display()
                    );
                    Map::new()
                }
            },
        };
        State {
            data: Mutex::new(data),
            readonly: AtomicBool::new(readonly),
        }
    }

    pub fn get(&self, device_id: &str, key: &str) -> Option<Value> {
        lock(&self.data)
            .get(device_id)
            .and_then(|d| d.get(key))
            .cloned()
    }

    /// Change one saved value and write the file; a crash mid-write must not leave a truncated file.
    pub fn put(&self, device_id: &str, key: &str, value: Value) -> Result<()> {
        let mut data = lock(&self.data);
        let entry = data
            .entry(device_id.to_string())
            .or_insert_with(|| json!({}));
        if !entry.is_object() {
            *entry = json!({});
        }
        entry[key] = value;
        if self.readonly.load(Ordering::Relaxed) {
            return Ok(());
        }
        let file = paths::state_file();
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = file.with_extension("json.tmp");
        fs::write(
            &tmp,
            serde_json::to_vec_pretty(&*data).map_err(|e| Error::Internal(e.to_string()))?,
        )?;
        fs::rename(&tmp, &file)?;
        Ok(())
    }
}

/// Stands in for hardware in `--demo` mode, for UI work and contributors without the keyboard.
struct Demo {
    connection: Value,
}

impl Demo {
    fn upload(&self, progress: Progress) -> Result<bool> {
        for i in 1..=10 {
            sleep(Duration::from_millis(50)); // slow enough that the UI's progress bar can be seen moving
            progress(i as f64 / 10.0);
        }
        Ok(true)
    }
}

impl Transport for Demo {
    fn connection(&self) -> &Value {
        &self.connection
    }
    fn path(&self) -> Option<&std::ffi::CStr> {
        None
    }
    fn status(&mut self) -> Result<Value> {
        Ok(json!({"linked": true}))
    }
    fn set_lighting(&mut self, _: &LightingState) -> Result<bool> {
        Ok(true)
    }
    fn set_perkey(&mut self, _: &PerKey, _: u8) -> Result<bool> {
        Ok(true)
    }
    fn set_keymap(&mut self, _: &Keymap) -> Result<bool> {
        Ok(true)
    }
    fn set_settings(&mut self, _: &Settings) -> Result<bool> {
        Ok(true)
    }
    fn sync_clock(&mut self, _: u8) -> Result<bool> {
        Ok(true)
    }
    fn upload_macros(&mut self, _: &[Vec<MacroEvent>]) -> Result<bool> {
        Ok(true)
    }
    fn upload_matrix(&mut self, _: &[Frame], _: MatrixOptions, progress: Progress) -> Result<bool> {
        self.upload(progress)
    }
    fn upload_lcd(&mut self, _: &[LcdFrame], _: u8, progress: Progress) -> Result<bool> {
        self.upload(progress)
    }
}

type Slot = Arc<Mutex<Option<Box<dyn Transport>>>>;
type SlotGuard<'a> = MutexGuard<'a, Option<Box<dyn Transport>>>;

/// A command that panicked mid-way poisons its slot. Close that handle (its state is unknown) and clear the
/// poison: otherwise `try_lock` in [`Manager::status`] would fail forever and report "busy" until restart.
fn recover_slot<'a>(
    slot: &'a Slot,
    guard: std::result::Result<SlotGuard<'a>, PoisonError<SlotGuard<'a>>>,
) -> SlotGuard<'a> {
    guard.unwrap_or_else(|p| {
        slot.clear_poison();
        let mut g = p.into_inner();
        *g = None;
        g
    })
}

/// Display data handed to [`Manager::upload_display`].
pub enum Upload {
    Matrix(Arc<Vec<Frame>>, MatrixOptions),
    Lcd(Arc<Vec<LcdFrame>>, u8),
}

pub struct Manager {
    defs: RwLock<Definitions>,
    pub state: State,
    slots: Mutex<HashMap<String, Slot>>,
    last_status: Mutex<HashMap<String, Value>>,
    hid: Arc<Hid>,
    demo: AtomicBool,
}

impl Manager {
    pub fn new(demo: bool) -> Manager {
        let state = State::load(demo); // demo sessions never touch the user's saved settings
        Manager {
            defs: RwLock::new(load_definitions()),
            state,
            slots: Mutex::new(HashMap::new()),
            last_status: Mutex::new(HashMap::new()),
            hid: Arc::new(Hid::default()),
            demo: AtomicBool::new(demo),
        }
    }

    pub fn demo(&self) -> bool {
        self.demo.load(Ordering::Relaxed)
    }

    pub fn reload(&self) {
        *self.defs.write().unwrap_or_else(|e| e.into_inner()) = load_definitions();
    }

    pub fn ids(&self) -> Vec<String> {
        self.defs
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .map
            .keys()
            .cloned()
            .collect()
    }

    pub fn problems(&self) -> Vec<Value> {
        self.defs
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .problems
            .clone()
    }

    pub fn def(&self, id: &str) -> Result<Value> {
        self.defs
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .map
            .get(id)
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("unknown device '{id}'")))
    }

    /// The definition with the user's effect names merged in.
    pub fn definition(&self, id: &str) -> Result<Value> {
        let mut d = self.def(id)?;
        let names = self
            .state
            .get(id, "effect_names")
            .unwrap_or_else(|| json!({}));
        if let Some(effects) = d["lighting"]["effects"].as_array_mut() {
            for e in effects {
                if let Some(n) = names.get(e["id"].to_string()).and_then(Value::as_str) {
                    e["name"] = json!(n);
                    e["named_by_user"] = json!(true);
                }
            }
        }
        Ok(d)
    }

    /// The first connected interface matching one of the definition's connections.
    fn find(&self, d: &Value) -> Result<Option<(Value, CString)>> {
        for c in d["connections"].as_array().into_iter().flatten() {
            let (vid, pid) = (usb_id(&c["vid"], "vid")?, usb_id(&c["pid"], "pid")?);
            for i in self.hid.enumerate(vid, pid)? {
                let page_ok = c.get("usage_page").is_none()
                    || usb_id(&c["usage_page"], "usage_page")? == i.usage_page;
                let usage_ok = c.get("usage").is_none() || usb_id(&c["usage"], "usage")? == i.usage;
                let product_ok = c["product"]
                    .as_str()
                    .is_none_or(|p| i.product.as_deref() == Some(p));
                if page_ok && usage_ok && product_ok {
                    return Ok(Some((c.clone(), i.path)));
                }
            }
        }
        Ok(None)
    }

    /// `{device id: connection type}` for every defined keyboard that is plugged in.
    pub fn connected(&self) -> BTreeMap<String, String> {
        // a copy, so a reload (which takes the write lock) never waits behind USB enumeration
        let defs = self
            .defs
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .map
            .clone();
        defs.iter()
            .filter_map(|(id, d)| {
                let c = if self.demo() {
                    d["connections"].as_array()?.last().cloned()
                } else {
                    self.find(d).ok()??.0.into()
                };
                Some((id.clone(), c?["type"].as_str()?.to_string()))
            })
            .collect()
    }

    fn slot(&self, id: &str) -> Slot {
        lock(&self.slots).entry(id.to_string()).or_default().clone()
    }

    fn open(&self, id: &str) -> Result<Box<dyn Transport>> {
        let d = self.def(id)?;
        if self.demo() {
            let connection = d["connections"]
                .as_array()
                .and_then(|a| a.last())
                .cloned()
                .unwrap_or_default();
            return Ok(Box::new(Demo { connection }));
        }
        let (connection, path) = self.find(&d)?.ok_or_else(|| {
            Error::Device(format!(
                "{} {} is not connected",
                d["vendor"].as_str().unwrap_or(""),
                d["name"].as_str().unwrap_or("")
            ))
        })?;
        let device = self.hid.open(&path)?;
        let kind = connection["type"].as_str().unwrap_or("").to_string();
        protocol::open(
            d["protocol"].as_str().unwrap_or(""),
            &kind,
            OpenArgs {
                connection,
                device,
                path,
                hid: self.hid.clone(),
            },
        )
    }

    /// Call `f` with the keyboard's open connection, opening it if needed. A failed HID read or write
    /// gets one retry on a fresh handle (the keyboard may have been re-plugged).
    fn run_locked<T>(
        &self,
        id: &str,
        guard: &mut Option<Box<dyn Transport>>,
        f: &mut dyn FnMut(&mut dyn Transport) -> Result<T>,
    ) -> Result<T> {
        for attempt in 0..2 {
            let result = match guard {
                Some(dev) => f(dev.as_mut()),
                None => self.open(id).and_then(|dev| f(guard.insert(dev).as_mut())),
            };
            match result {
                Err(Error::Io(m)) => {
                    *guard = None;
                    if attempt == 1 {
                        return Err(Error::Device(m));
                    }
                }
                other => return other,
            }
        }
        unreachable!("the loop returns on its second pass")
    }

    fn run<T>(&self, id: &str, mut f: impl FnMut(&mut dyn Transport) -> Result<T>) -> Result<T> {
        let slot = self.slot(id);
        let guard = slot.lock();
        let mut guard = recover_slot(&slot, guard);
        self.run_locked(id, &mut guard, &mut f)
    }

    pub fn status(&self, id: &str) -> Result<Value> {
        self.def(id)?;
        let slot = self.slot(id);
        let attempt = match slot.try_lock() {
            Ok(g) => Some(g),
            Err(TryLockError::Poisoned(p)) => Some(recover_slot(&slot, Err(p))),
            Err(TryLockError::WouldBlock) => None,
        };
        let Some(mut guard) = attempt else {
            // an upload or another command is running; don't queue behind it
            let mut last = lock(&self.last_status)
                .get(id)
                .cloned()
                .unwrap_or_else(|| json!({}));
            last["busy"] = json!(true);
            return Ok(last);
        };
        let demo = self.demo();
        let hid = self.hid.clone();
        let st = self.run_locked(id, &mut guard, &mut |dev: &mut dyn Transport| {
            // A wired status() never talks to the keyboard, so without this an unplugged cable would
            // keep reading as connected until the next command failed.
            if !demo {
                let c = dev.connection();
                let present = hid
                    .enumerate(usb_id(&c["vid"], "vid")?, usb_id(&c["pid"], "pid")?)?
                    .iter()
                    .any(|i| Some(i.path.as_c_str()) == dev.path());
                if !present {
                    return Err(Error::Io("the keyboard was unplugged".into()));
                }
            }
            let mut st = dev.status()?;
            st["connection"] = dev.connection()["type"].clone();
            Ok(st)
        });
        let st = st
            .unwrap_or_else(|e| json!({"connection": null, "linked": false, "error": e.message()}));
        lock(&self.last_status).insert(id.to_string(), st.clone());
        Ok(st)
    }

    pub fn lighting(&self, id: &str) -> Value {
        self.state
            .get(id, "lighting")
            .filter(Value::is_object)
            .unwrap_or_else(|| LightingState::default().to_value())
    }

    pub fn set_lighting(&self, id: &str, data: &Value) -> Result<bool> {
        let s = LightingState::from_value(data, &self.def(id)?["lighting"])?;
        let ok = self.run(id, |dev| dev.set_lighting(&s))?;
        if ok && !self.demo() {
            self.state.put(id, "lighting", s.to_value())?;
        }
        Ok(ok)
    }

    pub fn config(&self, id: &str) -> Value {
        let get = |k: &str, default: Value| self.state.get(id, k).unwrap_or(default);
        json!({"keymap": get("keymap", json!({})), "macros": get("macros", json!([])),
               "perkey": get("perkey", json!({})), "settings": get("settings", json!({}))})
    }

    /// Run `f` if the keyboard supports `feature` over the connection it is on now.
    fn feature<T>(
        &self,
        id: &str,
        feature: &str,
        mut f: impl FnMut(&mut dyn Transport) -> Result<T>,
    ) -> Result<T> {
        let d = self.def(id)?;
        let Some(spec) = d["features"].get(feature).cloned() else {
            return Err(Error::Device(format!(
                "{} does not support {feature}",
                d["name"].as_str().unwrap_or("this keyboard")
            )));
        };
        let demo = self.demo();
        self.run(id, |dev| {
            let kind = dev.connection()["type"].clone();
            let allowed = spec["connections"]
                .as_array()
                .is_none_or(|a| a.contains(&kind));
            if !demo && !allowed {
                return Err(Error::Device(
                    "This needs the USB cable. Plug the keyboard in by cable and try again.".into(),
                ));
            }
            f(dev)
        })
    }

    fn protocol_of(&self, id: &str) -> Result<String> {
        Ok(self.def(id)?["protocol"].as_str().unwrap_or("").to_string())
    }

    /// Stored actions reference macros by id; the keyboard wants their slot number.
    fn resolved_keymap(&self, id: &str, keymap: &Value) -> Result<Keymap> {
        let macros = self.state.get(id, "macros").unwrap_or_else(|| json!([]));
        let slots: HashMap<String, usize> = macros
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(i, m)| (m["id"].to_string(), i))
            .collect();
        let mut out = Keymap::new();
        for (k, a) in keymap.as_object().into_iter().flatten() {
            let index = k
                .parse::<u32>()
                .map_err(|_| Error::Bad(format!("'{k}' is not a key index")))?;
            let action = if a["type"] == "macro" {
                let Some(&slot) = slots.get(&a["macro"].to_string()) else {
                    continue;
                }; // a deleted macro: the key falls back to its default
                let mut a = a.clone();
                a["slot"] = json!(slot);
                Action::from_value(&a)?
            } else {
                Action::from_value(a)?
            };
            out.insert(index, action);
        }
        Ok(out)
    }

    pub fn save_keymap(&self, id: &str, keymap: &Value) -> Result<bool> {
        let obj = keymap
            .as_object()
            .ok_or_else(|| Error::Bad("keymap must map key indexes to actions".into()))?;
        let mut clean = Map::new();
        for (k, v) in obj {
            if v.is_null() || v["type"] == "default" || v.as_object().is_some_and(|o| o.is_empty())
            {
                continue;
            }
            if !v.is_object() {
                return bad("keymap must map key indexes to actions");
            }
            let index = int(&json!(k), "key index")?;
            clean.insert(index.to_string(), v.clone());
        }
        let clean = Value::Object(clean);
        protocol::validate_keymap(&self.protocol_of(id)?, &self.resolved_keymap(id, &clean)?)?; // before it is stored
        self.state.put(id, "keymap", clean.clone())?;
        let resolved = self.resolved_keymap(id, &clean)?;
        self.feature(id, "keymap", |dev| dev.set_keymap(&resolved))
    }

    pub fn save_macros(&self, id: &str, macros: &Value) -> Result<bool> {
        let list = macros
            .as_array()
            .ok_or_else(|| Error::Bad("each macro needs an id and a list of events".into()))?;
        let mut events = Vec::new();
        for m in list {
            let ok = (m["id"].is_string() || m["id"].is_number()) && m["events"].is_array();
            if !ok {
                return bad("each macro needs an id and a list of events");
            }
            events.push(
                m["events"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(MacroEvent::from_value)
                    .collect::<Result<Vec<_>>>()?,
            );
        }
        protocol::validate_macros(&self.protocol_of(id)?, &events)?; // e.g. size, before anything is stored or sent
        self.state.put(id, "macros", macros.clone())?;
        let keymap = self.resolved_keymap(
            id,
            &self.state.get(id, "keymap").unwrap_or_else(|| json!({})),
        )?;
        self.feature(id, "macros", |dev| {
            let ok = dev.upload_macros(&events)?;
            Ok(dev.set_keymap(&keymap)? && ok) // slot numbers may have moved
        })
    }

    pub fn save_perkey(&self, id: &str, colors: &Value, brightness: &Value) -> Result<bool> {
        let obj = colors
            .as_object()
            .ok_or_else(|| Error::Bad("colors must map LED indexes to [r, g, b]".into()))?;
        let mut cols = PerKey::new();
        for (k, v) in obj {
            let parts = v
                .as_array()
                .filter(|a| a.len() == 3)
                .ok_or_else(|| Error::Bad("each colour is [r, g, b]".into()))?;
            let mut c = [0u8; 3];
            for (slot, p) in c.iter_mut().zip(parts) {
                let n = int(p, "colour")?;
                if !(0..=255).contains(&n) {
                    return bad("colour values must be between 0 and 255");
                }
                *slot = n as u8;
            }
            let led = k
                .parse::<u32>()
                .map_err(|_| Error::Bad(format!("'{k}' is not an LED index")))?;
            cols.insert(led, c);
        }
        let max = self.def(id)?["lighting"]["brightness_max"]
            .as_i64()
            .unwrap_or(5);
        let brightness = int(brightness, "brightness")?.clamp(0, max.clamp(0, 255)) as u8;
        let stored: Map<String, Value> = cols
            .iter()
            .map(|(k, v)| (k.to_string(), json!(v)))
            .collect();
        self.state.put(id, "perkey", Value::Object(stored))?;
        self.feature(id, "perkey", |dev| dev.set_perkey(&cols, brightness))
    }

    pub fn save_settings(&self, id: &str, settings: &Value) -> Result<bool> {
        let s = Settings::from_value(settings)?;
        self.state.put(id, "settings", settings.clone())?;
        self.feature(id, "settings", |dev| dev.set_settings(&s))
    }

    pub fn sync_clock(&self, id: &str, slot: &Value) -> Result<bool> {
        let slot = int(slot, "slot")?;
        if !(0..=255).contains(&slot) {
            return bad("slot must be between 0 and 255");
        }
        self.feature(id, "clock", |dev| dev.sync_clock(slot as u8))
    }

    pub fn upload_display(&self, id: &str, upload: &Upload, progress: Progress) -> Result<bool> {
        match upload {
            Upload::Matrix(frames, opts) => self.feature(id, "matrix", |dev| {
                dev.upload_matrix(frames, *opts, &mut *progress)
            }),
            Upload::Lcd(frames, slot) => self.feature(id, "lcd", |dev| {
                dev.upload_lcd(frames, *slot, &mut *progress)
            }),
        }
    }

    pub fn name_effect(&self, id: &str, effect_id: i64, name: &str) -> Result<()> {
        let known = self.def(id)?["lighting"]["effects"]
            .as_array()
            .is_some_and(|a| a.iter().any(|e| e["id"].as_i64() == Some(effect_id)));
        if !known {
            return Err(Error::NotFound("unknown effect".into()));
        }
        let mut names = self
            .state
            .get(id, "effect_names")
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        let name: String = name.trim().chars().take(40).collect();
        if name.is_empty() {
            names.remove(&effect_id.to_string());
        } else {
            names.insert(effect_id.to_string(), json!(name));
        }
        self.state.put(id, "effect_names", Value::Object(names))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_definitions_are_valid() {
        for f in BUILTIN
            .files()
            .filter(|f| f.path().extension().is_some_and(|e| e == "json"))
        {
            let d: Value = serde_json::from_str(f.contents_utf8().unwrap()).unwrap();
            check_definition(&d).unwrap_or_else(|e| panic!("{}: {e}", f.path().display()));
            assert_eq!(
                d["id"].as_str().unwrap(),
                f.path().file_stem().unwrap().to_str().unwrap(),
                "id must equal the file name"
            );
            let effects = d["lighting"]["effects"].as_array().unwrap();
            let mut ids: Vec<i64> = effects.iter().map(|e| e["id"].as_i64().unwrap()).collect();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), effects.len(), "effect ids must be unique");
            let controls = [
                "color",
                "rainbow",
                "brightness",
                "speed",
                "direction",
                "paint",
            ];
            for e in effects {
                assert!(
                    e["controls"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|c| controls.contains(&c.as_str().unwrap()))
                );
            }
            let keys: Vec<&Value> = d["layout"]["keys"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|k| k["extra"] != true)
                .collect();
            let mut idx: Vec<i64> = keys
                .iter()
                .map(|k| k["key_index"].as_i64().unwrap())
                .collect();
            idx.sort();
            idx.dedup();
            assert_eq!(idx.len(), keys.len(), "key_index must be unique");
            for (name, o) in d["originals"].as_object().into_iter().flatten() {
                assert!(
                    o["file"].is_string() && o["path"].is_string(),
                    "originals.{name} needs file and path"
                );
            }
        }
    }

    #[test]
    fn rejects_broken_definitions() {
        assert!(check_definition(&json!([])).is_err());
        assert!(check_definition(&json!({"id": "x"})).is_err());
        let mut d: Value = serde_json::from_str(
            BUILTIN
                .get_file("leobog-amg65.json")
                .unwrap()
                .contents_utf8()
                .unwrap(),
        )
        .unwrap();
        let base = d.clone();
        d["id"] = json!("../evil");
        assert!(check_definition(&d).is_err());
        let rejects = |path: &str, v: Value| {
            let mut d = base.clone();
            let mut target = &mut d;
            for k in path.split('.') {
                target = match k.parse::<usize>() {
                    Ok(i) => &mut target[i],
                    Err(_) => &mut target[k],
                };
            }
            *target = v;
            assert!(check_definition(&d).is_err(), "{path}");
        };
        rejects("id", json!("nul"));
        rejects("lighting.effects.0.id", json!(256));
        rejects("lighting.effects.0.id", json!("3"));
        rejects("originals.product.file", json!("../../state.json"));
        rejects("originals.product.file", json!("C:\\Windows\\win.ini"));
        rejects("originals.screen.path", json!("../../.ssh/id_rsa"));
        rejects("originals.screen.path", json!("/etc/passwd"));
        rejects("features.lcd.width", json!(0));
        rejects("features.lcd.max_frames", json!(1_000_000));
        rejects(
            "features.matrix",
            json!({"width": 1024, "height": 1024, "max_frames": 1000}),
        );
    }

    #[test]
    fn a_poisoned_slot_is_recovered_not_busy_forever() {
        let slot: Slot = Arc::new(Mutex::new(Some(Box::new(Demo {
            connection: json!({}),
        }))));
        let s = slot.clone();
        let _ = std::thread::spawn(move || {
            let _g = s.lock().unwrap();
            panic!("a command panicked while holding the keyboard");
        })
        .join();
        assert!(matches!(slot.try_lock(), Err(TryLockError::Poisoned(_))));
        let guard = recover_slot(&slot, slot.lock());
        assert!(guard.is_none(), "the handle in an unknown state is closed");
        drop(guard);
        assert!(slot.try_lock().is_ok());
    }
}
