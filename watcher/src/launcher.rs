//! Starting and stopping the launcher.
//!
//! Shared by both things that can open one: a cartridge arriving on a USB port,
//! and a tag arriving on a reader. They differ in what wakes them and in what
//! path they end up with; from here on it is the same window.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use crate::log;

/// A command for the launcher, in the desktop session as it is now.
///
/// The watcher can start before anyone logs in — at boot when the user
/// lingers, or before the desktop has published its variables — and a child
/// inherits the watcher's environment from then: no `WAYLAND_DISPLAY`, so a
/// Steam game or a window opened for a trigger has no screen to go to. The
/// systemd user manager holds the session's current ones; take the display
/// variables from there.
///
/// Those can be stale too — a session that died, or one that failed to start
/// after publishing them — and a launcher pointed at a socket that is gone
/// fails to open GTK and exits. So a Wayland socket or an X authority file
/// that does not exist is not passed on, and a live Wayland socket in the
/// runtime directory is used instead.
fn launcher_command(launcher: &Path) -> Command {
    #[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
    let mut command = Command::new(launcher);
    #[cfg(target_os = "linux")]
    if let Ok(output) = Command::new("systemctl")
        .args(["--user", "show-environment"])
        .output()
    {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
        let socket_live = |name: &str| runtime.as_ref().is_some_and(|dir| dir.join(name).exists());
        let mut wayland = None;
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            match line.split_once('=') {
                Some(("WAYLAND_DISPLAY", value)) => wayland = Some(value.to_owned()),
                Some(("XAUTHORITY", value)) if !Path::new(value).exists() => {}
                Some((key @ ("DISPLAY" | "XAUTHORITY" | "XDG_SESSION_TYPE"), value)) => {
                    command.env(key, value);
                }
                _ => {}
            }
        }
        let wayland = wayland.filter(|name| socket_live(name)).or_else(|| {
            // ponytail: first wayland-N socket wins; with two live desktops for
            // one user, the manager's own value (checked first) decides.
            std::fs::read_dir(runtime.as_ref()?)
                .ok()?
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .find(|name| {
                    name.strip_prefix("wayland-")
                        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                })
        });
        if let Some(name) = wayland {
            command.env("WAYLAND_DISPLAY", name);
        }
    }
    command
}

/// Why a launcher is being opened.
///
/// It changes what the launcher does, not what this does. `on_cartridge_insert`
/// can be set to `none`, `notify_only` or `auto_launch_game`, and the launcher
/// applies it — but only to an insert. Somebody picking a cartridge out of the
/// tray menu has asked for the window in so many words, and must get it whatever
/// the setting says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// A cartridge turned up. The setting decides.
    Inserted,
    /// A person asked for this cartridge by name.
    Asked,
}

/// Open the launcher on a cartridge, keeping the handle so the window can be
/// closed again when the cartridge goes.
///
/// Not waited on: the launcher outlives the wake that started it.
pub fn open(path: &Path, why: Why) -> Option<Child> {
    let Some(launcher) = installed_at() else {
        log::line("pc-gamepak is not installed anywhere I can find it");
        return None;
    };

    let mut command = launcher_command(&launcher);
    command
        .arg("--drive")
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if why == Why::Asked {
        // "The window, please" — spelled out, because the launcher cannot
        // otherwise tell being asked from being triggered.
        command.arg("--show");
    }

    match command.spawn() {
        Ok(child) => {
            log::line(&format!("launcher started, pid {}", child.id()));
            Some(child)
        }
        Err(e) => {
            log::line(&format!("could not start the launcher: {e}"));
            None
        }
    }
}

/// Open the launcher with the selected card's bounded NDEF message.
pub fn open_nfc(message: &[u8]) -> Option<Child> {
    if message.is_empty() || message.len() > 4096 {
        log::line("NFC NDEF message is outside the supported size");
        return None;
    }
    let Some(launcher) = installed_at() else {
        log::line("pc-gamepak is not installed anywhere I can find it");
        return None;
    };
    let encoded = message
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    match launcher_command(&launcher)
        .args(["--nfc-ndef", &encoded])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => {
            log::line(&format!("NFC launcher started, pid {}", child.id()));
            Some(child)
        }
        Err(error) => {
            log::line(&format!("could not start NFC launcher: {error}"));
            None
        }
    }
}

