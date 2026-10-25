//! Taking a cartridge's volume away: the platform half of Eject.
//!
//! Moved here from the Tauri launcher so every front-end ejects the same way.
//! What happens before the volume goes — pushing saves and shader caches,
//! closing play sessions — belongs to the caller, which knows what it has
//! open; this only unmounts and powers the device down.
//!
//! On Windows the unelevated PnP request is tried first; when the enclosure
//! refuses it, the caller's own executable is re-run elevated with
//! `--eject <letter>`, so every front-end binary must hand that argument to
//! [`run_elevated`] before it builds a window.

#[cfg(not(target_os = "windows"))]
use std::process::Command;

#[cfg(target_os = "windows")]
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_Parent, CM_Request_Device_EjectW, CR_SUCCESS,
};

/// Write the cartridge's own files back before the volume goes: saves first
/// (they are data), then shader caches. Nothing may be using the volume by the
/// time this runs, so the save written here is not one being written by
/// anything else. Failures are logged, never raised: a save that could not be
/// written is not a reason to leave a drive mounted that was asked to go.
pub fn settle(drive_path: &str, log: &dyn Fn(String)) {
    let root = std::path::Path::new(drive_path);
    if crate::settings::load().save_sync {
        for result in crate::saves::detach_all(root) {
            if let Err(why) = result {
                log(format!("saves: {why}"));
            }
        }
    }
    if crate::shaders::wanted(root) {
        let mut bytes = 0;
        for result in crate::shaders::push_all(root) {
            match result {
                Ok(synced) => bytes += synced.bytes,
                Err(why) => log(format!("shaders: {why}")),
            }
        }
        if bytes > 0 {
            log(format!("shaders: carried {bytes} bytes"));
        }
    }
}

/// Unmount the volume at `drive_path` and power its device down.
pub fn eject(drive_path: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        eject_windows(drive_path)
    }
    #[cfg(not(target_os = "windows"))]
    {
        eject_linux(drive_path)
    }
}

/// The elevated half, for a front-end started with `--eject <drive>`.
/// Returns the process exit code the unelevated half reads back.
#[cfg(target_os = "windows")]
pub fn run_elevated(drive_path: &str) -> u32 {
    run_elevated_eject(drive_path)
}

/// Eject the cartridge, elevating only if it turns out to be necessary.
///
/// Asking PnP nicely works on a plain USB stick and prompts for nothing, so it
/// is tried first and is usually the end of it. It cannot work on the hardware
/// this project is actually built around: an NVMe stick in a UAS enclosure
/// advertises no `CM_DEVCAP_EJECTSUPPORTED`, Explorer offers no Eject verb for
/// it, and the request comes back `PNP_VetoDevice` — the device saying it does
/// not do this — from the volume rather than from anything holding a file open.
///
/// So the fallback does the work by force, which needs administrator because
/// Windows calls these disks fixed and will not hand out write access to a
/// fixed volume otherwise. That is one UAC prompt, at the moment the user asked
/// for something that cannot be done without one, and none at all on hardware
/// that never needed it.
#[cfg(target_os = "windows")]
fn eject_windows(drive_path: &str) -> Result<(), String> {
    let letter = drive_path.trim_end_matches(['\\', '/']);

    match pnp_eject(letter) {
        Ok(()) => Ok(()),
        // The unelevated refusal is kept only to be shown if elevation is
        // declined: it is the honest reason the prompt appeared.
        Err(refusal) => elevated_eject(letter, &refusal),
    }
}

