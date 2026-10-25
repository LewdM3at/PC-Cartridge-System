//! Windows settings that cost a cartridge real performance, and undoing them.
//!
//! Two of them matter enough to offer:
//!
//!   * **Defender scans the whole game folder** the first time anything reads
//!     it. On 60 GB of freshly copied files that is minutes of contention on a
//!     link that is already the slowest part of the machine.
//!   * **Search indexing** crawls the volume for the same reason, in the
//!     background, whenever it is plugged in.
//!
//! Both are opt-in, both are per-cartridge, and both can be put back — a tool
//! that quietly weakens someone's malware scanning and cannot undo it is not
//! a tool anyone should trust. Nothing here runs unless it is asked for.
//!
//! The Defender exclusion is the one with a memory. `Add-MpPreference` writes a
//! persistent record keyed on a path, and the only path a cartridge has is a
//! drive letter Windows hands out from whatever happens to be free — so the
//! cartridge that was `D:` last week is `H:` today, and `D:` is now something
//! else that nobody chose to stop scanning. Every apply reconciles that; see
//! `stale_drive_exclusions`.
//!
//! A third setting is worth changing and is deliberately *not* automated:
//! Windows sets removable drives to "Quick removal", which turns write caching
//! off. "Better performance" is the right choice for a cartridge that is always
//! ejected properly — which this project's launcher does — but the supported
//! way to set it is Device Manager, and the registry keys behind it are
//! per-device and undocumented. It is in the README as a manual step instead of
//! being guessed at here.

/// What was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tweak {
    /// Stop Defender's real-time scanner walking the cartridge.
    DefenderExclusion,
    /// Stop Search indexing the volume.
    SearchIndexing,
}

impl Tweak {
    pub fn describe(self, applying: bool) -> &'static str {
        match (self, applying) {
            (Tweak::DefenderExclusion, true) => "Excluded the cartridge from Defender scanning.",
            (Tweak::DefenderExclusion, false) => "Defender scans the cartridge again.",
            (Tweak::SearchIndexing, true) => "Search indexing switched off for the cartridge.",
            (Tweak::SearchIndexing, false) => "Search indexing switched back on.",
        }
    }
}

/// The PowerShell for one tweak, in one direction.
///
/// Built as a pure function so the commands can be read and tested on any
/// machine, rather than only being visible when they run on Windows.
pub fn script_for(tweak: Tweak, drive: &str, applying: bool) -> Result<String, String> {
    let letter = drive_letter(drive)?;
    Ok(match (tweak, applying) {
        (Tweak::DefenderExclusion, true) => {
            format!("Add-MpPreference -ExclusionPath '{letter}:\\'")
        }
        (Tweak::DefenderExclusion, false) => {
            format!("Remove-MpPreference -ExclusionPath '{letter}:\\'")
        }
        (Tweak::SearchIndexing, applying) => format!(
            "Get-CimInstance -ClassName Win32_Volume -Filter \"DriveLetter='{letter}:'\" | \
             Set-CimInstance -Property @{{IndexingEnabled=${}}}",
            if applying { "false" } else { "true" }
        ),
    })
}

/// Exclusions naming a drive root that is not a cartridge any more.
///
/// A cartridge cannot be excluded by anything steadier than its letter.
/// `\\?\Volume{...}\` would be steady, but Defender does not document
/// accepting it, and an exclusion that is silently ignored is worse than none:
/// it reads as protection on a screen while the scanner carries on regardless.
/// So the letter stays, and the drift it causes is cleaned up instead.
///
/// Only bare roots count — `D:\`, which is the shape this tool writes. A path
/// any deeper was written by somebody else, on purpose, and is not ours to
/// touch.
pub fn stale_drive_exclusions(exclusions: &[String], cartridges: &[String]) -> Vec<String> {
    let live: Vec<char> = cartridges
        .iter()
        .filter_map(|root| drive_letter(root).ok())
        .collect();

    exclusions
        .iter()
        .filter(|exclusion| root_letter(exclusion).is_some_and(|letter| !live.contains(&letter)))
        .cloned()
        .collect()
}

