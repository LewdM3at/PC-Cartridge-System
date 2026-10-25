//! Starting a cartridge's game: the one place any front-end does it.
//!
//! Moved here from the Tauri launcher so every front-end starts a game the same
//! way. Counting the launch and watching the session are the caller's, because
//! they live as long as the caller's process does; this only starts the game.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};

/// Schemes handed to whatever the OS has registered for them.
const KNOWN_SCHEMES: [&str; 9] = [
    "steam://",
    "heroic://",
    "gog://",
    "epic://",
    "playnite://",
    "lutris://",
    "moonlight://",
    "http://",
    "https://",
];

/// How a game was started.
pub enum Started {
    /// Handed to another launcher through a URI (Steam, Playnite, …). Nothing
    /// here can see the game's process.
    Handed,
    /// A program on the cartridge, started directly. The child is returned so
    /// the caller can reap it.
    Carried(Child),
}

/// Whether an `executable=` value is a URI rather than a path on the drive.
pub fn is_uri(executable: &str) -> bool {
    let lower = executable.to_lowercase();
    KNOWN_SCHEMES.iter().any(|scheme| lower.starts_with(scheme))
}

/// Start `executable` from the cartridge at `drive_path`.
///
/// `log` receives notes worth keeping in a debug log and nothing else; the game
/// starts whether or not anybody reads them.
pub fn start(drive_path: &str, executable: &str, log: &dyn Fn(String)) -> Result<Started, String> {
    if executable.is_empty() {
        return Err("No executable configured for this cartridge".into());
    }

    if is_uri(executable) {
        // Started by somebody else's launcher, in somebody else's environment.
        // Nothing here can decide where that game keeps its saves, which is
        // what `save=` lines are for.
        open_uri(executable)?;
        return Ok(Started::Handed);
    }

    let full_path = PathBuf::from(drive_path).join(executable);
    if !full_path.exists() {
        return Err(format!("Executable not found: {}", full_path.display()));
    }

    // A ROM is not a program. Whatever the desktop opens that file type with —
    // RetroArch, an emulator of the person's choosing — is the right player,
    // and choosing one is the host's business, not the cartridge's.
    if crate::emulated::is_rom(&full_path) {
        open_uri(&full_path.to_string_lossy())?;
        return Ok(Started::Handed);
    }

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new(&full_path);
        command.current_dir(full_path.parent().unwrap_or(Path::new(".")));
        command
    };
    // Through bash on purpose, not as a fallback: exFAT cannot store an
    // executable bit, so nothing on an exFAT cartridge is executable and a
    // carried Linux game is a shell script by necessity.
    #[cfg(not(target_os = "windows"))]
    let mut command = {
        let mut command = Command::new("bash");
        command
            .arg(&full_path)
            .current_dir(full_path.parent().unwrap_or(Path::new(".")));
        command
    };

    // This is a game the cartridge carries and that this launcher is starting
    // itself, which is the only case where its environment is ours to set — so
    // it is the only case where the cartridge can be handed the game's whole
    // home directory and catch every save without anyone having declared one.
    if crate::home::wanted(Path::new(drive_path)) {
        match crate::home::prepare(Path::new(drive_path)) {
            Ok(portable) => {
                for (name, value) in &portable.vars {
                    command.env(name, value);
                }
                log(format!("portable home: {}", portable.root));
            }
            // A cartridge asking for something the drive will not give it. The
            // game still starts, in the ordinary environment, because refusing
            // to launch would be a worse answer than saving to the host.
            Err(why) => log(format!("portable home unavailable: {why}")),
        }
    }

    command
        .spawn()
        .map(Started::Carried)
        .map_err(|e| format!("Failed to launch {}: {e}", full_path.display()))
}

/// Hand a URI to whatever the OS has registered for it.
pub fn open_uri(uri: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("cmd");
        command.args(["/c", "start", "", uri]);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(uri);
        command
    };
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(uri);
        command
    };
    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Failed to open URI {uri}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_a_uri_from_a_path() {
        assert!(is_uri("steam://rungameid/413150"));
        assert!(is_uri("Playnite://playnite/start/abc"));
        assert!(is_uri("moonlight://launch/host/game"));
        assert!(!is_uri("Games/Tunic/Tunic.exe"));
        assert!(!is_uri(""));
    }

    #[test]
    fn refuses_what_it_cannot_start() {
        let scratch = crate::testutil::Scratch::new("launch");
        let drive = scratch.path().to_string_lossy().into_owned();
        let quiet = |_: String| {};
        assert!(start(&drive, "", &quiet).is_err());
        let missing = start(&drive, "nowhere/game.exe", &quiet);
        assert!(matches!(missing, Err(e) if e.contains("not found")));
    }
}
