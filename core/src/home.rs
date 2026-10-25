//! The game's home directory, on the cartridge.
//!
//! [`crate::saves`] carries saves by declaring where they are: a `save=` line
//! per game, a token the host resolves, and a copy in whichever direction
//! changed. That works for a game the cartridge only *points at* — a Steam
//! game launched through Steam, whose environment this project never touches —
//! and it has the cost every declaration has. Somebody has to find out where
//! the game keeps its saves, write it down, and be right.
//!
//! For a game the cartridge **carries** and this launcher **starts itself**,
//! none of that is necessary, and Kazeta is the reason this is written down.
//! Reading it closely, its save capture is not the overlayfs at all — the
//! overlay is there so a read-only cart can be written to. The capture is one
//! line: `export HOME="${BASE_DIR}/run/cart"`. Point a game's home at the
//! writable layer and every save it writes lands there, with nobody having
//! researched anything.
//!
//! A PC GamePak cartridge is already writable, so it does not need the overlay
//! to get the same result. It needs the environment, which costs no privileges,
//! no mount, no root and no dependency:
//!
//! | | |
//! |---|---|
//! | Linux | `HOME`, and the four `XDG_*_HOME` directories |
//! | macOS | `HOME`, which `~/Library` follows on its own |
//! | Windows | `USERPROFILE`, `APPDATA`, `LOCALAPPDATA` |
//!
//! # Why it is off unless a cartridge asks
//!
//! Changing a game's idea of home is not free. A game that keeps its settings
//! per machine will find none and start at its defaults; one that expects
//! something else in `$HOME` may not start at all. The cartridge's author knows
//! whether their game is portable and the host does not, so the cartridge says:
//!
//! ```text
//! executable=Game/start.sh
//! portable_home=yes
//! ```
//!
//! # What this does not catch
//!
//! * **A game launched by something else.** A `steam://` cartridge is started
//!   by Steam, in Steam's environment. `save=` lines are still the answer there.
//! * **An absolute path outside home.** A game writing to `/var/games` escapes
//!   this as surely as it escapes a declaration.
//! * **Symlinks, on exFAT.** A game whose config directory contains one will
//!   fail to create it. btrfs and NTFS cartridges have no such problem, and
//!   `docs/MANUAL.md` already recommends against exFAT for carried games.
//!
//! The cache is the one directory deliberately left on the host. It is
//! rebuildable by definition, it is the largest and most rewritten thing a game
//! produces, and putting it on removable flash costs write cycles and speed to
//! preserve something nobody wants preserved.
//!
//! One part of it *is* worth carrying, and is carried elsewhere: the compiled
//! shader cache, by [`crate::shaders`], because that one's loss is felt as an
//! hour of stutter rather than a few seconds of rebuilding. It is copied at
//! insert and eject rather than written to over USB all session, which is the
//! difference between carrying a cache and living on one.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// Where a carried game's home goes, under [`crate::create::ASSET_DIR`].
pub const HOME_DIR: &str = "home";

/// The environment a carried game should be started with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableHome {
    /// The home directory itself, on the cartridge.
    pub root: String,
    /// Variables to set on the child process, in the order they were derived.
    pub vars: Vec<(String, String)>,
}