/// The letter of an exclusion that is exactly a drive root, and nothing else.
fn root_letter(exclusion: &str) -> Option<char> {
    let trimmed = exclusion.trim();
    let mut chars = trimmed.chars();
    let letter = chars.next()?;
    if !letter.is_ascii_alphabetic() {
        return None;
    }
    let rest: String = chars.collect();
    matches!(rest.as_str(), ":" | ":\\" | ":/").then_some(letter.to_ascii_uppercase())
}

/// Take one exclusion back off.
pub fn drop_exclusion_script(path: &str) -> String {
    format!("Remove-MpPreference -ExclusionPath '{path}'")
}

/// Everything a run would do, for showing before anything is changed.
///
/// The user sees the actual commands. This is elevated, it touches malware
/// scanning, and "trust me" is not good enough.
pub fn plan(drive: &str, tweaks: &[Tweak], applying: bool) -> Result<Vec<String>, String> {
    plan_with(
        &stale_paths(drive, tweaks, applying),
        drive,
        tweaks,
        applying,
    )
}

/// The plan, given the exclusions to take back.
///
/// Split out so it can be tested: the list of stale exclusions comes from
/// whatever Defender happens to be configured with on the machine running the
/// test, and a test that reads that is a test that passes or fails depending on
/// which drives are plugged in.
fn plan_with(
    stale: &[String],
    drive: &str,
    tweaks: &[Tweak],
    applying: bool,
) -> Result<Vec<String>, String> {
    let mut commands: Vec<String> = stale
        .iter()
        .map(|path| drop_exclusion_script(path))
        .collect();

    for tweak in tweaks {
        commands.push(script_for(*tweak, drive, applying)?);
    }
    Ok(commands)
}

/// Exclusions this run should take back, or nothing when it should not.
///
/// Only while adding one: undoing is already a removal, and a run that put
/// nothing back would be reaching past what it was asked to do.
fn stale_paths(drive: &str, tweaks: &[Tweak], applying: bool) -> Vec<String> {
    if !applying || !tweaks.contains(&Tweak::DefenderExclusion) {
        return Vec::new();
    }
    stale_drive_exclusions(&defender_exclusions(), &cartridge_roots(drive))
}

/// Every drive that currently holds a cartridge, plus the one being tuned.
///
/// The one being tuned is included outright: it counts from the moment the
/// wizard is pointed at it, which can be before anything has been written on
/// it, and excluding a drive and then immediately un-excluding it would be a
/// remarkable way to spend a UAC prompt.
#[cfg(windows)]
fn cartridge_roots(drive: &str) -> Vec<String> {
    crate::drives::list_drives()
        .into_iter()
        .filter(|target| target.has_cartridge)
        .map(|target| target.path)
        .chain(std::iter::once(drive.to_string()))
        .collect()
}

#[cfg(not(windows))]
fn cartridge_roots(drive: &str) -> Vec<String> {
    vec![drive.to_string()]
}

/// Every path Defender is currently told to skip.
///
/// Read without elevation — `Get-MpPreference` answers to anyone — so the plan
/// is complete before the prompt that changes anything.
#[cfg(windows)]
fn defender_exclusions() -> Vec<String> {
    let Ok(out) = crate::proc::command("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "(Get-MpPreference).ExclusionPath",
        ])
        .output()
    else {
        return Vec::new();
    };

    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(not(windows))]
fn defender_exclusions() -> Vec<String> {
    Vec::new()
}

/// A drive letter, from whatever form the window sent.
fn drive_letter(drive: &str) -> Result<char, String> {
    let trimmed = drive.trim().trim_end_matches('\\').trim_end_matches(':');
    let mut chars = trimmed.chars();
    match (chars.next(), chars.next()) {
        (Some(letter), None) if letter.is_ascii_alphabetic() => Ok(letter.to_ascii_uppercase()),
        _ => Err(format!("{drive} is not a drive letter")),
    }
}

