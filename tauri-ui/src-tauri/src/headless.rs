//! The launcher with no window, for front-ends that have their own buttons.
//!
//!   pc-gamepak --drive D:\ --play <n>          play game n, stay up while it runs
//!   pc-gamepak --drive D:\ --safe-eject        eject if nothing is using it
//!   pc-gamepak --drive D:\ --safe-eject --force   close what is, then eject
//!
//! Both do exactly what the window's buttons do, through the same functions, so
//! a cartridge played from Playnite carries the same saves, hours and shader
//! caches as one played from the window. What they cannot do is ask anything:
//! where the window would put up a dialog, `--play` opens the window instead and
//! `--safe-eject` reports and lets its caller ask.
//!
//! The timing rules for "is the game still running" are in
//! [`gamepak_core::play`], where they are tested.

use std::io::Write;
use std::path::Path;
use std::time::Instant;

use gamepak_core::cartridge;
use gamepak_core::play::{self, Step, Watch};
use gamepak_core::{busy, settings, stats};

use crate::{
    cartridge_busy, debug_log, eject_drive, eject_drive_forcing, end_every_session, launch_game,
    pull_shaders, push_saves, push_shaders, sync_saves,
};

/// `--play <n>`, if it was given. `Some(Err)` for a value that is not a number,
/// so a typo is reported rather than quietly opening the window.
pub fn play_index(args: &[String]) -> Option<Result<usize, String>> {
    let at = args.iter().position(|arg| arg == "--play")?;
    let value = args.get(at + 1).map(String::as_str).unwrap_or("");
    Some(
        value
            .parse::<usize>()
            .map_err(|_| format!("--play wants a game number, not {value:?}")),
    )
}

pub fn wants_safe_eject(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--safe-eject")
}

/// Play one game and return when it is over. The exit code is 0 unless the game
/// could not be started at all.
pub fn play(drive: &str, index: usize) -> i32 {
    let root = Path::new(drive);
    let info = match cartridge::read_cartridge_info(drive) {
        Ok(info) => info,
        Err(why) => return fail(&why),
    };
    let pick = match play::pick(&info, index) {
        Ok(pick) => pick,
        Err(why) => return fail(&why),
    };

    // What the window does as it opens. A session that never closed is settled
    // first so its hours are not added to this one.
    if settings::load().track_playtime {
        stats::recover(root);
    }
    let saves = sync_saves(drive.to_string()).unwrap_or_default();
    if play::needs_a_person(&saves) {
        // Both copies of a save changed. Only the window can ask which to keep,
        // and starting the game first would make a third.
        debug_log("play: a save conflicts; opening the window to ask".into());
        return open_window(drive);
    }
    let shaders = {
        let drive = drive.to_string();
        std::thread::spawn(move || pull_shaders(drive))
    };

    // Taken before the game starts, so an Eject pressed at any point from here
    // on finds this process and waits for it.
    let player = handoff::Player::start(drive);

    if let Err(why) = launch_game(
        pick.executable.clone(),
        drive.to_string(),
        Some(pick.title.clone()),
    ) {
        let _ = shaders.join();
        return fail(&why);
    }

    // A program path rather than a URI is on the drive by definition, wherever
    // in it the cartridge keeps it; the folder layout only covers the wizard's.
    let on_the_drive = info.holds_game || !gamepak_core::launch::is_uri(&pick.executable);
    let eject_asked = if on_the_drive {
        let install_dir = play::steam_install_dir(root, &pick.executable);
        watch_until_over(root, install_dir.as_deref(), &player, &pick.title)
    } else {
        // Nothing on the drive to watch: the game lives elsewhere and this is a
        // key to it. Closed at once, as auto-launch does, rather than left open
        // to be settled as zero on some later insert.
        debug_log("play: the game is not on the cartridge, so its end cannot be seen".into());
        false
    };

    let _ = shaders.join();
    end_every_session();
    if eject_asked {
        // The eject that asked is about to take the saves and caches back
        // itself, with nothing running. Doing it here too would be two writers.
        debug_log("play: eject asked; session closed, leaving the rest to it".into());
        return 0;
    }
    // Everything the window does on the way out, now rather than at eject: this
    // process is the only thing that knows the game ended, and a cartridge
    // pulled later without Eject should not be carrying a stale save.
    if root.exists() {
        let _ = push_saves(drive.to_string());
        push_shaders(drive.to_string());
    }
    0
}

