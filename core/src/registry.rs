//! Saves that live in the Windows registry.
//!
//! Unity's `PlayerPrefs` writes to `HKEY_CURRENT_USER\Software\<company>\<game>`,
//! and plenty of games keep their whole save there — Bluey: The Videogame among
//! them. [`crate::saves`] only knows how to carry folders, so a registry save is
//! turned into one: the key is exported to a `.reg` file in a folder beside
//! `settings.json`, that folder is the slot's host side, and everything else —
//! which side changed, backups, conflicts — is the folder logic unchanged.
//!
//! ```text
//! save.windows=Bluey|{registry}/HKCU/Software/Outright Games Ltd/Bluey The Videogame
//! ```
//!
//! The export is only rewritten when the registry actually changed, so the
//! file's modification time means what it means for any other save. A pull
//! imports the file back — after checking that every key in it is inside the
//! one declared, because the file came off a drive and `reg import` will write
//! anywhere it is told to.

use std::path::{Path, PathBuf};

/// The file a key is exported to, inside its folder.
#[cfg_attr(not(windows), allow(dead_code))]
const FILE: &str = "key.reg";

/// The folder `{registry}` stands for.
pub fn staging_root() -> PathBuf {
    crate::settings::settings_dir().join("registry")
}

/// The registry key a staged folder stands for: `HKCU\Software\…`, or `None`
/// for a folder that is not under [`staging_root`].
pub fn key_for(staged: &Path) -> Option<String> {
    let rest = staged.strip_prefix(staging_root()).ok()?;
    let parts: Vec<String> = rest
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    Some(parts.join("\\"))
}

/// Whether a `{registry}` template names something this is willing to touch:
/// one key inside the current user's `Software`, never a whole vendor's or
/// another hive.
pub fn allowed(parts: &[&str]) -> bool {
    parts.len() >= 3
        && parts[0].eq_ignore_ascii_case("HKCU")
        && parts[1].eq_ignore_ascii_case("Software")
}

/// Bring the staged export up to date with the registry. Leaves the file
/// untouched when nothing changed, and removes it when the key is gone.
#[cfg(windows)]
pub fn stage(staged: &Path) -> Result<(), String> {
    let Some(key) = key_for(staged) else {
        return Ok(());
    };
    let file = staged.join(FILE);
    let scratch = staging_root().join(".export.reg");
    std::fs::create_dir_all(staging_root()).map_err(|e| e.to_string())?;
    let exported = crate::proc::command("reg")
        .args(["export", &key])
        .arg(&scratch)
        .arg("/y")
        .output()
        .map_err(|e| format!("reg export: {e}"))?;
    if !exported.status.success() {
        // No such key: the game has never saved on this PC.
        let _ = std::fs::remove_file(&scratch);
        let _ = std::fs::remove_file(&file);
        return Ok(());
    }
    let fresh = std::fs::read(&scratch).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&scratch);
    if std::fs::read(&file).ok().as_deref() == Some(&fresh[..]) {
        return Ok(());
    }
    std::fs::create_dir_all(staged).map_err(|e| e.to_string())?;
    std::fs::write(&file, fresh).map_err(|e| format!("{}: {e}", file.display()))
}

#[cfg(not(windows))]
pub fn stage(_staged: &Path) -> Result<(), String> {
    Ok(())
}

/// Write a staged export back into the registry, replacing the key.
#[cfg(windows)]
pub fn apply(staged: &Path) -> Result<(), String> {
    let Some(key) = key_for(staged) else {
        return Ok(());
    };
    let file = staged.join(FILE);
    let text = read_reg(&file)?;
    check(&text, &key)?;
    // Replaced rather than merged, so a value the card's copy does not have
    // does not survive from the PC's. The PC's was exported and backed up
    // before this ran.
    let _ = crate::proc::command("reg")
        .args(["delete", &key, "/f"])
        .output();
    let imported = crate::proc::command("reg")
        .arg("import")
        .arg(&file)
        .output()
        .map_err(|e| format!("reg import: {e}"))?;
    if imported.status.success() {
        Ok(())
    } else {
        Err(format!(
            "reg import: {}",
            String::from_utf8_lossy(&imported.stderr).trim()
        ))
    }
}

#[cfg(not(windows))]
pub fn apply(_staged: &Path) -> Result<(), String> {
    Ok(())
}

