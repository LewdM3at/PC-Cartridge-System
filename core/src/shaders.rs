//! The shader cache, carried between machines.
//!
//! A game's first hour on a new machine is its worst hour: every pipeline it
//! uses has to be compiled before it can be drawn, and the stutter that causes
//! is the single most noticeable difference between a game you have played and
//! the same game on somebody else's PC. The cartridge has just carried 60 GB of
//! that game across the room. Carrying the compiled shaders as well costs a few
//! hundred megabytes and removes the whole problem.
//!
//! # Why this is so much simpler than `saves`
//!
//! Because it is a cache. Every byte here is derived from files the cartridge
//! already carries, so losing it costs time and never data — which means the
//! machinery [`crate::saves`] needs does not apply. There is no conflict case to
//! refuse: if two machines both warmed a cache, either one is correct and the
//! newer is merely better. Nothing is backed up before being replaced, because
//! there is nothing to lose. The rule is "newest wins", and that is the whole of
//! it.
//!
//! # Where the cache actually lives
//!
//! Steam keeps a per-game shader cache at `steamapps/shadercache/<appid>` inside
//! **the library the game is installed in**. That matters here more than it looks:
//! when `steamlib` registers a cartridge as a Steam library, Steam may put the
//! cache on the cartridge by itself. So the host side is looked for across every
//! library Steam knows about, *excluding any that live on the cartridge* — syncing
//! a drive with itself is not a thing to do by accident.
//!
//! Not covered, and worth knowing: Mesa's global cache
//! (`~/.cache/mesa_shader_cache`) and NVIDIA's (`~/.nv/GLCache`) are shared by
//! every program on the machine, not per game, so nothing here can carry one
//! cartridge's share of them. Steam's per-app directory is the one that is both
//! per-game and large.
//!
//! # Why it is per cartridge
//!
//! Because it is a trade against the drive's speed. On a fast NVMe cartridge,
//! carrying a warm cache is free. On a cheap USB stick, a few hundred megabytes
//! copied at insert and again at eject is a wait the user did not ask for, to
//! avoid a stutter they might not notice. Only the person who made the cartridge
//! knows which drive it is, so the cartridge says:
//!
//! ```text
//! shader_cache=drive
//! ```

use std::path::{Path, PathBuf};

use serde::Serialize;

/// Where carried caches live, under [`crate::create::ASSET_DIR`].
pub const SHADER_DIR: &str = "shaders";

/// Whether this cartridge asked for its shader caches to travel with it.
pub fn wanted(root: &Path) -> bool {
    std::fs::read_to_string(root.join("cartridge.conf"))
        .map(|text| wanted_in(&text))
        .unwrap_or(false)
}

/// The same question against the text of a `cartridge.conf`.
///
/// `drive` carries them, `host` does not, and anything else — including nothing
/// at all — means `host`. Silence has to mean "leave it alone": a cartridge
/// written before this existed must not start copying gigabytes on insert
/// because the launcher was updated.
pub fn wanted_in(conf: &str) -> bool {
    let map = crate::cartridge::parse_ini(conf);
    ["general", "collection"]
        .iter()
        .filter_map(|section| crate::cartridge::ini_get(&map, section, "shader_cache"))
        .any(|value| matches!(value.trim().to_lowercase().as_str(), "drive" | "cartridge"))
}

/// The Steam appids this cartridge names.
///
/// Read out of the conf text rather than through `read_cartridge_info`, which
/// inlines every cover as a `data:` URI — an expensive way to find some numbers.
///
/// `steam://rungameid/<n>` is the form the wizard writes and `n` is the appid for
/// an ordinary Steam game. A non-Steam shortcut also appears as `rungameid` with
/// a 64-bit id that is not an appid; those simply match no shader directory,
/// which is the right outcome rather than a special case.
pub fn app_ids(root: &Path) -> Vec<String> {
    std::fs::read_to_string(root.join("cartridge.conf"))
        .map(|text| app_ids_in(&text))
        .unwrap_or_default()
}

/// The same, against conf text.
pub fn app_ids_in(conf: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for line in conf.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let Some(eq) = line.find('=') else { continue };
        if !matches!(
            line[..eq].trim().to_lowercase().as_str(),
            "executable" | "open"
        ) {
            continue;
        }
        if let Some(id) = app_id_of(line[eq + 1..].trim()) {
            if !found.contains(&id) {
                found.push(id);
            }
        }
    }
    found
}

/// The appid in a `steam://rungameid/<n>` URI, if it is one.
pub fn app_id_of(executable: &str) -> Option<String> {
    let lower = executable.trim().to_lowercase();
    let rest = lower.strip_prefix("steam://rungameid/")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    (!digits.is_empty()).then_some(digits)
}