/// Apply (or undo) the tweaks. Windows only; elsewhere there is nothing to do.
#[cfg(windows)]
pub fn apply(drive: &str, tweaks: &[Tweak], applying: bool) -> Result<Vec<String>, String> {
    let mut done = Vec::new();

    let stale = stale_paths(drive, tweaks, applying);
    for path in &stale {
        elevate(&drop_exclusion_script(path)).map_err(|e| {
            format!("{path} could not be taken off Defender's list. Nothing else was changed. {e}")
        })?;
    }
    if !stale.is_empty() {
        done.push(format!(
            "Defender scans {} again — left excluded by a cartridge that has since moved letter.",
            stale.join(", ")
        ));
    }

    for tweak in tweaks {
        let script = script_for(*tweak, drive, applying)?;
        elevate(&script).map_err(|e| {
            format!(
                "{} did not go through. Nothing else was changed. {e}",
                tweak.describe(applying)
            )
        })?;
        done.push(tweak.describe(applying).to_string());
    }
    Ok(done)
}

/// Run one command as administrator.
///
/// Each is elevated on its own, so the wizard itself never has to run as
/// administrator, and a run that is declined halfway leaves behind only the
/// steps that were already agreed to.
///
/// `-PassThru` and the explicit `exit` are the whole point of the shape.
/// `Start-Process -Wait` reports whether *it* managed to start something, not
/// what that something then did — so without them a cmdlet that failed inside
/// the elevated window was indistinguishable from one that worked, which is how
/// a `Remove-MpPreference` that removed nothing got reported as a tidied-up
/// exclusion. `$ErrorActionPreference` is escaped so the outer shell hands it
/// on rather than expanding it here, where it would mean nothing.
#[cfg(windows)]
fn elevate(script: &str) -> Result<(), String> {
    let status = crate::proc::command("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!(
                "$p = Start-Process powershell -Verb RunAs -Wait -PassThru -WindowStyle Hidden \
                 -ArgumentList '-NoProfile','-NonInteractive','-Command',\
                 \"`$ErrorActionPreference='Stop'; {}\"; exit $p.ExitCode",
                script.replace('"', "`\"")
            ),
        ])
        .status()
        .map_err(|e| format!("could not run PowerShell: {e}"))?;

    match status.code() {
        Some(0) => Ok(()),
        // 1223 is ERROR_CANCELLED arriving through Start-Process: the prompt
        // appeared and was dismissed, which is an answer rather than a fault
        // and should not be read out as one.
        Some(1223) => Err("the administrator prompt was dismissed.".to_string()),
        Some(code) => Err(format!("PowerShell exited with {code}.")),
        None => Err("PowerShell was stopped before it finished.".to_string()),
    }
}

