//! Cartridge watcher.
//!
//! Replaces the resident PowerShell monitor. PowerShell holds the whole .NET
//! runtime and a WMI subscription open for the entire login session, which costs
//! tens of megabytes to do nothing. This does the same job by blocking on the
//! Windows message queue: no polling, no timer, no CPU while idle.
//!
//! On Linux the system install does not need this at all — udev is already
//! running as part of the OS and starts the launcher through a systemd unit, so
//! nothing is resident. See `linux/99-pc-gamepak.rules`.
//!
//! The Linux arm here is for the other shape of install: no root, no udev rule,
//! a systemd *user* service. It blocks in poll() on the mount table instead,
//! which is what a sandboxed package can do — and, as it happens, fires when the
//! cartridge is actually readable rather than when the kernel first sees the
//! partition. See `linux.rs`.
//!
//! Flow: volume arrives -> is there a cartridge.conf on it? -> start the
//! launcher with `--drive X:\` and go back to sleep.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod autoplay;
#[cfg(target_os = "linux")]
mod barcode;
mod launcher;
#[cfg(target_os = "linux")]
mod libnfc;
#[cfg(not(windows))]
mod linux;
mod log;
#[cfg(not(windows))]
mod mounts;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod nfc;
#[cfg(windows)]
mod tray;

#[cfg(not(windows))]
fn main() {
    linux::run()
}

#[cfg(windows)]
fn main() {
    windows_watcher::run()
}