/// Wait for the game to end. True if an eject asked this to stop instead.
fn watch_until_over(
    root: &Path,
    install_dir: Option<&str>,
    player: &handoff::Player,
    title: &str,
) -> bool {
    let me = [std::process::id()];
    let started = Instant::now();
    let mut watch = Watch::default();

    loop {
        if player.eject_asked_within(play::POLL) {
            return true;
        }
        if !root.exists() {
            debug_log("play: the cartridge went away".into());
            return false;
        }
        let holders = busy::holders(root).excluding(&me);
        // The same game run from another Steam library counts: Steam picks
        // whichever copy it likes, and the session is the game's, not the drive's.
        let elsewhere = match install_dir {
            Some(dir) if holders.is_empty() => {
                busy::running_where(&|exe| play::is_in_steam_install(exe, dir))
            }
            _ => Vec::new(),
        };
        let was_seen = watch.seen();
        match watch.observe(
            !holders.is_empty() || !elsewhere.is_empty(),
            started.elapsed(),
        ) {
            Step::Wait if !was_seen && watch.seen() => debug_log(format!(
                "play: {title} is running after {}s: {}",
                started.elapsed().as_secs(),
                if holders.is_empty() {
                    format!("from another Steam library, pid {elsewhere:?}")
                } else {
                    format!("from the cartridge: {}", holders.summary(3))
                }
            )),
            Step::Wait => {}
            Step::Ended => {
                debug_log(format!(
                    "play: {title} ended; {}s since launch",
                    started.elapsed().as_secs()
                ));
                return false;
            }
            Step::NeverSeen => {
                debug_log("play: nothing started from the cartridge; giving up watching".into());
                return false;
            }
        }
    }
}

/// Stop any `--play` on this drive before it is ejected, and wait for it.
///
/// Called by every eject, window or not, before the saves are pushed. A player
/// left running would close its session a moment later against a volume that
/// has gone — leaving the cartridge with a session that never closes — and push
/// saves after the dismount, which mounts the volume straight back.
pub fn stop_players(drive: &str) {
    handoff::stop_players(drive);
}

