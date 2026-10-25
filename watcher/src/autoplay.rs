//! Keeping AutoPlay's Explorer window away from a cartridge.
//!
//! Plug a drive in and Windows' AutoPlay does whatever the user chose for
//! removable drives — for most people, open it in Explorer. For a cartridge that
//! is a second window on top of the launcher, showing a folder of files nobody
//! plugged the drive in to see.
//!
//! Windows has a documented way to say no: before AutoPlay acts on a new volume,
//! the shell looks in the Running Object Table for an object registered under
//! `CLSID_QueryCancelAutoPlay` and asks it `IQueryCancelAutoPlay::AllowAutoPlay`
//! with the drive's path. Answering `S_FALSE` cancels AutoPlay for that drive
//! only. The watcher is resident anyway, so it registers one at startup and
//! answers no for a drive with a `cartridge.conf` or `memorycard.conf` at its
//! root, and yes for everything else — a USB stick still opens as it always did.
//!
//! No admin, no settings changed, nothing left behind: the registration goes
//! with the process. If it cannot be made, [`register`] says so and the watcher
//! falls back to closing the window after it appears, which is what it did
//! before this existed.
//!
//! The object is a single static with a hand-written vtable — four methods —
//! rather than a COM framework: the watcher keeps its dependencies to the
//! system's own libraries. Cross-process calls from Explorer are marshalled by
//! COM's standard proxy for the interface and arrive on the thread that
//! registered it, through its ordinary message loop.

use std::ffi::c_void;
use std::path::PathBuf;

use windows_sys::core::{GUID, HRESULT};
use windows_sys::Win32::System::Com::{
    CoInitializeEx, CreateClassMoniker, GetRunningObjectTable, COINIT_APARTMENTTHREADED,
    ROTFLAGS_REGISTRATIONKEEPSALIVE,
};

/// What makes a drive one AutoPlay should leave alone. Not `autorun.inf`: plenty
/// of ordinary drives carry one, and those keep their AutoPlay.
const MARKERS: [&str; 2] = ["cartridge.conf", "memorycard.conf"];

const IID_IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_c000_000000000046);
const IID_IQUERYCANCELAUTOPLAY: GUID = GUID::from_u128(0xddefe873_6997_4e68_be26_39b633adbe12);
const CLSID_QUERYCANCELAUTOPLAY: GUID = GUID::from_u128(0x331f1768_05a9_4ddd_b86e_dae34ddc998a);

const S_OK: HRESULT = 0;
const S_FALSE: HRESULT = 1;
const E_NOINTERFACE: HRESULT = 0x8000_4002_u32 as HRESULT;

#[repr(C)]
struct Vtbl {
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    allow_auto_play:
        unsafe extern "system" fn(*mut c_void, *const u16, u32, *const u16, u32) -> HRESULT,
}

#[repr(C)]
struct Canceller {
    vtbl: *const Vtbl,
}

// SAFETY: the object holds a pointer to a static vtable and no state at all;
// sharing it between threads shares nothing that can change.
unsafe impl Sync for Canceller {}

static VTBL: Vtbl = Vtbl {
    query_interface,
    add_ref,
    release,
    allow_auto_play,
};

/// The one object. Static, so reference counting has nothing to free.
static CANCELLER: Canceller = Canceller { vtbl: &VTBL };

fn same(a: &GUID, b: &GUID) -> bool {
    a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
}

unsafe extern "system" fn query_interface(
    this: *mut c_void,
    riid: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    if out.is_null() || riid.is_null() {
        return E_NOINTERFACE;
    }
    if same(&*riid, &IID_IUNKNOWN) || same(&*riid, &IID_IQUERYCANCELAUTOPLAY) {
        *out = this;
        return S_OK;
    }
    // Including IMarshal, which is what makes COM use its standard proxy.
    *out = std::ptr::null_mut();
    E_NOINTERFACE
}

unsafe extern "system" fn add_ref(_this: *mut c_void) -> u32 {
    1
}

unsafe extern "system" fn release(_this: *mut c_void) -> u32 {
    1
}

unsafe extern "system" fn allow_auto_play(
    _this: *mut c_void,
    path: *const u16,
    _content_type: u32,
    _label: *const u16,
    _serial: u32,
) -> HRESULT {
    let Some(root) = wide_path(path) else {
        return S_OK;
    };
    if is_cartridge(&root) {
        crate::log::line(&format!(
            "{}: cancelled AutoPlay for a cartridge",
            root.display()
        ));
        S_FALSE
    } else {
        S_OK
    }
}

/// One look, no retries: AutoPlay is waiting on the answer, and by the time it
/// asks, the volume has been mounted long enough to have been read for icons.
fn is_cartridge(root: &std::path::Path) -> bool {
    MARKERS.iter().any(|name| root.join(name).is_file())
}

unsafe fn wide_path(path: *const u16) -> Option<PathBuf> {
    if path.is_null() {
        return None;
    }
    let len = (0..).take_while(|&i| *path.add(i) != 0).count();
    let text = String::from_utf16_lossy(std::slice::from_raw_parts(path, len));
    (!text.is_empty()).then(|| PathBuf::from(text))
}

/// Register the canceller in the Running Object Table. Call once, on the thread
/// that runs the message loop: that thread's apartment is where Explorer's calls
/// arrive. True when AutoPlay will ask.
pub fn register() -> bool {
    // SAFETY: plain COM calls with out-pointers to locals; the interface
    // pointers they return are used through their vtables as documented.
    unsafe {
        // S_FALSE (already initialised on this thread) is as good as S_OK.
        if CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) < 0 {
            return false;
        }
        let mut rot: *mut c_void = std::ptr::null_mut();
        if GetRunningObjectTable(0, &mut rot) < 0 || rot.is_null() {
            return false;
        }
        let mut moniker: *mut c_void = std::ptr::null_mut();
        if CreateClassMoniker(&CLSID_QUERYCANCELAUTOPLAY, &mut moniker) < 0 || moniker.is_null() {
            return false;
        }
        // IRunningObjectTable::Register is the fourth entry, after IUnknown's three.
        type Register = unsafe extern "system" fn(
            *mut c_void,
            u32,
            *mut c_void,
            *mut c_void,
            *mut u32,
        ) -> HRESULT;
        let vtable = *(rot as *const *const usize);
        let register: Register = std::mem::transmute(*vtable.add(3));
        let mut cookie = 0u32;
        let object = &CANCELLER as *const Canceller as *mut c_void;
        let registered = register(
            rot,
            ROTFLAGS_REGISTRATIONKEEPSALIVE,
            object,
            moniker,
            &mut cookie,
        );
        // The table holds its own reference to the moniker; release ours (IUnknown::Release).
        type Release = unsafe extern "system" fn(*mut c_void) -> u32;
        let moniker_release: Release =
            std::mem::transmute(*(*(moniker as *const *const usize)).add(2));
        moniker_release(moniker);
        // The table itself is kept for the life of the process, and the
        // registration with it: both go when the watcher does.
        registered >= 0
    }
}
