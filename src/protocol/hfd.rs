//! HFD protocol family: Sonix-based boards that ship the "HFD RGB Keyboard" vendor driver.
//!
//! Reverse engineered for compatibility; see docs/PROTOCOL.md for the full write-up.
//!
//!   * dongle: 32-byte frames, `[cmd...][checksum]`, the device echoes the first 3 bytes
//!   * wired:  64-byte frames in begin (04 18) / command / data / end (04 02) / commit (04 F0) transactions

use std::ffi::{CStr, CString};
use std::sync::Arc;
use std::thread::sleep;
use std::time::{Duration, Instant};

use chrono::{Datelike, Local, Timelike};
use hidapi::HidDevice;
use serde_json::{Value, json};

use super::{
    Action, Frame, Keymap, LcdFrame, LightingState, MacroEvent, MatrixOptions, OpenArgs, PerKey,
    Progress, Settings, Transport,
};
use crate::error::{Error, Result, bad};
use crate::hid::Hid;
use crate::json::usb_id;

const SIGNATURE: [u8; 2] = [0xAA, 0x55];

/// The 16-byte lighting block shared by both transports.
pub fn lighting_block(s: &LightingState) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[0] = s.effect;
    b[1..4].copy_from_slice(&s.color);
    b[8] = if s.rainbow { 0 } else { 1 };
    b[9] = s.brightness;
    b[10] = s.speed;
    b[11] = s.direction;
    b[14..16].copy_from_slice(&SIGNATURE);
    b
}

// ---- Per-key colours (wired cmd 0x23 / dongle frames 0x14) ----
// Entry for LED i sits at offset i*4 and is {i, R, G, B}. Lighting mode 0x80 shows the stored colours.
pub const PERKEY_MODE: u8 = 0x80;

pub fn perkey_table(colors: &PerKey, size: usize) -> Vec<u8> {
    let mut t = vec![0u8; size];
    for (&led, c) in colors {
        let o = led as usize * 4;
        if o < size.min(0x200) {
            t[o] = (led & 0xFF) as u8;
            t[o + 1..o + 4].copy_from_slice(c);
        }
    }
    t
}

pub fn perkey_block(brightness: u8) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[0] = PERKEY_MODE;
    b[8] = 1;
    b[9] = brightness;
    b[10] = 0x0F; // what the vendor app sends for speed and direction in this mode
    b[11] = 0x0F;
    b[14..16].copy_from_slice(&SIGNATURE);
    b
}

// ---- Keymap (wired cmd 0x11 / dongle frames 0x10) ----
// 128 entries of 4 bytes at key_index*4; AA 55 at 0x1FE. All zeros = every key back to its factory function.

/// The keyboard's 4-byte entry for an action.
pub fn encode_action(a: &Action) -> [u8; 4] {
    match *a {
        Action::Default => [0; 4],
        Action::Disabled => [0x05, 0x03, 0, 0],
        Action::Key { hid, mods } if mods == 0 && (0xE0..=0xE7).contains(&hid) => {
            [0x02, 1 << (hid - 0xE0), 0, 0]
        } // a modifier alone
        Action::Key { hid, mods } => [0x02, mods, hid, 0],
        Action::Consumer(u) => [0x03, (u & 0xFF) as u8, (u >> 8) as u8, 0],
        Action::Mouse(buttons) => [0x01, 0x01, buttons, 0],
        Action::Scroll(d) => [0x01, 0x03, if d > 0 { 0x01 } else { 0xFF }, 0],
        Action::Macro { slot, mode, count } => [0x06, slot, mode, count],
        Action::Multi([a, b, c]) => [0x07, a, b, c],
    }
}

pub fn keymap_table(keymap: &Keymap) -> Result<[u8; 0x200]> {
    let mut t = [0u8; 0x200];
    for (&i, action) in keymap {
        if i >= 127 {
            return bad(format!("key index {i} out of range"));
        }
        let o = i as usize * 4;
        t[o..o + 4].copy_from_slice(&encode_action(action));
    }
    t[0x1FE..].copy_from_slice(&SIGNATURE);
    Ok(t)
}

// ---- Keyboard settings (wired cmd 0x17 / dongle 07 10) ----

/// data[0..8] as the wired command lays it out; the dongle sends the same bytes 3 positions later.
pub fn settings_block(s: &Settings) -> [u8; 8] {
    let mut b = [0u8; 8];
    b[1] = s.game_mode as u8;
    if s.game_mode {
        // the vendor app only honours the lock options while game mode is on
        b[2] = s.disable_alt_tab as u8;
        b[3] = s.disable_alt_f4 as u8;
        b[4] = s.disable_win as u8;
    }
    b[5] = s.fn_switch as u8;
    b[6] = s.sleep;
    b[7] = 7; // the vendor default; its meaning is not yet known
    b
}

// ---- Displays (wired only: bulk data goes over a second HID interface in 4 KB reports) ----
pub const MATRIX_W: usize = 63;
pub const MATRIX_LEDS: usize = 315;
pub const LCD_MAX_FRAMES: usize = 130;