/// Re-run this executable elevated, with `--eject`, and wait for it.
///
/// `ShellExecuteExW` with `runas` rather than a PowerShell hop: the elevated
/// half is this same binary doing the same Win32 calls, so it can report what
/// happened as an exit code instead of a parsed console message.
#[cfg(target_os = "windows")]
fn elevated_eject(letter: &str, refusal: &str) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, WaitForSingleObject, INFINITE,
    };
    use windows_sys::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

    let Ok(exe) = std::env::current_exe() else {
        return Err(refusal.to_string());
    };

    let verb = wide("runas");
    let file = wide(&exe.to_string_lossy());
    // Unquoted, and the letter rather than the root: `"G:\"` ends in a
    // backslash, which the Windows command line reads as escaping the quote
    // that closes it, so the elevated half was handed a mangled path and
    // reported a drive that was not there. A drive letter cannot contain a
    // space, so there is nothing for the quotes to have been protecting.
    let parameters = wide(&format!("--eject {letter}"));

    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = parameters.as_ptr();
    info.nShow = SW_HIDE;

    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        let error = unsafe { windows_sys::Win32::Foundation::GetLastError() };
        return Err(if error == ERROR_CANCELLED {
            "Ejecting this cartridge needs administrator, and the prompt was dismissed.".to_string()
        } else {
            refusal.to_string()
        });
    }

    if info.hProcess == 0 {
        return Err(refusal.to_string());
    }

    let waited = unsafe { WaitForSingleObject(info.hProcess, INFINITE) };
    let mut code = EJECT_OTHER;
    if waited == WAIT_OBJECT_0 {
        unsafe { GetExitCodeProcess(info.hProcess, &mut code) };
    }
    unsafe { CloseHandle(info.hProcess) };

    match code {
        EJECT_OK => Ok(()),
        // Administrator was already granted, so `FSCTL_LOCK_VOLUME` refusing
        // means what it says: files are open on the volume. Not a rights
        // problem, and not one a Defender exclusion fixes — that was tried on
        // a cartridge that would not eject, and changed nothing.
        //
        // What holds it is whatever has read the cartridge since it arrived.
        // A drive only just plugged in ejects every time; the same drive
        // after a game has been played from it often will not, and does not
        // let go until it is replugged. So the second half of the message is
        // the thing that always works, rather than a second guess at who.
        EJECT_IN_USE => Err(format!(
            "{letter} is still in use. Close the game, Steam, or any folder open on it, \
             or replug the cartridge — one that has just arrived always ejects."
        )),
        EJECT_MISSING => Err(format!("{letter} is not there any more.")),
        _ => Err(refusal.to_string()),
    }
}

/// Exit codes the elevated half reports back through.
#[cfg(target_os = "windows")]
const EJECT_OK: u32 = 0;
#[cfg(target_os = "windows")]
const EJECT_IN_USE: u32 = 1;
#[cfg(target_os = "windows")]
const EJECT_MISSING: u32 = 2;
#[cfg(target_os = "windows")]
const EJECT_OTHER: u32 = 3;

/// The elevated half: flush the volume, dismount it, then stop the device.
///
/// Runs instead of the window when the executable is started with `--eject`.
/// Locking is what needed the rights: with them, the filesystem is flushed and
/// dismounted, and the drive is safe to unplug whether or not PnP will then
/// take the device away — which it still declines to do on an enclosure that
/// never claimed it could.
#[cfg(target_os = "windows")]
fn run_elevated_eject(drive_path: &str) -> u32 {
    use std::time::Duration;
    use windows_sys::Win32::Foundation::{
        CloseHandle, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Ioctl::{FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME};
    use windows_sys::Win32::System::IO::DeviceIoControl;

    // Same three seconds the unelevated attempt used to allow: a cartridge
    // whose game has just been quit is released over a second or two.
    const ATTEMPTS: u32 = 12;
    const RETRY_DELAY: Duration = Duration::from_millis(250);

    let letter = drive_path.trim_end_matches(['\\', '/']);
    let path = wide(&format!("\\\\.\\{letter}"));

    let mut opened = false;

    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            std::thread::sleep(RETRY_DELAY);
        }

        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                0,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            continue;
        }
        opened = true;

        let mut returned = 0u32;
        let locked = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_LOCK_VOLUME,
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        if locked == 0 {
            unsafe { CloseHandle(handle) };
            continue;
        }

        let dismounted = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_DISMOUNT_VOLUME,
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        // The lock is released with the handle. Held until after the dismount
        // so nothing can mount the volume back in between.
        unsafe { CloseHandle(handle) };

        if dismounted != 0 {
            // Now that no filesystem is mounted there is nothing left to veto,
            // so ask PnP again. It still refuses on an enclosure with no eject
            // support, and that is fine: the cartridge is already safe to pull.
            let _ = pnp_eject(letter);
            return EJECT_OK;
        }
    }

    if opened {
        EJECT_IN_USE
    } else {
        EJECT_MISSING
    }
}

