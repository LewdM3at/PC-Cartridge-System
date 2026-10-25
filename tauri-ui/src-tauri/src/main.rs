// PC GamePak — Tauri 2.0 backend
//
// One binary, two modes, chosen by the arguments it was started with:
//
//   pc-gamepak --drive <path>    the popup, opened on insert
//   pc-gamepak --create          the create-cartridge wizard
//
// Exactly one window is built, so the wizard costs nothing when a cartridge is
// inserted and the popup costs nothing while making one. And two with none at
// all, for front-ends that have their own buttons (see headless.rs):
//
//   pc-gamepak --drive <path> --play <n>              play, stay up while it runs
//   pc-gamepak --drive <path> --safe-eject [--force]  eject, report on stdout
//
// Launcher commands:
//   drive_path()                             -> String | error
//   parse_cartridge(drive_path)              -> CartridgeInfo (cover included)
//   launch_game(executable, drive_path)      -> ()
//   eject_drive(drive_path)                  -> ()   (refuses while in use)
//   cartridge_busy(drive_path)               -> Holders
//   eject_drive_forcing(drive_path)          -> String (closes what is in the
//                                               way first, then ejects)
//   eject_with_guard(drive_path)             -> { ejected, message }  (the one
//                                               the button calls; asks first)
//   focus_window()                           -> ()
//   cartridge_health(drive_path)             -> Health
//   read_cartridge_for_edit(drive_path)      -> Editable
//   update_cartridge(request)                -> UpdateResult
//   cartridge_stats(drive_path)              -> Stats  (launches and hours,
//                                               read from the cartridge)
//   save_slots(drive_path)                   -> Vec<SlotStatus>
//   carried_home(drive_path)                 -> String | null
//   frontends()                              -> [{ id, name, kind, installed,
//                                               implemented, on }]
//   set_frontend(id, on)                     -> the same list, updated
//   sync_saves(drive_path)                   -> Vec<SyncOutcome>  (on insert)
//   shader_slots(drive_path)                 -> Vec<ShaderSlot>
//   pull_shaders(drive_path)                 -> Vec<Synced>  (on insert)
//   push_shaders(drive_path)                 -> Vec<Synced>  (on eject)
//   push_saves(drive_path)                   -> Vec<SyncOutcome>  (on eject)
//   resolve_save_conflict(drive_path, slot_id, keep) -> SyncOutcome
//   open_wizard_settings()                   -> ()  (opens/focuses the
//                                               wizard, straight to Settings)
//
// Wizard commands:
//   list_games()                             -> GameList { games, problems } (Playnite + Steam)
//   get_settings()                           -> Settings
//   set_settings(settings)                   -> Settings
//   suggest_collection_name(titles)          -> String
//   pick_cover_image()                       -> PickedCover | null
//   pick_game_folder()                       -> PickedGameFolder | null
//   pick_rom_file()                          -> PickedRom | null
//   host_platform()                          -> "windows" | "linux" | …
//   tuning_plan(drive_path, tweaks, applying) -> Vec<String>  (the commands)
//   apply_tuning(drive_path, tweaks, applying) -> Vec<String>  (what was done)
//   game_cover(library, id)                  -> String (data URI)
//   list_target_drives()                     -> Vec<TargetDrive>
//   list_unmounted_volumes()                 -> Vec<UnmountedVolume>
//   mount_volume(volume)                     -> String (the new root)
//   format_plan(drive_path)                  -> FormatPlan
//   executable_choices(playnite_id?, source_dir?, title?) -> Vec<Candidate>
//   steam_registration(drive_path)           -> bool
//   holds_steam_games(drive_path)            -> bool
//   steam_registration_plan(drive_path)      -> Vec<String>
//   register_with_steam(drive_path)          -> bool
//   unregister_from_steam(drive_path)        -> bool
//   create_cartridge(request)                -> CartridgeResult,
//                                               emitting cartridge://progress
//
// There is deliberately no command that takes a path to read. An earlier
// read_image_as_data_uri(path) let the webview turn any file on the system into
// a data URI; the cover is now read here, from a path this file derives and
// confines to the cartridge itself.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod headless;

// All of the real work lives in gamepak-core, which has no UI dependency and
// so can be tested without a webview. This file is the Tauri shell around it.
use gamepak_core::cartridge::{self, CartridgeInfo};
use gamepak_core::remote;
use gamepak_core::{
    busy, create, created, drives, edit, format, frontend, health, home, idle, insert, ludusavi,
    memcard, playlog, playtrack, saves, settings, sgdb, shaders, stats, tuning, unboxed,
};

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::webview::PageLoadEvent;
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::DialogExt;

// --------------------------------------------------------------------------
// Tauri commands
// --------------------------------------------------------------------------

/// Parse the cartridge at `drive_path` and return metadata.
#[tauri::command]
fn parse_cartridge(drive_path: String) -> Result<CartridgeInfo, String> {
    if let Some(id) = remote::marker_id(&drive_path) {
        return prime_registered(&drive_path, id);
    }
    cartridge::read_cartridge_info(&drive_path)
}

/// The registered GamePak behind a marker. The marker names any GamePak that
/// is not a cartridge — a game on another PC, a Steam game, a stream, a file —
/// and the window shows it at READY like one.
fn registered(id: &str) -> Result<gamepak_core::gamepak::GamePak, String> {
    gamepak_core::gamepak::lookup_from(
        &gamepak_core::gamepak::registry_path(),
        &gamepak_core::gamepak::GamePakId::parse(id)?,
    )
}

/// Ready a GamePak for Play: a game on another PC is started there; anything
/// else has its host woken and waited for, if it names one. Nothing starts
/// here until Play.
fn prime_registered(marker: &str, id: &str) -> Result<CartridgeInfo, String> {
    let pak = registered(id)?;
    if matches!(pak.action, gamepak_core::gamepak::Action::Remote { .. }) {
        return prime_remote(marker, id);
    }
    gamepak_core::gamepak::prepare_host(&gamepak_core::gamepak::registry_path(), &pak.id)?;
    let cover = match &pak.action {
        gamepak_core::gamepak::Action::Steam { app_id } => {
            create::game_cover(create::Library::Steam, &app_id.to_string())
        }
        _ => String::new(),
    };
    let title = pak.title.clone().unwrap_or_else(|| id.to_string());
    let mut info = CartridgeInfo::remote(marker, &title, &cover);
    fill_artwork(&mut info, id);
    debug_log(format!("gamepak {id}: ready"));
    Ok(info)
}

/// Cover, background and logo for a GamePak with no cartridge to carry them,
/// looked up on SteamGridDB by its title. Only what is missing is filled, and
/// only when SteamGridDB is switched on; a failure leaves the placeholder.
/// Downloads are cached, so a second tap does not go to the network.
fn fill_artwork(info: &mut CartridgeInfo, id: &str) {
    let Some(game) = sgdb::search_games(&info.title)
        .ok()
        .and_then(|games| games.into_iter().next())
    else {
        return;
    };
    let fetch = |kind: sgdb::ArtworkType, portrait: bool| {
        let art = sgdb::get_artwork(game.id, kind).ok()?;
        let pick = art
            .iter()
            .find(|a| !portrait || a.height > a.width)
            .or(art.first())?;
        let path = sgdb::download_artwork(&pick.url, &format!("gamepak-{id}-{kind:?}")).ok()?;
        let uri = sgdb::read_as_data_uri(&path)?;
        Some((uri, path.to_string_lossy().into_owned()))
    };
    if info.cover.is_empty() {
        if let Some((uri, path)) = fetch(sgdb::ArtworkType::Grid, true) {
            (info.cover, info.cover_path) = (uri, path);
        }
    }
    if info.background.is_empty() {
        if let Some((uri, path)) = fetch(sgdb::ArtworkType::Hero, false) {
            (info.background, info.background_path) = (uri, path);
        }
    }
    if info.logo.is_empty() {
        if let Some((uri, path)) = fetch(sgdb::ArtworkType::Logo, false) {
            (info.logo, info.logo_path) = (uri, path);
        }
    }
}

/// A game on another PC: start it there now, through Moonlight's pairing, and
/// show it here at READY with the host's title and box art. Play streams it;
/// Eject closes it.
fn prime_remote(marker: &str, id: &str) -> Result<CartridgeInfo, String> {
    let target = remote::Remote::resolve(&gamepak_core::gamepak::registry_path(), id)?;
    let launched = target.prime()?;
    debug_log(format!(
        "remote {id}: {} on {} ({})",
        launched.title,
        target.moonlight,
        if launched.already_running {
            "already running"
        } else {
            "started"
        }
    ));
    let mut info = CartridgeInfo::remote(
        marker,
        &launched.title,
        launched.cover.as_deref().unwrap_or(""),
    );
    fill_artwork(&mut info, id);
    Ok(info)
}

/// The cartridge this window was started for.
///
/// The frontend asks for this rather than reading a query string: the window is
/// declared in tauri.conf.json and loads `index.html` with no parameters, so
/// there is nothing in the URL to read.
#[tauri::command]
async fn drive_path() -> Result<String, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(encoded) = trigger_id(&args)? else {
        return Ok(cartridge::drive_from_args(args.into_iter()));
    };
    tauri::async_runtime::spawn_blocking(move || {
        let id = encoded.into_id()?;
        let registry = gamepak_core::gamepak::registry_path();
        let pak = gamepak_core::gamepak::lookup_from(
            &registry,
            &gamepak_core::gamepak::GamePakId::parse(id.trim())?,
        )?;
        // Started on Play, not here: the window opens on it at READY.
        if !matches!(
            pak.action,
            gamepak_core::gamepak::Action::Cartridge { .. }
                | gamepak_core::gamepak::Action::Remote { .. }
        ) {
            return Ok(format!("{}{}", remote::MARKER, pak.id.as_str()));
        }
        match gamepak_core::gamepak::trigger_from(&gamepak_core::gamepak::registry_path(), &id)? {
            gamepak_core::gamepak::Outcome::OpenCartridge(path) => {
                Ok(path.to_string_lossy().into_owned())
            }
            gamepak_core::gamepak::Outcome::OpenRemote(id) => {
                Ok(format!("{}{}", remote::MARKER, id.as_str()))
            }
            gamepak_core::gamepak::Outcome::Executed => {
                Err("this GamePak is not a cartridge".to_string())
            }
        }
    })
    .await
    .map_err(|error| format!("GamePak trigger failed: {error}"))?
}