#[cfg(windows)]
mod windows_watcher {
    use std::collections::HashMap;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::process::Child;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::CreateMutexW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EnumWindows,
        GetClassNameW, GetMessageW, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
        IsWindowVisible, KillTimer, PostMessageW, PostQuitMessage, RegisterClassW,
        RegisterWindowMessageW, SetForegroundWindow, SetTimer, ShowWindow, MSG, SW_RESTORE,
        WM_CLOSE, WM_COPYDATA, WM_DESTROY, WM_DEVICECHANGE, WM_LBUTTONUP, WM_RBUTTONUP, WM_TIMER,
        WNDCLASSW, WS_OVERLAPPED,
    };

    /// A volume has been inserted and is available.
    const DBT_DEVICEARRIVAL: u32 = 0x8000;
    /// `dbch_devicetype` for a logical volume.
    const DBT_DEVTYP_VOLUME: u32 = 0x0000_0002;

    /// Windows re-broadcasts arrival for the same volume; ignore repeats.
    const DEBOUNCE: Duration = Duration::from_secs(4);

    /// Files that mark a volume as a cartridge rather than an ordinary drive.
    const MARKERS: [&str; 3] = ["cartridge.conf", "memorycard.conf", "autorun.inf"];

    /// How long AutoPlay's own folder window takes to appear after arrival,
    /// before it is worth looking for.
    const AUTOPLAY_WINDOW_DELAY: Duration = Duration::from_millis(900);

    /// Header shared by every `WM_DEVICECHANGE` payload.
    ///
    /// Declared here rather than imported: the layout is fixed ABI, and writing
    /// it out keeps this file compiling against any windows-sys minor version.
    #[repr(C)]
    struct DevBroadcastHdr {
        dbch_size: u32,
        dbch_devicetype: u32,
        dbch_reserved: u32,
    }

    /// Payload for `DBT_DEVTYP_VOLUME`.
    #[repr(C)]
    struct DevBroadcastVolume {
        dbcv_size: u32,
        dbcv_devicetype: u32,
        dbcv_reserved: u32,
        /// Bit 0 is A:, bit 1 is B:, and so on.
        dbcv_unitmask: u32,
        dbcv_flags: u16,
    }

    /// Explorer's "I have restarted, add your icon again" broadcast, whose id
    /// is only known at runtime.
    static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

    /// Timer that keeps trying to add the tray icon until the shell takes it.
    const TRAY_RETRY_TIMER: usize = 1;
    /// How often to try. The logon task can start before Explorer has a
    /// notification area; it usually has one within a few seconds.
    const TRAY_RETRY_MS: u32 = 3_000;
    /// Give up after this many tries (about two minutes), and say so once.
    const TRAY_RETRY_LIMIT: u32 = 40;
    static TRAY_RETRIES: AtomicU32 = AtomicU32::new(0);

    /// Tags a `WM_COPYDATA` as the launcher asking for a notification. Must
    /// match `NOTIFY_COPYDATA` in `tauri-ui/src-tauri/src/main.rs`: "GPNT".
    const NOTIFY_COPYDATA: usize = 0x4750_4E54;
    /// A title and a body the balloon can hold, with room over. Anything much
    /// longer is not from the launcher.
    const NOTIFY_MAX_BYTES: u32 = 4 * 1024;

    /// `COPYDATASTRUCT`, declared here for the same reason as the broadcast
    /// structs above.
    #[repr(C)]
    struct CopyDataStruct {
        dw_data: usize,
        cb_data: u32,
        lp_data: *const std::ffi::c_void,
    }

    /// The launcher this tray opened, and the cartridge it was opened on.
    ///
    /// Tracked so a second click on the same cartridge brings that window back
    /// rather than starting a second copy of it — which matters more now than
    /// it did, because the launcher minimises itself when a game starts instead
    /// of closing, and the window the user wants is usually already there.
    static TRAY_LAUNCHER: Mutex<Option<(PathBuf, Child)>> = Mutex::new(None);

    /// Last time each drive letter was acted on, for debouncing.
    ///
    /// Behind a Mutex rather than a `static mut`: the message loop is
    /// single-threaded, so there is no contention to speak of, but a mutable
    /// reference to a static is undefined behaviour the moment that assumption
    /// stops holding, and the compiler is right to refuse it.
    static SEEN: Mutex<Option<HashMap<char, Instant>>> = Mutex::new(None);

    /// True once AutoPlay asks the watcher before opening a drive, so there is
    /// no Explorer window to close afterwards.
    static AUTOPLAY_CANCELLED: AtomicBool = AtomicBool::new(false);

    pub fn run() {
        crate::log::line("watcher starting");

        // Every logon starts this fresh, and a crash-restart from the
        // scheduled task can overlap the old instance for a moment — without
        // this, two watchers both hear the same WM_DEVICECHANGE broadcast and
        // each opens its own launcher window for the same cartridge. The
        // mutex is never released explicitly: it goes away when the process
        // does, which is the only time it should.
        if !acquired_single_instance() {
            crate::log::line("another watcher is already running; exiting");
            return;
        }

        *SEEN.lock().expect("no other thread to poison it") = Some(HashMap::new());
        crate::nfc::start();

        let class_name = wide("PcCartridgeWatcher");

        // A hidden *top-level* window, not a message-only (HWND_MESSAGE) one:
        // Windows does not deliver broadcast WM_DEVICECHANGE messages to
        // message-only windows, so a message-only window would never see a
        // volume arrive. The window is simply never shown.
        let hwnd = unsafe {
            let instance = GetModuleHandleW(std::ptr::null());

            let class = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(wnd_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance,
                hIcon: 0,
                hCursor: 0,
                hbrBackground: 0,
                lpszMenuName: std::ptr::null(),
                lpszClassName: class_name.as_ptr(),
            };

            if RegisterClassW(&class) == 0 {
                crate::log::line("could not register the window class; giving up");
                return;
            }

            CreateWindowExW(
                0,
                class_name.as_ptr(),
                wide("PC GamePak Watcher").as_ptr(),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                0,
                0,
                instance,
                std::ptr::null(),
            )
        };

        if hwnd == 0 {
            crate::log::line("could not create the listener window; giving up");
            return;
        }

        // Registered before the first add, not after it. The scheduled task
        // starts at logon, sometimes before Explorer has built the taskbar, and
        // then the add fails; `TaskbarCreated` is how Explorer says it is ready,
        // so the watcher has to be listening for it from the start. Waiting
        // for a successful add first is how the icon went missing for the
        // whole session on 2026-09-10 and 2026-09-11.
        TASKBAR_CREATED.store(
            unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) },
            Ordering::Relaxed,
        );
        if !crate::tray::add(hwnd) {
            // And a timer as well, because a shell that is up but still busy
            // refuses the add without ever broadcasting anything afterwards.
            crate::log::line("could not add the tray icon yet; retrying");
            unsafe { SetTimer(hwnd, TRAY_RETRY_TIMER, TRAY_RETRY_MS, None) };
        }

        // Asked before AutoPlay acts on a new volume, so a cartridge never gets
        // the Explorer window at all. Without it, the window is closed after it
        // appears instead — see `on_volume_arrived`.
        if crate::autoplay::register() {
            AUTOPLAY_CANCELLED.store(true, Ordering::Relaxed);
            crate::log::line("AutoPlay will ask before opening a cartridge");
        } else {
            crate::log::line("could not register with AutoPlay; its window will be closed instead");
        }

        crate::log::line("listening for volume arrivals");

        // Blocks here for the rest of the session. GetMessageW sleeps in the
        // kernel until something arrives, so idle CPU is exactly zero.
        let mut msg = MSG {
            hwnd: 0,
            message: 0,
            wParam: 0,
            lParam: 0,
            time: 0,
            pt: windows_sys::Win32::Foundation::POINT { x: 0, y: 0 },
        };
        unsafe {
            while GetMessageW(&mut msg, 0, 0, 0) > 0 {
                DispatchMessageW(&msg);
            }
        }
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_DEVICECHANGE => {
                if wparam as u32 == DBT_DEVICEARRIVAL && lparam != 0 {
                    let header = lparam as *const DevBroadcastHdr;
                    if (*header).dbch_devicetype == DBT_DEVTYP_VOLUME {
                        let volume = lparam as *const DevBroadcastVolume;
                        for letter in letters_from_mask((*volume).dbcv_unitmask) {
                            on_volume_arrived(letter);
                        }
                    }
                }
                0
            }
            crate::tray::WM_TRAYICON => {
                // The mouse message that caused this arrives in the low word of
                // lparam; everything else the icon reports is movement.
                match lparam as u32 & 0xffff {
                    WM_LBUTTONUP => on_tray_clicked(hwnd, true),
                    WM_RBUTTONUP => on_tray_clicked(hwnd, false),
                    _ => {}
                }
                0
            }
            WM_COPYDATA => {
                let data = lparam as *const CopyDataStruct;
                if data.is_null() || (*data).dw_data != NOTIFY_COPYDATA {
                    return 0;
                }
                let bytes = (*data).cb_data;
                if bytes > NOTIFY_MAX_BYTES || (*data).lp_data.is_null() {
                    return 0;
                }
                let raw = std::slice::from_raw_parts((*data).lp_data.cast::<u8>(), bytes as usize);
                let (title, body) = split_notification(raw);
                let shown = crate::tray::balloon(hwnd, &title, &body);
                crate::log::line(if shown {
                    "posted an insert notification"
                } else {
                    "could not post an insert notification; the launcher will open"
                });
                shown as LRESULT
            }
            WM_TIMER if wparam == TRAY_RETRY_TIMER => {
                let tries = TRAY_RETRIES.fetch_add(1, Ordering::Relaxed) + 1;
                if crate::tray::add(hwnd) {
                    KillTimer(hwnd, TRAY_RETRY_TIMER);
                    crate::log::line(&format!("added the tray icon after {tries} retries"));
                } else if tries >= TRAY_RETRY_LIMIT {
                    KillTimer(hwnd, TRAY_RETRY_TIMER);
                    crate::log::line(
                        "could not add the tray icon; carrying on without it \
                         until Explorer restarts",
                    );
                }
                0
            }
            WM_DESTROY => {
                crate::tray::remove(hwnd);
                PostQuitMessage(0);
                0
            }
            other => {
                // Explorer restarted and threw away every icon in the
                // notification area, this one included. Nothing says so except
                // this broadcast, and a watcher with no icon looks to the user
                // exactly like a watcher that has died.
                let taskbar = TASKBAR_CREATED.load(Ordering::Relaxed);
                if taskbar != 0 && other == taskbar {
                    // If this add fails too, the retry timer (when running)
                    // keeps going; it is only stopped once the icon is there.
                    if crate::tray::add(hwnd) {
                        KillTimer(hwnd, TRAY_RETRY_TIMER);
                    }
                    return 0;
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
    }

    /// A click on the tray icon.
    ///
    /// Left click on a single cartridge opens it, because that is the only
    /// thing anyone wants in the overwhelmingly common case and a menu to
    /// choose from a list of one is a menu for its own sake. Everything else —
    /// no cartridge, several cartridges, or a right click — gets the menu.
    fn on_tray_clicked(hwnd: HWND, left: bool) {
        let cartridges: Vec<crate::tray::Volume> = crate::tray::candidate_volumes()
            .into_iter()
            // The cheap test, not the retrying one: this is a menu being drawn
            // under the cursor, and a drive that is plugged in is readable.
            .filter(|volume| MARKERS.iter().any(|name| volume.root.join(name).is_file()))
            .collect();

        if left && cartridges.len() == 1 {
            open_cartridge(&cartridges[0].root);
            return;
        }

        match crate::tray::show_menu(hwnd, &cartridges) {
            0 => {}
            crate::tray::ID_WIZARD => {
                crate::launcher::open_wizard(false);
            }
            crate::tray::ID_SETTINGS => {
                crate::launcher::open_wizard(true);
            }
            crate::tray::ID_QUIT => {
                crate::log::line("quitting from the tray");
                unsafe { DestroyWindow(hwnd) };
            }
            chosen => {
                if let Some(cartridge) = cartridges.get(chosen as usize - 1) {
                    open_cartridge(&cartridge.root);
                }
            }
        }
    }

    /// Show the launcher for `root`, reusing the window if one is already open.
    fn open_cartridge(root: &Path) {
        let mut guard = TRAY_LAUNCHER.lock().expect("no other thread to poison it");

        if let Some((open_root, child)) = guard.as_mut() {
            if matches!(child.try_wait(), Ok(None)) {
                // Still running. If it is showing this cartridge, the window is
                // the answer — minimised behind a game, most likely.
                if open_root == root && focus_process(child.id()) {
                    return;
                }
                // A different cartridge, or a window that cannot be found. One
                // launcher at a time either way.
                crate::launcher::close(child);
            }
        }

        *guard = crate::launcher::open(root, crate::launcher::Why::Asked)
            .map(|child| (root.to_path_buf(), child));
    }

    /// Bring a window belonging to `pid` to the front. False if it has none.
    fn focus_process(pid: u32) -> bool {
        let mut found: (u32, HWND) = (pid, 0);
        unsafe {
            EnumWindows(
                Some(enum_process_windows),
                &mut found as *mut (u32, HWND) as LPARAM,
            );
        }

        let hwnd = found.1;
        if hwnd == 0 {
            return false;
        }

        unsafe {
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            SetForegroundWindow(hwnd);
        }
        true
    }

    unsafe extern "system" fn enum_process_windows(hwnd: HWND, lparam: LPARAM) -> i32 {
        let found = &mut *(lparam as *mut (u32, HWND));

        // A minimised window is still visible in this sense; a hidden one — the
        // launcher before it has drawn, or a helper window — is not, and
        // raising that would flash nothing at the user.
        if IsWindowVisible(hwnd) == 0 {
            return 1;
        }

        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == found.0 {
            found.1 = hwnd;
            return 0; // stop; the first one is the launcher's only window
        }
        1
    }

    /// A notification payload: UTF-16LE title, a NUL, UTF-16LE body.
    ///
    /// Read from bytes rather than cast to `&[u16]`, because nothing promises
    /// the sender's buffer is aligned for one. An odd trailing byte is dropped,
    /// and a payload with no NUL is all title.
    fn split_notification(raw: &[u8]) -> (String, String) {
        let units: Vec<u16> = raw
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        match units.iter().position(|unit| *unit == 0) {
            Some(nul) => (
                String::from_utf16_lossy(&units[..nul]),
                String::from_utf16_lossy(&units[nul + 1..]),
            ),
            None => (String::from_utf16_lossy(&units), String::new()),
        }
    }

    /// Expand a `dbcv_unitmask` bitfield into drive letters.
    fn letters_from_mask(mask: u32) -> Vec<char> {
        (0..26)
            .filter(|bit| mask & (1 << bit) != 0)
            .map(|bit| (b'A' + bit as u8) as char)
            .collect()
    }

    fn on_volume_arrived(letter: char) {
        let now = Instant::now();

        {
            let mut guard = SEEN.lock().expect("no other thread to poison it");
            let seen = guard.get_or_insert_with(HashMap::new);
            if let Some(last) = seen.get(&letter) {
                if now.duration_since(*last) < DEBOUNCE {
                    crate::log::line(&format!("{letter}: ignoring repeat arrival"));
                    return;
                }
            }
        }

        let root = PathBuf::from(format!("{letter}:\\"));

        // Not every drive is a cartridge. Without this check the launcher would
        // pop up for every USB stick and phone the user plugs in.
        if !is_cartridge(&root) {
            crate::log::line(&format!(
                "{letter}: no cartridge.conf, memorycard.conf or autorun.inf at the root; ignoring"
            ));
            return;
        }

        // Recorded only once it is known to be a cartridge, so a plain USB
        // stick plugged in twice is not debounced into silence.
        if let Some(seen) = SEEN.lock().expect("no other thread to poison it").as_mut() {
            seen.insert(letter, now);
        }

        if crate::launcher::open(&root, crate::launcher::Why::Inserted).is_some() {
            crate::log::line(&format!("{letter}: opened the launcher"));
        }

        // Every cartridge already opts out of the OS running anything on its
        // own — see cartridge.rs. AutoPlay's "open folder" is the same idea in
        // reverse: the launcher is the window a cartridge should show, so the
        // Explorer window AutoPlay opens for it is closed on its own thread,
        // rather than blocking WM_DEVICECHANGE while it waits for that window
        // to exist.
        if !AUTOPLAY_CANCELLED.load(Ordering::Relaxed) {
            std::thread::spawn(move || close_autoplay_window(letter));
        }
    }

    /// Close the Explorer window AutoPlay opened for `letter`, if one exists.
    ///
    /// There is no event for "AutoPlay opened a folder", so this waits a beat
    /// and then looks: Explorer's title for a drive root always includes
    /// `(X:)`, which is enough to find the right window without COM.
    fn close_autoplay_window(letter: char) {
        std::thread::sleep(AUTOPLAY_WINDOW_DELAY);

        let needle = format!("({letter}:)").encode_utf16().collect::<Vec<u16>>();
        unsafe {
            EnumWindows(
                Some(enum_explorer_windows),
                &needle as *const Vec<u16> as LPARAM,
            );
        }
    }

    unsafe extern "system" fn enum_explorer_windows(hwnd: HWND, lparam: LPARAM) -> i32 {
        let needle = &*(lparam as *const Vec<u16>);

        let mut class = [0u16; 64];
        let class_len = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
        // "CabinetWClass" is a real Explorer folder window, not the desktop or
        // a taskbar/tray host that also happens to own a top-level HWND.
        if class_len <= 0
            || &class[..class_len as usize]
                != "CabinetWClass"
                    .encode_utf16()
                    .collect::<Vec<u16>>()
                    .as_slice()
        {
            return 1; // keep enumerating
        }

        let mut title = [0u16; 512];
        let title_len = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);
        if title_len > 0 && contains_utf16(&title[..title_len as usize], needle) {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
        }
        1
    }

    fn contains_utf16(haystack: &[u16], needle: &[u16]) -> bool {
        !needle.is_empty()
            && haystack
                .windows(needle.len())
                .any(|window| window == needle)
    }

    /// A cartridge is a volume with a manifest at its root. Retried briefly:
    /// the volume is mounted by the time the message arrives, but the filesystem
    /// is not always readable on the very first attempt.
    fn is_cartridge(root: &Path) -> bool {
        for attempt in 0..6 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(250));
            }
            if MARKERS.iter().any(|name| root.join(name).is_file()) {
                return true;
            }
        }
        false
    }

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// True when this process is the only watcher running.
    ///
    /// The handle returned by `CreateMutexW` is deliberately leaked: it must
    /// live for the process's whole lifetime, and the OS reclaims it on exit
    /// regardless.
    fn acquired_single_instance() -> bool {
        let name = wide("Global\\PcCartridgeWatcherSingleInstance");
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if handle == 0 {
            // Could not even ask; do not block a real launch on this.
            return true;
        }
        unsafe { windows_sys::Win32::Foundation::GetLastError() != ERROR_ALREADY_EXISTS }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_notification_payload_splits_at_its_nul() {
            let bytes = |text: &str| -> Vec<u8> {
                text.encode_utf16().flat_map(u16::to_le_bytes).collect()
            };
            let mut raw = bytes("FTL: Faster Than Light");
            raw.extend([0, 0]);
            raw.extend(bytes("Open PC GamePak to play — ✓"));
            assert_eq!(
                split_notification(&raw),
                (
                    "FTL: Faster Than Light".to_string(),
                    "Open PC GamePak to play — ✓".to_string()
                )
            );

            // No NUL: all title. An odd byte on the end is not half a letter.
            let mut raw = bytes("Cartridge inserted");
            raw.push(0x41);
            assert_eq!(
                split_notification(&raw),
                ("Cartridge inserted".to_string(), String::new())
            );
        }

        #[test]
        fn expands_a_unit_mask_into_drive_letters() {
            assert_eq!(letters_from_mask(0b1000), vec!['D']);
            assert_eq!(letters_from_mask(0b1), vec!['A']);
            assert_eq!(letters_from_mask(0b1100), vec!['C', 'D']);
            assert_eq!(letters_from_mask(0), Vec::<char>::new());
            assert_eq!(letters_from_mask(1 << 25), vec!['Z']);
        }

        #[test]
        fn finds_the_drive_letter_wherever_it_sits_in_the_title() {
            let needle: Vec<u16> = "(D:)".encode_utf16().collect();
            assert!(contains_utf16(
                &"TEST (D:)".encode_utf16().collect::<Vec<u16>>(),
                &needle
            ));
            assert!(!contains_utf16(
                &"TEST (E:)".encode_utf16().collect::<Vec<u16>>(),
                &needle
            ));
            assert!(!contains_utf16(&[], &needle));
        }
    }
}