/// A `.reg` file's text. `reg export` writes UTF-16 with a byte-order mark.
#[cfg_attr(not(windows), allow(dead_code))]
fn read_reg(file: &Path) -> Result<String, String> {
    let bytes = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(match bytes.strip_prefix(&[0xFF, 0xFE]) {
        Some(utf16) => {
            let units: Vec<u16> = utf16
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_le_bytes(*pair))
                .collect();
            String::from_utf16_lossy(&units)
        }
        None => String::from_utf8_lossy(&bytes).into_owned(),
    })
}

/// Refuse a file that touches any key outside `key`.
#[cfg_attr(not(windows), allow(dead_code))]
fn check(text: &str, key: &str) -> Result<(), String> {
    let full = key.replacen("HKCU", "HKEY_CURRENT_USER", 1).to_lowercase();
    for line in text.lines() {
        let line = line.trim();
        let Some(header) = line.strip_prefix('[') else {
            continue;
        };
        let named = header
            .trim_start_matches('-')
            .trim_end_matches(']')
            .to_lowercase();
        let inside = named == full || named.starts_with(&format!("{full}\\"));
        if !inside {
            return Err(format!(
                "the saved registry file names {named}, outside {key}, so it was not imported"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_key_inside_the_users_software_is_allowed() {
        assert!(allowed(&[
            "HKCU",
            "Software",
            "DefaultCompany",
            "Project biscuits"
        ]));
        assert!(allowed(&["hkcu", "SOFTWARE", "Vendor"]));
        assert!(!allowed(&["HKCU", "Software"]));
        assert!(!allowed(&["HKLM", "Software", "Vendor"]));
        assert!(!allowed(&["HKCU", "Environment", "Path"]));
    }

    #[test]
    fn a_file_that_reaches_outside_its_key_is_refused() {
        let key = r"HKCU\Software\Outright Games Ltd\Bluey The Videogame";
        let good = "Windows Registry Editor Version 5.00\r\n\r\n\
            [HKEY_CURRENT_USER\\Software\\Outright Games Ltd\\Bluey The Videogame]\r\n\
            \"Save_h1\"=hex:01\r\n\r\n\
            [HKEY_CURRENT_USER\\Software\\Outright Games Ltd\\Bluey The Videogame\\Unity]\r\n";
        assert!(check(good, key).is_ok());
        let sibling = good.replace(
            "\\Unity]",
            "]\r\n[HKEY_CURRENT_USER\\Software\\Outright Games Ltd\\Bluey The Videogame 2]",
        );
        assert!(
            check(&sibling, key).is_err(),
            "a key that only starts with the name"
        );
        let run = format!(
            "{good}[HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Run]\r\n"
        );
        assert!(check(&run, key).is_err());
        let delete = format!("{good}[-HKEY_LOCAL_MACHINE\\SOFTWARE\\X]\r\n");
        assert!(check(&delete, key).is_err());
    }

    /// A real round trip through reg.exe, on a key made for the purpose.
    #[cfg(windows)]
    #[test]
    #[ignore = "writes to this user's registry"]
    fn round_trip() {
        let key = r"HKCU\Software\PCGamePakTest\Game";
        let reg = |args: &[&str]| crate::proc::command("reg").args(args).output().unwrap();
        reg(&["add", key, "/v", "Save", "/d", "one", "/f"]);
        let staged = staging_root().join("HKCU/Software/PCGamePakTest/Game");
        stage(&staged).unwrap();
        let first = std::fs::metadata(staged.join(FILE))
            .unwrap()
            .modified()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        stage(&staged).unwrap();
        let again = std::fs::metadata(staged.join(FILE))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(first, again, "unchanged key, untouched file");
        reg(&["add", key, "/v", "Save", "/d", "two", "/f"]);
        reg(&["add", key, "/v", "Extra", "/d", "x", "/f"]);
        // Put the staged "one" back: the extra value must go, the save revert.
        apply(&staged).unwrap();
        let out = String::from_utf8_lossy(&reg(&["query", key]).stdout).into_owned();
        reg(&["delete", r"HKCU\Software\PCGamePakTest", "/f"]);
        let _ = std::fs::remove_dir_all(staging_root().join("HKCU/Software/PCGamePakTest"));
        assert!(out.contains("one") && !out.contains("Extra"), "{out}");
    }
}