/// Logical pixel (row*63 + col) -> position in the device's LED chain: four 14x5 panels, then one 7x5.
pub fn matrix_dev_index(k: usize) -> usize {
    let (row, col) = (k / MATRIX_W, k % MATRIX_W);
    if col < 56 {
        (col / 14) * 70 + row * 14 + col % 14
    } else {
        280 + row * 7 + (col - 56)
    }
}

/// Stored animation: count, speed, frames in device order, AA 55.
pub fn matrix_image(frames: &[Frame], speed: u8) -> Vec<u8> {
    let n = frames.len();
    let mut buf = vec![0u8; 4 + n * 945 + 3];
    buf[0..2].copy_from_slice(&(n as u16).to_le_bytes());
    let interval = (0x66 - speed as i32).max(1) as u16; // frame interval; lower = faster
    buf[2..4].copy_from_slice(&interval.to_le_bytes());
    for (f, px) in frames.iter().enumerate() {
        for (k, rgb) in px.iter().take(MATRIX_LEDS).enumerate() {
            let o = 4 + 3 * (f * MATRIX_LEDS + matrix_dev_index(k));
            buf[o..o + 3].copy_from_slice(rgb);
        }
    }
    let end = 4 + 3 * MATRIX_LEDS * n;
    buf[end + 1..end + 3].copy_from_slice(&SIGNATURE); // the vendor leaves a one-byte gap before the signature
    buf
}