/// A GamePak ID as a trigger delivered it. Every trigger ends up as an ID.
enum TriggerInput {
    Id(String),
    /// NFC adapter transport: the hex of an NDEF message carrying the ID.
    NdefHex(String),
}

impl TriggerInput {
    fn into_id(self) -> Result<String, String> {
        match self {
            TriggerInput::Id(id) => Ok(id),
            TriggerInput::NdefHex(hex) => {
                gamepak_core::nfc::id_from_ndef_hex(&hex).map(|id| id.as_str().to_owned())
            }
        }
    }
}

/// `--trigger gp_id` (the generic entry point) or `--nfc-ndef <hex>` (what the
/// NFC adapter sends). `None` when the launcher was not started by a trigger.
fn trigger_id(args: &[String]) -> Result<Option<TriggerInput>, String> {
    for (flag, nfc) in [("--trigger", false), ("--nfc-ndef", true)] {
        if let Some(index) = args.iter().position(|arg| arg == flag) {
            let value = args
                .get(index + 1)
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))?;
            return Ok(Some(if nfc {
                TriggerInput::NdefHex(value)
            } else {
                TriggerInput::Id(value)
            }));
        }
    }
    Ok(None)
}

#[tauri::command]
fn nfc_selection() -> bool {
    std::env::args().any(|arg| arg == "--nfc-ndef")
}

/// `--memcard`: open on a combo cartridge's memory card rather than the
/// cartridge, for a front-end with a "Memory card" action of its own.
#[tauri::command]
fn opens_memory_card() -> bool {
    std::env::args().any(|arg| arg == "--memcard")
}

/// Each window's icons, as `tools/make-icons.mjs` writes them: every size from
/// 16 to 256 px, each shrunk from the artwork in steps. The wizard's is the
/// other logo: the launcher and the wizard are one executable, and their
/// taskbar buttons should not look the same.
const LAUNCHER_ICON: &[u8] = include_bytes!("../icons/icon.ico");
const WIZARD_ICON: &[u8] = include_bytes!("../icons/wizard.ico");

/// The picture in an `.ico` made for `size`, or the nearest larger one: the
/// generator's shrink is cleaner than Windows' at small sizes.
#[cfg(target_os = "windows")]
fn ico_entry(ico: &[u8], size: u32) -> Option<&[u8]> {
    let count = u16::from_le_bytes(ico.get(4..6)?.try_into().ok()?) as usize;
    let mut best: Option<(u32, &[u8])> = None;
    for i in 0..count {
        let entry = ico.get(6 + i * 16..6 + i * 16 + 16)?;
        let width = if entry[0] == 0 {
            256
        } else {
            u32::from(entry[0])
        };
        let len = u32::from_le_bytes(entry[8..12].try_into().ok()?) as usize;
        let at = u32::from_le_bytes(entry[12..16].try_into().ok()?) as usize;
        let data = ico.get(at..at + len)?;
        let better = match best {
            None => true,
            Some((have, _)) if have < size => width > have,
            Some((have, _)) => width >= size && width < have,
        };
        if better {
            best = Some((width, data));
        }
    }
    best.map(|(_, data)| data)
}

/// Give a window both of its icons, at the size its screen wants.
///
/// Windows keeps two per window: a small one for the title bar and Alt-Tab,
/// and a big one for the taskbar. Tauri sets only the small one, made from the
/// first picture in `icon.ico`, so the taskbar showed a stretched 16 px icon
/// for the launcher and nothing at all for the wizard. Both are set here at the
/// window's DPI — 16 and 32 px at 100%, 24 and 48 at 150% — from the `.ico`
/// entry made for that size.
#[cfg(target_os = "windows")]
fn set_window_icons(window: &tauri::WebviewWindow, ico: &[u8]) {
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateIconFromResourceEx, SendMessageW, ICON_BIG, ICON_SMALL, LR_DEFAULTCOLOR, WM_SETICON,
    };
    let Ok(handle) = window.hwnd() else { return };
    let hwnd = handle.0 as isize;
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    for (which, base) in [(ICON_SMALL, 16), (ICON_BIG, 32)] {
        let size = (base * dpi / 96) as i32;
        let Some(png) = ico_entry(ico, size as u32) else {
            continue;
        };
        // SAFETY: the bytes are a whole PNG, which Windows accepts as icon
        // resource data since Vista; 0x30000 is the icon format version it asks for.
        let icon = unsafe {
            CreateIconFromResourceEx(
                png.as_ptr(),
                png.len() as u32,
                1,
                0x0003_0000,
                size,
                size,
                LR_DEFAULTCOLOR,
            )
        };
        if icon != 0 {
            unsafe { SendMessageW(hwnd, WM_SETICON, which as usize, icon) };
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn set_window_icons(_window: &tauri::WebviewWindow, _ico: &[u8]) {}

/// Eject for a game on another PC: `moonlight quit`, which closes the app on
/// the host and any stream of it.
fn stop_remote(id: &str) -> Result<EjectOutcome, String> {
    let target = remote::Remote::resolve(&gamepak_core::gamepak::registry_path(), id)?;
    target.quit()?;
    Ok(EjectOutcome {
        ejected: true,
        message: format!("Closed on {}", target.moonlight),
    })
}

/// Launch the game.
/// `executable` can be a URI (steam://, heroic://, ...) or a path relative
/// to `drive_path`.
#[tauri::command]
fn launch_game(
    executable: String,
    drive_path: String,
    title: Option<String>,
) -> Result<(), String> {
    if executable.is_empty() {
        return Err("No executable configured for this cartridge".into());
    }
    if let Some(id) = remote::marker_id(&drive_path) {
        let pak = registered(id)?;
        if !matches!(pak.action, gamepak_core::gamepak::Action::Remote { .. }) {
            use gamepak_core::gamepak::GamePakAction;
            return pak.action.execute();
        }
        // The game is already running on the host and counting its own hours
        // there; Play only brings the picture over.
        let target = remote::Remote::resolve(&gamepak_core::gamepak::registry_path(), id)?;
        let mut moonlight = target.stream()?;
        std::thread::spawn(move || {
            let _ = moonlight.wait();
        });
        return Ok(());
    }

    // Before the game starts, and before anything that can fail. A launch that
    // is never counted is a missing row in a stats file; a launch that fails
    // because a stats file could not be written is a broken launcher.
    count_the_launch(&drive_path, &executable, title.unwrap_or_default());

    // Starting it is core's, shared with every front-end. Only the reaping of
    // a carried game's process is this window's.
    let started = gamepak_core::launch::start(&drive_path, &executable, &|line| debug_log(line))?;
    if let gamepak_core::launch::Started::Carried(child) = started {
        settle_when_it_exits(child, drive_path, stats::key_for(&executable));
    }
    Ok(())
}

/// Reap a carried game when it exits.
///
/// The session is not closed here any more. The tracker watches the game's
/// folder rather than this one process, because a carried game is often a
/// launcher stub or a shell script that starts the real executable and exits —
/// and the session, and the save push that follows it, belong to the game, not
/// to the stub.
fn settle_when_it_exits(mut child: std::process::Child, _drive_path: String, _key: String) {
    std::thread::spawn(move || {
        if let Err(why) = child.wait() {
            debug_log(format!("could not wait for the game to exit: {why}"));
        }
    });
}

// --------------------------------------------------------------------------
// What the cartridge remembers: hours played, and saves
// --------------------------------------------------------------------------

/// A session being counted, and the tracker counting it.
struct Tracked {
    session: stats::Session,
    root: PathBuf,
    /// A program the cartridge carries, as opposed to a URI handed to another
    /// launcher — only those have their saves pushed when they end.
    carried: bool,
    dirs: Vec<PathBuf>,
    tracker: Mutex<playtrack::Tracker>,
    /// Set when the session has been closed from outside: Eject, a second
    /// game, the window going away. The thread sees it and stops.
    closed: AtomicBool,
}

impl Tracked {
    fn active_seconds(&self) -> u64 {
        self.tracker
            .lock()
            .map(|tracker| tracker.active_seconds())
            .unwrap_or(0)
    }

    fn mode(&self) -> playtrack::Mode {
        self.tracker
            .lock()
            .map(|tracker| tracker.mode())
            .unwrap_or(playtrack::Mode::Window)
    }

    /// Close the session with what was counted, once.
    fn close(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        let played = self.active_seconds();
        match stats::record_session_played(&self.session, played) {
            Ok(seconds) => debug_log(format!("stats: session closed, {seconds}s played")),
            Err(why) => debug_log(format!("stats: {why}")),
        }
        if let Err(why) = playlog::mirror(&self.root) {
            debug_log(format!("playlog: {why}"));
        }
    }
}

/// Every session being counted, by game.
fn playing() -> &'static Mutex<HashMap<String, Arc<Tracked>>> {
    static PLAYING: OnceLock<Mutex<HashMap<String, Arc<Tracked>>>> = OnceLock::new();
    PLAYING.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The app, once Tauri has built it, so a tracker can end the process after
/// the window has been closed and the last game has too.
static APP: OnceLock<tauri::AppHandle> = OnceLock::new();

/// The window has been closed while a game was still being watched.
static WINDOW_GONE: AtomicBool = AtomicBool::new(false);

/// No window will ever exist: the game was started straight from an insert.
static HEADLESS: AtomicBool = AtomicBool::new(false);

/// Count a launch, if the user has left that switched on, and start watching
/// the game.
///
/// The title comes from the window rather than being re-read here. It is only
/// decoration inside the stats file — the `executable` is what keys a row —
/// and reading it back off the drive would mean parsing the whole cartridge,
/// covers inlined as `data:` URIs and all, on every press of Play.
///
/// Every failure here is swallowed into the debug log on purpose: a read-only
/// cartridge, a full drive, or a `.gamepak` directory somebody made read-only
/// are all reasons not to have a count, and none of them is a reason not to
/// play the game.
fn count_the_launch(drive_path: &str, executable: &str, title: String) {
    let settings = settings::load();
    if !settings.track_playtime {
        return;
    }

    // Pressing Play on a second game means the first one is over. Without
    // this both sessions stay open and both are credited with the evening.
    end_every_session();

    let root = PathBuf::from(drive_path);
    let session = match stats::record_launch(&root, executable, &title) {
        Ok(session) => session,
        Err(why) => return debug_log(format!("stats: {why}")),
    };
    if let Err(why) = playlog::mirror(&root) {
        debug_log(format!("playlog: {why}"));
    }

    let steam_root = gamepak_core::steam::steam_root();
    let dirs = playtrack::watch_dirs(&root, executable, steam_root.as_deref());
    debug_log(format!("tracking {executable} in {dirs:?}"));
    let tracked = Arc::new(Tracked {
        session,
        carried: !executable.contains("://"),
        root,
        dirs,
        tracker: Mutex::new(playtrack::Tracker::new(
            playtrack::Mode::Process,
            stats::now_unix(),
            settings.idle_pause_minutes,
        )),
        closed: AtomicBool::new(false),
    });
    if let Ok(mut open) = playing().lock() {
        open.insert(stats::key_for(executable), tracked.clone());
    }
    track(tracked);
}

/// Watch one session until it ends, on its own thread.
///
/// Every tick: is the game running, and is anybody there. Every minute: write
/// what has been counted to the drive, so a crash or a yanked cartridge costs
/// at most a minute — the heartbeat Kazeta uses, now carrying the played
/// seconds rather than the time since Play.
fn track(tracked: Arc<Tracked>) {
    std::thread::spawn(move || {
        let mut probe = idle::IdleProbe::new();
        let mut last_beat = stats::now_unix();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(playtrack::TICK_SECONDS));
            if tracked.closed.load(Ordering::SeqCst) {
                return;
            }
            let now = stats::now_unix();
            let running = busy::running_within(&tracked.dirs);
            let away = probe.idle_seconds();
            let tick = tracked
                .tracker
                .lock()
                .map(|mut tracker| tracker.tick(now, running, away))
                .unwrap_or(playtrack::Tick::Ended);

            match tick {
                playtrack::Tick::Running => {}
                playtrack::Tick::Ended => break,
                playtrack::Tick::NeverSeen => {
                    // Nothing to see: count the window instead, if there is
                    // one. With no window there is nothing left to measure.
                    if HEADLESS.load(Ordering::SeqCst) || WINDOW_GONE.load(Ordering::SeqCst) {
                        break;
                    }
                    debug_log("game not seen; counting while the launcher is open".to_string());
                    if let Ok(mut tracker) = tracked.tracker.lock() {
                        tracker.fall_back_to_window(now);
                    }
                }
            }

            if now.saturating_sub(last_beat) >= stats::HEARTBEAT_SECONDS {
                last_beat = now;
                if let Err(why) =
                    stats::touch_session_played(&tracked.session, tracked.active_seconds())
                {
                    // A cartridge that has gone is the ordinary way for this
                    // to end.
                    debug_log(format!("stats heartbeat: {why}"));
                    tracked.closed.store(true, Ordering::SeqCst);
                    break;
                }
            }
        }
        finish(&tracked);
    });
}