#[cfg(not(windows))]
pub fn apply(_drive: &str, _tweaks: &[Tweak], _applying: bool) -> Result<Vec<String>, String> {
    Err("These settings only exist on Windows.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_letters_are_accepted_in_the_forms_the_window_sends() {
        assert_eq!(drive_letter("D:\\").unwrap(), 'D');
        assert_eq!(drive_letter("D:").unwrap(), 'D');
        assert_eq!(drive_letter("d").unwrap(), 'D');
        assert!(drive_letter("/run/media/harry/CINDER").is_err());
        assert!(drive_letter("").is_err());
    }

    #[test]
    fn every_tweak_has_an_exact_opposite() {
        for tweak in [Tweak::DefenderExclusion, Tweak::SearchIndexing] {
            let on = script_for(tweak, "D:\\", true).unwrap();
            let off = script_for(tweak, "D:\\", false).unwrap();
            assert_ne!(on, off, "{tweak:?} undoes to the same command");
            assert!(on.contains("D:"), "{on}");
            assert!(off.contains("D:"), "{off}");
        }
    }

    #[test]
    fn the_defender_exclusion_is_added_and_removed_by_path() {
        assert_eq!(
            script_for(Tweak::DefenderExclusion, "D:\\", true).unwrap(),
            "Add-MpPreference -ExclusionPath 'D:\\'"
        );
        assert_eq!(
            script_for(Tweak::DefenderExclusion, "D:\\", false).unwrap(),
            "Remove-MpPreference -ExclusionPath 'D:\\'"
        );
    }

    #[test]
    fn indexing_is_turned_off_and_on_for_that_volume_only() {
        let off = script_for(Tweak::SearchIndexing, "E:", true).unwrap();
        assert!(off.contains("DriveLetter='E:'"), "{off}");
        assert!(off.contains("IndexingEnabled=$false"), "{off}");

        let on = script_for(Tweak::SearchIndexing, "E:", false).unwrap();
        assert!(on.contains("IndexingEnabled=$true"), "{on}");
    }

    #[test]
    fn a_root_exclusion_is_stale_unless_a_cartridge_is_on_that_letter() {
        let excluded = |paths: &[&str]| paths.iter().map(|p| p.to_string()).collect::<Vec<_>>();

        // The case that started this. A cartridge was tuned while it was D:,
        // Windows gave it H: the next time, and the exclusion stayed pointing
        // at a letter that now belongs to something else — or to nothing.
        assert_eq!(
            stale_drive_exclusions(&excluded(&["D:\\", "H:\\"]), &excluded(&["H:\\", "G:\\"])),
            vec!["D:\\".to_string()]
        );

        // Written in any of the shapes Windows hands back.
        assert_eq!(
            stale_drive_exclusions(&excluded(&["d:", "E:/"]), &["G:\\".to_string()]),
            vec!["d:".to_string(), "E:/".to_string()]
        );

        // Somebody else's exclusion, on purpose, and deeper than a root. Not
        // this tool's to remove however long it has been there.
        assert!(stale_drive_exclusions(
            &excluded(&[
                r"C:\Users\Harry\.gradle",
                r"D:\Games",
                r"\\server\share",
                "%ProgramData%",
            ]),
            &[]
        )
        .is_empty());

        // Every cartridge accounted for, nothing to do.
        assert!(
            stale_drive_exclusions(&excluded(&["G:\\"]), &excluded(&["G:\\", "H:\\"])).is_empty()
        );
    }

    #[test]
    fn cleaning_up_is_offered_only_while_adding_an_exclusion() {
        // Undoing is already a removal; sweeping other entries at the same time
        // would be doing something nobody asked for under cover of a UAC prompt
        // they agreed to for a different reason.
        assert!(stale_paths("G:\\", &[Tweak::DefenderExclusion], false).is_empty());
        // And indexing has no persistent record to go stale in the first place.
        assert!(stale_paths("G:\\", &[Tweak::SearchIndexing], true).is_empty());
    }

    #[test]
    fn the_plan_is_the_commands_themselves() {
        let tweaks = [Tweak::DefenderExclusion, Tweak::SearchIndexing];

        let commands = plan_with(&[], "D:\\", &tweaks, true).unwrap();
        assert_eq!(commands.len(), 2);
        assert!(commands[0].starts_with("Add-MpPreference"));

        // Anything to take back comes first, so the drive is not excluded and
        // then un-excluded by a later line in the same run.
        let with_stale = plan_with(&["E:\\".to_string()], "D:\\", &tweaks, true).unwrap();
        assert_eq!(with_stale.len(), 3);
        assert_eq!(with_stale[0], "Remove-MpPreference -ExclusionPath 'E:\\'");
        assert!(with_stale[1].starts_with("Add-MpPreference"));

        // A bad drive fails the whole plan rather than half of it.
        assert!(plan_with(&[], "nonsense", &[Tweak::DefenderExclusion], true).is_err());
    }
}
