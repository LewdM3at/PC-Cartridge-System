//! The Linux watcher: one process, no root, woken by the mount table.
//!
//! The system install does not need this at all — udev is already running and
//! starts the launcher through a systemd unit, so nothing is resident. This is
//! for the other shape of install: no root, no udev rule, a systemd *user*
//! service, and everything a sandboxed package like a Flatpak can actually do.
//!
//! It does not subscribe to udev. A sandbox has no `/run/udev`, and udev's
//! netlink group is not something a confined process should count on. Instead it
//! blocks in `poll()` on `/proc/self/mountinfo`, which the kernel wakes on any
//! mount activity — no timer, no polling loop, no CPU while it waits.
//!
//! That is also more accurate than the udev route. udev fires when the kernel
//! sees the partition, which is before the desktop has mounted it, so the helper
//! it starts spends up to sixty seconds waiting for a mount point to appear.
//! Waking on the mount table means the cartridge is readable the moment we hear
//! about it.

use std::collections::HashMap;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::launcher;
use crate::log;
use crate::mounts::{self, Mount};

/// The same drive arriving twice in quick succession — a remount, or a desktop
/// that mounts and immediately re-mounts — should open one window, not two.
const DEBOUNCE: Duration = Duration::from_secs(4);

/// Where the kernel reports mount activity. `/proc/self/mounts` can be polled
/// too, but mountinfo is the one documented to signal POLLPRI, and it is
/// per-namespace, which is what makes this work inside a sandbox.
const MOUNTINFO: &str = "/proc/self/mountinfo";

/// The table itself, in the format `mounts.rs` parses.
const MOUNTS: &str = "/proc/self/mounts";

/// Marks a sandbox. Its presence is how a Flatpak knows it is one.
const FLATPAK_INFO: &str = "/.flatpak-info";

/// Ask the desktop to start this watcher at login.
///
/// The rootless install enables `pc-gamepak-watcher.service` and that is the
/// end of it. A Flatpak has no user unit to enable: it cannot write one into
/// the host's systemd directory, and a unit written inside the sandbox is a
/// unit nothing outside will ever read. The Background portal is the only door,
/// and the entry it writes is one the user can revoke from their own settings
/// rather than one hidden in a dotfile.
///
/// `gdbus` rather than a D-Bus crate. It is already in the GNOME runtime this
/// builds against, this is one call made once per login, and a dependency that
/// exists to send a single message is a dependency to justify at every audit.
///
/// Best effort, and never fatal. A desktop with no portal, or a user who
/// declines, leaves a watcher that works for this session and does not come
/// back at the next login — a worse product, not a broken one, and one that
/// still opens from the menu entry.
fn request_autostart() {
    if !Path::new(FLATPAK_INFO).exists() {
        return;
    }

    // `commandline` is what the desktop will run, and it runs it inside the
    // sandbox — so it names the wrapper in /app/bin, not a path on the host.
    let options = concat!(
        "{'reason': <'Open a launcher when a cartridge is plugged in'>,",
        " 'autostart': <true>,",
        " 'background': <true>,",
        " 'commandline': <['pc-gamepak-launch', '--watch']>}"
    );

    let asked = std::process::Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.portal.Desktop",
            "--object-path",
            "/org/freedesktop/portal/desktop",
            "--method",
            "org.freedesktop.portal.Background.RequestBackground",
            // No parent window: the watcher has none, and the portal accepts
            // an empty identifier for exactly this case.
            "",
            options,
        ])
        .output();

    match asked {
        Ok(out) if out.status.success() => {
            log::line("asked the desktop to start this at login");
        }
        Ok(out) => {
            let why = String::from_utf8_lossy(&out.stderr);
            log::line(&format!(
                "autostart request refused: {}",
                why.trim().lines().next().unwrap_or("no reason given")
            ));
        }
        Err(error) => {
            log::line(&format!("could not reach the portal: {error}"));
        }
    }
}

pub fn run() -> ! {
    log::line("watcher starting (mount table)");
    #[cfg(target_os = "linux")]
    {
        crate::nfc::start();
        crate::barcode::start();
    }

    // Outside a sandbox the installer enabled a systemd user unit and this does
    // nothing. Inside one there is no unit to enable, so it asks instead.
    request_autostart();

    let Ok(watch) = std::fs::File::open(MOUNTINFO) else {
        log::line("could not open /proc/self/mountinfo; is this Linux?");
        std::process::exit(1);
    };

    // Whatever is already mounted at login is not an arrival. Starting a
    // session should not pop a window for a cartridge left plugged in.
    let mut previous = read_mounts();
    log::line(&format!("watching {} mounted filesystems", previous.len()));

    let mut recent: HashMap<PathBuf, Instant> = HashMap::new();
    // The launchers this watcher started, by the cartridge they are showing.
    let mut open: HashMap<PathBuf, std::process::Child> = HashMap::new();

    loop {
        wait_for_change(watch.as_raw_fd());

        // Collect any launcher that has closed itself, so a long session does
        // not accumulate zombies.
        open.retain(|_, child| !matches!(child.try_wait(), Ok(Some(_))));

        let current = read_mounts();

        for gone in mounts::departures(&previous, &current) {
            log::line(&format!("cartridge removed: {}", gone.display()));
            close_launcher_for(&gone, open.remove(&gone));
            recent.remove(&gone);
        }

        for arrived in mounts::arrivals(&previous, &current) {
            if !mounts::is_cartridge(&arrived) {
                continue;
            }
            let now = Instant::now();
            if let Some(last) = recent.get(&arrived) {
                if now.duration_since(*last) < DEBOUNCE {
                    continue;
                }
            }
            recent.insert(arrived.clone(), now);
            log::line(&format!("cartridge detected at {}", arrived.display()));
            if let Some(child) = launcher::open(&arrived, launcher::Why::Inserted) {
                open.insert(arrived.clone(), child);
            }
        }

        previous = current;
    }
}