/// Ask PnP to stop the device behind a drive letter.
///
/// The obvious implementation — lock the volume, dismount it — cannot work on
/// the hardware this is for. `FSCTL_LOCK_VOLUME` needs administrator on a
/// volume Windows considers fixed, and `GetDriveTypeW` calls an NVMe stick in a
/// USB enclosure fixed, exactly like the internal disk. So the lock came back
/// `ERROR_ACCESS_DENIED` every time, on a cartridge nothing was using, and
/// `mountvol /P` behind it needed the same rights and failed the same way.
///
/// `CM_Request_Device_Eject` is what the notification area's own eject calls.
/// It asks the PnP manager to stop the device rather than taking the volume by
/// force: the filesystem is flushed and dismounted on the way, no elevation is
/// involved, and the device is actually powered down at the end — which the
/// dismount never did, so "safe to remove" had been describing a drive that was
/// still spinning.
///
/// When something refuses, PnP says what: the veto names the application or
/// driver holding the device, which is a better answer than any guess made from
/// an error code.
#[cfg(target_os = "windows")]
fn pnp_eject(letter: &str) -> Result<(), String> {
    let disk = device_number(letter)
        .ok_or_else(|| format!("{letter} could not be identified as a disk."))?;
    let devinst = disk_devinst(disk)
        .ok_or_else(|| format!("Windows has no device for {letter} to eject."))?;

    // The parent first: for a USB enclosure that is the mass-storage device,
    // and stopping it is what "Safely Remove Hardware" stops. The disk itself
    // is the fallback for anything shaped differently — a card reader slot, or
    // a device that is its own parent as far as PnP is concerned.
    let mut parent = 0u32;
    let targets = if unsafe { CM_Get_Parent(&mut parent, devinst, 0) } == CR_SUCCESS {
        vec![parent, devinst]
    } else {
        vec![devinst]
    };

    let mut refusal = None;
    for target in targets {
        match request_eject(target) {
            Ok(()) => return Ok(()),
            Err(why) => refusal = refusal.or(Some(why)),
        }
    }

    Err(refusal.unwrap_or_else(|| format!("Windows would not eject {letter}.")))
}

/// Ask PnP to stop one device node.
#[cfg(target_os = "windows")]
fn request_eject(devinst: u32) -> Result<(), String> {
    // Aliased in upper case because they are matched on as patterns, and a
    // constant named in camel case there is read as a fresh binding that
    // matches everything — the lint that fires on it is warning about a match
    // arm that would silently swallow every other veto.
    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
        PNP_VetoDevice, PNP_VetoDriver, PNP_VetoOutstandingOpen, PNP_VetoPendingClose,
        PNP_VetoWindowsApp, PNP_VetoWindowsService,
    };
    const VETO_APP: i32 = PNP_VetoWindowsApp;
    const VETO_SERVICE: i32 = PNP_VetoWindowsService;
    const VETO_OPEN: i32 = PNP_VetoOutstandingOpen;
    const VETO_CLOSING: i32 = PNP_VetoPendingClose;
    const VETO_DEVICE: i32 = PNP_VetoDevice;
    const VETO_DRIVER: i32 = PNP_VetoDriver;

    let mut veto_type = 0;
    let mut veto_name = [0u16; 260];

    let result = unsafe {
        CM_Request_Device_EjectW(
            devinst,
            &mut veto_type,
            veto_name.as_mut_ptr(),
            veto_name.len() as u32,
            0,
        )
    };
    if result == CR_SUCCESS {
        return Ok(());
    }

    let end = veto_name
        .iter()
        .position(|c| *c == 0)
        .unwrap_or(veto_name.len());
    let name = String::from_utf16_lossy(&veto_name[..end]);
    let name = name.trim();

    // The veto name is a process name or a driver's, so it is worth printing
    // verbatim: "Steam is still using the cartridge" is the whole answer, where
    // an error number would send someone looking for a fault that is not there.
    Err(match veto_type {
        VETO_APP | VETO_SERVICE | VETO_OPEN if !name.is_empty() => {
            format!("{name} is still using the cartridge. Close it, then Eject.")
        }
        VETO_APP | VETO_SERVICE | VETO_OPEN => {
            "Something is still using the cartridge. Quit the game or Steam, then Eject."
                .to_string()
        }
        VETO_CLOSING => "The cartridge is still finishing up. Try Eject again.".to_string(),
        VETO_DEVICE | VETO_DRIVER if !name.is_empty() => {
            format!("{name} would not release the cartridge.")
        }
        _ => "Windows would not release the cartridge. Unplug it once the drive light settles."
            .to_string(),
    })
}

/// Which physical disk a drive letter sits on.
///
/// Opened with no access rights at all, which is enough for a query and is the
/// reason none of this prompts: asking for read or write on a fixed volume is
/// what needed administrator in the first place.
#[cfg(target_os = "windows")]
fn device_number(letter: &str) -> Option<u32> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Ioctl::{
        IOCTL_STORAGE_GET_DEVICE_NUMBER, STORAGE_DEVICE_NUMBER,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;

    let path = wide(&format!("\\\\.\\{letter}"));
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            0,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }

    let mut number: STORAGE_DEVICE_NUMBER = unsafe { std::mem::zeroed() };
    let mut returned = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_GET_DEVICE_NUMBER,
            std::ptr::null(),
            0,
            &mut number as *mut _ as *mut std::ffi::c_void,
            std::mem::size_of::<STORAGE_DEVICE_NUMBER>() as u32,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    unsafe { CloseHandle(handle) };

    (ok != 0).then_some(number.DeviceNumber)
}