impl PortableHome {
    /// The value of one variable, for tests and for the details sheet.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.vars
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Whether this cartridge asked for its own home directory.
///
/// `portable_home` in the general or `[collection]` section — a property of the
/// cartridge rather than of one game, because it decides what environment the
/// launcher builds and there is only one of those.
pub fn wanted(root: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(root.join("cartridge.conf")) else {
        return false;
    };
    wanted_in(&text)
}

/// The same question against the text of a `cartridge.conf`.
pub fn wanted_in(conf: &str) -> bool {
    let map = crate::cartridge::parse_ini(conf);
    // A single-game cartridge has no sections and lands in `general`; a bundle
    // puts its cartridge-wide keys in `[collection]`. Checking both means an
    // author does not have to know which one this reads.
    ["general", "collection"]
        .iter()
        .filter_map(|section| crate::cartridge::ini_get(&map, section, "portable_home"))
        .any(|value| is_yes(value))
}

/// The spellings a person might reasonably write.
///
/// Deliberately generous on the yes side and silent on everything else: a
/// cartridge that says `portable_home=maybe` gets the safe answer rather than an
/// error nobody will see, because this is read on insert and there is no one to
/// tell.
fn is_yes(value: &str) -> bool {
    matches!(
        value.trim().to_lowercase().as_str(),
        "yes" | "y" | "true" | "1" | "on" | "portable"
    )
}

/// Build the home directory on the cartridge and the environment for it.
///
/// Creates the directories rather than leaving the game to: a game that cannot
/// create `$XDG_DATA_HOME` usually does not say so, it just loses the save. They
/// are made once, on the first launch, and cost nothing after that.
pub fn prepare(root: &Path) -> Result<PortableHome, String> {
    let home = root.join(crate::create::ASSET_DIR).join(HOME_DIR);
    std::fs::create_dir_all(&home).map_err(|e| format!("{}: {e}", home.display()))?;

    let mut vars: Vec<(String, String)> = Vec::new();
    let mut set = |name: &str, path: &Path| {
        vars.push((name.to_string(), path.display().to_string()));
    };

    set("HOME", &home);

    #[cfg(target_os = "windows")]
    {
        // Windows has no HOME, and the three that matter are these. Set anyway
        // above, because a game built with a cross-platform toolkit may look
        // for it.
        let appdata = home.join("AppData").join("Roaming");
        let local = home.join("AppData").join("Local");
        for dir in [&appdata, &local] {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        set("USERPROFILE", &home);
        set("APPDATA", &appdata);
        set("LOCALAPPDATA", &local);
    }

    #[cfg(target_os = "macos")]
    {
        // `~/Library/Application Support` and `~/Library/Preferences` are
        // derived from HOME by the frameworks themselves, so there is nothing
        // else to set. Created so a game that writes without checking finds
        // them there.
        for rest in ["Library/Application Support", "Library/Preferences"] {
            let dir = home.join(rest);
            std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let data = home.join(".local").join("share");
        let config = home.join(".config");
        let state = home.join(".local").join("state");
        for dir in [&data, &config, &state] {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        set("XDG_DATA_HOME", &data);
        set("XDG_CONFIG_HOME", &config);
        set("XDG_STATE_HOME", &state);
    }

    // The cache stays on the host, whatever the platform: rebuildable by
    // definition, the biggest and most rewritten thing a game produces, and not
    // worth the write cycles. The shaders inside it are the exception and are
    // carried by `shaders`, which copies them twice per session instead of
    // writing to the drive all session long. Per cartridge, so two cartridges do
    // not fight over one directory.
    let cache = cache_dir(root);
    if std::fs::create_dir_all(&cache).is_ok() {
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        set("XDG_CACHE_HOME", &cache);
        #[cfg(any(target_os = "windows", target_os = "macos"))]
        let _ = &cache;
    }

    Ok(PortableHome {
        root: home.display().to_string(),
        vars,
    })
}

/// Somewhere on the host for this cartridge's throwaway files.
///
/// Named after the cartridge's own directory rather than shared, so two
/// cartridges plugged in together do not write over each other's caches. In the
/// system temp directory because that is the one place every platform agrees is
/// allowed to be emptied.
pub fn cache_dir(root: &Path) -> PathBuf {
    let tag: String = root
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "cartridge".to_string())
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    std::env::temp_dir()
        .join("pc-gamepak-cache")
        .join(tag.trim_matches('-'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Scratch;

    #[test]
    fn a_cartridge_that_says_nothing_does_not_get_one() {
        // Changing a game's home can stop it working, so silence means no.
        assert!(!wanted_in("executable=Game/start.sh\ntitle=X\n"));
        assert!(!wanted_in(""));
    }

    #[test]
    fn the_spellings_somebody_would_actually_write_are_accepted() {
        for yes in ["yes", "Yes", "YES", "true", "1", "on", " portable "] {
            assert!(
                wanted_in(&format!("executable=x\nportable_home={yes}\n")),
                "{yes:?} should mean yes"
            );
        }
        for no in ["no", "false", "0", "off", "", "maybe", "sometimes"] {
            assert!(
                !wanted_in(&format!("executable=x\nportable_home={no}\n")),
                "{no:?} should not mean yes"
            );
        }
    }

    #[test]
    fn a_bundle_can_ask_for_it_in_its_collection_section() {
        let conf = "[collection]\ntitle=Two\nportable_home=yes\n\n\
                    [game]\ntitle=A\nexecutable=Games/a/start.sh\n";
        assert!(wanted_in(conf));
    }

    #[test]
    fn asking_reads_the_cartridge_on_the_drive() {
        let scratch = Scratch::new("home-wanted");
        assert!(!wanted(scratch.path()), "no conf at all");
        scratch.write("cartridge.conf", b"executable=x\nportable_home=yes\n");
        assert!(wanted(scratch.path()));
    }

    #[test]
    fn the_home_is_built_on_the_cartridge() {
        let scratch = Scratch::new("home-prepare");
        let home = prepare(scratch.path()).expect("prepare");

        // Joined a component at a time: the code under test builds the path the
        // same way, and on Windows that means a backslash, not the slash a
        // single joined string would keep.
        let expected = scratch.join(".gamepak").join("home");
        assert_eq!(home.root, expected.display().to_string());
        assert!(expected.is_dir(), "the directory should exist already");
        assert_eq!(home.get("HOME"), Some(home.root.as_str()));
    }

    #[test]
    fn every_directory_the_environment_names_exists() {
        // A game that cannot create its own config directory usually does not
        // say so — it loses the save quietly.
        let scratch = Scratch::new("home-dirs");
        let home = prepare(scratch.path()).expect("prepare");
        for (name, path) in &home.vars {
            assert!(
                Path::new(path).is_dir(),
                "{name} points at {path}, which does not exist"
            );
        }
    }

    #[test]
    fn every_save_directory_is_on_the_cartridge_and_the_cache_is_not() {
        // The whole point, in one assertion: what the game writes goes with the
        // cartridge, except the one thing nobody wants to carry.
        let scratch = Scratch::new("home-placement");
        let home = prepare(scratch.path()).expect("prepare");

        for (name, path) in &home.vars {
            let on_cartridge = Path::new(path).starts_with(scratch.path());
            if name == "XDG_CACHE_HOME" {
                assert!(
                    !on_cartridge,
                    "the cache should not be on the drive: {path}"
                );
            } else {
                assert!(on_cartridge, "{name} should be on the cartridge: {path}");
            }
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    #[test]
    fn the_xdg_directories_are_all_named() {
        let scratch = Scratch::new("home-xdg");
        let home = prepare(scratch.path()).expect("prepare");
        for name in [
            "HOME",
            "XDG_DATA_HOME",
            "XDG_CONFIG_HOME",
            "XDG_STATE_HOME",
            "XDG_CACHE_HOME",
        ] {
            assert!(home.get(name).is_some(), "{name} was not set");
        }
        // The conventional layout, so a game that builds the path itself from
        // $HOME rather than reading $XDG_DATA_HOME finds the same directory.
        assert_eq!(
            home.get("XDG_DATA_HOME"),
            Some(
                scratch
                    .join(".gamepak/home/.local/share")
                    .display()
                    .to_string()
                    .as_str()
            )
        );
    }

    #[test]
    fn two_cartridges_do_not_share_a_cache() {
        let one = Scratch::new("home-cache-a");
        let two = Scratch::new("home-cache-b");
        assert_ne!(cache_dir(one.path()), cache_dir(two.path()));
    }

    #[test]
    fn preparing_twice_keeps_what_the_game_wrote() {
        // Every launch calls this. It must not be a reset.
        let scratch = Scratch::new("home-idempotent");
        let home = prepare(scratch.path()).expect("prepare");
        let save = Path::new(home.get("HOME").unwrap()).join("slot1.sav");
        std::fs::write(&save, b"chapter one").expect("write");

        let again = prepare(scratch.path()).expect("prepare again");
        assert_eq!(again, home);
        assert_eq!(std::fs::read_to_string(&save).unwrap(), "chapter one");
    }

    #[test]
    fn a_cartridge_that_cannot_be_written_to_reports_rather_than_panicking() {
        let scratch = Scratch::new("home-readonly");
        scratch.write("root", b"not a directory");
        assert!(prepare(&scratch.join("root")).is_err());
    }
}
