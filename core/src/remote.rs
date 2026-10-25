//! Start on tap: a game on another PC, started now and streamed on Play.
//!
//! ```text
//! COUCH (this PC)                          GAMING PC (Apollo / Sunshine)
//! tap ── GameStream launch, no stream ───▶ the app starts and keeps running
//! Play ── moonlight stream <host> <app> ─▶ Moonlight resumes the running app
//! Eject ── moonlight quit <host> ────────▶ the app is closed
//! ```
//!
//! Everything happens from this PC, with the pairing Moonlight already has
//! (see [`crate::moonlight`]). The gaming PC needs nothing but the game in its
//! app list.

use std::path::Path;
use std::time::Duration;

use crate::gamepak::GamePakId;

/// The drive path the launcher is given for a remote GamePak: not a drive,
/// but the commands that would read one recognise it and answer for the host.
pub const MARKER: &str = "gamepak-remote://";

/// The GamePak ID a launcher marker names, or `None` for a real drive path.
pub fn marker_id(drive_path: &str) -> Option<&str> {
    drive_path.strip_prefix(MARKER)
}

/// A remote GamePak as this PC's registry describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    pub id: GamePakId,
    /// The host, as Moonlight names it.
    pub moonlight: String,
    /// The app, as the host lists it.
    pub app: String,
}

impl Remote {
    /// Look `id` up in the registry. An error unless it is a remote action.
    pub fn resolve(registry: &Path, id: &str) -> Result<Remote, String> {
        let id = GamePakId::parse(id.trim())?;
        let pak = crate::gamepak::lookup_from(registry, &id)?;
        let crate::gamepak::Action::Remote { moonlight, app } = pak.action else {
            return Err(format!("GamePak {} is not a remote game", id.as_str()));
        };
        // Handed to Moonlight as arguments, so never something it would read
        // as an option.
        if moonlight.trim().is_empty()
            || app.trim().is_empty()
            || moonlight.starts_with('-')
            || app.starts_with('-')
        {
            return Err("a remote GamePak needs the host as Moonlight names it, and an app".into());
        }
        Ok(Remote { id, moonlight, app })
    }

    /// The launcher's stand-in drive path for this GamePak.
    pub fn marker(&self) -> String {
        format!("{MARKER}{}", self.id.as_str())
    }

    /// Wake the host if it is asleep and start the app there, without a
    /// stream.
    pub fn prime(&self) -> Result<crate::moonlight::Launched, String> {
        let settings = crate::moonlight::Settings::load()?;
        let (cert, key) = settings.identity()?;
        let host = settings.host(&self.moonlight)?;
        let mut client = crate::moonlight::Client::new(host, &cert, &key)?;
        client.launch(&self.app, settings.mode())
    }

    /// Start streaming. Returns once Moonlight has been started.
    pub fn stream(&self) -> Result<std::process::Child, String> {
        std::process::Command::new(crate::gamepak::moonlight_program())
            .args(["stream", &self.moonlight, &self.app])
            .stdin(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("could not start Moonlight: {e}"))
    }

    /// Close the app on the host, and the stream with it, waiting up to a
    /// minute for Moonlight to say it is done: a game asked to close may save
    /// first.
    pub fn quit(&self) -> Result<(), String> {
        let mut child = std::process::Command::new(crate::gamepak::moonlight_program())
            .args(["quit", &self.moonlight])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("could not start Moonlight: {e}"))?;
        let started = std::time::Instant::now();
        while started.elapsed() < Duration::from_secs(60) {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                return if status.success() {
                    Ok(())
                } else {
                    Err(format!(
                        "Moonlight could not quit the app on {}",
                        self.moonlight
                    ))
                };
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        let _ = child.kill();
        Err("Moonlight did not finish quitting within a minute".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_name_their_id_and_drives_do_not() {
        assert_eq!(marker_id("gamepak-remote://gp_x"), Some("gp_x"));
        assert_eq!(marker_id("E:\\"), None);
        assert_eq!(marker_id("/run/media/you/CART"), None);
    }

    #[test]
    fn a_remote_gamepak_resolves_from_the_registry() {
        let scratch = crate::testutil::Scratch::new("remote-registry");
        let registry = scratch.path().join("gamepaks.json");
        std::fs::write(
            &registry,
            r#"{"gamepaks":[
              {"id":"gp_cp","action":{"type":"remote","moonlight":"GAMING-PC","app":"Cyberpunk 2077"}},
              {"id":"gp_bad","action":{"type":"remote","moonlight":"--help","app":"x"}},
              {"id":"gp_steam","action":{"type":"steam","appId":1}}
            ]}"#,
        )
        .unwrap();
        let remote = Remote::resolve(&registry, "gp_cp").unwrap();
        assert_eq!(remote.moonlight, "GAMING-PC");
        assert_eq!(remote.app, "Cyberpunk 2077");
        assert_eq!(remote.marker(), "gamepak-remote://gp_cp");
        assert!(Remote::resolve(&registry, "gp_bad").is_err());
        assert!(Remote::resolve(&registry, "gp_steam").is_err());
        assert_eq!(
            crate::gamepak::trigger_from(&registry, "gp_cp").unwrap(),
            crate::gamepak::Outcome::OpenRemote(GamePakId::parse("gp_cp").unwrap())
        );
    }
}
