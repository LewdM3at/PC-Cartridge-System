//! Saying a cartridge is there, without opening a window.
//!
//! This is what [`crate::insert::InsertAction::NotifyOnly`] needs, and the two
//! desktops make it a different job each.
//!
//! On Linux the desktop owns notifications and `notify-send` is the way in.
//! Every distribution ships it, it needs no identity registered in advance, and
//! whatever notification daemon the user runs answers it.
//!
//! On Windows a notification belongs to **an icon in the notification area**,
//! and an icon needs a window to send its messages to. This process has neither:
//! it is deciding not to open a window, and it is about to exit. So it makes
//! both, briefly — a message-only window, an icon, the balloon — and then it
//! waits before taking the icon away again, because an icon removed while its
//! balloon is still up takes the balloon with it.
//!
//! The alternative was a real toast, and it does not fit. A toast needs an
//! `AppUserModelID` registered against an installed shortcut; this project
//! installs by unzipping into `%LOCALAPPDATA%`, so there is no such identity to
//! use, and inventing one that Windows has not seen produces no notification and
//! no error either. The balloon is the older mechanism and it is the one that
//! works from where this code stands.
//!
//! Before this existed the Windows side of `notify_only` did nothing at all —
//! not the notification, and not the window it was documented as falling back
//! to — so the one setting nobody could see working was the one that promised to
//! be quiet.

/// Post a cartridge-arrival notification, if this desktop can.
///
/// `false` means nothing was shown and the caller should fall back to whatever
/// it would have done otherwise, rather than leaving the insert silent.
pub fn cartridge_arrived(title: &str, body: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        windows_balloon::show(title, body)
    }
    #[cfg(not(target_os = "windows"))]
    {
        crate::proc::command("notify-send")
            .args(["--app-name=PC GamePak", "--icon=pc-gamepak", title, body])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
}

#[cfg(all(test, not(target_os = "windows")))]
mod tests {
    use super::cartridge_arrived;
    use crate::testutil::Scratch;
    use std::path::Path;

    /// Put a fake `notify-send` first on PATH for the duration of a test.
    ///
    /// PATH is process-wide, so these cannot run beside each other; they take a
    /// lock rather than being marked serial by hand, which would only hold until
    /// somebody added a third.
    fn with_fake_notify_send<T>(dir: &Path, script: &str, body: impl FnOnce() -> T) -> T {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let guard = LOCK.get_or_init(|| Mutex::new(())).lock();
        let _guard = match guard {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };

        let fake = dir.join("notify-send");
        std::fs::write(&fake, script).expect("write the fake");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755))
                .expect("make it executable");
        }

        let before = std::env::var_os("PATH");
        let mut paths = vec![dir.to_path_buf()];
        if let Some(existing) = &before {
            paths.extend(std::env::split_paths(existing));
        }
        std::env::set_var("PATH", std::env::join_paths(paths).expect("join PATH"));
        let out = body();
        match before {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }
        out
    }

    #[test]
    fn the_desktop_is_told_the_title_and_the_body() {
        let scratch = Scratch::new("notify-args");
        let out = scratch.join("said");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > {}\nexit 0\n",
            out.display()
        );

        let shown = with_fake_notify_send(scratch.path(), &script, || {
            cartridge_arrived("Tomb Raider", "3 games — open PC GamePak to play")
        });

        assert!(shown, "a notify-send that succeeded means it was shown");
        let said = std::fs::read_to_string(&out).expect("the fake should have run");
        let lines: Vec<&str> = said.lines().collect();
        // The title and body last, and in that order: notify-send takes them
        // positionally, so a flag inserted between them would swap the two.
        assert_eq!(
            lines
                .iter()
                .rev()
                .take(2)
                .rev()
                .copied()
                .collect::<Vec<_>>(),
            vec!["Tomb Raider", "3 games — open PC GamePak to play"]
        );
        assert!(
            said.contains("--app-name=PC GamePak"),
            "the notification should say who it is from: {said}"
        );
    }

    #[test]
    fn a_desktop_that_refuses_is_reported_rather_than_assumed() {
        // The caller falls back to opening the window on false, so a
        // notify-send that failed must not read as a notification shown.
        let scratch = Scratch::new("notify-fails");
        let shown = with_fake_notify_send(scratch.path(), "#!/bin/sh\nexit 1\n", || {
            cartridge_arrived("Anything", "at all")
        });
        assert!(!shown);
    }
}

/// How long the icon stays after the balloon goes up.
///
/// Windows will not show a balloon for less than five seconds and lets the user
/// raise that; eight is the floor plus room, and it is spent by a process whose
/// only remaining job is to exit.
#[cfg(target_os = "windows")]
const LINGER_MILLIS: u32 = 8_000;

