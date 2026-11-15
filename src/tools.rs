//! Command-line helpers for definition authors and owners of vendor software.

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::paths;
use crate::registry::load_definitions;

/// `halcyon import-layout KeyboardLayout.xml`: convert an HFD vendor driver's layout file into the
/// "layout" block of a Halcyon definition, printed as JSON.
pub fn import_layout(xml_path: &Path) -> Result<Value> {
    let text = fs::read(xml_path)?;
    let text = String::from_utf8_lossy(&text);
    let keys: Vec<Vec<(String, String)>> = elements(&text, "key");
    if keys.is_empty() {
        return Err(Error::Bad(
            "no <key> elements found; is this a KeyboardLayout.xml?".into(),
        ));
    }
    let num = |k: &[(String, String)], name: &str| -> Result<i64> {
        let v = k
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.trim())
            .ok_or_else(|| Error::Bad(format!("a key has no {name}")))?;
        let parsed = match v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")) {
            Some(h) => i64::from_str_radix(h, 16),
            None => v.parse(),
        };
        parsed.map_err(|_| Error::Bad(format!("{name}=\"{v}\" is not a number")))
    };
    // The vendor image uses a 50 px key pitch with a 34 px keycap; derive key units from that.
    let pitch = 50.0;
    let snap = |v: f64| (v * 4.0).round() / 4.0;
    let left0 = keys
        .iter()
        .map(|k| num(k, "rect_left"))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .min()
        .unwrap_or(0);
    let top0 = keys
        .iter()
        .map(|k| num(k, "rect_top"))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .filter(|&t| t > 100)
        .min()
        .unwrap_or(0);
    let mut out = Vec::new();
    for k in &keys {
        let (x, y, w) = (
            num(k, "rect_left")?,
            num(k, "rect_top")?,
            num(k, "rect_width")?,
        );
        let name = k
            .iter()
            .find(|(n, _)| n == "name")
            .map(|(_, v)| v.as_str())
            .unwrap_or("");
        let mut entry = json!({
            "label": label(name),
            "x": snap((x - left0) as f64 / pitch),
            "y": snap((y - top0) as f64 / pitch),
            "w": snap((w + 16) as f64 / pitch),
            "code": num(k, "code")?,
            "key_index": num(k, "key_index")?,
            "led": num(k, "light_index")?,
        });
        if y < top0 {
            entry["extra"] = json!(true); // controls above the key block, e.g. a volume knob
        }
        out.push(entry);
    }
    let main: Vec<&Value> = out.iter().filter(|k| k["extra"] != true).collect();
    let width = main
        .iter()
        .map(|k| k["x"].as_f64().unwrap_or(0.0) + k["w"].as_f64().unwrap_or(0.0))
        .fold(0.0, f64::max);
    let height = main
        .iter()
        .map(|k| k["y"].as_f64().unwrap_or(0.0))
        .fold(0.0, f64::max)
        + 1.0;
    Ok(json!({"width": width, "height": height, "keys": out}))
}

/// Two-character legends like "!1" show the unshifted symbol; long names get short labels.
fn label(name: &str) -> String {
    let short = match name {
        "CapsLock" => "Caps",
        "Shift_L" | "Shift_R" => "Shift",
        "Ctrl_L" | "Ctrl_R" => "Ctrl",
        "Win_L" => "Win",
        "Alt_L" | "Alt_R" => "Alt",
        "del" => "Del",
        "page up" => "PgUp",
        "page down" => "PgDn",
        "VOLINC" => "Vol+",
        "VOLDEC" => "Vol-",
        _ => "",
    };
    if !short.is_empty() {
        return short.to_string();
    }
    let chars: Vec<char> = name.chars().collect();
    if chars.len() == 2 && !chars.iter().all(|c| c.is_alphabetic()) {
        chars[1].to_string()
    } else {
        name.to_string()
    }
}

