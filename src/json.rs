//! Lenient readers for JSON the UI or a definition file sends.

use serde_json::Value;

use crate::error::{Error, Result};

/// An integer from a JSON number (fractions are cut off) or a numeric string.
pub fn int(v: &Value, what: &str) -> Result<i64> {
    let n = match v {
        Value::Number(n) => n.as_i64().or_else(|| {
            n.as_f64()
                .filter(|f| f.is_finite() && f.abs() < 9e15)
                .map(|f| f.trunc() as i64)
        }),
        Value::String(s) => s.trim().parse::<i64>().ok().or_else(|| {
            let t = s.trim();
            t.strip_prefix("0x")
                .or_else(|| t.strip_prefix("0X"))
                .and_then(|h| i64::from_str_radix(h, 16).ok())
        }),
        Value::Bool(b) => Some(*b as i64),
        _ => None,
    };
    n.ok_or_else(|| Error::Bad(format!("{what} must be a number")))
}

/// JavaScript-style truthiness: false, 0, "", null and missing are false.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// A USB id written as "0x0C45" (hex) or as a number.
pub fn usb_id(v: &Value, what: &str) -> Result<u16> {
    let n = match v {
        Value::String(s) => {
            let t = s.trim();
            let h = t
                .strip_prefix("0x")
                .or_else(|| t.strip_prefix("0X"))
                .unwrap_or(t);
            i64::from_str_radix(h, 16).ok()
        }
        Value::Number(n) => n.as_i64(),
        _ => None,
    };
    n.filter(|n| (0..=0xFFFF).contains(n))
        .map(|n| n as u16)
        .ok_or_else(|| Error::Bad(format!("{what} must be a 16-bit id like \"0x0C45\"")))
}
