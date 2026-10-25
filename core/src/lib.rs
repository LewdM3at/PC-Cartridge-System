//! Cartridge logic, free of any UI.
//!
//! Everything the launcher and the wizard actually *do* lives here: reading a
//! cartridge, finding games, deciding which drives may be written to,
//! formatting, copying, and writing the files that make a cartridge.
//!
//! It is a separate crate from the Tauri binary for one reason: the binary
//! cannot be compiled without webkit2gtk and a display, so tests living inside
//! it could not run in CI or on a contributor's machine. Here, `cargo test`
//! works anywhere.

#[cfg(test)]
mod testutil;

pub mod autorun;
pub mod busy;
pub mod cartridge;
pub mod create;
pub mod created;
pub mod drives;
pub mod edit;
pub mod eject;
pub mod emulated;
pub mod folders;
pub mod format;
pub mod frontend;
pub mod gamepak;
pub mod health;
pub mod hltb;
pub mod home;
pub mod idle;
pub mod insert;
pub mod launch;
pub mod ludusavi;
pub mod memcard;
pub mod moonlight;
pub mod nfc;
pub mod notify;
pub mod play;
pub mod playlog;
pub mod playnite;
pub mod playtrack;
pub mod portable;
pub mod proc;
pub mod registry;
pub mod remote;
pub mod saves;
pub mod settings;
pub mod sgdb;
pub mod shaders;
pub mod stats;
pub mod steam;
pub mod steamlib;
pub mod throttle;
pub mod trim;
pub mod tuning;
pub mod unboxed;
pub mod verify;

pub use cartridge::base64_encode;
