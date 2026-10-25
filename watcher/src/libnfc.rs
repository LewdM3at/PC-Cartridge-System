//! Read GamePak cards on a PN532 through libnfc, for readers PC/SC cannot use.
//!
//! A PN532 module on a USB-serial adapter has no PC/SC driver of its own, and
//! the libnfc-to-PC/SC bridge (ifdnfc) drops any card that is not ISO-DEP —
//! which is every NTAG and MIFARE Classic. So this talks to the reader the way
//! libnfc's own tools do, and reads the NDEF message off the card itself:
//!
//! * NTAG / Ultralight (NFC Type 2): pages from 4 on.
//! * MIFARE Classic: the NDEF sectors, opened with the NFC Forum's public key,
//!   which is how a phone writes a Classic card.
//!
//! The message goes to the launcher exactly as the PC/SC path sends it, and
//! the launcher finds the ID in it — a `gamepak://gp_…` URI or a Text record
//! holding just `gp_…`.
//!
//! libnfc is loaded when this runs, not linked: an install without a PN532
//! does not need it. It is used only when `/etc/nfc/libnfc.conf` names a
//! device, so it never goes looking for, and grabbing, a reader on its own.

use std::ffi::{c_char, c_int, c_void, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use crate::nfc::{find_type2_ndef, MAX_NDEF_SIZE};

const CONF: &str = "/etc/nfc/libnfc.conf";

/// The `device.connstring` libnfc.conf sets, if it sets one.
pub fn configured_device() -> Option<String> {
    let conf = std::fs::read_to_string(CONF).ok()?;
    conf.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "device.connstring")
            .then(|| value.trim().trim_matches('"').to_owned())
            .filter(|value| !value.is_empty())
    })
}

/// Set by SIGTERM. A PN532 dropped in the middle of a command stays wedged
/// until it is unplugged, so `systemctl stop` and restarts exit from the
/// monitor, between commands, with the reader closed.
static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_term(_: c_int) {
    STOP.store(true, Ordering::Relaxed);
}

pub fn start(device: String) {
    // SAFETY: the handler only stores to an atomic.
    unsafe {
        libc::signal(
            libc::SIGTERM,
            on_term as extern "C" fn(c_int) as libc::sighandler_t,
        )
    };
    if let Err(error) = thread::Builder::new()
        .name("pc-gamepak-libnfc".into())
        .spawn(move || monitor(&device))
    {
        crate::log::line(&format!("could not start NFC reader monitor: {error}"));
    }
}

// The parts of libnfc 1.8's ABI this uses.

#[repr(C)]
#[derive(Clone, Copy)]
struct Modulation {
    nmt: c_int,
    nbr: c_int,
}
const ISO14443A_106: Modulation = Modulation { nmt: 1, nbr: 1 }; // NMT_ISO14443A, NBR_106
const NP_INFINITE_SELECT: c_int = 7;

/// `nfc_target`, read through its ISO 14443-A member. The tail covers the
/// union's other members and the modulation after it; nothing reads it.
/// Packed, as libnfc's nfc-types.h declares everything under `pragma pack(1)`.
#[repr(C, packed)]
struct Target {
    atqa: [u8; 2],
    sak: u8,
    uid_len: usize,
    uid: [u8; 10],
    ats_len: usize,
    ats: [u8; 254],
    _rest: [u8; 512],
}

struct Lib {
    init: unsafe extern "C" fn(*mut *mut c_void),
    open: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
    close: unsafe extern "C" fn(*mut c_void),
    initiator_init: unsafe extern "C" fn(*mut c_void) -> c_int,
    set_property_bool: unsafe extern "C" fn(*mut c_void, c_int, bool) -> c_int,
    select: unsafe extern "C" fn(*mut c_void, Modulation, *const u8, usize, *mut Target) -> c_int,
    transceive: unsafe extern "C" fn(*mut c_void, *const u8, usize, *mut u8, usize, c_int) -> c_int,
    is_present: unsafe extern "C" fn(*mut c_void, *const Target) -> c_int,
}