/// Attributes of every `<tag ...>` element. Enough XML for vendor layout files: no nesting is needed.
fn elements(text: &str, tag: &str) -> Vec<Vec<(String, String)>> {
    let open = format!("<{tag}");
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find(&open) {
        rest = &rest[i + open.len()..];
        if !rest.starts_with(|c: char| c.is_whitespace() || c == '/' || c == '>') {
            continue;
        }
        let end = rest.find('>').unwrap_or(rest.len());
        let mut attrs = Vec::new();
        let mut s = &rest[..end];
        while let Some(eq) = s.find('=') {
            let name = s[..eq].trim().to_string();
            let after = s[eq + 1..].trim_start();
            let Some(q) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else {
                break;
            };
            let Some(close) = after[1..].find(q) else {
                break;
            };
            attrs.push((name, unescape(&after[1..1 + close])));
            s = &after[close + 2..];
        }
        out.push(attrs);
        rest = &rest[end..];
    }
    out
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// `halcyon import-originals <keyboard id> <vendor app folder>`: copy the files a definition's "originals"
/// block names from the user's own vendor software into Halcyon's private folder. Returns (copied, missing).
pub fn import_originals(device_id: &str, vendor_dir: &Path) -> Result<(Vec<String>, Vec<String>)> {
    let defs = load_definitions();
    let d = defs.map.get(device_id).ok_or_else(|| {
        Error::NotFound(format!(
            "unknown keyboard '{device_id}' (known: {})",
            defs.map.keys().cloned().collect::<Vec<_>>().join(", ")
        ))
    })?;
    let dest = paths::vendor(device_id);
    fs::create_dir_all(&dest)?;
    let (mut copied, mut missing) = (Vec::new(), Vec::new());
    for (name, entry) in d["originals"].as_object().into_iter().flatten() {
        let (Some(rel), Some(file)) = (entry["path"].as_str(), entry["file"].as_str()) else {
            continue;
        };
        // Definitions are untrusted: read only inside the vendor folder, write only into Halcyon's own.
        let (true, Some(target)) = (
            paths::is_safe_relative(rel),
            paths::vendor_file(device_id, file),
        ) else {
            return Err(Error::Bad(format!(
                "originals.{name} names a file outside the vendor folder; check the definition"
            )));
        };
        let src = vendor_dir.join(rel);
        if src.is_file() {
            fs::copy(&src, target)?;
            copied.push(format!("{name}: {rel}"));
        } else {
            missing.push(format!("{name}: {}", src.display()));
        }
    }
    Ok((copied, missing))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vendor_layout() {
        let dir = std::env::temp_dir().join(format!("halcyon-layout-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("KeyboardLayout.xml");
        fs::write(&f, "2<?xml version=\"1.0\"?><KeyboardLayout><KeyItems>\
            <key code=\"0x29\" name=\"Esc\" key_index=\"0\" light_index=\"0\" rect_left=\"39\" rect_top=\"122\" rect_width=\"34\"/>\
            <key code=\"0x1e\" name=\"!1\" key_index=\"17\" light_index=\"17\" rect_left=\"89\" rect_top=\"122\" rect_width=\"34\"/>\
            <key code=\"0x2a\" name=\"&amp;7\" key_index=\"92\" light_index=\"92\" rect_left=\"139\" rect_top=\"171\" rect_width=\"83\"/>\
            <key code=\"0xe9\" name=\"VOLINC\" key_index=\"13\" light_index=\"13\" rect_left=\"922\" rect_top=\"28\" rect_width=\"38\"/>\
            </KeyItems></KeyboardLayout>").unwrap();
        let l = import_layout(&f).unwrap();
        let keys = l["keys"].as_array().unwrap();
        assert_eq!(keys.len(), 4);
        assert_eq!(
            (
                keys[0]["label"].as_str(),
                keys[0]["code"].as_i64(),
                keys[0]["x"].as_f64()
            ),
            (Some("Esc"), Some(0x29), Some(0.0))
        );
        assert_eq!(
            (keys[1]["label"].as_str(), keys[1]["x"].as_f64()),
            (Some("1"), Some(1.0))
        );
        assert_eq!(
            (
                keys[2]["label"].as_str(),
                keys[2]["y"].as_f64(),
                keys[2]["w"].as_f64()
            ),
            (Some("7"), Some(1.0), Some(2.0))
        );
        assert_eq!(keys[3]["extra"], json!(true));
        let _ = fs::remove_dir_all(dir);
    }
}
