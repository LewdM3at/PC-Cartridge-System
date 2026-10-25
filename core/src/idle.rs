//! How long since somebody last touched the machine.
//!
//! Playtime that keeps counting while a game sits paused overnight is not
//! playtime, so the tracker in [`crate::playtrack`] asks this how long the
//! person has been away and stops counting once it passes a threshold. Taken
//! from GamingGaiden, which does the same for the same reason.
//!
//! The one rule this module keeps: **a controller counts as input.** Every
//! desktop's own idle clock watches the keyboard and mouse only, so a game
//! played on a pad looks idle to it from the first minute. Pads are read here
//! too, and an answer that cannot see the pads is not given at all — `None`
//! means "no idea", and the tracker counts through a `None` rather than pausing
//! on a guess.
//!
//! * **Windows:** `GetLastInputInfo` for keyboard and mouse, and XInput for
//!   pads — a packet number that has moved since the last look is a press.
//! * **Linux:** logind's `IdleHint` for keyboard and mouse, which the desktop
//!   sets once it considers the session idle, and `/dev/input/js*` for pads.
//!   The joystick nodes are the ones a desktop session is given access to, so
//!   no group membership is needed. A compositor that never sets the hint gives
//!   no keyboard answer, and then nothing pauses — the same as before this
//!   existed.

use std::time::Instant;

/// Reads idle time, remembering what it saw of the pads last time.
///
/// Stateful because a pad has no "last input" clock of its own: the only way
/// to tell it was touched is to see that something changed since the last look.
pub struct IdleProbe {
    /// When a pad last did anything, as far as this probe has seen.
    pad_input_at: Option<Instant>,
    #[cfg(target_os = "windows")]
    packets: [Option<u32>; 4],
    #[cfg(target_os = "linux")]
    pads: linux::Pads,
}

impl Default for IdleProbe {
    fn default() -> Self {
        Self::new()
    }
}

impl IdleProbe {
    pub fn new() -> Self {
        Self {
            pad_input_at: None,
            #[cfg(target_os = "windows")]
            packets: [None; 4],
            #[cfg(target_os = "linux")]
            pads: linux::Pads::default(),
        }
    }

    /// Seconds since the last input from anything, or `None` when the keyboard
    /// and mouse cannot be seen.
    pub fn idle_seconds(&mut self) -> Option<u64> {
        if self.pads_moved() {
            self.pad_input_at = Some(Instant::now());
        }
        let desk = desk_idle_seconds()?;
        let pad = self.pad_input_at.map(|at| at.elapsed().as_secs());
        Some(combine(desk, pad))
    }

    #[cfg(target_os = "windows")]
    fn pads_moved(&mut self) -> bool {
        windows::pads_moved(&mut self.packets)
    }

