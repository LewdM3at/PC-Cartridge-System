//! Telling the drive which blocks it no longer has to keep.
//!
//! A cartridge is a DRAM-less SSD behind a USB bridge, which is the worst case
//! for a flash translation layer: it cannot borrow host memory over USB the way
//! it would over PCIe, so its garbage collector has less to work with and needs
//! erased blocks to be genuinely free. Deleting a game only frees the space in
//! the filesystem — the drive still believes every one of those blocks holds
//! live data until something says otherwise.
//!
//! That something is TRIM, and on removable media nothing sends it on a
//! schedule: Linux mounts external volumes without `discard`, and Windows only
//! optimises drives it considers fixed.
//!
//! It is best-effort by nature. Many USB bridges do not pass the command
//! through at all, and a refusal is not a failure worth stopping a build for.

use std::path::Path;

/// What came of asking.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TrimOutcome {
    /// The drive took it.
    Trimmed,
    /// The bridge or the filesystem does not support it. Not an error: plenty
    /// of enclosures simply do not pass UNMAP through.
    Unsupported,
    /// The tool is missing, or something else went wrong.
    Failed(String),
}

/// Ask the drive to release everything the filesystem is no longer using.
pub fn trim(mount: &str) -> TrimOutcome {
    if !Path::new(mount).is_dir() {
        return TrimOutcome::Failed(format!("{mount} is not there any more"));
    }
    run(mount)
}

#[cfg(not(windows))]
fn run(mount: &str) -> TrimOutcome {
    // fstrim needs root for the ioctl, so it goes through the same desktop
    // authentication prompt the formatter uses.
    let out = crate::proc::command("pkexec")
        .args(["fstrim", "-v", mount])
        .output();

    match out {
        Ok(out) if out.status.success() => TrimOutcome::Trimmed,
        Ok(out) => TrimOutcome::from_message(&String::from_utf8_lossy(&out.stderr)),
        Err(e) => TrimOutcome::Failed(format!("could not run fstrim: {e}")),
    }
}

#[cfg(windows)]
fn run(mount: &str) -> TrimOutcome {
    let letter = mount.trim_end_matches('\\').trim_end_matches(':');
    if letter.len() != 1 {
        return TrimOutcome::Failed(format!("{mount} is not a drive letter"));
    }

    // ReTrim is the one Optimize-Volume verb that sends UNMAP for every free
    // block, which is exactly the point here.
    let script =
        format!("$ErrorActionPreference='Stop'; Optimize-Volume -DriveLetter {letter} -ReTrim");
    let out = crate::proc::command("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output();

    match out {
        Ok(out) if out.status.success() => TrimOutcome::Trimmed,
        Ok(out) => TrimOutcome::from_message(&String::from_utf8_lossy(&out.stderr)),
        Err(e) => TrimOutcome::Failed(format!("could not run Optimize-Volume: {e}")),
    }
}

impl TrimOutcome {
    /// Read a tool's complaint and decide whether it is a real failure.
    ///
    /// "The drive will not do this" is the common, boring case and should not
    /// be dressed up as an error.
    fn from_message(stderr: &str) -> TrimOutcome {
        let lowered = stderr.to_lowercase();
        let unsupported = [
            "discard operation is not supported",
            "operation not supported",
            "not supported",
            "does not support",
        ];
        if unsupported.iter().any(|needle| lowered.contains(needle)) {
            return TrimOutcome::Unsupported;
        }
        let message = stderr.trim();
        TrimOutcome::Failed(if message.is_empty() {
            "the drive refused, without saying why".to_string()
        } else {
            crate::format::friendly_pkexec_error(message)
        })
    }

    /// A sentence for the wizard's status line.
    pub fn message(&self) -> String {
        match self {
            TrimOutcome::Trimmed => "Free space released back to the drive.".to_string(),
            TrimOutcome::Unsupported => {
                "This enclosure does not pass TRIM through, so the drive keeps treating deleted \
                 blocks as used. Nothing is broken; it just means leaving spare room matters more."
                    .to_string()
            }
            TrimOutcome::Failed(why) => format!("Free space was not released: {why}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unsupported_bridge_is_not_an_error() {
        // What fstrim says on an enclosure that drops UNMAP.
        assert_eq!(
            TrimOutcome::from_message(
                "fstrim: /run/media/harry/CINDER: the discard operation is not supported"
            ),
            TrimOutcome::Unsupported
        );
        // And roughly what Windows says.
        assert_eq!(
            TrimOutcome::from_message("Optimize-Volume : The storage does not support ReTrim"),
            TrimOutcome::Unsupported
        );
    }

    #[test]
    fn a_real_failure_keeps_what_the_tool_said() {
        let outcome = TrimOutcome::from_message("fstrim: command not found");
        assert_eq!(
            outcome,
            TrimOutcome::Failed("fstrim: command not found".to_string())
        );
        assert!(outcome.message().contains("command not found"));
    }

    #[test]
    fn silence_still_produces_a_sentence() {
        let outcome = TrimOutcome::from_message("   \n");
        assert!(matches!(outcome, TrimOutcome::Failed(_)));
        assert!(outcome.message().contains("without saying why"));
    }

    #[test]
    fn an_unsupported_bridge_explains_what_it_means_for_the_user() {
        let said = TrimOutcome::Unsupported.message();
        assert!(said.contains("spare room"), "{said}");
        assert!(said.contains("Nothing is broken"), "{said}");
    }

    #[test]
    fn a_vanished_cartridge_is_refused_before_anything_runs() {
        assert!(matches!(
            trim("/nonexistent/cartridge"),
            TrimOutcome::Failed(_)
        ));
    }
}