/// A game has stopped: close its session, put its save back, and leave if the
/// window was only being kept for this.
fn finish(tracked: &Arc<Tracked>) {
    tracked.close();
    if let Ok(mut open) = playing().lock() {
        open.retain(|_, other| !Arc::ptr_eq(other, tracked));
    }
    if tracked.carried && settings::load().save_sync {
        let results = saves::push_all(&tracked.root);
        let pushed = results.iter().filter(|result| result.is_ok()).count();
        debug_log(format!("game exited; pushed {pushed} save slot(s)"));
        for why in results.into_iter().filter_map(Result::err) {
            debug_log(format!("save push after play: {why}"));
        }
    }
    if WINDOW_GONE.load(Ordering::SeqCst) && !still_watching() {
        if let Some(app) = APP.get() {
            app.exit(0);
        }
    }
}

/// Whether a game is being watched that outlives the window.
fn still_watching() -> bool {
    playing()
        .lock()
        .map(|open| {
            open.values()
                .any(|tracked| tracked.mode() == playtrack::Mode::Process)
        })
        .unwrap_or(false)
}

/// Block until every session has ended. For a game started straight from an
/// insert, where this process has no window to keep it alive.
fn wait_for_sessions() {
    while playing()
        .lock()
        .map(|open| !open.is_empty())
        .unwrap_or(false)
    {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

/// Everything this cartridge has recorded, for the details sheet.
///
/// Settles an abandoned session before reading, which is why this is the one
/// read here that writes. A cartridge turning up is exactly the moment to
/// account for the last run that did not get to finish — and doing it here
/// means the hours are in the total the sheet is about to show, rather than
/// appearing on the next launch as if from nowhere.
#[tauri::command]
fn cartridge_stats(drive_path: String) -> stats::Stats {
    if remote::marker_id(&drive_path).is_some() {
        return stats::Stats::default();
    }
    let root = Path::new(&drive_path);
    if settings::load().track_playtime {
        if let Some(seconds) = stats::recover(root) {
            if let Err(why) = playlog::mirror(root) {
                debug_log(format!("playlog: {why}"));
            }
            if seconds > 0 {
                debug_log(format!(
                    "stats: recovered {seconds}s from a session that did not close"
                ));
            }
        }
    }
    stats::read(root)
}

/// Close every open session with what it has counted.
///
/// Called when the cartridge is ejected and when a second game is started.
/// The window closing only calls it when nothing outlives the window — see
/// the window's close handler.
/// Close the sessions only the window was measuring, and leave the watched
/// ones running.
fn end_window_sessions() {
    let window_only: Vec<Arc<Tracked>> = match playing().lock() {
        Ok(mut open) => {
            let keys: Vec<String> = open
                .iter()
                .filter(|(_, tracked)| tracked.mode() == playtrack::Mode::Window)
                .map(|(key, _)| key.clone())
                .collect();
            keys.into_iter()
                .filter_map(|key| open.remove(&key))
                .collect()
        }
        Err(_) => return,
    };
    for tracked in window_only {
        tracked.close();
    }
}

fn end_every_session() {
    let open: Vec<Arc<Tracked>> = match playing().lock() {
        Ok(mut open) => open.drain().map(|(_, tracked)| tracked).collect(),
        Err(_) => return,
    };
    for tracked in open {
        tracked.close();
    }
}

/// What the cartridge declares, resolved against this machine.
///
/// Read-only, and callable whether or not syncing is switched on: the details
/// sheet shows what a cartridge *would* sync, which is how somebody decides
/// whether to turn it on.
#[tauri::command]
fn save_slots(drive_path: String) -> Vec<saves::SlotStatus> {
    if remote::marker_id(&drive_path).is_some() {
        return Vec::new();
    }
    saves::status(Path::new(&drive_path))
}

/// Every front-end this build knows about, and whether each is on.
///
/// The settings dialog needs all three parts: the name, whether the plugin is
/// actually installed, and whether it has been switched on. A switch for a plugin
/// that is not there would be a switch that does nothing.
#[tauri::command]
fn frontends() -> Vec<FrontEndState> {
    let enabled = settings::load().frontends;
    frontend::known()
        .into_iter()
        .map(|front| FrontEndState {
            on: enabled.is_on(front.id),
            front,
        })
        .collect()
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FrontEndState {
    #[serde(flatten)]
    front: frontend::FrontEnd,
    on: bool,
}

/// Switch one front-end on or off.
///
/// Its own command rather than a field on `set_settings`, because the dialog
/// toggles these one at a time and a round trip through the whole settings
/// object would make two toggles in quick succession lose the first.
#[tauri::command]
fn set_frontend(id: String, on: bool) -> Result<Vec<FrontEndState>, String> {
    let mut current = settings::load();
    current.frontends.set(&id, on);
    settings::save(&current)?;
    Ok(frontends())
}

/// The home directory this cartridge carries, if it carries one.
///
/// Read-only and cheap, so the details sheet can say "this cartridge keeps the
/// game's whole home" — which is otherwise invisible, there being no `save=`
/// line to show for it.
#[tauri::command]
fn carried_home(drive_path: String) -> Option<String> {
    if remote::marker_id(&drive_path).is_some() {
        return None;
    }
    let root = Path::new(&drive_path);
    if !home::wanted(root) {
        return None;
    }
    let home = root.join(create::ASSET_DIR).join(home::HOME_DIR);
    Some(home.display().to_string())
}

/// Reconcile the cartridge's saves with this machine's, on insert.
#[tauri::command]
fn sync_saves(drive_path: String) -> Result<Vec<saves::SyncOutcome>, String> {
    if remote::marker_id(&drive_path).is_some() {
        return Ok(Vec::new());
    }
    if !settings::load().save_sync {
        return Ok(Vec::new());
    }
    Ok(collect(saves::attach_all(Path::new(&drive_path))))
}

/// What the cartridge's shader caches would do here, whether or not it asked.
///
/// Read-only, so the details sheet can show what a cartridge *could* carry —
/// which is how somebody decides whether to add `shader_cache=drive` to it.
#[tauri::command]
fn shader_slots(drive_path: String) -> Vec<shaders::ShaderSlot> {
    shaders::slots(Path::new(&drive_path))
}

/// Bring a warmer shader cache off the cartridge, on insert.
///
/// The cartridge decides, not the settings dialog: only the person who made it
/// knows whether the drive is fast enough for this to be a gain rather than a
/// wait. Errors are per slot and logged — a cache that could not be copied costs
/// compile time and nothing else, which is not worth interrupting anyone for.
#[tauri::command]
fn pull_shaders(drive_path: String) -> Vec<shaders::Synced> {
    if remote::marker_id(&drive_path).is_some() {
        return Vec::new();
    }
    let root = Path::new(&drive_path);
    if !shaders::wanted(root) {
        return Vec::new();
    }
    gather(shaders::pull_all(root))
}

/// Take this machine's warmer caches with the cartridge, on eject.
#[tauri::command]
fn push_shaders(drive_path: String) -> Vec<shaders::Synced> {
    if remote::marker_id(&drive_path).is_some() {
        return Vec::new();
    }
    let root = Path::new(&drive_path);
    if !shaders::wanted(root) {
        return Vec::new();
    }
    gather(shaders::push_all(root))
}

/// [`collect`] for shader syncs, which have their own outcome type.
fn gather(results: Vec<Result<shaders::Synced, String>>) -> Vec<shaders::Synced> {
    let mut done = Vec::new();
    for result in results {
        match result {
            Ok(synced) => done.push(synced),
            Err(why) => debug_log(format!("shaders: {why}")),
        }
    }
    done
}

/// The same, on the way out.
#[tauri::command]
fn push_saves(drive_path: String) -> Result<Vec<saves::SyncOutcome>, String> {
    if remote::marker_id(&drive_path).is_some() {
        return Ok(Vec::new());
    }
    if !settings::load().save_sync {
        return Ok(Vec::new());
    }
    Ok(collect(saves::detach_all(Path::new(&drive_path))))
}

/// Settle a conflict the way the user chose.
///
/// `keep` is `"host"` or `"cartridge"` — which copy to keep, not which to
/// throw away, because that is the question somebody is actually answering.
/// The other one is moved aside and kept regardless.
#[tauri::command]
fn resolve_save_conflict(
    drive_path: String,
    slot_id: String,
    keep: String,
) -> Result<saves::SyncOutcome, String> {
    let root = Path::new(&drive_path);
    let mut status = saves::status(root)
        .into_iter()
        .find(|status| status.slot.id == slot_id)
        .ok_or_else(|| format!("no save slot called {slot_id} on this cartridge"))?;

    status.direction = match keep.as_str() {
        "host" => saves::Direction::Push,
        "cartridge" => saves::Direction::Pull,
        other => return Err(format!("keep must be host or cartridge, not {other}")),
    };
    saves::sync_slot(root, &status)
}

/// Report what worked and log what did not, rather than failing the lot.
///
/// One unwritable save directory must not stop the other three from arriving,
/// and the window has nothing useful to do with a half-failure anyway.
fn collect(results: Vec<Result<saves::SyncOutcome, String>>) -> Vec<saves::SyncOutcome> {
    let mut done = Vec::new();
    for result in results {
        match result {
            Ok(outcome) => done.push(outcome),
            Err(why) => debug_log(format!("saves: {why}")),
        }
    }
    done
}

/// What to do about this insert, or `None` to open the window as usual.
///
/// `--drive` with nothing else means the launcher was opened for a cartridge,
/// which is the only case the setting applies to. Somebody running
/// `pc-gamepak --drive X:` by hand gets a window whatever the setting says,
/// because they asked for one — `--show` is how the watcher and the tray say
/// "the user asked for this specifically".
fn reaction_on_insert(args: &[String]) -> Option<insert::Reaction> {
    // Asked for by a front-end, not a cartridge arriving: always the window.
    if args.iter().any(|arg| arg == "--show" || arg == "--memcard") {
        return None;
    }
    let settings = settings::load();
    if settings.frontends.is_on(frontend::LAUNCHER)
        && settings.on_cartridge_insert == insert::InsertAction::FocusUi
    {
        return None; // The common path, and no cartridge read needed to know it.
    }
    // The launcher not being a front-end at all is decided before the cartridge
    // is read: there is nothing to read it for.
    if !settings.frontends.is_on(frontend::LAUNCHER) {
        return Some(insert::Reaction::Quit);
    }

    let drive = cartridge::drive_from_args(args.iter().cloned());
    if drive.is_empty() {
        return None;
    }
    // A cartridge that cannot be read is a window's problem to explain, not
    // something to silently act on.
    let info = cartridge::read_cartridge_info(&drive).ok()?;
    Some(insert::decide_for(&settings, &info))
}

/// Carry out a reaction that does not need a window.
///
/// Only ever called with `Quit`, `Launch` or `Notify`: `ShowWindow` is returned
/// to `main` as `None` so the ordinary path runs untouched.
///
/// `false` means the reaction could not be carried out and the window should
/// open after all — which today only happens when a notification cannot be
/// posted on this desktop.
fn act_on_insert(reaction: insert::Reaction) -> bool {
    match reaction {
        insert::Reaction::ShowWindow => {}
        insert::Reaction::Quit => {}
        insert::Reaction::Launch { executable, title } => {
            let drive = cartridge::drive_from_args(std::env::args().skip(1));
            // The same call the Play button makes, counting included, so an
            // auto-launched game is not missing from the cartridge's history.
            HEADLESS.store(true, Ordering::SeqCst);
            if let Err(why) = launch_game(executable, drive, Some(title)) {
                eprintln!("could not start the game: {why}");
            }
            // No window, so this process stays up only to watch the game, and
            // exits when it does — or when it never turns up.
            wait_for_sessions();
        }
        insert::Reaction::Notify { title, body } => return notify(&title, &body),
    }
    true
}

/// Say a cartridge is there, without a window. False if nothing was shown.
///
/// Both desktops are handled in [`gamepak_core::notify`], which explains what
/// each one needs. The answer here is what to do when neither works: show the
/// window. That is the behaviour `notify_only` was chosen *instead* of, so it is
/// a poor outcome — but an insert that produces nothing at all looks like a
/// cartridge that was not seen, and a setting that silently does nothing is
/// indistinguishable from a bug.
fn notify(title: &str, body: &str) -> bool {
    if gamepak_core::notify::cartridge_arrived(title, body) {
        return true;
    }
    eprintln!("could not post a notification for {title}; opening the window instead");
    false
}

/// Take the keyboard, not just the front of the screen.
///
/// The popup is `always_on_top` and is opened by the watcher, which is a
/// background process. Windows lets a background process show a window but not
/// take focus — the foreground lock is there to stop programs stealing your
/// typing — so the launcher landed in front of whatever you were doing while
/// your keystrokes carried on going to the window behind it.
///
/// That is worse than untidy for a controller. Chromium only reports gamepad
/// state to a focused document, so a pad would raise `gamepadconnected`,
/// light up the indicator, and then read as though every button were resting:
/// connected, visible, and completely dead.
#[tauri::command]
fn focus_window(window: tauri::WebviewWindow) -> Result<(), String> {
    let _ = window.set_focus();

    #[cfg(target_os = "windows")]
    {
        let target = window.clone();
        window
            .run_on_main_thread(move || take_foreground(&target))
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Ask Windows for the foreground the way it will actually grant it.
///
/// `SetForegroundWindow` on its own is refused for a process that is not
/// already in front; it flashes the taskbar button instead. Three things are
/// tried in turn, because which of them works depends on what has the
/// foreground and how it got it:
///
/// 1. Attach to the input queue of the window that is currently in front. Two
///    threads sharing a focus state may hand it between themselves, which is
///    the long-standing way through the lock.
/// 2. `SwitchToThisWindow`, which is what Alt-Tab uses and is not bound by the
///    same rule.
/// 3. A synthetic Alt tap. The lock is lifted for the process that owns the
///    most recent input event, so producing one is enough — Alt on its own
///    presses nothing.
///
/// Each step is followed by asking whether it worked, so the noisier ones only
/// happen when the quiet one was not enough.
#[cfg(target_os = "windows")]
fn take_foreground(window: &tauri::WebviewWindow) {
    use windows_sys::Win32::System::Threading::AttachThreadInput;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        keybd_event, SetFocus, KEYEVENTF_KEYUP, VK_MENU,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
        SwitchToThisWindow,
    };

    let Ok(handle) = window.hwnd() else {
        return;
    };
    let hwnd = handle.0 as isize;

    let ours = unsafe { GetWindowThreadProcessId(hwnd, std::ptr::null_mut()) };
    let holds_it = || unsafe { GetForegroundWindow() } == hwnd;

    unsafe {
        BringWindowToTop(hwnd);

        let foreground = GetForegroundWindow();
        let theirs = GetWindowThreadProcessId(foreground, std::ptr::null_mut());
        let attached = theirs != 0 && ours != 0 && theirs != ours;
        if attached {
            AttachThreadInput(theirs, ours, 1);
        }
        SetForegroundWindow(hwnd);
        SetFocus(hwnd);
        if attached {
            AttachThreadInput(theirs, ours, 0);
        }

        if holds_it() {
            return;
        }

        SwitchToThisWindow(hwnd, 1);
        if holds_it() {
            return;
        }

        // Alt down and straight back up. Nothing is typed by it; it exists so
        // that this process owns the last input event, which is one of the
        // conditions under which Windows allows the foreground to be taken.
        keybd_event(VK_MENU as u8, 0, 0, 0);
        keybd_event(VK_MENU as u8, 0, KEYEVENTF_KEYUP, 0);
        SetForegroundWindow(hwnd);
        SetFocus(hwnd);
    }
}

/// Whether the window should write what it is doing to a log.
///
/// Off unless `PC_GAMEPAK_DEBUG` is set, and asked once at start-up rather than
/// per line, so a launcher nobody is debugging pays a single call for it.
#[tauri::command]
fn debug_logging() -> bool {
    std::env::var_os("PC_GAMEPAK_DEBUG").is_some_and(|value| value != "0")
}

/// Append one line to the launcher log.
///
/// The webview has no console anyone can reach in a release build — there are
/// no devtools, and a window that opens on insert and closes on eject is gone
/// before anything could be attached to it. So it writes next to the watcher's
/// log, which is where someone would already be looking.
#[tauri::command]
fn debug_log(line: String) {
    if !debug_logging() {
        return;
    }
    let Some(base) = std::env::var_os("LOCALAPPDATA") else {
        return;
    };

    let path = PathBuf::from(base).join("PC-GamePak").join("launcher.log");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        use std::io::Write;
        // Truncated hard: this is a diagnostic, not somewhere for a cartridge's
        // title to be written at whatever length it happens to be.
        let line: String = line.chars().take(400).collect();
        let _ = writeln!(file, "{line}");
    }
}

/// Can this cartridge be ejected, or is it a tag standing in for one?
///
/// The launcher hides the button when the answer is no. `eject_drive` asks the
/// same question again rather than trusting it: a command is reachable whatever
/// the interface chose to show.
#[tauri::command]
fn can_eject(drive_path: String) -> bool {
    if remote::marker_id(&drive_path).is_some() {
        // Eject is how a remote game is stopped.
        return true;
    }
    drives::is_ejectable(std::path::Path::new(&drive_path))
}

/// Who is still using the cartridge, so the interface can say so by name.
///
/// Read-only, and answered before the confirm dialog rather than after it: "are
/// you sure" is a different question from "Tomb Raider is running from this
/// drive", and only the second one is worth interrupting somebody for.
#[tauri::command]
fn cartridge_busy(drive_path: String) -> busy::Holders {
    // Never this process. By the time Eject is pressed the launcher has read
    // the cartridge's artwork and closed it, and a program that refused to
    // eject a drive on account of its own finished reads would be unusable.
    busy::holders(Path::new(&drive_path)).excluding(&[std::process::id()])
}

/// Safely eject the cartridge drive.
///
/// Refuses while anything is using the volume. That check is here and not only
/// in the window because a command is reachable whatever the interface chose to
/// show — the same reasoning `can_eject` is built on — and because the cost of
/// getting this wrong went up when saves started travelling on the cartridge:
/// unmounting mid-write now loses a save as well as a session.
#[tauri::command]
fn eject_drive(drive_path: String) -> Result<(), String> {
    let holders = cartridge_busy(drive_path.clone());
    if !holders.is_empty() {
        return Err(format!(
            "Still in use: {}. Close it and try again, or choose Force quit and eject.",
            holders.summary(3)
        ));
    }
    unmount(&drive_path)
}

/// What came of pressing Eject.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct EjectOutcome {
    /// False when the user chose to leave the cartridge where it was, which is
    /// not an error and must not animate the cartridge out of the slot.
    ejected: bool,
    message: String,
}

/// Eject, asking what to do about anything still using the drive.
///
/// The one the button calls. `eject_drive` and `eject_drive_forcing` stay as
/// they are underneath — plain, scriptable, and each doing exactly one thing —
/// with the choice between them put to the person holding the cartridge.
///
/// A native dialog rather than something drawn in the window: this interrupts
/// an action already in progress, it needs to be the thing in front, and the
/// launcher has no modal of its own to borrow. It also has to work when the
/// window is a 420-pixel slot with a cartridge sliding out of it.
#[tauri::command]
async fn eject_with_guard(
    window: tauri::WebviewWindow,
    drive_path: String,
) -> Result<EjectOutcome, String> {
    if let Some(id) = remote::marker_id(&drive_path).map(str::to_string) {
        // Nothing to unmount and nothing of ours to stop: the game, stream or
        // file runs on its own once started.
        if !matches!(
            registered(&id)?.action,
            gamepak_core::gamepak::Action::Remote { .. }
        ) {
            return Ok(EjectOutcome {
                ejected: true,
                message: "Done".to_string(),
            });
        }
        return tauri::async_runtime::spawn_blocking(move || stop_remote(&id))
            .await
            .map_err(|e| e.to_string())?;
    }
    let holders = cartridge_busy(drive_path.clone());
    if holders.is_empty() {
        unmount(&drive_path)?;
        return Ok(EjectOutcome {
            ejected: true,
            message: "Safe to remove".to_string(),
        });
    }

    let mut body = format!(
        "{}.

",
        holders.summary(4)
    );
    if holders.unchecked > 0 {
        // Said out loud rather than swallowed: on Linux an unprivileged process
        // can only look at its owner's processes, so "nothing else is using it"
        // is a claim this cannot always make.
        body.push_str(&format!(
            "({} other processes could not be checked.)

",
            holders.unchecked
        ));
    }
    body.push_str(
        "Closing them first lets a game write its save to the cartridge.          Forcing it may lose whatever was part-way through being written.",
    );

    let force = window
        .dialog()
        .message(body)
        .title("The cartridge is still in use")
        .kind(tauri_plugin_dialog::MessageDialogKind::Warning)
        .buttons(tauri_plugin_dialog::MessageDialogButtons::OkCancelCustom(
            "Force quit and eject".to_string(),
            "Keep it mounted".to_string(),
        ))
        .blocking_show();

    if !force {
        return Ok(EjectOutcome {
            ejected: false,
            message: "Left mounted.".to_string(),
        });
    }

    let message = eject_drive_forcing(drive_path)?;
    Ok(EjectOutcome {
        ejected: true,
        message,
    })
}

/// Eject, closing whatever is in the way first.
///
/// The other half of the dialog. Asks every process holding the volume to quit
/// and waits for it — which is what gives a game the chance to write its save
/// to the cartridge — then kills what is left, and only then unmounts. If
/// something survives all of that the drive stays mounted: ejecting on top of a
/// process that would not die is the exact outcome this guard exists to prevent.
#[tauri::command]
fn eject_drive_forcing(drive_path: String) -> Result<String, String> {
    if !drives::is_ejectable(Path::new(&drive_path)) {
        return Err(
            "This cartridge is not on a removable drive, so there is nothing to eject.".to_string(),
        );
    }

    let stopped = busy::stop_all(Path::new(&drive_path));
    for note in &stopped.notes {
        debug_log(format!("eject: {note}"));
    }
    if !stopped.survived.is_empty() {
        return Err(format!(
            "{} could not be closed, so the cartridge was left mounted. \
             Log out or restart, rather than pulling the drive.",
            stopped
                .survived
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    unmount(&drive_path)?;
    Ok(match (stopped.asked.len(), stopped.killed.len()) {
        (0, 0) => "Ejected.".to_string(),
        (asked, 0) => format!("Closed {asked} and ejected."),
        (_, killed) => format!("Ejected; {killed} had to be forced."),
    })
}

/// Settle the cartridge's own files, then take the volume away.
///
/// Shared by both eject paths so neither can forget the save. Ordering is the
/// whole of it: nothing is using the volume by the time this runs, so the save
/// written here is a save nothing else is part-way through.
fn unmount(drive_path: &str) -> Result<(), String> {
    if !drives::is_ejectable(Path::new(drive_path)) {
        return Err(
            "This cartridge is not on a removable drive, so there is nothing to eject.".to_string(),
        );
    }

    // A `--play` still watching this drive closes its session now, while the
    // volume is there to write it to, and gets out of the way of the saves.
    headless::stop_players(drive_path);

    // Before the drive goes. This is the only moment a linked save can be
    // turned back into a real directory, and the last moment a copied one can
    // be written — after this the volume is gone and both are somebody's
    // afternoon. Errors are logged rather than raised: a save that could not
    // be written is not a reason to leave a drive mounted that the user has
    // asked to remove, and holding the cartridge hostage over it is worse.
    gamepak_core::eject::settle(drive_path, &|line| debug_log(line));
    end_every_session();

    gamepak_core::eject::eject(drive_path)
}

// --------------------------------------------------------------------------
// Wizard commands
// --------------------------------------------------------------------------

/// Everything installed, from Playnite where available and Steam otherwise.
///
/// `playnite_root` lets the wizard pass a user-supplied Playnite data directory
/// when auto-discovery failed. If absent, the usual lookup is used.
#[tauri::command]
fn list_games(playnite_root: Option<String>) -> Result<create::GameList, String> {
    create::list_games(playnite_root.as_deref())
}

#[tauri::command]
fn game_cover(library: create::Library, id: String) -> String {
    create::game_cover(library, &id)
}

/// What the user has switched on. Read on open, so the wizard can hide what is
/// off — though the backend refuses either way.
#[tauri::command]
fn get_settings() -> settings::Settings {
    settings::load()
}

/// Store the settings and hand back what was stored, so the window and the file
/// cannot drift apart.
///
/// The front-end switches are kept as they are on disk: the settings form does
/// not send them, and `set_frontend` is what changes them. See
/// [`settings::save_form`].
#[tauri::command]
fn set_settings(settings: settings::Settings) -> Result<settings::Settings, String> {
    settings::save_form(settings)
}

/// How well this cartridge is actually connected.
///
/// Read on demand rather than at startup: on Windows it asks PowerShell, and
/// the launcher opening half a second slower is worse than the details sheet
/// filling in half a second late.
#[tauri::command]
async fn cartridge_health(drive_path: String) -> health::Health {
    if remote::marker_id(&drive_path).is_some() {
        return health::Health::default();
    }
    tauri::async_runtime::spawn_blocking(move || health::inspect(&drive_path))
        .await
        .unwrap_or_default()
}

/// Read a cartridge that already exists, so its metadata can be changed without
/// writing the whole thing again.
#[tauri::command]
fn read_cartridge_for_edit(drive_path: String) -> Result<edit::Editable, String> {
    edit::read(&drive_path)
}

/// Rewrite a cartridge's metadata. Copies nothing, deletes no game.
#[tauri::command]
fn update_cartridge(request: edit::UpdateRequest) -> Result<edit::UpdateResult, String> {
    edit::update(&request)
}

/// Replace every game's poster on a cartridge with one from SteamGridDB.
///
/// One request per game, so the window asks before calling it.
#[tauri::command]
fn refetch_cartridge_artwork(drive_path: String) -> Result<edit::UpdateResult, String> {
    edit::refetch_artwork(&drive_path)
}

/// Which OS the wizard is running on, so it can offer only what exists here.
#[tauri::command]
fn host_platform() -> &'static str {
    std::env::consts::OS
}

/// Which tweaks the window is asking about, by name.
fn parse_tweaks(names: &[String]) -> Result<Vec<tuning::Tweak>, String> {
    names
        .iter()
        .map(|name| match name.as_str() {
            "defender" => Ok(tuning::Tweak::DefenderExclusion),
            "indexing" => Ok(tuning::Tweak::SearchIndexing),
            other => Err(format!("{other} is not a setting this tool changes")),
        })
        .collect()
}

/// The exact commands a tuning run would execute.
///
/// Shown before anything happens: this is elevated and it touches malware
/// scanning, so the user reads the commands first.
#[tauri::command]
fn tuning_plan(
    drive_path: String,
    tweaks: Vec<String>,
    applying: bool,
) -> Result<Vec<String>, String> {
    tuning::plan(&drive_path, &parse_tweaks(&tweaks)?, applying)
}

/// Apply or undo the Windows tuning. Each step elevates on its own.
#[tauri::command]
async fn apply_tuning(
    drive_path: String,
    tweaks: Vec<String>,
    applying: bool,
) -> Result<Vec<String>, String> {
    let parsed = parse_tweaks(&tweaks)?;
    tauri::async_runtime::spawn_blocking(move || tuning::apply(&drive_path, &parsed, applying))
        .await
        .map_err(|e| format!("the tuning thread failed: {e}"))?
}

/// A picture the user chose for a collection's artwork.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PickedCover {
    /// Handed back with the create request, so the file is copied from here.
    path: String,
    /// The picture itself, for the wizard's preview. Empty when it is too big
    /// to inline; the build then refuses it with a proper message.
    preview: String,
}

/// Ask for artwork through the desktop's own file dialog.
///
/// The window never names a path: it gets one back only after the user has
/// pointed at a file themselves. This is also the offline way to give a
/// collection its own art, with no SteamGridDB lookup involved.
#[tauri::command]
async fn pick_cover_image(window: tauri::WebviewWindow) -> Option<PickedCover> {
    let file = window
        .dialog()
        .file()
        .set_title("Choose collection artwork")
        .add_filter("Images", &["png", "jpg", "jpeg", "webp", "bmp"])
        .blocking_pick_file()?;

    let path = file.into_path().ok()?;
    Some(PickedCover {
        preview: sgdb::read_as_data_uri(&path).unwrap_or_default(),
        path: path.to_string_lossy().into_owned(),
    })
}

/// What the picker came back with, and what is inside it.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PickedGameFolder {
    path: String,
    /// Folder name, offered as the title so it does not have to be typed.
    name: String,
    size_bytes: u64,
    /// What Play could start, best guess first. Empty when nothing here looks
    /// like a program, which is worth showing before the copy rather than after.
    choices: Vec<gamepak_core::portable::Candidate>,
}

/// Ask for a game's folder through the desktop's own file dialog.
///
/// The counterpart to picking a game from the list: a game that no launcher
/// knows about still lives in a folder, and a folder is all the copy needs. As
/// with the cover picker, the window never names a path — it gets one back only
/// after the user has pointed at it.
#[tauri::command]
async fn pick_game_folder(
    window: tauri::WebviewWindow,
) -> Result<Option<PickedGameFolder>, String> {
    let Some(folder) = window
        .dialog()
        .file()
        .set_title("Choose the game's folder")
        .blocking_pick_folder()
    else {
        return Ok(None);
    };
    let path = folder
        .into_path()
        .map_err(|e| format!("That folder cannot be read: {e}"))?;

    // Checked here, not merely in the picker: the same rules apply however the
    // path arrived.
    let dir = create::check_source_dir(&path.to_string_lossy())?;
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    Ok(Some(PickedGameFolder {
        size_bytes: gamepak_core::portable::tree_size_of(&dir),
        choices: gamepak_core::portable::find_executables(&dir, &name, None),
        path: dir.to_string_lossy().into_owned(),
        name,
    }))
}

/// A ROM the person chose, and the platform its file type suggests.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PickedRom {
    path: String,
    /// The file name without its extension, offered as the title.
    name: String,
    size_bytes: u64,
    /// `SNES`, `GBA`... when the extension belongs to one system, else null and
    /// the person picks.
    platform: Option<&'static str>,
}

/// Ask for a ROM through the desktop's own file dialog, for an emulated
/// cartridge. As with the folder picker, the window gets a path back only
/// after the person has pointed at a file.
#[tauri::command]
async fn pick_rom_file(window: tauri::WebviewWindow) -> Result<Option<PickedRom>, String> {
    let Some(file) = window
        .dialog()
        .file()
        .set_title("Choose the game's ROM")
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let path = file
        .into_path()
        .map_err(|e| format!("That file cannot be read: {e}"))?;
    let meta = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a file.", path.display()));
    }
    Ok(Some(PickedRom {
        name: path
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        platform: path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(gamepak_core::emulated::platform_for_extension),
        size_bytes: meta.len(),
        path: path.to_string_lossy().into_owned(),
    }))
}

