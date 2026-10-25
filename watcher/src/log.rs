//! A log for the watcher.
//!
//! The watcher runs under `windows_subsystem = "windows"`, and on Linux as a
//! systemd user service, so it has no console either way and every `println!`
//! goes nowhere. When it fails to fire there is otherwise
//! no way to find out why — the process is alive, silent, and doing nothing
//! visible. This writes the few lines that answer "why didn't my cartridge open
//! the launcher?".
//!
//! Deliberately tiny: an append, a timestamp, and a size cap. No dependency, no
//! background thread, no allocation while idle.

use std::io::Write;
use std::path::PathBuf;

/// Truncate once the log passes this, so a machine left on for months does not
/// grow an unbounded file.
const MAX_BYTES: u64 = 256 * 1024;

/// `%LOCALAPPDATA%\PC-GamePak\watcher.log`, matching where the
/// installer puts everything else.
#[cfg(windows)]
fn log_path() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    let dir = PathBuf::from(base).join("PC-GamePak");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("watcher.log"))
}

/// `~/.local/state/pc-gamepak/watcher.log`, beside the log the udev helpers
/// write, so there is one place to look whichever install this is.
#[cfg(not(windows))]
fn log_path() -> Option<PathBuf> {
    let dir = match std::env::var_os("XDG_STATE_HOME") {
        Some(state) if !state.is_empty() => PathBuf::from(state),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".local/state"),
    }
    .join("pc-gamepak");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("watcher.log"))
}

/// Append one line. Failing to log is never worth crashing over.
pub fn line(message: &str) {
    let Some(path) = log_path() else { return };

    if std::fs::metadata(&path)
        .map(|m| m.len() > MAX_BYTES)
        .unwrap_or(false)
    {
        let _ = std::fs::write(&path, b"");
    }

    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    // One write, so lines from the NFC and barcode threads cannot interleave:
    // writeln! on a File is a write per piece.
    let _ = file.write_all(format!("[{}] {message}\n", timestamp()).as_bytes());
}

/// `YYYY-MM-DDTHH:MM:SSZ`, from the clock alone.
///
/// UTC, and marked as UTC. It used to be written without the `Z`, so a line
/// logged at 20:13 local read as 18:13 beside an event log that is local; the
/// launcher's log already writes ISO 8601 with the `Z`, and now both agree.
///
/// Formatting a civil date by hand rather than taking a dependency: this is the
/// only place the watcher needs one, and the crate would be linked for the whole
/// session.
pub fn timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (y, mo, d, h, mi, s) = civil_from_unix(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Split a Unix timestamp into civil date and time (UTC).
///
/// Howard Hinnant's days-from-civil algorithm, run backwards: it shifts the
/// year to start in March so the leap day lands at the end of the cycle, which
/// removes every special case except the 400-year rule.
pub fn civil_from_unix(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let days = secs / 86_400;
    let rem = secs % 86_400;

    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };

    (year, month, day, rem / 3600, (rem % 3600) / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_known_instants() {
        assert_eq!(civil_from_unix(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil_from_unix(1), (1970, 1, 1, 0, 0, 1));
        // 2001-09-09T01:46:40Z, the 1e9 second.
        assert_eq!(civil_from_unix(1_000_000_000), (2001, 9, 9, 1, 46, 40));
        // 2038-01-19T03:14:07Z, the 32-bit rollover.
        assert_eq!(civil_from_unix(2_147_483_647), (2038, 1, 19, 3, 14, 7));
    }

    #[test]
    fn handles_leap_days() {
        // 2000 is a leap year (divisible by 400), 1900 was not.
        assert_eq!(civil_from_unix(951_782_400), (2000, 2, 29, 0, 0, 0));
        // The day after, to prove the rollover.
        assert_eq!(civil_from_unix(951_782_400 + 86_400), (2000, 3, 1, 0, 0, 0));
        // 2024-02-29, a more recent leap day.
        assert_eq!(civil_from_unix(1_709_164_800), (2024, 2, 29, 0, 0, 0));
    }

    #[test]
    fn end_of_year_rolls_over() {
        assert_eq!(civil_from_unix(1_735_689_599), (2024, 12, 31, 23, 59, 59));
        assert_eq!(civil_from_unix(1_735_689_600), (2025, 1, 1, 0, 0, 0));
    }

    #[test]
    fn timestamp_is_the_expected_shape() {
        let t = timestamp();
        assert_eq!(t.len(), 20, "{t}");
        assert_eq!(&t[4..5], "-");
        assert_eq!(&t[10..11], "T");
        assert_eq!(&t[13..14], ":");
        assert!(t.ends_with('Z'), "a UTC time has to say it is one: {t}");
    }
}
