//! Embeds the app icon and version metadata into pc-gamepak-watcher.exe.
//!
//! Without this the watcher shows the default Rust executable icon in Task
//! Manager, the logon-task list and Explorer — the one place a user goes
//! looking when they wonder what is running in the background.
//!
//! Gated on the host being Windows, matching the target-gated build-dependency:
//! `rc.exe` comes from the MSVC toolchain, so this is the case that matters.
//! Cross-compiling to Windows from Linux still produces a working binary, just
//! without the icon.

#[cfg(windows)]
fn main() {
    // Shares the launcher's icon: they are one product, and this is the same
    // .ico the bundler ships.
    const ICON: &str = "../tauri-ui/src-tauri/icons/icon.ico";

    println!("cargo:rerun-if-changed={ICON}");
    println!("cargo:rerun-if-changed=build.rs");

    if !std::path::Path::new(ICON).is_file() {
        // Regenerate with `node tools/make-icons.mjs`. Not fatal: a watcher
        // without an icon is better than a build that will not finish.
        println!("cargo:warning={ICON} is missing, building without an icon");
        return;
    }

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(ICON);
    resource.set("ProductName", "PC GamePak");
    resource.set("FileDescription", "Cartridge watcher");
    resource.set("CompanyName", "PC GamePak contributors");
    resource.set("LegalCopyright", "MIT licensed");
    resource.set("OriginalFilename", "pc-gamepak-watcher.exe");

    if let Err(error) = resource.compile() {
        println!("cargo:warning=could not embed the icon: {error}");
    }
}

#[cfg(not(windows))]
fn main() {}