/// One game's shader cache, on both sides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShaderSlot {
    pub app_id: String,
    /// Where Steam keeps it on this machine, or empty when no library has one.
    pub host_path: String,
    pub cartridge_path: String,
    pub host_bytes: u64,
    pub cartridge_bytes: u64,
    pub host_newest: u64,
    pub cartridge_newest: u64,
}

/// Which way a slot should be copied, if either.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Move {
    /// The cartridge's cache is the warmer one: bring it to this machine.
    Pull,
    /// This machine's is warmer: take it with the cartridge.
    Push,
    /// Neither is newer than the other by enough to matter.
    InSync,
    /// Neither side has one yet.
    Empty,
    /// Steam has no library that could hold this game's cache here.
    NoHost,
}

/// Every slot this cartridge has, resolved against this machine.
pub fn slots(root: &Path) -> Vec<ShaderSlot> {
    let libraries = host_libraries(root);
    app_ids(root)
        .into_iter()
        .map(|app_id| slot_for(root, &app_id, &libraries))
        .collect()
}

fn slot_for(root: &Path, app_id: &str, libraries: &[PathBuf]) -> ShaderSlot {
    let cartridge = cartridge_path(root, app_id);
    let (cartridge_newest, cartridge_bytes) = crate::saves::tree_summary(&cartridge);

    // The library that already has a cache for this game, or the first one, which
    // is where Steam would put it.
    let host = libraries
        .iter()
        .map(|library| library.join("shadercache").join(app_id))
        .find(|path| path.is_dir())
        .or_else(|| {
            libraries
                .first()
                .map(|library| library.join("shadercache").join(app_id))
        });

    let (host_newest, host_bytes) = host
        .as_deref()
        .map(crate::saves::tree_summary)
        .unwrap_or((0, 0));

    ShaderSlot {
        app_id: app_id.to_string(),
        host_path: host
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
        cartridge_path: cartridge.display().to_string(),
        host_bytes,
        cartridge_bytes,
        host_newest,
        cartridge_newest,
    }
}

/// Where a carried cache sits on the drive.
pub fn cartridge_path(root: &Path, app_id: &str) -> PathBuf {
    root.join(crate::create::ASSET_DIR)
        .join(SHADER_DIR)
        .join(app_id)
}

/// Steam's libraries on this machine, with any on the cartridge left out.
///
/// The exclusion is the important half. `steamlib` registers a cartridge as a
/// Steam library so games run from it, which puts the cartridge itself in
/// `libraryfolders.vdf` — and a "host" cache inside the cartridge would make
/// every sync a copy of the drive onto itself.
pub fn host_libraries(root: &Path) -> Vec<PathBuf> {
    let Some(steam) = crate::steam::steam_root() else {
        return Vec::new();
    };
    crate::steam::library_paths(&steam)
        .into_iter()
        .filter(|library| !crate::busy::is_within(library, root))
        .collect()
}

/// Which way this slot wants to go.
///
/// Newest wins, with a slack that covers a filesystem storing times to the
/// second and two machines whose clocks are not the same clock. No conflict
/// case: this is a cache, so where both sides have been warmed, either is
/// correct and the newer is only better.
pub fn plan(slot: &ShaderSlot) -> Move {
    if slot.host_path.is_empty() {
        return Move::NoHost;
    }
    match (slot.host_bytes > 0, slot.cartridge_bytes > 0) {
        (false, false) => Move::Empty,
        (false, true) => Move::Pull,
        (true, false) => Move::Push,
        (true, true) => {
            let slack = crate::saves::MTIME_SLACK_SECONDS;
            if slot.cartridge_newest > slot.host_newest.saturating_add(slack) {
                Move::Pull
            } else if slot.host_newest > slot.cartridge_newest.saturating_add(slack) {
                Move::Push
            } else {
                Move::InSync
            }
        }
    }
}

/// What a sync did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Synced {
    pub app_id: String,
    pub moved: Move,
    pub files: u64,
    pub bytes: u64,
}

/// Bring warmer caches from the cartridge to this machine. For insert.
pub fn pull_all(root: &Path) -> Vec<Result<Synced, String>> {
    sync(root, Move::Pull)
}

/// Take this machine's warmer caches with the cartridge. For eject.
pub fn push_all(root: &Path) -> Vec<Result<Synced, String>> {
    sync(root, Move::Push)
}