/// Block until the mount table changes.
///
/// `poll()` with no timeout: the process is off the run queue entirely until the
/// kernel has something to say, which is the whole reason this is affordable as
/// a resident service.
fn wait_for_change(fd: std::os::fd::RawFd) {
    // POLLPRI is what mountinfo signals; POLLERR is what /proc/mounts signals
    // and costs nothing to accept as well.
    let mut pollfd = libc::pollfd {
        fd,
        events: libc::POLLPRI | libc::POLLERR,
        revents: 0,
    };

    loop {
        // SAFETY: one initialised pollfd, count 1, no timeout. poll() writes
        // only revents.
        let ready = unsafe { libc::poll(&mut pollfd, 1, -1) };
        if ready >= 0 {
            return;
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::Interrupted {
            continue; // A signal, not a problem.
        }
        // Anything else would spin: back off rather than burn a core.
        log::line(&format!("poll failed: {error}; retrying"));
        std::thread::sleep(Duration::from_secs(1));
        return;
    }
}

fn read_mounts() -> Vec<Mount> {
    let mut text = String::new();
    match std::fs::File::open(MOUNTS).and_then(|mut f| f.read_to_string(&mut text)) {
        Ok(_) => mounts::parse(&text),
        Err(e) => {
            log::line(&format!("could not read {MOUNTS}: {e}"));
            Vec::new()
        }
    }
}

/// Close the launcher window for a cartridge that has gone.
///
/// The launcher this watcher started is closed by its own handle, which is
/// exact — no name matching, and it works whatever the launcher is wrapped in.
/// The scan afterwards is for windows this watcher did not start, from a
/// previous run of it or from udev.
///
/// The game itself is left alone: pulling a cartridge while it runs is the
/// user's business, and killing their session would be a worse surprise than a
/// stale window.
fn close_launcher_for(mount: &Path, ours: Option<std::process::Child>) {
    if let Some(mut child) = ours {
        // Reaped on the next wake by the retain() in the loop.
        launcher::close(&mut child);
        return;
    }

    let Ok(entries) = std::fs::read_dir("/proc") else {
        return;
    };

    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        if !shows_drive(&cmdline, mount) {
            continue;
        }

        log::line(&format!(
            "closing launcher pid {pid} (not started by this watcher)"
        ));
        terminate(pid);
    }
}

/// SIGTERM rather than SIGKILL, so the window closes the way it would if the
/// user had dismissed it.
fn terminate(pid: i32) {
    // SAFETY: a pid we started or read from /proc, and a constant signal.
    unsafe { libc::kill(pid, libc::SIGTERM) };
}

/// Was this process started with `--drive <mount>`?
///
/// argv arrives NUL-separated, and the check is exact: a launcher showing
/// /run/media/x/CINDER2 must not be closed when CINDER goes.
fn shows_drive(cmdline: &[u8], mount: &Path) -> bool {
    let args: Vec<&[u8]> = cmdline.split(|b| *b == 0).collect();
    let Some(name) = args.first() else {
        return false;
    };
    let name = Path::new(std::str::from_utf8(name).unwrap_or(""));
    if name.file_name().and_then(|n| n.to_str()) != Some("pc-gamepak") {
        return false;
    }

    let wanted = mount.as_os_str().as_encoded_bytes();
    args.windows(2)
        .any(|pair| pair[0] == b"--drive" && pair[1] == wanted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmdline(parts: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for part in parts {
            out.extend_from_slice(part.as_bytes());
            out.push(0);
        }
        out
    }

    #[test]
    fn only_the_launcher_showing_that_drive_is_closed() {
        let mount = Path::new("/run/media/harry/CINDER");

        assert!(shows_drive(
            &cmdline(&[
                "/usr/local/bin/pc-gamepak",
                "--drive",
                "/run/media/harry/CINDER"
            ]),
            mount
        ));

        // A different cartridge keeps its window.
        assert!(!shows_drive(
            &cmdline(&[
                "/usr/local/bin/pc-gamepak",
                "--drive",
                "/run/media/harry/HOLLOW"
            ]),
            mount
        ));

        // A prefix is not a match: CINDER2 is somebody else.
        assert!(!shows_drive(
            &cmdline(&[
                "/usr/local/bin/pc-gamepak",
                "--drive",
                "/run/media/harry/CINDER2"
            ]),
            mount
        ));

        // The wizard is not showing a cartridge at all.
        assert!(!shows_drive(
            &cmdline(&["/usr/local/bin/pc-gamepak", "--create"]),
            mount
        ));

        // And nothing else on the system is ours to kill, whatever it was
        // started with.
        assert!(!shows_drive(
            &cmdline(&["/usr/bin/vlc", "--drive", "/run/media/harry/CINDER"]),
            mount
        ));
        assert!(!shows_drive(&[], mount));
    }
}
