//! The notification-area icon.
//!
//! The launcher is opened when a cartridge arrives and closes when it is
//! dismissed, which is right for a popup and wrong for the only way back to it:
//! close the window and the cartridge is still plugged in, still readable, and
//! completely unreachable short of unplugging it and plugging it in again.
//!
//! The icon lives here rather than in the launcher because the watcher is the
//! process that is already resident. Keeping the launcher alive to hold a tray
//! icon would keep a WebView2 host resident too — tens of megabytes for a 16
//! pixel picture, on a machine that is meant to be running a game. This process
//! is a hidden window and a message loop, and it already knows how to start a
//! launcher, so the icon costs a struct and a menu.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;

use windows_sys::Win32::Foundation::{HWND, POINT};
use windows_sys::Win32::Storage::FileSystem::{
    GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, GetSystemMetrics, LoadIconW,
    LoadImageW, SetForegroundWindow, TrackPopupMenu, IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTCOLOR,
    MF_SEPARATOR, MF_STRING, SM_CXSMICON, SM_CYSMICON, TPM_BOTTOMALIGN, TPM_RETURNCMD,
    TPM_RIGHTBUTTON,
};

/// Our own `WM_` for icon activity. `WM_APP` and up are reserved for exactly
/// this and never collide with anything Windows sends.
pub const WM_TRAYICON: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 1;

/// The one icon this process owns.
const ICON_ID: u32 = 1;

/// `GetDriveTypeW` results worth looking at.
///
/// `DRIVE_FIXED` is in the list because `GetDriveTypeW` describes the volume
/// rather than the bus, and calls an NVMe stick in a USB enclosure FIXED —
/// exactly like the internal disk. Which is the whole hardware this project is
/// built around, so filtering on REMOVABLE alone found no cartridges at all.
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;

/// Menu ids. Cartridges take 1.., so the fixed entries start well clear of any
/// plausible number of drive letters.
pub const ID_WIZARD: u32 = 100;
pub const ID_SETTINGS: u32 = 101;
pub const ID_QUIT: u32 = 199;

/// A removable volume, named the way the user would name it.
pub struct Volume {
    pub root: PathBuf,
    pub letter: char,
    /// The volume label, or the letter again when the drive has none.
    pub name: String,
}

/// Every mounted volume that could hold a cartridge.
///
/// Not the bus test `core` uses before offering a drive to the formatter. That
/// one opens the device and asks it, because getting it wrong there means
/// erasing the wrong disk. Nothing here erases anything: the caller keeps the
/// volumes with a cartridge marker at the root, which is the same definition
/// the watcher uses to decide whether to open the launcher on a drive that has
/// just arrived. A stricter tray than that would refuse to reopen a cartridge
/// it had opened by itself ten seconds earlier.
///
/// The system drive is dropped regardless, since it is the one volume that is
/// certainly not a cartridge however it is labelled.
pub fn candidate_volumes() -> Vec<Volume> {
    let mask = unsafe { GetLogicalDrives() };
    let system = std::env::var("SystemDrive")
        .ok()
        .and_then(|drive| drive.chars().next())
        .unwrap_or('C')
        .to_ascii_uppercase();

    (0..26u32)
        .filter(|bit| mask & (1 << bit) != 0)
        .filter_map(|bit| {
            let letter = (b'A' + bit as u8) as char;
            if letter == system {
                return None;
            }

            let root = format!("{letter}:\\");
            let wide_root = wide(&root);

            if !matches!(
                unsafe { GetDriveTypeW(wide_root.as_ptr()) },
                DRIVE_REMOVABLE | DRIVE_FIXED
            ) {
                return None;
            }

            Some(Volume {
                root: PathBuf::from(&root),
                letter,
                name: label(&wide_root).unwrap_or_else(|| format!("{letter}:")),
            })
        })
        .collect()
}

/// The volume label, if it has one worth printing.
fn label(wide_root: &[u16]) -> Option<String> {
    let mut buffer = [0u16; 64];

    let ok = unsafe {
        GetVolumeInformationW(
            wide_root.as_ptr(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    };
    if ok == 0 {
        return None;
    }

    let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    let name = String::from_utf16_lossy(&buffer[..end]);
    (!name.trim().is_empty()).then_some(name)
}

/// Put the icon in the notification area.
///
/// Called again after `TaskbarCreated`: when Explorer restarts it forgets every
/// icon that was there, and a process that does not add its own back has simply
/// vanished as far as the user can tell.
pub fn add(hwnd: HWND) -> bool {
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = ICON_ID;
    data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    data.uCallbackMessage = WM_TRAYICON;
    data.hIcon = icon();

    let tip = wide("PC GamePak");
    data.szTip[..tip.len()].copy_from_slice(&tip);

    unsafe { Shell_NotifyIconW(NIM_ADD, &data) != 0 }
}

/// Show a balloon from the icon — the `notify_only` insert action on Windows.
///
/// The launcher decides to notify but cannot: a notification there has to come
/// from something resident with an icon, which is this process. False when the
/// icon is not there to post from, so the launcher can open its window instead.
/// Text longer than the struct holds is cut, not refused.
pub fn balloon(hwnd: HWND, title: &str, body: &str) -> bool {
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = ICON_ID;
    data.uFlags = NIF_INFO;
    data.dwInfoFlags = NIIF_INFO;
    copy_truncated(&mut data.szInfoTitle, title);
    copy_truncated(&mut data.szInfo, body);

    unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) != 0 }
}

