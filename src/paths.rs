//! Where Halcyon keeps the user's own files: definitions, saved state and vendor originals.

use std::path::{Component, Path, PathBuf};

/// `$HALCYON_HOME` if set, else `~/.halcyon`. Read on every call so tests can point it elsewhere.
pub fn home() -> PathBuf {
    match std::env::var_os("HALCYON_HOME") {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".halcyon"),
    }
}

pub fn user_devices() -> PathBuf {
    home().join("devices")
}

pub fn state_file() -> PathBuf {
    home().join("state.json")
}

pub fn vendor(device_id: &str) -> PathBuf {
    home().join("vendor").join(device_id)
}

/// A file in the keyboard's vendor folder, or `None` when `name` (from a definition, so untrusted) is not a
/// plain file name: no folders, `..`, drive letters or absolute paths.
pub fn vendor_file(device_id: &str, name: &str) -> Option<PathBuf> {
    is_plain_name(name).then(|| vendor(device_id).join(name))
}

/// Windows refuses these as file names (with any extension), so they are rejected everywhere for portability.
const RESERVED: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// One path component that is safe as a file or folder name on every system.
pub fn is_plain_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").to_ascii_lowercase();
    !name.is_empty()
        && name.len() <= 128
        && !name.ends_with(['.', ' '])
        && !name.contains(['/', '\\', ':', '\0'])
        && !RESERVED.contains(&stem.trim_end())
        && matches!(
            Path::new(name).components().collect::<Vec<_>>().as_slice(),
            [Component::Normal(_)]
        )
}

/// A relative path that stays inside the folder it is joined to (for files inside the vendor software folder).
pub fn is_safe_relative(rel: &str) -> bool {
    !rel.is_empty()
        && !rel.contains(['\0', ':'])
        && !rel.starts_with(['/', '\\'])
        && Path::new(rel)
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
        && rel.split(['/', '\\']).all(|part| part != "..")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_names_only() {
        for ok in [
            "product.png",
            "ledlayer_v2.data",
            "leobog-amg65",
            "screen.gif",
        ] {
            assert!(is_plain_name(ok), "{ok}");
        }
        for bad in [
            "",
            ".",
            "..",
            "../x",
            "a/b",
            "a\\b",
            "/etc/passwd",
            "C:\\x",
            "C:x",
            "nul",
            "CON.txt",
            "Com1",
            "x.",
            "x ",
        ] {
            assert!(!is_plain_name(bad), "{bad}");
        }
        assert!(vendor_file("k", "../../state.json").is_none());
    }

    #[test]
    fn relative_paths_stay_inside() {
        assert!(is_safe_relative("db/ledlayer_v2.data"));
        assert!(is_safe_relative("gif/LEOBOG AMG65/0.gif"));
        for bad in [
            "",
            "../x",
            "db/../../x",
            "db\\..\\x",
            "/abs",
            "\\abs",
            "C:\\abs",
            "C:rel",
        ] {
            assert!(!is_safe_relative(bad), "{bad}");
        }
    }
}
