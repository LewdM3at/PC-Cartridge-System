//! Which cartridges this machine has opened before.
//!
//! The first time a cartridge meets a PC the launcher plays the unboxing, the
//! way a game used to come out of its shrink-wrap once. This is the record that
//! keeps it to once: a list of cartridge names beside `settings.json`, never on
//! the cartridge, because "new to this machine" is a fact about the machine.

use std::path::{Path, PathBuf};

fn path() -> PathBuf {
    crate::settings::settings_dir().join("unboxed.json")
}

/// True the first time `name` is asked about on this machine, and records it
/// so every later call is false. A record that cannot be read or written errs
/// towards not playing it again rather than playing it every time.
pub fn first_time(name: &str) -> bool {
    first_time_in(&path(), name)
}

fn first_time_in(path: &Path, name: &str) -> bool {
    let key = name.trim().to_lowercase();
    if key.is_empty() {
        return false;
    }
    let mut seen: Vec<String> = match std::fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str(&text) {
            Ok(list) => list,
            Err(_) => return false,
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(_) => return false,
    };
    if seen.contains(&key) {
        return false;
    }
    seen.push(key);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    serde_json::to_string_pretty(&seen)
        .ok()
        .is_some_and(|text| std::fs::write(path, text).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_first_time_is_the_first_time() {
        let scratch = crate::testutil::Scratch::new("unboxed");
        let path = scratch.join("state/unboxed.json");

        assert!(first_time_in(&path, "Hollow Knight"));
        assert!(!first_time_in(&path, "Hollow Knight"));
        // The same cartridge, spelled the way a rename might leave it.
        assert!(!first_time_in(&path, "  hollow knight "));
        assert!(first_time_in(&path, "Hades"));
        // Nothing to name, nothing to open.
        assert!(!first_time_in(&path, "   "));

        // A record that will not parse is not a reason to play it every time.
        std::fs::write(&path, "not json").unwrap();
        assert!(!first_time_in(&path, "Tunic"));
    }
}