/// How a player and an eject find each other: a named event the eject sets, and
/// a named mutex the player holds for as long as it runs. Per drive letter, per
/// session. Nothing on Linux yet, where no front-end runs `--play`.
#[cfg(target_os = "windows")]
mod handoff {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_ABANDONED, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        CreateEventW, CreateMutexW, OpenEventW, OpenMutexW, ReleaseMutex, SetEvent,
        WaitForSingleObject, EVENT_MODIFY_STATE, SYNCHRONIZATION_SYNCHRONIZE,
    };

    /// Long enough for a player to close a session and let go of a shader copy
    /// in progress; short enough that a player stuck on a dead drive does not
    /// hold an eject hostage.
    const WAIT_FOR_PLAYER_MS: u32 = 20_000;

    fn names(drive: &str) -> (Vec<u16>, Vec<u16>) {
        let letter: String = drive
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_uppercase();
        let wide = |s: String| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        (
            wide(format!("Local\\PCGamePak-Eject-{letter}")),
            wide(format!("Local\\PCGamePak-Playing-{letter}")),
        )
    }

    pub struct Player {
        event: isize,
        mutex: isize,
    }

    impl Player {
        pub fn start(drive: &str) -> Player {
            let (event, mutex) = names(drive);
            // SAFETY: names are NUL-terminated and outlive the calls; a null
            // handle back is tolerated everywhere below.
            unsafe {
                Player {
                    event: CreateEventW(std::ptr::null(), 1, 0, event.as_ptr()),
                    mutex: CreateMutexW(std::ptr::null(), 1, mutex.as_ptr()),
                }
            }
        }

        /// Sleeps for `wait`, or less if an eject asks first.
        pub fn eject_asked_within(&self, wait: std::time::Duration) -> bool {
            if self.event == 0 {
                std::thread::sleep(wait);
                return false;
            }
            unsafe { WaitForSingleObject(self.event, wait.as_millis() as u32) == WAIT_OBJECT_0 }
        }
    }

    impl Drop for Player {
        fn drop(&mut self) {
            unsafe {
                if self.mutex != 0 {
                    ReleaseMutex(self.mutex);
                    CloseHandle(self.mutex);
                }
                if self.event != 0 {
                    CloseHandle(self.event);
                }
            }
        }
    }

    pub fn stop_players(drive: &str) {
        let (event, mutex) = names(drive);
        unsafe {
            let mutex = OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, 0, mutex.as_ptr());
            if mutex == 0 {
                return; // Nobody is playing from this drive.
            }
            let event = OpenEventW(EVENT_MODIFY_STATE, 0, event.as_ptr());
            if event != 0 {
                SetEvent(event);
            }
            let waited = WaitForSingleObject(mutex, WAIT_FOR_PLAYER_MS);
            if waited == WAIT_OBJECT_0 || waited == WAIT_ABANDONED {
                ReleaseMutex(mutex);
            }
            CloseHandle(mutex);
            if event != 0 {
                CloseHandle(event);
            }
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod handoff {
    pub struct Player;

    impl Player {
        pub fn start(_drive: &str) -> Player {
            Player
        }

        pub fn eject_asked_within(&self, wait: std::time::Duration) -> bool {
            std::thread::sleep(wait);
            false
        }
    }

    pub fn stop_players(_drive: &str) {}
}

/// Eject without a window, reporting on stdout for whoever ran it.
///
/// The first line is one word — `ejected`, `busy` or `error` — and the lines
/// after it are for a person: the result, or one line per program still using
/// the drive. Exit code 0, 2 and 1 to match, so a caller can use either.
pub fn safe_eject(drive: &str, force: bool) -> i32 {
    if force {
        return match eject_drive_forcing(drive.to_string()) {
            Ok(message) => report("ejected", &[message], 0),
            Err(why) => report("error", &[why], 1),
        };
    }

    let holders = cartridge_busy(drive.to_string());
    if !holders.is_empty() {
        let lines: Vec<String> = holders.holders.iter().map(|h| h.describe()).collect();
        return report("busy", &lines, 2);
    }
    match eject_drive(drive.to_string()) {
        Ok(()) => report("ejected", &["Safe to remove.".to_string()], 0),
        Err(why) => report("error", &[why], 1),
    }
}

fn report(word: &str, lines: &[String], code: i32) -> i32 {
    // Errors writing are ignored: a GUI-subsystem process started without a
    // redirected stdout has nowhere to write, and the exit code still says it.
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{word}");
    for line in lines {
        let _ = writeln!(out, "{}", line.replace(['\r', '\n'], " "));
    }
    let _ = out.flush();
    code
}

fn fail(why: &str) -> i32 {
    debug_log(format!("play: {why}"));
    eprintln!("{why}");
    1
}

/// The window, for the one thing `--play` cannot do without it.
fn open_window(drive: &str) -> i32 {
    let Ok(me) = std::env::current_exe() else {
        return 1;
    };
    match std::process::Command::new(me)
        .args(["--drive", drive, "--show"])
        .spawn()
    {
        Ok(_) => 0,
        Err(why) => fail(&format!("could not open the window: {why}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn play_takes_a_number() {
        assert_eq!(
            play_index(&args(&["--drive", "D:\\", "--play", "2"])),
            Some(Ok(2))
        );
        assert!(matches!(
            play_index(&args(&["--play", "two"])),
            Some(Err(_))
        ));
        assert!(matches!(play_index(&args(&["--play"])), Some(Err(_))));
        assert_eq!(play_index(&args(&["--drive", "D:\\", "--show"])), None);
    }

    #[test]
    fn safe_eject_is_its_own_flag_not_the_elevated_one() {
        assert!(wants_safe_eject(&args(&[
            "--drive",
            "D:\\",
            "--safe-eject"
        ])));
        assert!(!wants_safe_eject(&args(&["--eject", "D:"])));
    }
}