/// The device node for a physical disk, found by matching its number.
///
/// There is no call from a disk number to a device node, so this walks the disk
/// interfaces, opens each one and asks which disk it is — the same question
/// `device_number` asked of the volume, from the other end.
#[cfg(target_os = "windows")]
fn disk_devinst(disk: u32) -> Option<u32> {
    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
        SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
        SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT,
        SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W, SP_DEVINFO_DATA,
    };

    // Written out because windows-sys 0.52 does not export it. It is a fixed
    // interface class id — {53F56307-B6BF-11D0-94F2-00A0C91EFB8B}, the one
    // every disk registers — not a value that varies by machine or version.
    const GUID_DEVINTERFACE_DISK: windows_sys::core::GUID = windows_sys::core::GUID {
        data1: 0x53F5_6307,
        data2: 0xB6BF,
        data3: 0x11D0,
        data4: [0x94, 0xF2, 0x00, 0xA0, 0xC9, 0x1E, 0xFB, 0x8B],
    };
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Ioctl::{
        IOCTL_STORAGE_GET_DEVICE_NUMBER, STORAGE_DEVICE_NUMBER,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;

    let set = unsafe {
        SetupDiGetClassDevsW(
            &GUID_DEVINTERFACE_DISK,
            std::ptr::null(),
            0,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    };
    if set == INVALID_HANDLE_VALUE {
        return None;
    }

    let mut found = None;

    for index in 0.. {
        let mut interface: SP_DEVICE_INTERFACE_DATA = unsafe { std::mem::zeroed() };
        interface.cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;

        if unsafe {
            SetupDiEnumDeviceInterfaces(
                set,
                std::ptr::null(),
                &GUID_DEVINTERFACE_DISK,
                index,
                &mut interface,
            )
        } == 0
        {
            break;
        }

        // The detail struct is variable length: a fixed head and the device
        // path running off the end of it. `cbSize` describes the head only,
        // which is why it is not the size of the buffer being passed.
        let mut buffer = [0u8; 1024];
        let detail = buffer.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
        unsafe {
            (*detail).cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
        }

        let mut info: SP_DEVINFO_DATA = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<SP_DEVINFO_DATA>() as u32;

        if unsafe {
            SetupDiGetDeviceInterfaceDetailW(
                set,
                // Read, not written: the interface identifies which detail to
                // fetch, and `info` on the end is the out-parameter.
                &interface,
                detail,
                buffer.len() as u32,
                std::ptr::null_mut(),
                &mut info,
            )
        } == 0
        {
            continue;
        }

        let handle = unsafe {
            CreateFileW(
                (*detail).DevicePath.as_ptr(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                0,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            continue;
        }

        let mut number: STORAGE_DEVICE_NUMBER = unsafe { std::mem::zeroed() };
        let mut returned = 0u32;
        let ok = unsafe {
            DeviceIoControl(
                handle,
                IOCTL_STORAGE_GET_DEVICE_NUMBER,
                std::ptr::null(),
                0,
                &mut number as *mut _ as *mut std::ffi::c_void,
                std::mem::size_of::<STORAGE_DEVICE_NUMBER>() as u32,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        unsafe { CloseHandle(handle) };

        if ok != 0 && number.DeviceNumber == disk {
            found = Some(info.DevInst);
            break;
        }
    }

    unsafe { SetupDiDestroyDeviceInfoList(set) };
    found
}

#[cfg(target_os = "windows")]
fn wide(s: &str) -> Vec<u16> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(not(target_os = "windows"))]
fn eject_linux(drive_path: &str) -> Result<(), String> {
    let findmnt = Command::new("findmnt")
        .args(["-n", "-o", "SOURCE", drive_path])
        .output()
        .map_err(|e| format!("findmnt failed: {e}"))?;

    let device = String::from_utf8_lossy(&findmnt.stdout).trim().to_string();

    if device.is_empty() {
        return Err(format!("Cannot find block device for {drive_path}"));
    }

    let unmount = Command::new("udisksctl")
        .args(["unmount", "-b", &device, "--no-user-interaction"])
        .status()
        .map_err(|e| format!("udisksctl unmount failed: {e}"))?;

    if !unmount.success() {
        let _ = Command::new("umount").arg(&device).status();
    }

    let parent = get_parent_device(&device);
    let _ = Command::new("udisksctl")
        .args(["power-off", "-b", &parent, "--no-user-interaction"])
        .status();

    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn get_parent_device(partition: &str) -> String {
    let out = Command::new("lsblk")
        .args(["-no", "PKNAME", partition])
        .output();
    if let Ok(o) = out {
        let parent_name = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if !parent_name.is_empty() {
            return format!("/dev/{parent_name}");
        }
    }
    partition.to_string()
}