/// A name for a cartridge carrying several games, worked out from what they are
/// called. The wizard offers it; the user can always type their own.
#[tauri::command]
fn suggest_collection_name(titles: Vec<String>) -> String {
    create::suggest_collection_name(&titles)
}

#[tauri::command]
fn sgdb_search_games(query: String) -> Result<Vec<sgdb::SteamGridGame>, String> {
    sgdb::search_games(&query)
}

#[tauri::command]
fn sgdb_get_artwork(
    game_id: u32,
    art_type: sgdb::ArtworkType,
) -> Result<Vec<sgdb::Artwork>, String> {
    sgdb::get_artwork(game_id, art_type)
}

#[tauri::command]
fn sgdb_download_artwork(
    url: String,
    cache_key: String,
    game_key: Option<String>,
) -> Result<sgdb::CachedArtwork, String> {
    let path = sgdb::download_artwork(&url, &cache_key)?;
    if let Some(key) = game_key.filter(|k| !k.trim().is_empty()) {
        sgdb::remember_last_used(&key, &path)?;
    }
    sgdb::read_as_data_uri(&path)
        .map(|data_uri| sgdb::CachedArtwork {
            path: path.to_string_lossy().into_owned(),
            data_uri,
        })
        .ok_or_else(|| "SteamGridDB saved the image, but it could not be previewed.".to_string())
}