pub fn rgb565(frame: &LcdFrame) -> Vec<u8> {
    let mut out = Vec::with_capacity(frame.image.as_raw().len() / 3 * 2);
    for p in frame.image.pixels() {
        let [r, g, b] = p.0;
        let v = (r as u16 & 0xF8) << 8 | (g as u16 & 0xFC) << 3 | (b as u16) >> 3;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// LCD upload: 256-byte header (count, delays in 2 ms units, 0xFF padding), then RGB565 frames.
pub fn lcd_image(frames: &[LcdFrame]) -> Vec<u8> {
    let frames = &frames[..frames.len().min(LCD_MAX_FRAMES)];
    let mut out = vec![0xFFu8; 0x100];
    out[0] = frames.len() as u8;
    for (i, f) in frames.iter().enumerate() {
        out[1 + i] = (f.delay_ms / 2).clamp(1, 255) as u8;
    }
    for f in frames {
        out.extend(rgb565(f));
    }
    out
}

/// Time (and on wired, PC stats) for the screen's clock. `long` makes the 64-byte wired form.
pub fn clock_block(slot: u8, long: bool) -> Vec<u8> {
    let t = Local::now();
    let mut b = vec![0u8; if long { 64 } else { 16 }];
    b[1] = slot;
    b[2] = 0x5A;
    b[3] = (t.year() % 2000) as u8;
    b[4] = t.month() as u8;
    b[5] = t.day() as u8;
    b[6] = t.hour() as u8;
    b[7] = t.minute() as u8;
    b[8] = t.second() as u8;
    b[10] = t.weekday().num_days_from_sunday() as u8; // Windows SYSTEMTIME: 0 = Sunday
    let n = b.len();
    b[n - 2..].copy_from_slice(&SIGNATURE);
    b
}

// ---- Macros ----
// Play mode lives in the key binding, not here. Mouse bits: 1 = left, 2 = right, 4 = middle.
pub const MACRO_IMAGE_SIZE: usize = 0xE00;
pub const MACRO_SLOTS: usize = 100;

/// Build the 0xE00-byte macro image. Returns (image, used length).
pub fn encode_macros(macros: &[Vec<MacroEvent>]) -> Result<(Vec<u8>, usize)> {
    if macros.len() > MACRO_SLOTS {
        return bad(format!("at most {MACRO_SLOTS} macros"));
    }
    let mut img = vec![0u8; MACRO_IMAGE_SIZE];
    let mut off = MACRO_SLOTS * 4;
    for (slot, events) in macros.iter().enumerate() {
        if events.is_empty() {
            img[slot * 4..slot * 4 + 4].copy_from_slice(&[0xFF; 4]);
            continue;
        }
        img[slot * 4..slot * 4 + 2].copy_from_slice(&(off as u16).to_le_bytes());
        let mut body: Vec<[u8; 4]> = Vec::new();
        let mut prev_delay = true;
        for e in events {
            let (value, flag) = match *e {
                MacroEvent::DelayMs(ms) => {
                    body.push([(ms & 0xFF) as u8, (ms >> 8) as u8, 0, 0x50]);
                    prev_delay = true;
                    continue;
                }
                MacroEvent::Down(h) => (h, 0xB0),
                MacroEvent::Up(h) => (h, 0x30),
                MacroEvent::MouseDown(b) => (b, 0x90),
                MacroEvent::MouseUp(b) => (b, 0x10),
            };
            if !prev_delay {
                body.push([0x0A, 0, 0, 0x50]); // the vendor encoder puts 10 ms between back-to-back events
            }
            body.push([0, 0, value, flag]);
            prev_delay = false;
        }
        let end = off + 8 + 4 * body.len();
        if end > MACRO_IMAGE_SIZE - 2 {
            return bad("macros are too long to fit in the keyboard's memory");
        }
        img[off..off + 2].copy_from_slice(&(body.len() as u16).to_le_bytes());
        for (i, ev) in body.iter().enumerate() {
            img[off + 8 + i * 4..off + 12 + i * 4].copy_from_slice(ev);
        }
        off = end;
    }
    Ok((img, off))
}

fn chunk(data: &[u8], start: usize, len: usize) -> &[u8] {
    let s = start.min(data.len());
    &data[s..(start + len).min(data.len())]
}

fn hid_err(e: hidapi::HidError) -> Error {
    Error::Io(e.to_string())
}

/// The two calls the transports make on a HID handle; tests swap in a recorder.
pub trait Port: Send {
    fn write(&self, data: &[u8]) -> Result<usize>;
    fn read_timeout(&self, buf: &mut [u8], ms: i32) -> Result<usize>;
}

impl Port for HidDevice {
    fn write(&self, data: &[u8]) -> Result<usize> {
        HidDevice::write(self, data).map_err(hid_err)
    }
    fn read_timeout(&self, buf: &mut [u8], ms: i32) -> Result<usize> {
        HidDevice::read_timeout(self, buf, ms).map_err(hid_err)
    }
}

// ---- Dongle ----

pub struct Dongle {
    connection: Value,
    path: CString,
    dev: Box<dyn Port>,
}

impl Dongle {
    pub fn new(a: OpenArgs) -> Self {
        Dongle {
            connection: a.connection,
            path: a.path,
            dev: Box::new(a.device),
        }
    }

    /// Send one frame and wait for the keyboard to echo its first three bytes.
    fn transact(&self, payload: &[u8], timeout: Duration) -> Result<Option<Vec<u8>>> {
        let mut frame = [0u8; 33]; // report ID 0 + 31 payload bytes + checksum
        frame[1..1 + payload.len()].copy_from_slice(payload);
        frame[32] = frame[..32].iter().fold(0u8, |s, b| s.wrapping_add(*b));
        if self.dev.write(&frame)? == 0 {
            return Err(Error::Io("write to dongle failed".into()));
        }
        let deadline = Instant::now() + timeout;
        let mut buf = [0u8; 33];
        while Instant::now() < deadline {
            let n = self.dev.read_timeout(&mut buf, 50)?;
            if n >= 3 && buf[..3] == payload[..3] {
                return Ok(Some(buf[..n].to_vec()));
            }
        }
        Ok(None)
    }

    fn send(&self, payload: &[u8]) -> Result<bool> {
        Ok(self
            .transact(payload, Duration::from_millis(400))?
            .is_some())
    }

    /// Send a table as numbered 28-byte frames. True only if the keyboard echoed every one of them, so a
    /// save over a dropped link is reported instead of silently looking successful.
    fn frames(&self, cmd: u8, data: &[u8], count: usize, last_len: u8) -> Result<bool> {
        let mut all = true;
        for k in 0..count {
            let len = if k == count - 1 { last_len } else { 28 };
            let mut p = vec![cmd, len, (k & 0xFF) as u8];
            p.extend_from_slice(chunk(data, k * 28, 28));
            all &= self.transact(&p, Duration::from_millis(100))?.is_some();
            sleep(Duration::from_millis(2));
        }
        Ok(all)
    }
}

impl Transport for Dongle {
    fn connection(&self) -> &Value {
        &self.connection
    }

    fn path(&self) -> Option<&CStr> {
        Some(&self.path)
    }

    fn status(&mut self) -> Result<Value> {
        Ok(
            match self.transact(&[0x02, 0, 0], Duration::from_millis(400))? {
                Some(r) if r.len() >= 13 => json!({
                    "linked": r[12] == 1,
                    "firmware": format!("{:02X}{:02X}", r[4], r[3]), // inferred
                }),
                _ => json!({"linked": false}),
            },
        )
    }

    fn set_lighting(&mut self, s: &LightingState) -> Result<bool> {
        let mut p = vec![0x05, 0x10, 0x00];
        p.extend_from_slice(&lighting_block(s));
        self.send(&p)
    }

    fn set_perkey(&mut self, colors: &PerKey, brightness: u8) -> Result<bool> {
        // The dongle variant of the mode block: colour type, speed and direction are 0.
        let mut select = vec![0x05, 0x10, 0x00, PERKEY_MODE];
        select.extend_from_slice(&[0; 15]);
        select[3 + 9] = brightness;
        select[3 + 14..3 + 16].copy_from_slice(&SIGNATURE);
        if !self.send(&select)? {
            return Ok(false);
        }
        let mut table = perkey_table(colors, 0x30C);
        table[0x23E..0x240].copy_from_slice(&SIGNATURE);
        let all = self.frames(0x14, &table, 21, 0x10)?;
        // Verified on hardware: the colours only show once the mode is selected again after the upload.
        Ok(self.send(&select)? && all)
    }

    fn set_keymap(&mut self, keymap: &Keymap) -> Result<bool> {
        let table = keymap_table(keymap)?;
        self.frames(0x10, &table, 19, 0x08)
    }

    fn set_settings(&mut self, s: &Settings) -> Result<bool> {
        let mut p = [0u8; 19];
        p[0] = 0x07;
        p[1] = 0x10;
        p[3..11].copy_from_slice(&settings_block(s));
        p[4] = 1; // the vendor app always sends 1 here on the dongle
        p[10] = 0; // the dongle variant sends 0 where wired sends brightness
        p[11] = s.key_response;
        p[17..19].copy_from_slice(&SIGNATURE);
        self.send(&p)
    }

    fn sync_clock(&mut self, slot: u8) -> Result<bool> {
        let mut p = vec![0x0C, 0x10, 0x00];
        p.extend(clock_block(slot, false));
        self.send(&p)
    }

    fn upload_macros(&mut self, macros: &[Vec<MacroEvent>]) -> Result<bool> {
        let (mut img, total) = encode_macros(macros)?;
        img.extend_from_slice(&[0; 128]); // room for the AA 55 marker when the image is nearly full
        let (q, r) = (total / 28, total % 28);
        let frames = q + 1 + (r != 0) as usize;
        img[frames * 28 - 2..frames * 28].copy_from_slice(&SIGNATURE);
        self.frames(0x09, &img, frames, r as u8)
    }
}

// ---- Wired ----

pub struct Wired {
    connection: Value,
    path: CString,
    dev: Box<dyn Port>,
    hid: Arc<Hid>,
    bulk: Option<Box<dyn Port>>,
}

impl Wired {
    pub fn new(a: OpenArgs) -> Self {
        Wired {
            connection: a.connection,
            path: a.path,
            dev: Box::new(a.device),
            hid: a.hid,
            bulk: None,
        }
    }

    fn send(&self, payload: &[u8]) -> Result<()> {
        let mut frame = [0u8; 65];
        frame[1..1 + payload.len()].copy_from_slice(payload);
        if self.dev.write(&frame)? == 0 {
            return Err(Error::Io("write to keyboard failed".into()));
        }
        let mut reply = [0u8; 64];
        self.dev.read_timeout(&mut reply, 200)?;
        Ok(())
    }

    fn transaction(&self, cmd: u8, packets: &[&[u8]], commit: bool) -> Result<()> {
        let mut header = [0u8; 64];
        header[0] = 0x04;
        header[1] = cmd;
        header[8] = packets.len() as u8;
        self.send(&[0x04, 0x18])?;
        self.send(&header)?;
        for p in packets {
            self.send(p)?;
        }
        self.send(&[0x04, 0x02])?;
        if commit {
            self.send(&[0x04, 0xF0])?;
        }
        Ok(())
    }

    /// The second interface display data goes over, named by the definition's "bulk" entry.
    fn bulk(&mut self) -> Result<&dyn Port> {
        if self.bulk.is_none() {
            let spec = self.connection.get("bulk").ok_or_else(|| {
                Error::Device(
                    "this keyboard's definition has no bulk interface for display uploads".into(),
                )
            })?;
            let (vid, pid) = (
                usb_id(&self.connection["vid"], "vid")?,
                usb_id(&self.connection["pid"], "pid")?,
            );
            let (page, usage) = (
                usb_id(&spec["usage_page"], "usage_page")?,
                usb_id(&spec["usage"], "usage")?,
            );
            let found = self
                .hid
                .enumerate(vid, pid)?
                .into_iter()
                .find(|i| i.usage_page == page && i.usage == usage);
            let info = found.ok_or_else(|| {
                Error::Device(
                    "display interface not found; is the keyboard connected by cable?".into(),
                )
            })?;
            self.bulk = Some(Box::new(self.hid.open(&info.path)?));
        }
        Ok(self.bulk.as_deref().expect("opened above"))
    }

    fn big_upload(
        &mut self,
        header: &[u8],
        data: &[u8],
        pause_after_first: Duration,
        commit: bool,
        progress: Progress,
    ) -> Result<()> {
        let blocks = data.len().div_ceil(4096);
        self.bulk()?;
        let mut hdr = [0u8; 64];
        hdr[..header.len()].copy_from_slice(header);
        hdr[8..10].copy_from_slice(&(blocks as u16).to_le_bytes());
        self.send(&[0x04, 0x18])?;
        self.send(&hdr)?;
        let mut report = vec![0u8; 4097];
        let mut unconfirmed = 0;
        for i in 0..blocks {
            report[1..].fill(0);
            let part = chunk(data, i * 4096, 4096);
            report[1..1 + part.len()].copy_from_slice(part);
            let bulk = self.bulk.as_deref().expect("opened above");
            if bulk.write(&report)? == 0 {
                return Err(Error::Io("display upload failed".into()));
            }
            sleep(Duration::from_millis(5));
            let mut reply = [0u8; 64];
            let mut confirmed = false;
            for _ in 0..25 {
                // verified on hardware: replies look like "04 xx 00 01", OK flag in byte 3
                let n = self.dev.read_timeout(&mut reply, 200)?;
                if n > 3 && reply[3] == 1 {
                    confirmed = true;
                    break;
                }
            }
            // Give up rather than spend hours on an upload the keyboard has stopped taking (each silent block
            // costs 5 s). Not an Io error, so the registry doesn't restart the whole upload.
            unconfirmed = if confirmed { 0 } else { unconfirmed + 1 };
            if unconfirmed >= 3 {
                return Err(Error::Device("the keyboard stopped confirming the upload; unplug it, plug it back in and try again".into()));
            }
            if i == 0 && !pause_after_first.is_zero() {
                sleep(pause_after_first);
            }
            progress((i + 1) as f64 / blocks as f64);
        }
        self.send(&[0x04, 0x02])?;
        if commit {
            self.send(&[0x04, 0xF0])?;
        }
        Ok(())
    }
}

impl Transport for Wired {
    fn connection(&self) -> &Value {
        &self.connection
    }

    fn path(&self) -> Option<&CStr> {
        Some(&self.path)
    }

    fn status(&mut self) -> Result<Value> {
        Ok(json!({"linked": true}))
    }

    fn set_lighting(&mut self, s: &LightingState) -> Result<bool> {
        self.transaction(0x13, &[&lighting_block(s)], true)?;
        Ok(true)
    }

    fn set_keymap(&mut self, keymap: &Keymap) -> Result<bool> {
        let table = keymap_table(keymap)?;
        let packets: Vec<&[u8]> = table.chunks(64).collect();
        self.transaction(0x11, &packets, true)?;
        Ok(true)
    }

    fn set_perkey(&mut self, colors: &PerKey, brightness: u8) -> Result<bool> {
        let table = perkey_table(colors, 0x200);
        let packets: Vec<&[u8]> = table.chunks(64).collect();
        self.transaction(0x23, &packets, true)?;
        self.transaction(0x13, &[&perkey_block(brightness)], true)?;
        Ok(true)
    }

    fn set_settings(&mut self, s: &Settings) -> Result<bool> {
        let mut data = [0u8; 64];
        data[..8].copy_from_slice(&settings_block(s));
        data[62..].copy_from_slice(&SIGNATURE);
        let mut header = [0u8; 64];
        header[..3].copy_from_slice(&[0x04, 0x17, 0x01]); // [2] = profile, always 1 in the vendor app
        header[8] = 1;
        for p in [&[0x04u8, 0x18][..], &header, &data, &[0x04, 0x02]] {
            self.send(p)?;
        }
        Ok(true)
    }

    fn sync_clock(&mut self, slot: u8) -> Result<bool> {
        self.transaction(0x28, &[&clock_block(slot, true)], false)?;
        Ok(true)
    }

    fn upload_macros(&mut self, macros: &[Vec<MacroEvent>]) -> Result<bool> {
        let (mut img, total) = encode_macros(macros)?;
        img.extend_from_slice(&[0; 128]); // room for the AA 55 marker when the image is nearly full
        let nn = total / 64 + if total % 64 == 0 { 1 } else { 2 };
        img[nn * 64 - 2..nn * 64].copy_from_slice(&SIGNATURE);
        let mut header = [0u8; 64];
        header[0] = 0x04;
        header[1] = 0x15;
        header[8] = nn as u8;
        self.send(&[0x04, 0x19])?;
        self.send(&header)?;
        sleep(Duration::from_millis(30));
        for i in 0..=nn {
            // the vendor sends one trailing zero packet
            if i < nn {
                self.send(&img[i * 64..(i + 1) * 64])?
            } else {
                self.send(&[0u8; 64])?
            }
            sleep(Duration::from_millis(2));
        }
        sleep(Duration::from_millis(30));
        self.send(&[0x04, 0x02])?;
        Ok(true)
    }

    fn upload_matrix(
        &mut self,
        frames: &[Frame],
        opts: MatrixOptions,
        progress: Progress,
    ) -> Result<bool> {
        let frames = &frames[..frames.len().min(if opts.boot { 200 } else { 260 })];
        let scaled: Vec<Frame>;
        let frames = if opts.brightness < 100 {
            // the vendor app scales every colour by the layer's brightness
            let a = opts.brightness as u32 * 255 / 100;
            scaled = frames
                .iter()
                .map(|f| {
                    f.iter()
                        .map(|c| c.map(|v| (v as u32 * a / 255) as u8))
                        .collect()
                })
                .collect();
            &scaled[..]
        } else {
            frames
        };
        let cmd = if opts.boot { 0x31 } else { 0x33 };
        self.big_upload(
            &[0x04, cmd],
            &matrix_image(frames, opts.speed),
            Duration::ZERO,
            true,
            progress,
        )?;
        Ok(true)
    }

    fn upload_lcd(&mut self, frames: &[LcdFrame], slot: u8, progress: Progress) -> Result<bool> {
        self.big_upload(
            &[0x04, 0x72, slot],
            &lcd_image(frames),
            Duration::from_millis(500),
            false,
            progress,
        )?;
        Ok(true)
    }
}

#[cfg(test)]
mod parity {
    //! Every byte the transports write must equal what the original Python implementation wrote for the same
    //! commands (recorded in tests/golden/hfd_python.json), so behaviour proven on hardware carries over.
    use super::*;
    use std::sync::Mutex;

    type Log = Arc<Mutex<Vec<(String, String)>>>;

    struct Recorder {
        log: Log,
        name: &'static str,
        dongle: bool,
        last: Mutex<Vec<u8>>,
    }

    impl Port for Recorder {
        fn write(&self, data: &[u8]) -> Result<usize> {
            let hex: String = data.iter().map(|b| format!("{b:02x}")).collect();
            self.log.lock().unwrap().push((self.name.to_string(), hex));
            *self.last.lock().unwrap() = data.to_vec();
            Ok(data.len())
        }
        fn read_timeout(&self, buf: &mut [u8], _ms: i32) -> Result<usize> {
            let last = self.last.lock().unwrap().clone();
            buf.fill(0);
            if self.dongle {
                // echo the first three payload bytes, like the keyboard
                buf[..3].copy_from_slice(&last[1..4]);
                if last[1..4] == [2, 0, 0] {
                    (buf[3], buf[4], buf[11], buf[12]) = (0x40, 0x30, 51, 1);
                }
                Ok(33)
            } else {
                buf[..4].copy_from_slice(&[0x04, *last.get(2).unwrap_or(&0), 0, 1]);
                Ok(4)
            }
        }
    }

    fn port(log: &Log, name: &'static str, dongle: bool) -> Box<dyn Port> {
        Box::new(Recorder {
            log: log.clone(),
            name,
            dongle,
            last: Mutex::new(vec![]),
        })
    }

    fn transport(kind: &str) -> (Box<dyn Transport>, Log) {
        let log: Log = Arc::default();
        let connection = json!({"type": kind});
        let t: Box<dyn Transport> = if kind == "dongle" {
            Box::new(Dongle {
                connection,
                path: CString::default(),
                dev: port(&log, "main", true),
            })
        } else {
            Box::new(Wired {
                connection,
                path: CString::default(),
                dev: port(&log, "main", false),
                hid: Arc::default(),
                bulk: Some(port(&log, "bulk", false)),
            })
        };
        (t, log)
    }

    #[test]
    fn wire_bytes_match_python() {
        let golden: Value =
            serde_json::from_str(include_str!("../../tests/golden/hfd_python.json")).unwrap();
        let frame_a: Frame = (0..315i32)
            .map(|i| {
                [
                    ((i * 7) % 256) as u8,
                    ((i * 3) % 256) as u8,
                    ((255 - i).rem_euclid(256)) as u8,
                ]
            })
            .collect();
        let frame_b: Frame = (0..315)
            .map(|i| [((i * 11) % 256) as u8, 0, (i % 256) as u8])
            .collect();
        let lcd = vec![
            LcdFrame {
                image: image::RgbImage::from_pixel(240, 135, image::Rgb([200, 30, 90])),
                delay_ms: 60,
            },
            LcdFrame {
                image: image::RgbImage::from_pixel(240, 135, image::Rgb([5, 250, 17])),
                delay_ms: 33,
            },
        ];
        let keymap = Keymap::from([
            (0, Action::Key { hid: 0x29, mods: 0 }),
            (48, Action::Disabled),
            (49, Action::Key { hid: 6, mods: 1 }),
            (60, Action::Consumer(0x223)),
            (70, Action::Mouse(4)),
            (71, Action::Scroll(1)),
            (
                72,
                Action::Macro {
                    slot: 1,
                    mode: 1,
                    count: 3,
                },
            ),
        ]);
        use MacroEvent::*;
        let macros = vec![
            vec![Down(4), DelayMs(50), Up(4)],
            vec![],
            vec![
                Down(0xE0),
                Down(6),
                Up(6),
                Up(0xE0),
                MouseDown(1),
                MouseUp(1),
            ],
        ];
        let settings = Settings {
            game_mode: true,
            disable_win: true,
            disable_alt_f4: true,
            fn_switch: true,
            sleep: 2,
            key_response: 3,
            ..Default::default()
        };
        let light = LightingState {
            effect: 11,
            color: [10, 20, 30],
            rainbow: false,
            brightness: 4,
            speed: 2,
            direction: 3,
        };
        let perkey = PerKey::from([(0, [255, 0, 0]), (17, [0, 255, 0]), (108, [1, 2, 3])]);

        let mut checked = 0;
        for (name, steps) in golden.as_object().unwrap() {
            let (kind, scenario) = name.split_once('/').unwrap();
            let (mut t, log) = transport(kind);
            let mut nothing = |_: f64| {};
            match scenario {
                "lighting" => drop(t.set_lighting(&light).unwrap()),
                "perkey" => drop(t.set_perkey(&perkey, 5).unwrap()),
                "keymap" => drop(t.set_keymap(&keymap).unwrap()),
                "settings" => drop(t.set_settings(&settings).unwrap()),
                "macros" => drop(t.upload_macros(&macros).unwrap()),
                "status" => drop(t.status().unwrap()),
                "matrix" => drop(
                    t.upload_matrix(
                        &[frame_a.clone(), frame_b.clone()],
                        MatrixOptions {
                            speed: 30,
                            boot: false,
                            brightness: 40,
                        },
                        &mut nothing,
                    )
                    .unwrap(),
                ),
                "matrix_boot" => drop(
                    t.upload_matrix(
                        std::slice::from_ref(&frame_a),
                        MatrixOptions {
                            speed: 10,
                            boot: true,
                            brightness: 100,
                        },
                        &mut nothing,
                    )
                    .unwrap(),
                ),
                "lcd" => drop(t.upload_lcd(&lcd, 1, &mut nothing).unwrap()),
                other => panic!("unknown scenario {other}"),
            }
            let got = log.lock().unwrap().clone();
            let want: Vec<(String, String)> = steps
                .as_array()
                .unwrap()
                .iter()
                .map(|s| (s[0].as_str().unwrap().into(), s[1].as_str().unwrap().into()))
                .collect();
            assert_eq!(got.len(), want.len(), "{name}: number of writes");
            for (i, (g, w)) in got.iter().zip(&want).enumerate() {
                assert_eq!(g, w, "{name}: write {i} differs");
            }
            checked += 1;
        }
        assert_eq!(checked, 15);
    }
}

#[cfg(test)]
mod tests {
    //! Byte-level checks against the layouts documented in docs/protocol/.
    use super::*;
    use serde_json::json;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    fn action(v: Value) -> [u8; 4] {
        encode_action(&Action::from_value(&v).unwrap())
    }

    #[test]
    fn lighting_block_layout() {
        let b = lighting_block(&LightingState {
            effect: 7,
            color: [1, 2, 3],
            rainbow: false,
            brightness: 4,
            speed: 2,
            direction: 1,
        });
        assert_eq!(b[0], 7);
        assert_eq!(&b[1..4], &[1, 2, 3]);
        assert_eq!((b[8], b[9], b[10], b[11]), (1, 4, 2, 1));
        assert_eq!(&b[14..16], &SIGNATURE);
        assert_eq!(
            lighting_block(&LightingState {
                rainbow: true,
                ..Default::default()
            })[8],
            0
        );
    }

    #[test]
    fn macro_image_matches_documented_example() {
        // docs/protocol/macros.md section 6: press A, 50 ms, release A
        let (img, total) = encode_macros(&[vec![
            MacroEvent::Down(4),
            MacroEvent::DelayMs(50),
            MacroEvent::Up(4),
        ]])
        .unwrap();
        assert_eq!((total, img.len()), (0x1A4, 0xE00));
        assert_eq!(&img[0..4], &hex("90010000")[..]);
        assert_eq!(
            &img[0x190..0x1A4],
            &hex("0300000000000000000004b03200005000000430")[..]
        );
    }

    #[test]
    fn macro_inserts_gap_between_back_to_back_events() {
        let (img, _) = encode_macros(&[vec![MacroEvent::Down(4), MacroEvent::Up(4)]]).unwrap();
        assert_eq!(&img[0x198..0x1A4], &hex("000004b00a00005000000430")[..]);
    }

    #[test]
    fn macro_empty_slot_and_limits() {
        let (img, _) = encode_macros(&[vec![], vec![MacroEvent::Down(5)]]).unwrap();
        assert_eq!(&img[0..4], &[0xFF; 4]);
        assert_eq!(&img[4..6], &0x190u16.to_le_bytes());
        assert!(encode_macros(&[vec![MacroEvent::Down(4); 2000]]).is_err());
        assert!(encode_macros(&vec![vec![]; 101]).is_err());
    }

    #[test]
    fn key_actions() {
        assert_eq!(action(json!(null)), [0; 4]);
        assert_eq!(action(json!({"type": "default"})), [0; 4]);
        assert_eq!(action(json!({"type": "disabled"}))[..], hex("05030000")[..]);
        assert_eq!(
            action(json!({"type": "key", "hid": 5}))[..],
            hex("02000500")[..]
        ); // A -> B
        assert_eq!(
            action(json!({"type": "key", "hid": 6, "mods": 1}))[..],
            hex("02010600")[..]
        ); // Ctrl+C
        assert_eq!(
            action(json!({"type": "key", "hid": 0xE0}))[..],
            hex("02010000")[..]
        ); // a modifier alone
        assert_eq!(
            action(json!({"type": "consumer", "usage": 0xE9}))[..],
            hex("03e90000")[..]
        ); // volume up
        assert_eq!(
            action(json!({"type": "consumer", "usage": 0x223}))[..],
            hex("03230200")[..]
        );
        assert_eq!(
            action(json!({"type": "mouse", "buttons": 1}))[..],
            hex("01010100")[..]
        );
        assert_eq!(
            action(json!({"type": "scroll", "delta": -1}))[..],
            hex("0103ff00")[..]
        );
        assert_eq!(
            action(json!({"type": "macro", "slot": 2, "mode": 1, "count": 5}))[..],
            hex("06020105")[..]
        );
        assert_eq!(
            action(json!({"type": "macro", "slot": 2, "mode": 0, "count": 5}))[..],
            hex("06020000")[..]
        );
        assert!(Action::from_value(&json!({"type": "teleport"})).is_err());
    }

    #[test]
    fn keymap_table_layout() {
        let t = keymap_table(&Keymap::from([(49, Action::Key { hid: 5, mods: 0 })])).unwrap();
        assert_eq!(&t[0xC4..0xC8], &hex("02000500")[..]);
        assert_eq!(&t[0x1FE..], &SIGNATURE);
        assert!(
            keymap_table(&Keymap::new()).unwrap()[..0x1FE]
                .iter()
                .all(|&b| b == 0)
        ); // all zeros = factory defaults
        assert!(keymap_table(&Keymap::from([(127, Action::Disabled)])).is_err());
    }

    #[test]
    fn perkey_table_layout() {
        let t = perkey_table(&PerKey::from([(17, [255, 0, 10])]), 0x200);
        assert_eq!(&t[68..72], &[17, 255, 0, 10]);
        assert_eq!(t.iter().map(|&b| b as u32).sum::<u32>(), 17 + 255 + 10);
    }

    #[test]
    fn settings_block_locks_need_game_mode() {
        let off = settings_block(&Settings {
            game_mode: false,
            disable_win: true,
            sleep: 2,
            ..Default::default()
        });
        assert_eq!(&off[1..5], &[0; 4]);
        assert_eq!(off[6], 2);
        let on = settings_block(&Settings {
            game_mode: true,
            disable_win: true,
            disable_alt_tab: true,
            ..Default::default()
        });
        assert_eq!((on[1], on[2], on[3], on[4]), (1, 1, 0, 1));
        assert_eq!(Settings::from_value(&json!({"sleep": 9})).unwrap().sleep, 0);
    }

    #[test]
    fn matrix_device_order_is_a_permutation() {
        let mut idx: Vec<usize> = (0..MATRIX_LEDS).map(matrix_dev_index).collect();
        idx.sort();
        assert_eq!(idx, (0..MATRIX_LEDS).collect::<Vec<_>>());
        assert_eq!(
            (
                matrix_dev_index(0),
                matrix_dev_index(14),
                matrix_dev_index(56)
            ),
            (0, 70, 280)
        );
    }

    #[test]
    fn matrix_image_layout() {
        let frame: Frame = (0..MATRIX_LEDS).map(|i| [(i % 256) as u8, 0, 0]).collect();
        let img = matrix_image(&[frame.clone(), frame], 10);
        assert_eq!(&img[0..2], &2u16.to_le_bytes());
        assert_eq!(&img[2..4], &(0x66u16 - 10).to_le_bytes());
        assert_eq!(img.len(), 4 + 2 * 945 + 3);
        assert_eq!(&img[img.len() - 2..], &SIGNATURE);
        assert_eq!(img[4 + 3 * matrix_dev_index(20)], 20);
        assert_eq!(&matrix_image(&[], 255)[2..4], &1u16.to_le_bytes()); // interval never drops below 1
    }

    #[test]
    fn lcd_image_layout() {
        let f = LcdFrame {
            image: image::RgbImage::from_pixel(240, 135, image::Rgb([255, 0, 0])),
            delay_ms: 40,
        };
        let img = lcd_image(&[f.clone(), f]);
        assert_eq!((img[0], img[1], img[2], img[3]), (2, 20, 20, 0xFF));
        assert_eq!(img.len(), 0x100 + 2 * 240 * 135 * 2);
        assert_eq!(&img[0x100..0x102], &0xF800u16.to_le_bytes()); // pure red in RGB565
    }

    #[test]
    fn clock_block_layout() {
        let b = clock_block(1, false);
        assert_eq!((b.len(), b[1], b[2]), (16, 1, 0x5A));
        assert_eq!(&b[14..16], &SIGNATURE);
        assert!((1..=12).contains(&b[4]) && b[10] <= 6);
        assert_eq!(clock_block(1, true).len(), 64);
    }
}