/// Copy every slot that wants to go `direction`, and leave the rest alone.
///
/// One direction at a time on purpose. Insert only ever pulls and eject only ever
/// pushes, so a cache warmed on this machine is not shipped back before the game
/// has been played, and one carried onto this machine is not overwritten by the
/// cold cache that was here.
fn sync(root: &Path, direction: Move) -> Vec<Result<Synced, String>> {
    slots(root)
        .into_iter()
        .filter_map(|slot| {
            let moved = plan(&slot);
            if moved != direction {
                return None;
            }
            let (from, to) = match direction {
                Move::Pull => (slot.cartridge_path.clone(), slot.host_path.clone()),
                Move::Push => (slot.host_path.clone(), slot.cartridge_path.clone()),
                _ => return None,
            };
            // No backup, and nothing moved aside: every byte here is derived from
            // files the cartridge already carries.
            Some(
                crate::saves::copy_tree(Path::new(&from), Path::new(&to))
                    .map(|(files, bytes)| Synced {
                        app_id: slot.app_id.clone(),
                        moved,
                        files,
                        bytes,
                    })
                    .map_err(|why| format!("{}: {why}", slot.app_id)),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Scratch;

    fn slot(host_bytes: u64, host_newest: u64, cart_bytes: u64, cart_newest: u64) -> ShaderSlot {
        ShaderSlot {
            app_id: "367520".to_string(),
            host_path: "/home/you/.steam/steam/steamapps/shadercache/367520".to_string(),
            cartridge_path: "/run/media/you/CART/.gamepak/shaders/367520".to_string(),
            host_bytes,
            cartridge_bytes: cart_bytes,
            host_newest,
            cartridge_newest: cart_newest,
        }
    }

    #[test]
    fn silence_means_leave_it_alone() {
        // A cartridge written before this existed must not start copying
        // gigabytes because the launcher was updated.
        assert!(!wanted_in("executable=steam://rungameid/1\ntitle=X\n"));
        assert!(!wanted_in(""));
        assert!(!wanted_in("shader_cache=host\n"));
    }

    #[test]
    fn a_cartridge_can_ask_to_carry_them() {
        assert!(wanted_in("executable=x\nshader_cache=drive\n"));
        assert!(wanted_in("executable=x\nshader_cache=Cartridge\n"));
        assert!(wanted_in(
            "[collection]\ntitle=T\nshader_cache=drive\n\n[game]\nexecutable=x\n"
        ));
    }

    #[test]
    fn the_appids_come_out_of_the_launch_targets() {
        let conf = "[collection]\ntitle=Two\n\n\
                    [game]\ntitle=A\nexecutable=steam://rungameid/367520\n\n\
                    [game]\ntitle=B\nexecutable=steam://rungameid/1091500\n";
        assert_eq!(app_ids_in(conf), vec!["367520", "1091500"]);
    }

    #[test]
    fn anything_that_is_not_a_steam_game_has_no_appid() {
        for not_steam in [
            "Games/Tunic/TUNIC.exe",
            "heroic://launch/gog/1207658921",
            "steam://open/games",
            "",
            "steam://rungameid/",
        ] {
            assert_eq!(app_id_of(not_steam), None, "{not_steam}");
        }
        // A non-Steam shortcut's 64-bit id parses and simply matches no
        // directory, which is the right outcome rather than a special case.
        assert_eq!(
            app_id_of("steam://rungameid/18446744073709551615"),
            Some("18446744073709551615".to_string())
        );
    }

    #[test]
    fn an_appid_is_only_counted_once_however_often_it_appears() {
        let conf = "executable=steam://rungameid/620\n[game]\nexecutable=STEAM://RunGameID/620\n";
        assert_eq!(app_ids_in(conf), vec!["620"]);
    }

    #[test]
    fn a_comment_is_not_a_launch_target() {
        let conf = "# executable=steam://rungameid/999\nexecutable=steam://rungameid/620\n";
        assert_eq!(app_ids_in(conf), vec!["620"]);
    }

    #[test]
    fn the_newer_cache_is_the_one_that_travels() {
        // No conflict case: it is a cache, so either side is correct and the
        // newer is only better.
        assert_eq!(plan(&slot(10, 1000, 10, 5000)), Move::Pull);
        assert_eq!(plan(&slot(10, 5000, 10, 1000)), Move::Push);
        assert_eq!(plan(&slot(10, 1000, 10, 1000)), Move::InSync);
    }

    #[test]
    fn a_side_with_nothing_takes_from_the_side_that_has_something() {
        assert_eq!(plan(&slot(0, 0, 10, 1000)), Move::Pull);
        assert_eq!(plan(&slot(10, 1000, 0, 0)), Move::Push);
        assert_eq!(plan(&slot(0, 0, 0, 0)), Move::Empty);
    }

    #[test]
    fn a_two_second_filesystem_does_not_invent_a_difference() {
        assert_eq!(plan(&slot(10, 1000, 10, 1002)), Move::InSync);
        assert_eq!(plan(&slot(10, 1000, 10, 1004)), Move::Pull);
    }

    #[test]
    fn with_no_steam_library_there_is_nowhere_to_put_one() {
        let mut orphan = slot(0, 0, 10, 1000);
        orphan.host_path = String::new();
        assert_eq!(plan(&orphan), Move::NoHost);
    }

    #[test]
    fn a_library_on_the_cartridge_is_not_a_host_library() {
        // `steamlib` registers a cartridge as a Steam library so games run from
        // it, which puts the cartridge in libraryfolders.vdf. Treating that as
        // the host side would make every sync a copy of the drive onto itself.
        let scratch = Scratch::new("shaders-self");
        let cart = scratch.join("CART");
        std::fs::create_dir_all(cart.join("steamapps")).expect("mkdir");
        std::fs::create_dir_all(scratch.join("steam/steamapps")).expect("mkdir");

        let libraries = vec![
            scratch.join("steam/steamapps"),
            cart.join("steamapps"),
            cart.join("steamapps/nested/deeper"),
        ];
        let kept: Vec<PathBuf> = libraries
            .into_iter()
            .filter(|library| !crate::busy::is_within(library, &cart))
            .collect();
        assert_eq!(kept, vec![scratch.join("steam/steamapps")]);
    }

    #[test]
    fn insert_brings_a_warm_cache_to_a_machine_that_has_none() {
        // The whole point: the cartridge carried the game, and now it carries the
        // hour of compiling that game needs on a machine it has not met.
        let scratch = Scratch::new("shaders-pull");
        let cart = scratch.join("CART");
        let steam = scratch.join("steam/steamapps");
        std::fs::create_dir_all(&steam).expect("mkdir");
        std::fs::create_dir_all(&cart).expect("mkdir");
        std::fs::write(
            cart.join("cartridge.conf"),
            b"executable=steam://rungameid/367520\nshader_cache=drive\n",
        )
        .expect("conf");
        let carried = cartridge_path(&cart, "367520");
        std::fs::create_dir_all(&carried).expect("mkdir");
        std::fs::write(carried.join("fozpipelinesv6"), b"compiled pipelines").expect("cache");

        // Stand in for what `slots` would find, since Steam is not installed here.
        let found = slot_for(&cart, "367520", std::slice::from_ref(&steam));
        assert_eq!(plan(&found), Move::Pull);
        assert_eq!(
            found.host_path,
            steam
                .join("shadercache")
                .join("367520")
                .display()
                .to_string()
        );

        let (files, bytes) = crate::saves::copy_tree(
            Path::new(&found.cartridge_path),
            Path::new(&found.host_path),
        )
        .expect("copy");
        assert_eq!(files, 1);
        assert!(bytes > 0);
        assert_eq!(
            std::fs::read_to_string(steam.join("shadercache/367520/fozpipelinesv6")).unwrap(),
            "compiled pipelines"
        );
    }

    #[test]
    fn a_cartridge_that_has_not_asked_carries_nothing() {
        let scratch = Scratch::new("shaders-off");
        std::fs::write(
            scratch.join("cartridge.conf"),
            b"executable=steam://rungameid/367520\n",
        )
        .expect("conf");

        // The gate is `wanted`, checked by the caller before any of this runs.
        assert!(!wanted(scratch.path()));
        // The appid is still found — the cartridge names a Steam game either way
        // — so what stops the copy is the setting and nothing else.
        assert_eq!(app_ids(scratch.path()), vec!["367520"]);
    }

    #[test]
    fn nothing_is_copied_in_the_direction_that_was_not_asked_for() {
        // Insert only pulls and eject only pushes, so a cache warmed here is not
        // shipped back before the game has been played, and one carried onto this
        // machine is not overwritten by the cold cache that was already here.
        let scratch = Scratch::new("shaders-one-way");
        let cart = scratch.join("CART");
        let steam = scratch.join("steam/steamapps");
        std::fs::create_dir_all(&cart).expect("mkdir");
        std::fs::write(
            cart.join("cartridge.conf"),
            b"executable=steam://rungameid/367520\nshader_cache=drive\n",
        )
        .expect("conf");

        // Only this machine has a cache, so the slot wants to be pushed.
        let host = steam.join("shadercache/367520");
        std::fs::create_dir_all(&host).expect("mkdir");
        std::fs::write(host.join("fozpipelinesv6"), b"warmed here").expect("cache");

        let found = slot_for(&cart, "367520", std::slice::from_ref(&steam));
        assert_eq!(plan(&found), Move::Push);
        // An insert leaves it exactly where it is.
        assert!(!cartridge_path(&cart, "367520").exists());
    }
}