#[tauri::command]
fn sgdb_last_used_artwork(game_key: String) -> Option<sgdb::CachedArtwork> {
    sgdb::last_used_artwork_data_uri(&game_key)
}

#[tauri::command]
fn list_target_drives() -> Vec<drives::TargetDrive> {
    create::target_drives()
}

/// The generic wizard step: register an action under a stable ID and return
/// the ID. `kind` is an action type; `a` and `b` are its one or two settings
/// (see `action_from`). Nothing here knows how the ID will be delivered.
#[tauri::command]
fn add_gamepak(
    title: String,
    kind: String,
    a: String,
    b: Option<String>,
    id: Option<String>,
) -> Result<String, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("name the GamePak".into());
    }
    let action = action_from(&kind, a.trim(), b.as_deref().map(str::trim))?;
    let id = match id.as_deref().map(str::trim).filter(|id| !id.is_empty()) {
        Some(id) => id.to_owned(),
        None => gamepak_core::gamepak::new_id(),
    };
    gamepak_core::gamepak::register_action_from(
        &gamepak_core::gamepak::registry_path(),
        &id,
        Some(title),
        action,
        None,
        None,
    )?;
    Ok(id)
}

fn action_from(
    kind: &str,
    a: &str,
    b: Option<&str>,
) -> Result<gamepak_core::gamepak::Action, String> {
    use gamepak_core::gamepak::Action;
    if a.is_empty() {
        return Err("fill in the action's setting".into());
    }
    Ok(match kind {
        "steam" => Action::Steam {
            app_id: a.parse().map_err(|_| "Steam app ID must be a number")?,
        },
        "local" => Action::Local { path: a.into() },
        "exec" => Action::Exec {
            program: a.to_owned(),
            args: b
                .filter(|args| !args.is_empty())
                .map(|args| args.split_whitespace().map(str::to_owned).collect())
                .unwrap_or_default(),
        },
        "moonlight" => Action::Moonlight {
            target: a.to_owned(),
            app: b
                .filter(|app| !app.is_empty())
                .ok_or("Moonlight needs an app name")?
                .to_owned(),
        },
        "remote" => Action::Remote {
            moonlight: a.to_owned(),
            app: b
                .filter(|app| !app.is_empty())
                .ok_or("name the app, as the host lists it")?
                .to_owned(),
        },
        "service" => Action::Service {
            target: a.to_owned(),
        },
        "open" => Action::Open {
            target: a.to_owned(),
        },
        _ => return Err(format!("unknown action type {kind}")),
    })
}