#[cfg(target_os = "windows")]
mod windows_balloon {
    use std::ffi::c_void;

    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIIF_INFO, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, LoadIconW, PeekMessageW, HWND_MESSAGE,
        IDI_APPLICATION, MSG, PM_REMOVE, WS_OVERLAPPED,
    };

    /// Our own id for our own icon. Scoped to this window, so it cannot collide
    /// with the watcher's tray icon even while both exist.
    const ICON_ID: u32 = 1;

    pub fn show(title: &str, body: &str) -> bool {
        let Some(window) = message_window() else {
            return false;
        };

        let shown = unsafe {
            let mut data: NOTIFYICONDATAW = std::mem::zeroed();
            data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            data.hWnd = window;
            data.uID = ICON_ID;
            // NIF_INFO is the balloon. NIF_ICON alongside it because an icon
            // with no image is a gap in the notification area for as long as
            // this lives, and the generic application icon is better than a gap.
            data.uFlags = NIF_INFO | NIF_ICON;
            data.dwInfoFlags = NIIF_INFO;
            data.hIcon = LoadIconW(0, IDI_APPLICATION);
            write_wide(&mut data.szInfoTitle, title);
            write_wide(&mut data.szInfo, body);
            // The tooltip, for the seconds the icon is visible.
            write_wide(&mut data.szTip, "PC GamePak");

            Shell_NotifyIconW(NIM_ADD, &data) != 0
        };

        if shown {
            // Not a bare sleep: a balloon is only shown while somebody is
            // servicing the window's messages, and this window's messages are
            // nobody else's job.
            pump_for(super::LINGER_MILLIS);
            unsafe {
                let mut data: NOTIFYICONDATAW = std::mem::zeroed();
                data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
                data.hWnd = window;
                data.uID = ICON_ID;
                Shell_NotifyIconW(NIM_DELETE, &data);
            }
        }

        unsafe { DestroyWindow(window) };
        shown
    }

    /// A window that exists only to be the icon's owner.
    ///
    /// `HWND_MESSAGE` as the parent means it is never drawn, never appears in
    /// the taskbar and never takes focus — which matters, because the whole
    /// point of this path is that the user asked for nothing to appear.
    ///
    /// Built from `STATIC`, one of the classes the system has already
    /// registered, rather than a class of our own. A balloon needs an owning
    /// window, not a window that answers anything: no callback message is asked
    /// for, so there is nothing here a window procedure of ours would do that
    /// the default one does not, and registering a class to say so costs a
    /// `WNDCLASSW`, a `wndproc` and two more `windows-sys` features.
    fn message_window() -> Option<HWND> {
        // "STATIC", as UTF-16, terminated.
        const CLASS: &[u16] = &[
            b'S' as u16,
            b'T' as u16,
            b'A' as u16,
            b'T' as u16,
            b'I' as u16,
            b'C' as u16,
            0,
        ];

        unsafe {
            let window = CreateWindowExW(
                0,
                CLASS.as_ptr(),
                std::ptr::null(),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                0,
                // A predefined class carries its own instance.
                0,
                std::ptr::null::<c_void>(),
            );
            (window != 0).then_some(window)
        }
    }

    /// Service this thread's messages for a while, then return.
    ///
    /// `PeekMessageW` rather than `GetMessageW`: this window is sent nothing it
    /// has to answer, so waiting for a message that may never come would hold
    /// the balloon up for as long as the machine stays idle.
    fn pump_for(millis: u32) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(millis as u64);
        while std::time::Instant::now() < deadline {
            unsafe {
                let mut message: MSG = std::mem::zeroed();
                while PeekMessageW(&mut message, 0, 0, 0, PM_REMOVE) != 0 {
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    /// Copy a `&str` into one of `NOTIFYICONDATAW`'s fixed UTF-16 buffers.
    ///
    /// Truncated to fit rather than refused — a notification with a clipped body
    /// still says a cartridge is there, which is the whole message.
    fn write_wide(buffer: &mut [u16], text: &str) {
        if buffer.is_empty() {
            return;
        }
        let wide: Vec<u16> = text.encode_utf16().collect();
        let count = wide.len().min(buffer.len() - 1);
        buffer[..count].copy_from_slice(&wide[..count]);
        buffer[count] = 0;
    }

    #[cfg(test)]
    mod tests {
        use super::write_wide;

        #[test]
        fn a_long_body_is_clipped_and_still_terminated() {
            let mut buffer = [0xffffu16; 8];
            write_wide(&mut buffer, "abcdefghijkl");
            assert_eq!(
                buffer[..7],
                "abcdefg".encode_utf16().collect::<Vec<_>>()[..]
            );
            assert_eq!(buffer[7], 0, "the buffer must stay a C string");
        }

        #[test]
        fn text_that_fits_is_copied_whole() {
            let mut buffer = [0xffffu16; 8];
            write_wide(&mut buffer, "abc");
            assert_eq!(buffer[..3], "abc".encode_utf16().collect::<Vec<_>>()[..]);
            assert_eq!(buffer[3], 0);
        }

        #[test]
        fn an_empty_buffer_is_left_alone_rather_than_indexed() {
            // `buffer.len() - 1` on an empty slice is the kind of arithmetic
            // that panics in release builds too.
            write_wide(&mut [], "anything");
        }
    }
}