impl Lib {
    fn load() -> Result<Self, String> {
        // SAFETY: dlopen/dlsym with NUL-terminated names; each symbol is cast
        // to the signature libnfc 1.8 declares for it.
        unsafe {
            let handle = [c"libnfc.so.6", c"libnfc.so"]
                .iter()
                .map(|name| libc::dlopen(name.as_ptr(), libc::RTLD_NOW))
                .find(|handle| !handle.is_null())
                .ok_or("libnfc is not installed")?;
            let symbol = |name: &std::ffi::CStr| {
                let found = libc::dlsym(handle, name.as_ptr());
                if found.is_null() {
                    Err(format!("libnfc has no {}", name.to_string_lossy()))
                } else {
                    Ok(found)
                }
            };
            Ok(Lib {
                init: std::mem::transmute(symbol(c"nfc_init")?),
                open: std::mem::transmute(symbol(c"nfc_open")?),
                close: std::mem::transmute(symbol(c"nfc_close")?),
                initiator_init: std::mem::transmute(symbol(c"nfc_initiator_init")?),
                set_property_bool: std::mem::transmute(symbol(c"nfc_device_set_property_bool")?),
                select: std::mem::transmute(symbol(c"nfc_initiator_select_passive_target")?),
                transceive: std::mem::transmute(symbol(c"nfc_initiator_transceive_bytes")?),
                is_present: std::mem::transmute(symbol(c"nfc_initiator_target_is_present")?),
            })
        }
    }
}

/// One open reader. Closed on drop, so an error anywhere reopens it cleanly.
struct Device<'a> {
    lib: &'a Lib,
    device: *mut c_void,
}

impl Device<'_> {
    /// Send one command to the selected card; the reply, or `None`.
    fn exchange(&self, command: &[u8]) -> Option<Vec<u8>> {
        let mut reply = [0u8; 64];
        // SAFETY: both buffers are valid for the lengths given.
        let n = unsafe {
            (self.lib.transceive)(
                self.device,
                command.as_ptr(),
                command.len(),
                reply.as_mut_ptr(),
                reply.len(),
                500,
            )
        };
        usize::try_from(n).ok().map(|n| reply[..n].to_vec())
    }
}

impl Drop for Device<'_> {
    fn drop(&mut self) {
        // SAFETY: opened by nfc_open and closed once.
        unsafe { (self.lib.close)(self.device) }
    }
}

fn monitor(connstring: &str) {
    crate::log::line(&format!(
        "NFC reader monitor starting (libnfc, {connstring})"
    ));
    let lib = match Lib::load() {
        Ok(lib) => lib,
        Err(error) => {
            crate::log::line(&format!("NFC: {error}"));
            return;
        }
    };
    let mut context = std::ptr::null_mut();
    // SAFETY: nfc_init fills in the context pointer.
    unsafe { (lib.init)(&mut context) };
    if context.is_null() {
        crate::log::line("NFC: libnfc could not start");
        return;
    }
    let Ok(connstring_c) = CString::new(connstring) else {
        return;
    };
    let mut last_error = String::new();
    loop {
        let error = watch(&lib, context, &connstring_c);
        // Logged once per kind of failure, not every 5 s while it lasts.
        if error != last_error {
            crate::log::line(&format!("NFC: {error}"));
            last_error = error;
        }
        thread::sleep(Duration::from_secs(5));
        if STOP.load(Ordering::Relaxed) {
            std::process::exit(0);
        }
    }
}

/// Open the reader and watch it until it fails. Returns why.
fn watch(lib: &Lib, context: *mut c_void, connstring: &CString) -> String {
    release_stale_claim(&connstring.to_string_lossy());
    // SAFETY: context came from nfc_init; connstring is NUL-terminated.
    let device = unsafe { (lib.open)(context, connstring.as_ptr()) };
    if device.is_null() {
        return format!(
            "cannot open {} (is the reader plugged in, and your user in the uucp or dialout group?)",
            connstring.to_string_lossy()
        );
    }
    let reader = Device { lib, device };
    // SAFETY: device is open.
    if unsafe { (lib.initiator_init)(device) } < 0
        || unsafe { (lib.set_property_bool)(device, NP_INFINITE_SELECT, false) } < 0
    {
        return "the reader would not start".into();
    }
    crate::log::line("NFC reader ready");

    // The card last read, while it stays on the reader. Polling again would
    // see it drop out and come back after a read, and fire it again; asking
    // libnfc whether this target is still there does not.
    let mut on_reader: Option<Target> = None;
    let mut launchers: Vec<std::process::Child> = Vec::new();
    loop {
        thread::sleep(Duration::from_millis(300));
        if STOP.load(Ordering::Relaxed) {
            drop(reader); // nfc_close: the PN532 is left idle
            std::process::exit(0);
        }
        launchers.retain_mut(|child| !matches!(child.try_wait(), Ok(Some(_))));
        if let Some(card) = &on_reader {
            // SAFETY: device is open; card came from select.
            if unsafe { (lib.is_present)(device, card) } == 0 {
                continue;
            }
            on_reader = None;
        }
        // SAFETY: Target is at least as large as nfc_target.
        let mut target: Target = unsafe { std::mem::zeroed() };
        let found =
            unsafe { (lib.select)(device, ISO14443A_106, std::ptr::null(), 0, &mut target) };
        if found < 0 {
            return "the reader stopped answering".into();
        }
        if found == 0 {
            continue;
        }
        let (uid, uid_len, sak) = (target.uid, target.uid_len, target.sak);
        let uid = uid[..uid_len.min(10)].to_vec();
        on_reader = Some(target);
        match read_ndef(&reader, sak, &uid) {
            Ok(message) => {
                crate::log::line("NFC GamePak card detected");
                launchers.extend(crate::launcher::open_nfc(&message));
            }
            Err(error) => crate::log::line(&format!("NFC card with no GamePak ID: {error}")),
        }
    }
}

