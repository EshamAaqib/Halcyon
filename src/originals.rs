//! The keyboard's original content, read from the user's own copy of the vendor software.
//!
//! Nothing here ships with Halcyon. A definition's "originals" block names files the vendor software contains;
//! Halcyon looks for them in `~/.halcyon/vendor/<device id>/` (see `halcyon import-originals`).

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::media::box_resize;
use crate::paths;
use crate::protocol::Frame;

/// Upper bounds for what a vendor library or a definition can ask to allocate (the AMG65 uses 315 LEDs and
/// at most a few hundred frames).
const MAX_FRAMES: usize = 1000;
const MAX_PIXELS: usize = 16 * 1024;

fn open_db(path: &Path) -> Result<Connection> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| unreadable(path, e))
}

fn unreadable(path: &Path, e: impl std::fmt::Display) -> Error {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Error::Internal(format!(
        "{name} is not a vendor LED library Halcyon can read ({e})"
    ))
}

fn colorref(v: i64) -> [u8; 3] {
    [
        (v & 0xFF) as u8,
        ((v >> 8) & 0xFF) as u8,
        ((v >> 16) & 0xFF) as u8,
    ] // stored as Windows COLORREF 0x00BBGGRR
}

/// (frame index, LED index, colour) rows of one library layer, in order.
fn layer_rows(db: &Path, layer: i64) -> Result<Vec<(i64, i64, i64)>> {
    let con = open_db(db)?;
    let mut stmt = con
        .prepare("SELECT frameIndex, led_index, rgb_value FROM t_default_led_frames WHERE layer_id = ? ORDER BY frameIndex, led_index")
        .map_err(|e| unreadable(db, e))?;
    let rows = stmt
        .query_map([layer], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(|e| unreadable(db, e))?;
    rows.collect::<std::result::Result<_, _>>()
        .map_err(|e| unreadable(db, e))
}

pub struct BootAnimation {
    pub frames: Vec<Frame>,
    pub speed: u8,
    pub brightness: u8,
}

/// The factory boot animation, or `None` when the vendor file is not on this computer.
pub fn boot_animation(device_id: &str, spec: &Value) -> Result<Option<BootAnimation>> {
    let Some(db) = file_in_vendor_folder(device_id, spec) else {
        return Ok(None);
    };
    let layer = spec["layer"].as_i64().unwrap_or(1);
    let (speed, brightness): (i64, i64) = open_db(&db)?
        .query_row(
            "SELECT speed, brightness FROM t_ledlayer_data WHERE led_id = ?",
            [layer],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap_or((10, 10));
    let pixels = spec["pixels"]
        .as_u64()
        .unwrap_or(315)
        .min(MAX_PIXELS as u64) as usize;
    let rows = layer_rows(&db, layer)?;
    let count = rows
        .iter()
        .map(|r| r.0)
        .max()
        .map(|m| m.saturating_add(1))
        .unwrap_or(0)
        .clamp(0, MAX_FRAMES as i64) as usize; // a damaged file must not ask for millions of frames
    let mut frames = vec![vec![[0u8; 3]; pixels]; count];
    for (f, k, v) in rows {
        if (0..count as i64).contains(&f) && (0..pixels as i64).contains(&k) {
            frames[f as usize][k as usize] = colorref(v);
        }
    }
    Ok(Some(BootAnimation {
        frames,
        speed: speed.clamp(0, 255) as u8,
        brightness: brightness.clamp(0, 100) as u8,
    }))
}

/// Animations in the vendor LED library that have frames: `[{"layer", "name", "frames", "width", "height"}]`.
pub fn library(device_id: &str, spec: &Value) -> Result<Vec<Value>> {
    let Some(db) = file_in_vendor_folder(device_id, spec) else {
        return Ok(vec![]);
    };
    let con = open_db(&db)?;
    let mut stmt = con
        .prepare("SELECT layer_id, MAX(frameIndex) + 1, COUNT(*) FROM t_default_led_frames GROUP BY layer_id")
        .map_err(|e| unreadable(&db, e))?;
    let rows: Vec<(i64, i64, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(|e| unreadable(&db, e))?
        .collect::<std::result::Result<_, _>>()
        .map_err(|e| unreadable(&db, e))?;
    let sizes: Vec<(i64, i64)> = spec["sizes"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| Some((s[0].as_i64()?, s[1].as_i64()?)))
                .collect()
        })
        .unwrap_or_else(|| vec![(63, 5), (70, 7)]);
    Ok(rows
        .into_iter()
        .filter_map(|(layer, frames, count)| {
            let name = spec["names"][layer.to_string()].as_str()?;
            let &(w, h) = sizes.iter().find(|&&(w, h)| {
                (1..=MAX_FRAMES as i64).contains(&frames)
                    && w > 0
                    && h > 0
                    && w.checked_mul(h)
                        .is_some_and(|n| n <= MAX_PIXELS as i64 && n == count / frames)
            })?;
            Some(json!({"layer": layer, "name": name, "frames": frames, "width": w, "height": h}))
        })
        .collect())
}

/// One library animation, scaled to the keyboard's matrix when it was made for a different size.
pub fn library_frames(
    device_id: &str,
    spec: &Value,
    layer: i64,
    width: usize,
    height: usize,
) -> Result<Option<Vec<Frame>>> {
    let Some(entry) = library(device_id, spec)?
        .into_iter()
        .find(|e| e["layer"].as_i64() == Some(layer))
    else {
        return Ok(None);
    };
    let (sw, sh) = (
        entry["width"].as_u64().unwrap_or(0) as usize,
        entry["height"].as_u64().unwrap_or(0) as usize,
    );
    let count = entry["frames"].as_u64().unwrap_or(0) as usize;
    let Some(db) = file_in_vendor_folder(device_id, spec) else {
        return Ok(None);
    };
    let mut raw = vec![vec![[0u8; 3]; sw * sh]; count];
    for (f, k, v) in layer_rows(&db, layer)? {
        if (0..count as i64).contains(&f) && (0..(sw * sh) as i64).contains(&k) {
            raw[f as usize][k as usize] = colorref(v);
        }
    }
    Ok(Some(
        raw.into_iter()
            .map(|f| {
                if (sw, sh) == (width, height) {
                    f
                } else {
                    box_resize(&f, sw, sh, width, height)
                }
            })
            .collect(),
    ))
}

/// The stock screen animation file, if present.
pub fn screen_file(device_id: &str, spec: &Value) -> Option<Vec<u8>> {
    fs::read(file_in_vendor_folder(device_id, spec)?).ok()
}

/// The existing file a definition's originals entry names. Definitions are untrusted, so the name must be a
/// plain file name inside the keyboard's vendor folder.
fn file_in_vendor_folder(device_id: &str, spec: &Value) -> Option<PathBuf> {
    paths::vendor_file(device_id, spec["file"].as_str()?).filter(|p| p.is_file())
}