    #[cfg(target_os = "linux")]
    fn pads_moved(&mut self) -> bool {
        self.pads.moved()
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    fn pads_moved(&mut self) -> bool {
        false
    }
}

/// The more recent of the two: somebody holding a pad is not idle because the
/// keyboard is.
fn combine(desk: u64, pad: Option<u64>) -> u64 {
    pad.map_or(desk, |pad| desk.min(pad))
}

#[cfg(target_os = "windows")]
fn desk_idle_seconds() -> Option<u64> {
    windows::desk_idle_seconds()
}

#[cfg(target_os = "linux")]
fn desk_idle_seconds() -> Option<u64> {
    linux::desk_idle_seconds()
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn desk_idle_seconds() -> Option<u64> {
    None
}

// --------------------------------------------------------------------------
// Windows
// --------------------------------------------------------------------------

#[cfg(target_os = "windows")]
mod windows {
    use windows_sys::Win32::System::SystemInformation::GetTickCount;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    use windows_sys::Win32::UI::Input::XboxController::{XInputGetState, XINPUT_STATE};

    pub fn desk_idle_seconds() -> Option<u64> {
        // SAFETY: a zeroed struct with its size set is what the call expects,
        // and it writes nothing outside it.
        unsafe {
            let mut info: LASTINPUTINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<LASTINPUTINFO>() as u32;
            if GetLastInputInfo(&mut info) == 0 {
                return None;
            }
            // Both are milliseconds since boot in a u32 that wraps every 49.7
            // days; wrapping subtraction is right across the wrap.
            let idle_ms = GetTickCount().wrapping_sub(info.dwTime);
            Some(u64::from(idle_ms) / 1000)
        }
    }

    /// Whether any of the four XInput pads has reported a change.
    ///
    /// The first look at a pad only records its packet number: a number seen
    /// for the first time says nothing about whether it was just pressed.
    pub fn pads_moved(packets: &mut [Option<u32>; 4]) -> bool {
        let mut moved = false;
        for (index, last) in packets.iter_mut().enumerate() {
            // SAFETY: a zeroed XINPUT_STATE is valid, and the call only writes
            // into it.
            let packet = unsafe {
                let mut state: XINPUT_STATE = std::mem::zeroed();
                if XInputGetState(index as u32, &mut state) != 0 {
                    // Not connected.
                    *last = None;
                    continue;
                }
                state.dwPacketNumber
            };
            if last.is_some_and(|seen| seen != packet) {
                moved = true;
            }
            *last = Some(packet);
        }
        moved
    }
}

// --------------------------------------------------------------------------
// Linux
// --------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::HashMap;
    use std::fs::File;
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::PathBuf;

    /// `O_NONBLOCK`, which std does not name. The same value on every
    /// architecture Linux games run on (x86_64, aarch64).
    const O_NONBLOCK: i32 = 0o4000;

    /// `JS_EVENT_INIT`: the synthetic events a joystick node replays on open,
    /// one per axis and button, which describe its state rather than a press.
    const JS_EVENT_INIT: u8 = 0x80;

    /// The joystick nodes this probe holds open, by path.
    #[derive(Default)]
    pub struct Pads {
        open: HashMap<PathBuf, File>,
    }

    impl Pads {
        /// Drain every pad's queue, and say whether any of it was a real event.
        pub fn moved(&mut self) -> bool {
            self.rescan();
            let mut moved = false;
            let mut gone = Vec::new();
            for (path, file) in &mut self.open {
                match drain(file) {
                    Some(any) => moved |= any,
                    None => gone.push(path.clone()),
                }
            }
            for path in gone {
                self.open.remove(&path);
            }
            moved
        }

        /// Pick up pads plugged in since the last look.
        fn rescan(&mut self) {
            let Ok(entries) = std::fs::read_dir("/dev/input") else {
                return;
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                let is_js = name.to_str().is_some_and(|n| n.starts_with("js"));
                let path = entry.path();
                if !is_js || self.open.contains_key(&path) {
                    continue;
                }
                if let Ok(mut file) = std::fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(O_NONBLOCK)
                    .open(&path)
                {
                    // The replayed state is not a press.
                    let _ = drain(&mut file);
                    self.open.insert(path, file);
                }
            }
        }
    }

    /// Read everything queued. `Some(true)` when a real event was among it,
    /// `None` when the pad has gone away.
    fn drain(file: &mut File) -> Option<bool> {
        let mut any = false;
        let mut event = [0u8; 8];
        loop {
            match file.read(&mut event) {
                Ok(8) => any |= event[6] & JS_EVENT_INIT == 0,
                Ok(0) => return None,
                Ok(_) => return Some(any),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Some(any),
                Err(_) => return None,
            }
        }
    }

    /// logind's view of this session: idle for how long, or active.
    pub fn desk_idle_seconds() -> Option<u64> {
        let session = std::env::var("XDG_SESSION_ID").unwrap_or_else(|_| "auto".to_string());
        let out = crate::proc::command("loginctl")
            .args([
                "show-session",
                &session,
                "-p",
                "IdleHint",
                "-p",
                "IdleSinceHint",
            ])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_micros() as u64;
        super::parse_logind(&String::from_utf8_lossy(&out.stdout), now_us)
    }
}

/// Read `loginctl show-session` output into seconds idle.
///
/// `IdleHint=no` is an answer — zero — and so is `yes` with a start time. A
/// hint of `yes` with no usable time is idle for an unknown stretch, which is
/// reported as "just now": pausing on it would be guessing.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_logind(text: &str, now_us: u64) -> Option<u64> {
    let mut hint = None;
    let mut since = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("IdleHint=") {
            hint = Some(value.trim() == "yes");
        } else if let Some(value) = line.strip_prefix("IdleSinceHint=") {
            since = value.trim().parse::<u64>().ok();
        }
    }
    match hint? {
        false => Some(0),
        true => match since {
            Some(start) if start > 0 && start <= now_us => Some((now_us - start) / 1_000_000),
            _ => Some(0),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pad_in_hand_is_not_idle_because_the_keyboard_is() {
        assert_eq!(combine(900, Some(3)), 3);
        assert_eq!(combine(4, Some(600)), 4);
        assert_eq!(combine(900, None), 900);
    }

    #[test]
    fn logind_active_is_zero_idle() {
        assert_eq!(
            parse_logind("IdleHint=no\nIdleSinceHint=0\n", 10_000_000),
            Some(0)
        );
    }

    #[test]
    fn logind_idle_counts_from_its_hint() {
        let now = 1_000_000_000_000;
        let since = now - 420 * 1_000_000;
        let text = format!("IdleHint=yes\nIdleSinceHint={since}\n");
        assert_eq!(parse_logind(&text, now), Some(420));
    }

    #[test]
    fn logind_without_a_hint_is_no_answer() {
        assert_eq!(parse_logind("", 1), None);
        assert_eq!(parse_logind("Foo=bar\n", 1), None);
    }

    #[test]
    fn an_idle_hint_with_no_time_does_not_invent_a_long_absence() {
        assert_eq!(
            parse_logind("IdleHint=yes\nIdleSinceHint=0\n", 5_000_000),
            Some(0)
        );
    }
}