/// Submit a GamePak ID from a trigger, exactly as `pc-gamepak --trigger` does.
/// The launcher validates it and refuses anything unregistered.
#[cfg(target_os = "linux")]
pub fn open_trigger(id: &str) -> Option<Child> {
    let Some(launcher) = installed_at() else {
        log::line("pc-gamepak is not installed anywhere I can find it");
        return None;
    };
    match launcher_command(&launcher)
        .args(["--trigger", id])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => {
            log::line(&format!("trigger {id} started, pid {}", child.id()));
            Some(child)
        }
        Err(error) => {
            log::line(&format!("could not start the launcher for {id}: {error}"));
            None
        }
    }
}

/// Open the wizard, on its first page or straight on Settings.
///
/// The tray is the only door to the wizard that is always there: the launcher
/// itself only exists while a cartridge is plugged in, and the window that used
/// to carry this menu went away with it.
///
/// Windows only, because the tray is: on Linux the desktop entry is the door,
/// and the watcher never opens the wizard itself.
#[cfg(windows)]
pub fn open_wizard(settings: bool) -> Option<Child> {
    let Some(launcher) = installed_at() else {
        log::line("pc-gamepak is not installed anywhere I can find it");
        return None;
    };

    match Command::new(&launcher)
        .arg(if settings { "--settings" } else { "--create" })
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => Some(child),
        Err(e) => {
            log::line(&format!("could not start the wizard: {e}"));
            None
        }
    }
}

/// Close a launcher this watcher started.
///
/// Asked to close rather than killed, so the window goes the way it would if
/// the user had dismissed it. The game, if one is running, is left alone.
pub fn close(child: &mut Child) {
    if !matches!(child.try_wait(), Ok(None)) {
        return; // Already gone.
    }
    log::line(&format!("closing launcher pid {}", child.id()));
    terminate(child);
}

#[cfg(unix)]
fn terminate(child: &Child) {
    // SAFETY: a pid this process started and has not reaped, and a constant
    // signal. SIGTERM rather than SIGKILL — see above.
    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
}

#[cfg(windows)]
fn terminate(child: &mut Child) {
    // Windows has no SIGTERM. A GUI process could be asked to close with
    // WM_CLOSE, but that means finding its window, and the launcher holds
    // nothing that needs flushing.
    if let Err(e) = child.kill() {
        log::line(&format!("could not close the launcher: {e}"));
    }
}

/// Where the launcher was installed.
#[cfg(not(windows))]
pub fn installed_at() -> Option<PathBuf> {
    // A rootless install puts it in ~/.local/bin; a system install in
    // /usr/local/bin or /usr/bin. `PC_GAMEPAK_LAUNCHER` overrides all of it,
    // which is what a Flatpak or a development build uses.
    if let Some(from_env) = std::env::var_os("PC_GAMEPAK_LAUNCHER") {
        let path = PathBuf::from(from_env);
        if path.is_file() {
            return Some(path);
        }
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join(".local/bin/pc-gamepak"));
    }
    candidates.push(PathBuf::from("/usr/local/bin/pc-gamepak"));
    candidates.push(PathBuf::from("/usr/bin/pc-gamepak"));

    candidates.into_iter().find(|path| path.is_file())
}

/// Where the launcher was installed: next to this executable, since the
/// installer puts both in the same directory.
#[cfg(windows)]
pub fn installed_at() -> Option<PathBuf> {
    if let Some(from_env) = std::env::var_os("PC_GAMEPAK_LAUNCHER") {
        let path = PathBuf::from(from_env);
        if path.is_file() {
            return Some(path);
        }
    }

    let beside = std::env::current_exe()
        .ok()?
        .parent()?
        .join("pc-gamepak.exe");
    beside.is_file().then_some(beside)
}