/// libnfc marks a serial port it opens with a spare termios bit (`CCLAIMED`)
/// and refuses one already marked. A process killed while holding the port —
/// this watcher on `systemctl restart`, a crashed pcscd — leaves the mark, and
/// the reader stays "already claimed" until it is unplugged. This reader is
/// the one libnfc.conf gives the watcher, so it takes it back.
// ponytail: also clears a claim another live process holds; the reader is
// configured for this watcher, so nothing else should be holding it.
fn release_stale_claim(connstring: &str) {
    const CCLAIMED: libc::tcflag_t = 0x8000_0000;
    let Some(port) = connstring
        .strip_prefix("pn532_uart:")
        .map(|rest| rest.split(':').next().unwrap_or(rest))
    else {
        return;
    };
    let Ok(port) = CString::new(port) else { return };
    // SAFETY: plain open/tcgetattr/tcsetattr/close on a path we own the fd of.
    unsafe {
        let fd = libc::open(
            port.as_ptr(),
            libc::O_RDWR | libc::O_NOCTTY | libc::O_NONBLOCK,
        );
        if fd < 0 {
            return;
        }
        let mut termios: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(fd, &mut termios) == 0 && termios.c_iflag & CCLAIMED != 0 {
            termios.c_iflag &= !CCLAIMED;
            libc::tcsetattr(fd, libc::TCSANOW, &termios);
        }
        libc::close(fd);
    }
}

fn read_ndef(reader: &Device, sak: u8, uid: &[u8]) -> Result<Vec<u8>, String> {
    if sak & 0x08 != 0 {
        read_classic(reader, uid)
    } else if sak == 0x00 {
        read_type2(reader)
    } else {
        Err(format!(
            "card type (SAK {sak:02x}) is not NTAG or MIFARE Classic"
        ))
    }
}

/// NTAG / Ultralight: READ returns four pages, 16 bytes, at a time.
fn read_type2(reader: &Device) -> Result<Vec<u8>, String> {
    let mut memory = Vec::new();
    for page in (4u8..=252).step_by(4) {
        let bytes = reader
            .exchange(&[0x30, page])
            .filter(|bytes| bytes.len() == 16)
            .ok_or("card read failed")?;
        memory.extend_from_slice(&bytes);
        if let Some(message) = find_type2_ndef(&memory)? {
            return Ok(message);
        }
        if memory.len() > MAX_NDEF_SIZE {
            break;
        }
    }
    Err("no NDEF message on the card".into())
}

/// The NFC Forum's public key A for NDEF sectors on MIFARE Classic.
const NDEF_KEY: [u8; 6] = [0xd3, 0xf7, 0xd3, 0xf7, 0xd3, 0xf7];

/// MIFARE Classic 1K: NDEF data runs through the data blocks of sectors 1–15,
/// in the same TLV format as Type 2.
// ponytail: assumes the NDEF sectors start at 1 and run on, as phones write
// them, instead of reading the MAD in sector 0; read the MAD if a card ever
// puts them elsewhere.
fn read_classic(reader: &Device, uid: &[u8]) -> Result<Vec<u8>, String> {
    let uid4 = &uid[uid.len().saturating_sub(4)..];
    let mut memory = Vec::new();
    for sector in 1u8..16 {
        let first = sector * 4;
        let mut auth = vec![0x60, first];
        auth.extend_from_slice(&NDEF_KEY);
        auth.extend_from_slice(uid4);
        reader
            .exchange(&auth)
            .ok_or("the card has no NDEF sector (not written as NDEF?)")?;
        for block in first..first + 3 {
            let bytes = reader
                .exchange(&[0x30, block])
                .filter(|bytes| bytes.len() == 16)
                .ok_or("card read failed")?;
            memory.extend_from_slice(&bytes);
        }
        if let Some(message) = find_type2_ndef(&memory)? {
            return Ok(message);
        }
    }
    Err("no NDEF message on the card".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_layout_matches_libnfc() {
        // nfc_iso14443a_info, packed: atqa[2], sak, size_t uid_len, uid[10], size_t ats_len
        assert_eq!(std::mem::offset_of!(Target, uid_len), 3);
        assert_eq!(std::mem::offset_of!(Target, uid), 11);
        assert_eq!(std::mem::offset_of!(Target, ats_len), 21);
    }
}