/// Copy as much of `text` as fits, always leaving the terminating NUL.
fn copy_truncated(buffer: &mut [u16], text: &str) {
    let Some(room) = buffer.len().checked_sub(1) else {
        return;
    };
    let mut count = 0;
    for unit in text.encode_utf16().take(room) {
        buffer[count] = unit;
        count += 1;
    }
    // Never end on half a surrogate pair.
    if count > 0 && (0xD800..0xDC00).contains(&buffer[count - 1]) {
        count -= 1;
    }
    buffer[count] = 0;
}

/// Take the icon away, so it does not linger as a ghost until something hovers
/// over it.
pub fn remove(hwnd: HWND) {
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = ICON_ID;
    unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
}

/// The application icon `build.rs` embedded, or Windows' generic one.
fn icon() -> windows_sys::Win32::UI::WindowsAndMessaging::HICON {
    // winresource writes the icon as resource 1. `MAKEINTRESOURCE` is a
    // pointer-shaped integer rather than a pointer — LoadIconW reads the low
    // word as an ordinal and never dereferences it — so it is built without
    // provenance rather than cast from one, which is also what stops the
    // dangling-pointer lint firing on something that is not a pointer at all.
    let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
    // At the notification area's own size — 16 at 100%, 24 at 150% — so
    // Windows picks that entry from the .ico rather than shrinking the 32.
    let embedded = unsafe {
        LoadImageW(
            instance,
            std::ptr::without_provenance(1),
            IMAGE_ICON,
            GetSystemMetrics(SM_CXSMICON),
            GetSystemMetrics(SM_CYSMICON),
            LR_DEFAULTCOLOR,
        )
    };
    if embedded != 0 {
        return embedded;
    }
    let embedded = unsafe { LoadIconW(instance, std::ptr::without_provenance(1)) };
    if embedded != 0 {
        return embedded;
    }
    unsafe { LoadIconW(0, IDI_APPLICATION) }
}

/// Show the menu at the cursor and return what was chosen, or 0 for nothing.
///
/// `TPM_RETURNCMD` hands the id straight back instead of posting `WM_COMMAND`,
/// which keeps the whole interaction — build the list, show it, act on it — in
/// one place with the list still in scope.
pub fn show_menu(hwnd: HWND, cartridges: &[Volume]) -> u32 {
    let menu = unsafe { CreatePopupMenu() };
    if menu == 0 {
        return 0;
    }

    unsafe {
        for (index, cartridge) in cartridges.iter().enumerate() {
            let text = wide(&format!("Open {} ({}:)", cartridge.name, cartridge.letter));
            AppendMenuW(menu, MF_STRING, index + 1, text.as_ptr());
        }

        if !cartridges.is_empty() {
            AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        } else {
            // Something has to say why the list is empty, or the menu reads as
            // broken rather than as "nothing is plugged in".
            let text = wide("No cartridge plugged in");
            AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, text.as_ptr());
            AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        }

        AppendMenuW(
            menu,
            MF_STRING,
            ID_WIZARD as usize,
            wide("Make a cartridge…").as_ptr(),
        );
        AppendMenuW(
            menu,
            MF_STRING,
            ID_SETTINGS as usize,
            wide("Settings…").as_ptr(),
        );
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(menu, MF_STRING, ID_QUIT as usize, wide("Quit").as_ptr());

        let mut cursor = POINT { x: 0, y: 0 };
        GetCursorPos(&mut cursor);

        // Required, and long-documented as such: without it the menu does not
        // close when the user clicks somewhere else, and stays on top of
        // whatever they clicked on instead.
        SetForegroundWindow(hwnd);

        let chosen = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            cursor.x,
            cursor.y,
            0,
            hwnd,
            std::ptr::null(),
        );

        DestroyMenu(menu);
        chosen as u32
    }
}

/// `MF_GRAYED`, which windows-sys does not re-export next to the rest.
const MF_GRAYED: u32 = 0x0000_0001;

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}