/// A host Moonlight has paired with, and its apps as Moonlight last saw them.
#[derive(serde::Serialize)]
struct MoonlightHost {
    name: String,
    apps: Vec<String>,
}

/// Moonlight's paired hosts, for the wizard to suggest. Empty when Moonlight
/// is not installed or not paired: the fields are still free text.
#[tauri::command]
fn moonlight_hosts() -> Vec<MoonlightHost> {
    gamepak_core::moonlight::Settings::load()
        .map(|settings| {
            settings
                .hosts()
                .into_iter()
                .map(|host| MoonlightHost {
                    name: host.name,
                    apps: host.apps.into_iter().map(|(name, _)| name).collect(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Leave the ID of every trigger beside the registry, registered or not: the
/// wizard's Learn button waits for this file to change, so whatever can submit
/// an ID — a card, a barcode, a script — can be learned onto a GamePak.
fn remember_trigger(registry: &Path, id: &str) {
    let id = id.trim();
    if gamepak_core::gamepak::GamePakId::parse(id).is_ok() {
        if let Some(dir) = registry.parent() {
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::write(dir.join("last-trigger"), id);
        }
    }
}

/// The last trigger's ID and when it arrived (ms since the epoch), for Learn.
#[tauri::command]
fn last_trigger() -> Option<(String, u64)> {
    let path = gamepak_core::gamepak::registry_path().with_file_name("last-trigger");
    let id = std::fs::read_to_string(&path).ok()?;
    let at = std::fs::metadata(&path)
        .and_then(|meta| meta.modified())
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;
    Some((id.trim().to_owned(), at))
}

/// Make a learned card or barcode start an existing GamePak too.
#[tauri::command]
fn add_gamepak_trigger(id: String, trigger: String) -> Result<(), String> {
    gamepak_core::gamepak::add_trigger_from(
        &gamepak_core::gamepak::registry_path(),
        &id,
        trigger.trim(),
    )
}

/// Every registered GamePak — what it starts and what starts it — for the
/// GamePak actions list.
#[tauri::command]
fn registered_gamepaks() -> Result<Vec<gamepak_core::gamepak::Listed>, String> {
    gamepak_core::gamepak::list_from(&gamepak_core::gamepak::registry_path())
}

/// The games a Moonlight host offers, as `moonlight list <host>` prints them:
/// one name per line. The host has to be paired in Moonlight already.
#[tauri::command]
async fn moonlight_apps(host: String) -> Result<Vec<String>, String> {
    let host = host.trim().to_owned();
    if host.is_empty() || host.starts_with('-') {
        return Err("enter the Moonlight host first".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let output = std::process::Command::new(gamepak_core::gamepak::moonlight_program())
            .args(["list", &host])
            // No window: Moonlight is a Qt app even when it only lists.
            .env("QT_QPA_PLATFORM", "offscreen")
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|error| format!("could not run Moonlight: {error}"))?;
        let apps: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect();
        if apps.is_empty() {
            return Err(format!(
                "Moonlight listed no games on {host}. Is it on, and paired in Moonlight?"
            ));
        }
        Ok(apps)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn remove_gamepak(id: String) -> Result<(), String> {
    gamepak_core::gamepak::remove_from(&gamepak_core::gamepak::registry_path(), &id)
}

#[tauri::command]
fn remove_gamepak_trigger(id: String, trigger: String) -> Result<(), String> {
    gamepak_core::gamepak::remove_trigger_from(
        &gamepak_core::gamepak::registry_path(),
        &id,
        &trigger,
    )
}

/// Every filesystem a cartridge can be made with, and what each one costs.
///
/// The wizard used to carry its own two-entry list, with the label limit
/// written out a second time and already disagreeing with the one in core. This
/// is the single place that knows.
#[tauri::command]
fn list_filesystems() -> Vec<format::FilesystemInfo> {
    format::all_filesystems()
}

/// Readable volumes Windows has left without a drive letter.
///
/// Listed separately from `list_target_drives` because they are not targets
/// yet: nothing can write to a volume with no mount point, so the wizard shows
/// them as drives that need one rather than pretending they are ready.
#[tauri::command]
fn list_unmounted_volumes() -> Vec<drives::UnmountedVolume> {
    drives::unmounted_volumes()
}

/// Give one of those volumes a drive letter, and return its new root.
///
/// Elevates, so the user sees a UAC prompt and can decline it — which is the
/// whole consent step for changing how their disks are mounted.
#[tauri::command]
fn mount_volume(volume: drives::UnmountedVolume) -> Result<String, String> {
    drives::mount_volume(&volume)
}

/// What formatting a drive would destroy, for the warning shown before it runs.
#[tauri::command]
fn format_plan(drive_path: String) -> Result<format::FormatPlan, String> {
    create::format_plan(&drive_path)
}

/// What Play could start, for a game whose folder is about to be copied.
///
/// Ranked best-first; the window offers the top one and lets the user change it.
#[tauri::command]
fn executable_choices(
    playnite_id: Option<String>,
    source_dir: Option<String>,
    title: Option<String>,
) -> Result<Vec<gamepak_core::portable::Candidate>, String> {
    // A folder the user chose is the more specific answer, so it wins.
    if let Some(dir) = source_dir
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
    {
        return create::executable_choices_in(dir, title.as_deref().unwrap_or_default());
    }
    match playnite_id
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        Some(id) => create::executable_choices(id),
        None => Err("No game folder to look in.".to_string()),
    }
}

/// Whether the drive is currently a registered Steam library folder.
#[tauri::command]
fn steam_registration(drive_path: String) -> bool {
    create::steam_registration(&drive_path)
}

/// Does this cartridge carry Steam games?
///
/// Asked alongside `steam_registration` so the picker can offer to register a
/// cartridge that needs it, and stay quiet about one that has nothing to
/// register.
#[tauri::command]
fn holds_steam_games(drive_path: String) -> bool {
    create::holds_steam_games(&drive_path)
}

/// What registering this cartridge would change, before it changes anything.
#[tauri::command]
fn steam_registration_plan(drive_path: String) -> Vec<String> {
    create::steam_registration_plan(&drive_path)
}

/// Add the cartridge to Steam's library list.
#[tauri::command]
fn register_with_steam(drive_path: String) -> Result<bool, String> {
    create::register_with_steam(&drive_path)
}

/// Remove the cartridge from Steam's library list.
#[tauri::command]
fn unregister_from_steam(drive_path: String) -> Result<bool, String> {
    create::unregister_from_steam(&drive_path)
}

/// Build the cartridge, streaming progress to the window.
///
/// Copying a game is minutes of work, so it runs on a blocking thread and emits
/// `cartridge://progress` instead of leaving the window frozen.
#[tauri::command]
async fn create_cartridge(
    window: tauri::WebviewWindow,
    request: create::CartridgeRequest,
) -> Result<create::CartridgeResult, String> {
    // Read here rather than carried on the request: the cap describes the drive
    // and the enclosure, not this cartridge, and it should hold for a write the
    // window started before the setting was last changed.
    gamepak_core::throttle::set_limit_mb_s(settings::load().default_copy_rate_mb_s);

    tauri::async_runtime::spawn_blocking(move || {
        let mut result = create::create_cartridge(&request, &mut |progress| {
            let _ = window.emit("cartridge://progress", progress);
        })?;
        // Here rather than in the window, so a write is on the shelf even if
        // the window is closed before it hears back. At the drive's mount now,
        // not the one it was asked for: a format relabels it, and Linux
        // remounts it under the new label.
        let written_to = Path::new(&result.conf_path)
            .parent()
            .map(|root| root.to_string_lossy().into_owned())
            .unwrap_or_else(|| request.drive_path.clone());
        let label = drives::list_drives()
            .into_iter()
            .find(|drive| drive.path == written_to)
            .map(|drive| drive.label)
            .unwrap_or_default();
        if let Err(e) = created::record(&written_to, &label, result.bytes_copied) {
            result
                .warnings
                .push(format!("Not added to Created cartridges: {e}"));
        }
        Ok(result)
    })
    .await
    .map_err(|e| format!("the build thread failed: {e}"))?
}

// --------------------------------------------------------------------------
// Memory cards
// --------------------------------------------------------------------------

/// The memory card view of a drive: every save on it, as blocks. `cardOnly`
/// says whether the drive is a memory card and nothing else.
#[tauri::command]
fn memory_card(drive_path: String) -> memcard::CardView {
    if remote::marker_id(&drive_path).is_some() {
        return memcard::CardView {
            title: String::new(),
            blocks: Vec::new(),
            card_only: false,
        };
    }
    memcard::view(Path::new(&drive_path))
}

/// Copy one save to the PC (`to: "pc"`) or onto the card (`to: "card"`).
/// Whatever it replaces is moved aside and kept.
#[tauri::command]
fn memcard_copy(
    drive_path: String,
    slot_id: String,
    to: String,
) -> Result<saves::SyncOutcome, String> {
    memcard::copy(Path::new(&drive_path), &slot_id, &to)
}

/// Take one save off the card. Moved aside on the card, never deleted.
#[tauri::command]
fn memcard_remove(drive_path: String, slot_id: String) -> Result<(), String> {
    memcard::remove(Path::new(&drive_path), &slot_id)
}

/// Show one save's folder in the file manager. The window names the save, not
/// a path: the backend works the folder out, so nothing on the page can point
/// Explorer anywhere else.
#[tauri::command]
fn memcard_reveal(drive_path: String, slot_id: String) -> Result<(), String> {
    let folder = memcard::folder(Path::new(&drive_path), &slot_id)?;
    #[cfg(target_os = "windows")]
    let opener = "explorer.exe";
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let opener = "xdg-open";
    Command::new(opener)
        .arg(&folder)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not open {}: {e}", folder.display()))
}

/// The games already on a memory card, for the wizard to add to.
#[tauri::command]
fn memcard_games(drive_path: String) -> Vec<memcard::CardGameView> {
    memcard::games(Path::new(&drive_path))
}

/// Write a memory card. Returns what was left off, and why.
#[tauri::command]
fn create_memory_card(request: memcard::CardRequest) -> Result<Vec<String>, String> {
    memcard::write(&request)
}

/// Where a game keeps its saves, from Ludusavi. Refused unless switched on,
/// because the first call downloads the list.
#[tauri::command]
async fn lookup_save_location(
    title: String,
    executable: String,
) -> Result<Option<ludusavi::SaveLocations>, String> {
    if !settings::load().ludusavi_enabled {
        return Err("Ludusavi lookups are off in Settings.".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let steam_id = executable.strip_prefix("steam://rungameid/");
        ludusavi::lookup(&title, steam_id)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PickedSaveFolder {
    path: String,
    /// The portable form, or empty when the folder is somewhere no other
    /// machine could find — inside a game's install folder, say.
    template: String,
}

/// Point at a game's save folder by hand.
#[tauri::command]
async fn pick_save_folder(
    window: tauri::WebviewWindow,
) -> Result<Option<PickedSaveFolder>, String> {
    let Some(folder) = window
        .dialog()
        .file()
        .set_title("Choose the folder the game saves into")
        .blocking_pick_folder()
    else {
        return Ok(None);
    };
    let path = folder
        .into_path()
        .map_err(|e| format!("That folder cannot be read: {e}"))?;
    Ok(Some(PickedSaveFolder {
        template: saves::template_from_path(&path).unwrap_or_default(),
        path: path.to_string_lossy().into_owned(),
    }))
}

/// Whether this machine is seeing this cartridge for the first time, which is
/// when the launcher plays the unboxing. Asking records the answer.
#[tauri::command]
fn first_insert(title: String) -> bool {
    unboxed::first_time(&title)
}

/// Every cartridge this wizard has written, newest first.
#[tauri::command]
fn list_created() -> Vec<created::CreatedView> {
    created::list()
}

/// Take a cartridge off that list. Touches no drive.
#[tauri::command]
fn forget_created(id: String) -> Result<(), String> {
    created::forget(&id)
}

// --------------------------------------------------------------------------
// Entry point
// --------------------------------------------------------------------------

/// The launcher popup's own way into the wizard, alongside the tray menu's.
///
/// The popup is a cartridge's home, not a settings surface, so this jumps
/// straight past cartridge creation to Settings — the same place the tray
/// menu's "Open settings" lands.
#[tauri::command]
fn open_wizard_settings(app: tauri::AppHandle) {
    spawn_open_wizard(app, true);
}

/// The wizard itself, on the tab it opens with.
///
/// The popup had a way to Settings but none to the thing Settings belongs to,
/// so making a second cartridge meant finding the tray icon.
#[tauri::command]
fn open_wizard_window(app: tauri::AppHandle) {
    spawn_open_wizard(app, false);
}

/// Build the wizard from a thread that is not the event loop's.
///
/// A command handler and a tray-menu handler both run on the main thread, and
/// building a webview window there does not work: the native window is created,
/// centred and sized, and then its webview never finishes initialising — so
/// `on_page_load` never fires, create.js never runs, and the window sits
/// invisible forever. Nothing is deadlocked, which is what makes it confusing;
/// the message pump answers, the launcher redraws, and the only symptom is that
/// Settings and the tray's wizard entries appear to do nothing at all.
///
/// Creating it from another thread leaves the event loop free to service the
/// creation it has been asked for. `setup` is the exception that shows the rule:
/// it runs before the event loop starts, so it can and does call open_wizard
/// directly.
fn spawn_open_wizard(app: tauri::AppHandle, open_settings: bool) {
    std::thread::spawn(move || {
        if let Err(error) = open_wizard(&app, open_settings) {
            eprintln!("could not open the wizard: {error}");
        }
    });
}

/// Ask Windows to round the window, and let it be the only thing that does.
///
/// Two curves were fighting. The app drew its own rounded corner in CSS, which
/// on a transparent window leaves the area outside the curve see-through; DWM
/// rounds an undecorated window at its own radius and paints a border on that.
/// Neither radius is the other's, so between them sat a crescent of DWM's
/// border over the app's transparent corner — a pale hook at every corner.
///
/// Squaring both was one way out and looked like what it was. The other is to
/// have exactly one rounder: the window is opaque and square, painted edge to
/// edge, and DWM clips the corner. That is the shape Windows draws for every
/// other window, anti-aliased, with the shadow that belongs to it.
#[cfg(windows)]
fn round_dwm_corners(window: &tauri::WebviewWindow) {
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    };

    let Ok(handle) = window.hwnd() else { return };
    let preference = DWMWCP_ROUND;
    unsafe {
        DwmSetWindowAttribute(
            handle.0 as _,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            std::ptr::addr_of!(preference).cast(),
            std::mem::size_of_val(&preference) as u32,
        );
    }
}

#[cfg(not(windows))]
fn round_dwm_corners(_window: &tauri::WebviewWindow) {}

/// Put the launcher away while the wizard is up, and bring it back after.
///
/// The popup is always_on_top, because a cartridge going in has to land over
/// whatever is already running. That makes it the one window a wizard opened
/// from its own Settings link cannot appear in front of: the wizard is there,
/// focused and taking input, with the popup sitting on top of the part you
/// clicked to get it. Hiding it is not decoration — it is the difference
/// between the Settings link working and appearing not to.
///
/// A no-op when the app was started with --create, which has no popup at all.
fn hide_launcher(app: &tauri::AppHandle) {
    if let Some(launcher) = app.get_webview_window("main") {
        let _ = launcher.hide();
    }
}

fn show_launcher(app: &tauri::AppHandle) {
    if let Some(launcher) = app.get_webview_window("main") {
        let _ = launcher.show();
        let _ = launcher.set_focus();
    }
}

fn open_wizard(app: &tauri::AppHandle, open_settings: bool) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("create") {
        window.show()?;
        window.set_focus()?;
        hide_launcher(app);
        if open_settings {
            window.emit("open-settings", ())?;
        }
        return Ok(());
    }

    // Resizable, unlike the popup: 1030x660 is logical pixels, so at 150% or
    // 200% desktop scaling the wizard is taller than the screen it opens on and
    // a fixed window leaves the title bar and the game list off the edge with
    // no way back. The minimum keeps the sidebar and both columns usable.
    let wizard = WebviewWindowBuilder::new(app, "create", WebviewUrl::App("create.html".into()))
        .title("Create cartridge")
        .inner_size(1030.0, 660.0)
        .min_inner_size(870.0, 520.0)
        .resizable(true)
        .decorations(false)
        // Opaque on purpose. Transparency is what made the corner artefact
        // possible: it gave DWM's border somewhere to show through. Nothing
        // here needs to see the desktop — the card fills the window.
        .transparent(false)
        // WebView2 installs an OS-level drag-and-drop handler and Tauri turns it
        // on by default. It swallows the events before the page sees them, so
        // HTML5 drag-and-drop inside the webview does nothing — a row could be
        // gripped and then would not move. Nothing here wants files dropped onto
        // the window; the wizard wants to reorder its own list.
        .disable_drag_drop_handler()
        .center()
        .visible(false)
        // Built hidden and shown here, once the page has loaded.
        //
        // This used to be the frontend's job. It is not a job the frontend can
        // be trusted with: create.js shows the window at the end of its startup,
        // so any await in there that never settles leaves a window that exists,
        // has size and position, and is permanently invisible — which from the
        // outside is indistinguishable from the app having frozen. Page load is
        // a signal this side already has, and it does not depend on the wizard
        // finishing its library scan.
        .on_page_load(move |window, payload| {
            if payload.event() != PageLoadEvent::Finished {
                return;
            }
            let _ = window.show();
            let _ = window.set_focus();
            hide_launcher(window.app_handle());
            if open_settings {
                let _ = window.emit("open-settings", ());
            }
        })
        .build()?;

    round_dwm_corners(&wizard);
    set_window_icons(&wizard, WIZARD_ICON);

    // The popup comes back when the wizard goes away, whichever way it goes:
    // create.js closes the window, so this is a destroy rather than a hide.
    // Registered on the window rather than inside on_page_load, so a wizard
    // that never finishes loading still gives the launcher back when it is
    // closed.
    let handle = app.clone();
    wizard.on_window_event(move |event| {
        if matches!(
            event,
            tauri::WindowEvent::Destroyed | tauri::WindowEvent::CloseRequested { .. }
        ) {
            show_launcher(&handle);
        }
    });

    Ok(())
}

fn main() {
    // Both windows start hidden; the frontend shows itself once it has drawn,
    // so the user never sees an empty frame.
    // `--settings` is `--create` that lands on the Settings page. The tray is
    // in another process now, so "open settings" has to survive being asked for
    // across a process boundary rather than through a menu handler.
    let args: Vec<String> = std::env::args().skip(1).collect();

    // The elevated half of Eject, which is this same executable run again with
    // administrator. It does its work and exits before any window is built, so
    // the UAC prompt does not flash a second launcher at the user.
    #[cfg(target_os = "windows")]
    if let Some(index) = args.iter().position(|arg| arg == "--eject") {
        let drive = args.get(index + 1).cloned().unwrap_or_default();
        std::process::exit(gamepak_core::eject::run_elevated(&drive) as i32);
    }

    // Play and Eject for a front-end with its own buttons. Before the insert
    // reaction, which is about a cartridge arriving and not about being asked.
    let play = headless::play_index(&args);
    if play.is_some() || headless::wants_safe_eject(&args) {
        let drive = cartridge::drive_from_args(args.iter().cloned());
        if drive.is_empty() {
            eprintln!("--play and --safe-eject need --drive");
            std::process::exit(1);
        }
        std::process::exit(match play {
            Some(Ok(index)) => headless::play(&drive, index),
            Some(Err(why)) => {
                eprintln!("{why}");
                1
            }
            None => headless::safe_eject(&drive, args.iter().any(|arg| arg == "--force")),
        });
    }
    let settings = args.iter().any(|arg| arg == "--settings");
    let wizard = settings || args.iter().any(|arg| arg == "--create");

    // What the user asked for on insert, settled before Tauri is touched — so
    // `none` costs a process start and nothing else, and `auto_launch_game`
    // never initialises a webview it is not going to show.
    //
    // Decided here rather than in whatever started us, because three separate
    // things do: the resident watcher, the udev helper on a system install, and
    // the tray menu. A setting honoured by one of those and not the others
    // would be worse than no setting.
    let is_trigger = args
        .iter()
        .any(|arg| arg == "--nfc-ndef" || arg == "--trigger");
    if is_trigger {
        // Every GamePak opens the launcher at READY and starts on Play. With
        // "Launch directly on trigger" on, anything that is not a cartridge or
        // a game on another PC is started here and now, with no window.
        match trigger_id(&args).and_then(|input| input.map_or(Ok(None), |i| i.into_id().map(Some)))
        {
            Ok(Some(id)) => {
                let registry = gamepak_core::gamepak::registry_path();
                remember_trigger(&registry, &id);
                let is_cartridge = gamepak_core::gamepak::GamePakId::parse(id.trim())
                    .and_then(|parsed| gamepak_core::gamepak::lookup_from(&registry, &parsed))
                    .map(|pak| {
                        matches!(
                            pak.action,
                            gamepak_core::gamepak::Action::Cartridge { .. }
                                | gamepak_core::gamepak::Action::Remote { .. }
                        ) || !settings::load().launch_on_trigger
                    });
                match is_cartridge {
                    Ok(false) => {
                        if let Err(error) = gamepak_core::gamepak::trigger_from(&registry, &id) {
                            eprintln!("GamePak trigger failed: {error}");
                            std::process::exit(1);
                        }
                        return;
                    }
                    Ok(true) => {}
                    Err(error) => {
                        eprintln!("GamePak trigger failed: {error}");
                        std::process::exit(1);
                    }
                }
            }
            Ok(None) => {}
            Err(error) => {
                eprintln!("GamePak trigger failed: {error}");
                std::process::exit(1);
            }
        }
    }
    if !wizard && !is_trigger {
        if let Some(reaction) = reaction_on_insert(&args) {
            if act_on_insert(reaction) {
                return;
            }
            // Falls through to the window, which is what a reaction that could
            // not be carried out asked for by returning false.
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            drive_path,
            nfc_selection,
            opens_memory_card,
            parse_cartridge,
            launch_game,
            eject_drive,
            focus_window,
            debug_logging,
            debug_log,
            can_eject,
            cartridge_busy,
            eject_drive_forcing,
            eject_with_guard,
            cartridge_stats,
            save_slots,
            carried_home,
            frontends,
            set_frontend,
            sync_saves,
            push_saves,
            shader_slots,
            pull_shaders,
            push_shaders,
            resolve_save_conflict,
            list_games,
            list_filesystems,
            game_cover,
            get_settings,
            set_settings,
            suggest_collection_name,
            pick_cover_image,
            pick_game_folder,
            pick_rom_file,
            cartridge_health,
            read_cartridge_for_edit,
            update_cartridge,
            refetch_cartridge_artwork,
            host_platform,
            tuning_plan,
            apply_tuning,
            sgdb_search_games,
            sgdb_get_artwork,
            sgdb_download_artwork,
            sgdb_last_used_artwork,
            list_target_drives,
            moonlight_hosts,
            add_gamepak,
            registered_gamepaks,
            remove_gamepak,
            remove_gamepak_trigger,
            moonlight_apps,
            last_trigger,
            add_gamepak_trigger,
            list_unmounted_volumes,
            mount_volume,
            format_plan,
            executable_choices,
            steam_registration,
            holds_steam_games,
            steam_registration_plan,
            register_with_steam,
            unregister_from_steam,
            create_cartridge,
            list_created,
            forget_created,
            first_insert,
            memory_card,
            memcard_copy,
            memcard_remove,
            memcard_reveal,
            memcard_games,
            create_memory_card,
            lookup_save_location,
            pick_save_folder,
            open_wizard_settings,
            open_wizard_window,
        ])
        .setup(move |app| {
            let _ = APP.set(app.handle().clone());
            if wizard {
                // The same door the launcher's Settings link uses, rather than
                // a second builder that had drifted to a fixed size the
                // resizing comment in open_wizard explicitly argues against.
                open_wizard(app.handle(), settings)?;
            } else {
                let launcher =
                    WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                        .title("PC GamePak")
                        .inner_size(420.0, 630.0)
                        .resizable(false)
                        .decorations(false)
                        .transparent(false)
                        // The popup has to land on top of whatever is running.
                        .always_on_top(true)
                        .center()
                        .visible(false)
                        .build()?;
                round_dwm_corners(&launcher);
                set_window_icons(&launcher, LAUNCHER_ICON);
                // Closing the window while a game it can see is running only
                // hides it: the process stays to watch the game, and exits when
                // the game does (see `finish`). A session counted by the window
                // alone ends with the window, as it always has.
                let hideable = launcher.clone();
                launcher.on_window_event(move |event| match event {
                    tauri::WindowEvent::CloseRequested { api, .. } if still_watching() => {
                        api.prevent_close();
                        WINDOW_GONE.store(true, Ordering::SeqCst);
                        let _ = hideable.hide();
                        end_window_sessions();
                    }
                    tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed => {
                        end_every_session();
                    }
                    _ => {}
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Tauri application");
}
