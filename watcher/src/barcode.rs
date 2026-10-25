//! Read a USB barcode scanner and submit each scan as a GamePak trigger.
//!
//! A scanner is a keyboard: it types the code and presses Enter. This takes the
//! scanner's input device for itself (EVIOCGRAB), so a scan never lands in the
//! window that has focus, and hands each code to `pc-gamepak --trigger`. A code
//! without the `gp_` prefix gets one, so the EAN on a game box, 3501170818305,
//! arrives as gp_3501170818305: register that ID like any other.
//!
//! The scanner is found by name, in /dev/input/by-id, as a keyboard whose name
//! says "barcode" or "scanner". Reading it needs the device handed to the user,
//! which linux/70-pc-gamepak-barcode.rules does.

use std::fs::{self, File};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

const BY_ID: &str = "/dev/input/by-id";
/// `_IOW('E', 0x90, int)`: take every event from the device for this fd.
const EVIOCGRAB: libc::c_ulong = 0x4004_4590;
const EV_KEY: u16 = 1;
const KEY_ENTER: u16 = 28;
const KEY_KPENTER: u16 = 96;
const KEY_LEFTSHIFT: u16 = 42;
const KEY_RIGHTSHIFT: u16 = 54;
const KEY_MINUS: u16 = 12;
/// `gp_` and the longest ID; anything longer is not a GamePak code.
const MAX_CODE: usize = 67;

pub fn start() {
    if let Err(error) = thread::Builder::new()
        .name("pc-gamepak-barcode".into())
        .spawn(monitor)
    {
        crate::log::line(&format!("could not start barcode scanner monitor: {error}"));
    }
}

fn monitor() {
    // ponytail: looks for a scanner every 5 s while none is plugged in, as the
    // NFC monitor does for cards; inotify on /dev/input/by-id if that ever matters.
    let mut last_error = String::new();
    loop {
        if let Some(path) = find() {
            let error = read_scans(&path);
            // Logged once per kind of failure, not every 5 s while it lasts.
            if error != last_error {
                crate::log::line(&format!("barcode scanner: {error}"));
                last_error = error;
            }
        }
        thread::sleep(Duration::from_secs(5));
    }
}

fn find() -> Option<PathBuf> {
    fs::read_dir(BY_ID)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|path| {
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            name.ends_with("-event-kbd") && (name.contains("barcode") || name.contains("scanner"))
        })
}

/// Read until the device goes away. Returns why it stopped.
fn read_scans(path: &PathBuf) -> String {
    let mut device = match File::open(path) {
        Ok(device) => device,
        Err(error) => return format!("cannot open {}: {error}", path.display()),
    };
    // SAFETY: EVIOCGRAB takes an int by value; the fd is open for the call.
    if unsafe { libc::ioctl(device.as_raw_fd(), EVIOCGRAB, 1 as libc::c_int) } != 0 {
        return format!(
            "cannot take {} for scans: {}",
            path.display(),
            std::io::Error::last_os_error()
        );
    }
    crate::log::line(&format!("barcode scanner ready at {}", path.display()));
    let mut scan = Scan::default();
    let mut event = [0u8; std::mem::size_of::<libc::input_event>()];
    loop {
        if let Err(error) = device.read_exact(&mut event) {
            return format!("{} stopped: {error}", path.display());
        }
        // SAFETY: the buffer is exactly one input_event, read whole.
        let event: libc::input_event = unsafe { std::ptr::read_unaligned(event.as_ptr().cast()) };
        if event.type_ != EV_KEY {
            continue;
        }
        if let Some(code) = scan.key(event.code, event.value) {
            crate::log::line(&format!("barcode scanned: {code}"));
            if let Some(mut child) = crate::launcher::open_trigger(&code) {
                // The launcher's own message goes nowhere, and an unregistered
                // code is what someone setting this up needs to see.
                thread::spawn(move || {
                    if child.wait().is_ok_and(|status| !status.success()) {
                        crate::log::line(&format!(
                            "trigger {code} was refused: not registered, or not a GamePak ID"
                        ));
                    }
                });
            }
        }
    }
}

/// Keys in, codes out: US layout, the one scanners type by default.
#[derive(Default)]
struct Scan {
    code: String,
    shift: bool,
}

impl Scan {
    /// Feed one key event (`value` 1 press, 0 release, 2 repeat). Returns the
    /// finished code on Enter.
    fn key(&mut self, key: u16, value: i32) -> Option<String> {
        if key == KEY_LEFTSHIFT || key == KEY_RIGHTSHIFT {
            self.shift = value != 0;
            return None;
        }
        if value != 1 {
            return None;
        }
        if key == KEY_ENTER || key == KEY_KPENTER {
            let code = std::mem::take(&mut self.code);
            if code.is_empty() || code.len() > MAX_CODE {
                return None;
            }
            return Some(if code.starts_with("gp_") {
                code
            } else {
                format!("gp_{code}")
            });
        }
        // A character an ID cannot hold stays as '?', so the launcher refuses
        // the code and the log shows what was scanned.
        self.code.push(char_for(key, self.shift).unwrap_or('?'));
        None
    }
}

/// Letters come out lowercase whatever the shift, since IDs are lowercase.
fn char_for(key: u16, shift: bool) -> Option<char> {
    const ROWS: [(u16, &str); 4] = [
        (2, "1234567890"),
        (16, "qwertyuiop"),
        (30, "asdfghjkl"),
        (44, "zxcvbnm"),
    ];
    if key == KEY_MINUS {
        return Some(if shift { '_' } else { '-' });
    }
    ROWS.iter().find_map(|(first, keys)| {
        key.checked_sub(*first)
            .and_then(|i| keys.chars().nth(usize::from(i)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_keys(scan: &mut Scan, keys: &[(u16, bool)]) -> Option<String> {
        let mut out = None;
        for &(key, shift) in keys {
            if shift {
                scan.key(KEY_LEFTSHIFT, 1);
            }
            out = scan.key(key, 1).or(out);
            scan.key(key, 0);
            if shift {
                scan.key(KEY_LEFTSHIFT, 0);
            }
        }
        out
    }

    #[test]
    fn digits_get_the_prefix() {
        // 3 5 0 1 then Enter
        let keys = [
            (4, false),
            (6, false),
            (11, false),
            (2, false),
            (KEY_ENTER, false),
        ];
        assert_eq!(
            type_keys(&mut Scan::default(), &keys).as_deref(),
            Some("gp_3501")
        );
    }

    #[test]
    fn a_full_id_passes_through_lowercased() {
        // G P shift-minus T E S T, as a Code 128 label of "GP_TEST" types it
        let keys = [
            (34, true),
            (25, true),
            (KEY_MINUS, true),
            (20, true),
            (18, true),
            (31, true),
            (20, true),
            (KEY_ENTER, false),
        ];
        assert_eq!(
            type_keys(&mut Scan::default(), &keys).as_deref(),
            Some("gp_test")
        );
    }

    #[test]
    fn other_characters_reach_the_launcher_as_question_marks() {
        let mut scan = Scan::default();
        // 1 . 2 Enter — the dot is KEY_DOT (52)
        let keys = [(2, false), (52, false), (3, false), (KEY_ENTER, false)];
        assert_eq!(type_keys(&mut scan, &keys).as_deref(), Some("gp_1?2"));
        assert_eq!(
            type_keys(&mut scan, &[(2, false), (KEY_ENTER, false)]).as_deref(),
            Some("gp_1")
        );
    }
}
