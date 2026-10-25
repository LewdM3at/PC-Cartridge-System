//! `autorun.inf`, for the drive's name and icon in Windows Explorer.
//!
//! This is the one legitimate remaining use of the file. Windows has ignored
//! `open=` and `shellexecute=` on non-optical media since Windows 7, so nothing
//! here is executable — but `label=` and `icon=` are still honoured, which is
//! what makes a cartridge show up in Explorer as "HOLLOW KNIGHT" with its cover
//! art instead of "Removable Disk (D:)".
//!
//! Explorer only accepts `.ico`, `.bmp`, `.exe` or `.dll` for `icon=`, not the
//! PNG or JPEG that SteamGridDB serves, so whatever art is chosen has to be
//! converted here. An `.ico` may *contain* PNGs verbatim (Vista and later), so
//! the container itself is a 6-byte header plus 16 bytes per image and needs no
//! encoder of its own: `image` decodes and resizes the source, and everything
//! below assembles the file by hand.

use std::path::Path;

/// PNG-in-ICO entries record their size in one byte, where 0 means 256, so
/// anything larger cannot be described.
const MAX_ICON_EDGE: u32 = 256;

/// Render the file. `icon` is a filename on the cartridge, if there is one.
pub fn render_autorun(label: &str, icon: Option<&str>) -> String {
    let mut out = String::from("[autorun]\r\n");
    // CRLF throughout: this file is read by Windows.
    out.push_str(&format!("label={}\r\n", sanitize_inf_value(label)));
    if let Some(icon) = icon {
        out.push_str(&format!("icon={}\r\n", sanitize_inf_value(icon)));
    }
    out.push_str("\r\n; Written by the PC GamePak create wizard.\r\n");
    out.push_str("; label and icon only - this cartridge is launched by the\r\n");
    out.push_str("; launcher app, never by Windows autorun.\r\n");
    out
}

/// Strip anything that would break the INI or inject another key.
pub fn sanitize_inf_value(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .filter(|c| !matches!(c, '[' | ']' | '='))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Width and height from a PNG's IHDR chunk.
pub fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if bytes.len() < 24 || bytes[..8] != SIGNATURE {
        return None;
    }
    // IHDR is always first: length (4) + "IHDR" (4) + width (4) + height (4).
    if &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    (width > 0 && height > 0).then_some((width, height))
}

/// The sizes Explorer asks for, smallest first.
///
/// A container holding only 256px is legal, and it is what an unresized icon
/// produced - but then the shell downscales it for the drive list, which is
/// where SteamGridDB icons turned to mush. Resizing each size properly here
/// costs a few kilobytes and fixes that.
const ICON_SIZES: [u32; 4] = [16, 32, 48, 256];

/// Wrap a PNG in a single-image `.ico` container.
///
/// Returns `None` when the PNG is too large to describe in an icon directory
/// entry, or is not a PNG at all.
pub fn ico_from_png(png: &[u8]) -> Option<Vec<u8>> {
    let (width, height) = png_dimensions(png)?;
    ico_from_pngs(&[(width, height, png.to_vec())])
}

/// Convert any PNG or JPEG into a multi-size `.ico`.
///
/// This is what a downloaded icon goes through. SteamGridDB serves icons at
/// whatever size the uploader had, commonly 512px, which is past what an icon
/// directory entry can even describe - so before this existed every downloaded
/// icon was silently dropped and the cartridge kept Explorer's default.
///
/// Sizes above the source are skipped rather than upscaled: inventing detail
/// looks worse than letting the shell stretch the largest size on offer.
pub fn ico_from_image(bytes: &[u8]) -> Option<Vec<u8>> {
    let source = image::load_from_memory(bytes).ok()?.into_rgba8();
    let edge = source.width().max(source.height());
    if edge == 0 {
        return None;
    }

    let mut sizes: Vec<u32> = ICON_SIZES.iter().copied().filter(|s| *s <= edge).collect();
    // Art smaller than the smallest icon still gets one, scaled up: a 12px
    // source with no sizes at all would otherwise convert to nothing.
    if sizes.is_empty() {
        sizes.push(ICON_SIZES[0]);
    }

    let frames: Vec<(u32, u32, Vec<u8>)> = sizes
        .into_iter()
        .map(|size| square_png(&source, size).map(|png| (size, size, png)))
        .collect::<Option<_>>()?;

    ico_from_pngs(&frames)
}

/// Resize onto a transparent square and encode as PNG.
///
/// Fit rather than fill: a source that is not square keeps its proportions and
/// sits in the middle, because an icon stretched to square reads as a mistake.
fn square_png(source: &image::RgbaImage, size: u32) -> Option<Vec<u8>> {
    use image::ImageEncoder;

    let edge = f64::from(source.width().max(source.height()));
    let scale =
        |side: u32| ((f64::from(side) * f64::from(size) / edge).round() as u32).clamp(1, size);
    let scaled = image::imageops::resize(
        source,
        scale(source.width()),
        scale(source.height()),
        image::imageops::FilterType::Lanczos3,
    );

    let mut canvas = image::RgbaImage::new(size, size);
    image::imageops::overlay(
        &mut canvas,
        &scaled,
        i64::from(size - scaled.width()) / 2,
        i64::from(size - scaled.height()) / 2,
    );

    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&canvas, size, size, image::ExtendedColorType::Rgba8)
        .ok()?;
    Some(png)
}

/// Assemble PNG frames into one `.ico` container.
///
/// Every frame is stored verbatim; the header is the whole format here.
fn ico_from_pngs(frames: &[(u32, u32, Vec<u8>)]) -> Option<Vec<u8>> {
    if frames.is_empty() || frames.len() > u16::MAX as usize {
        return None;
    }
    if frames
        .iter()
        .any(|(width, height, _)| *width > MAX_ICON_EDGE || *height > MAX_ICON_EDGE)
    {
        return None;
    }

    let mut out = Vec::new();

    // ICONDIR: reserved, type 1 (icon), how many images follow.
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(frames.len() as u16).to_le_bytes());

    // An ICONDIRENTRY each, then the images, so the first offset clears them all.
    let mut offset = 6 + 16 * frames.len() as u32;
    for (width, height, png) in frames {
        // 256 is encoded as 0.
        out.push(if *width == MAX_ICON_EDGE {
            0
        } else {
            *width as u8
        });
        out.push(if *height == MAX_ICON_EDGE {
            0
        } else {
            *height as u8
        });
        out.push(0); // palette count: 0 for non-palettised
        out.push(0); // reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // colour planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }

    for (_, _, png) in frames {
        out.extend_from_slice(png);
    }
    Some(out)
}

/// Write `autorun.inf` to the cartridge, and the drive icon when one can be made.
///
/// `cover` is the art already copied onto the cartridge, if any. Returns the
/// icon filename that ended up in the file.
pub fn write_autorun(
    root: &Path,
    label: &str,
    cover: Option<&Path>,
) -> std::io::Result<Option<String>> {
    let icon = cover.and_then(|path| make_icon(root, path));
    remove_stale_icons(root, icon.as_deref());
    let contents = render_autorun(label, icon.as_deref());
    let path = root.join("autorun.inf");
    unprotect(&path);
    std::fs::write(&path, contents)?;
    // Explorer reads this file for the drive's icon only when it is marked
    // hidden and system. Written without them it is simply ignored, and a
    // cartridge with a perfectly good icon still comes up as a generic
    // drive — which is what happened to every cartridge built before this.
    protect(&path, true);
    // Telling the shell the drive changed makes it re-read autorun.inf, but
    // not the icon: that sits in iconcache_*.db keyed by its path, which is
    // why the icon is named after its content (see `make_icon`).
    notify_shell(root);
    Ok(icon)
}

/// Tell Explorer that this drive's appearance has changed.
#[cfg(windows)]
fn notify_shell(root: &Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{
        SHChangeNotify, SHCNE_UPDATEDIR, SHCNE_UPDATEITEM, SHCNF_FLUSH, SHCNF_PATHW,
    };

    let wide: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
    let path = wide.as_ptr().cast();
    // The icon hangs off the drive itself; the contents changed too, and
    // Explorer treats those as separate notifications.
    unsafe {
        SHChangeNotify(
            SHCNE_UPDATEITEM as i32,
            SHCNF_PATHW | SHCNF_FLUSH,
            path,
            std::ptr::null(),
        );
        SHChangeNotify(
            SHCNE_UPDATEDIR as i32,
            SHCNF_PATHW | SHCNF_FLUSH,
            path,
            std::ptr::null(),
        );
    }
}

#[cfg(not(windows))]
fn notify_shell(_root: &Path) {}

/// Hide the cartridge's asset folder, so the drive root shows what is on the
/// cartridge rather than what the wizard needed to put there.
pub fn hide(path: &Path) {
    protect(path, false);
}

/// Mark a file hidden, and system too when Explorer requires it.
///
/// Hiding also keeps both files out of the way on a drive someone is using for
/// something else, which is what the visible clutter was.
#[cfg(windows)]
fn protect(path: &Path, system: bool) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        SetFileAttributesW, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM,
    };

    let mut attributes = FILE_ATTRIBUTE_HIDDEN;
    if system {
        attributes |= FILE_ATTRIBUTE_SYSTEM;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // Best effort: a drive that will not take the attribute still gets the
    // file, and the cartridge is worth finishing either way.
    unsafe { SetFileAttributesW(wide.as_ptr(), attributes) };
}

/// Clear the attributes `protect` sets, so the file can be overwritten.
///
/// `File::create` on Windows fails with access denied when the file it would
/// truncate is hidden and the new handle does not claim to be, so rebuilding a
/// cartridge over an existing one has to undo this first.
#[cfg(windows)]
fn unprotect(path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{SetFileAttributesW, FILE_ATTRIBUTE_NORMAL};

    if !path.exists() {
        return;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe { SetFileAttributesW(wide.as_ptr(), FILE_ATTRIBUTE_NORMAL) };
}

#[cfg(not(windows))]
fn protect(_path: &Path, _system: bool) {}

#[cfg(not(windows))]
fn unprotect(_path: &Path) {}

/// Produce the drive icon on the cartridge if we can, and return its name.
fn make_icon(root: &Path, cover: &Path) -> Option<String> {
    let bytes = std::fs::read(cover).ok()?;

    // What the file *is*, never what it is called. SteamGridDB serves real .ico
    // files, and they arrive through a cache that names everything it does not
    // recognise `.jpg` — so an extension check sent an icon that needed nothing
    // done to it down the decode path, where it failed and the cartridge ended
    // up with no icon at all.
    let ico = if is_ico(&bytes) {
        bytes
    } else {
        // Anything else is decoded and resized into a proper multi-size icon. A
        // 600x900 Steam grid converts too - it is letterboxed into the square
        // rather than refused, which beats no drive icon at all.
        ico_from_image(&bytes)?
    };

    // Explorer caches the icon image by file path and keeps it across unplugs,
    // SHChangeNotify and `ie4uinit -show` alike, so a cartridge rewritten with
    // new art under the same name kept showing the old picture. A name that
    // changes with the content is a path the cache has never seen.
    let name = icon_name(&ico);
    let path = root.join(&name);
    unprotect(&path);
    std::fs::write(&path, ico).ok()?;
    // Hidden, but not system: Explorer only insists on that for autorun.inf,
    // and the icon is just a file the cartridge would rather not show.
    protect(&path, false);
    Some(name)
}

/// `icon-<hash>.ico`, so different art always lands at a different path.
fn icon_name(ico: &[u8]) -> String {
    use std::hash::{DefaultHasher, Hasher};
    let mut hasher = DefaultHasher::new();
    hasher.write(ico);
    format!("icon-{:08x}.ico", hasher.finish() as u32)
}

/// Is this a drive icon some build of the writer put at the root?
fn is_drive_icon(name: &str) -> bool {
    let name = name.to_lowercase();
    name == crate::create::ICON_NAME
        || name == "cover.ico"
        || name
            .strip_prefix("icon-")
            .and_then(|rest| rest.strip_suffix(".ico"))
            .is_some_and(|hash| !hash.is_empty() && hash.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Delete every earlier drive icon but the one autorun.inf is about to name.
fn remove_stale_icons(root: &Path, keep: Option<&str>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_drive_icon(&name) && Some(name.as_str()) != keep {
            let path = entry.path();
            unprotect(&path);
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Does this start with an ICONDIR: reserved 0, type 1 (icon), one image or more?
pub fn is_ico(bytes: &[u8]) -> bool {
    bytes.len() >= 6 && bytes[0..4] == [0, 0, 1, 0] && u16::from_le_bytes([bytes[4], bytes[5]]) > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn autorun_is_hidden_and_system_so_explorer_reads_it() {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM,
        };

        let scratch = crate::testutil::Scratch::new("autorun-attrs");
        write_autorun(scratch.path(), "Tomb Raider", None).unwrap();

        let attributes = std::fs::metadata(scratch.join("autorun.inf"))
            .unwrap()
            .file_attributes();
        // Without both of these Explorer ignores the file and the cartridge
        // shows the generic drive icon, however good its .ico is.
        assert!(attributes & FILE_ATTRIBUTE_HIDDEN != 0, "not hidden");
        assert!(attributes & FILE_ATTRIBUTE_SYSTEM != 0, "not system");
    }

    #[cfg(windows)]
    #[test]
    fn a_cartridge_can_be_rebuilt_over_a_hidden_autorun() {
        // Truncating a hidden file with a handle that does not claim to be
        // hidden fails with access denied, so writing the attributes without
        // clearing them first would break every rebuild after the first.
        let scratch = crate::testutil::Scratch::new("autorun-rebuild");
        write_autorun(scratch.path(), "First", None).unwrap();
        write_autorun(scratch.path(), "Second", None).unwrap();

        let written = std::fs::read_to_string(scratch.join("autorun.inf")).unwrap();
        assert!(written.contains("label=Second"), "{written}");
    }

    #[test]
    fn renders_label_and_icon_with_crlf() {
        let out = render_autorun("Hollow Knight", Some("icon.ico"));
        assert!(out.starts_with("[autorun]\r\n"));
        assert!(out.contains("label=Hollow Knight\r\n"));
        assert!(out.contains("icon=icon.ico\r\n"));
        // Nothing executable may ever appear in a file we write.
        assert!(!out.to_lowercase().contains("open="));
        assert!(!out.to_lowercase().contains("shellexecute="));
    }

    #[test]
    fn omits_the_icon_key_when_there_is_no_icon() {
        let out = render_autorun("Cinder & Salt", None);
        assert!(out.contains("label=Cinder & Salt"));
        assert!(!out.contains("icon="));
    }

    #[test]
    fn sanitises_values_that_would_break_the_ini() {
        // A newline plus a key would otherwise let a title add its own entry.
        assert_eq!(
            sanitize_inf_value("Doom\r\nopen=evil.exe"),
            "Doom openevil.exe"
        );
        assert_eq!(sanitize_inf_value("[autorun]"), "autorun");
        assert_eq!(sanitize_inf_value("  spaced   out  "), "spaced out");
    }

    #[test]
    fn a_sanitised_title_cannot_introduce_a_key() {
        let out = render_autorun("X\r\nicon=C:\\evil.dll", None);
        // Only one line may start with a key name.
        let keys: Vec<&str> = out
            .lines()
            .filter(|l| l.contains('=') && !l.trim_start().starts_with(';'))
            .collect();
        assert_eq!(keys.len(), 1, "{keys:?}");
        assert!(keys[0].starts_with("label="));
    }

    #[test]
    fn reads_png_dimensions() {
        let png = fake_png(64, 64);
        assert_eq!(png_dimensions(&png), Some((64, 64)));
        assert_eq!(png_dimensions(b"not a png at all"), None);
        assert_eq!(png_dimensions(&[]), None);
        // A JPEG must not be mistaken for one.
        assert_eq!(
            png_dimensions(&[0xff, 0xd8, 0xff, 0xe0, 0, 0, 0, 0, 0, 0]),
            None
        );
    }

    #[test]
    fn wraps_a_small_png_into_a_valid_ico() {
        let png = fake_png(128, 128);
        let ico = ico_from_png(&png).expect("128px wraps");

        // ICONDIR: reserved 0, type 1, count 1.
        assert_eq!(&ico[0..2], &[0, 0]);
        assert_eq!(&ico[2..4], &[1, 0]);
        assert_eq!(&ico[4..6], &[1, 0]);
        // Entry dimensions.
        assert_eq!(ico[6], 128);
        assert_eq!(ico[7], 128);
        // Size and offset.
        assert_eq!(
            u32::from_le_bytes([ico[14], ico[15], ico[16], ico[17]]),
            png.len() as u32
        );
        assert_eq!(u32::from_le_bytes([ico[18], ico[19], ico[20], ico[21]]), 22);
        // The PNG follows verbatim.
        assert_eq!(&ico[22..], &png[..]);
    }

    #[test]
    fn encodes_256_pixels_as_zero() {
        let ico = ico_from_png(&fake_png(256, 256)).expect("256px is the maximum");
        assert_eq!(ico[6], 0);
        assert_eq!(ico[7], 0);
    }

    #[test]
    fn declines_art_it_cannot_describe() {
        // The verbatim wrap only ever fit an already-small PNG.
        assert_eq!(ico_from_png(&fake_png(600, 900)), None);
        assert_eq!(ico_from_png(&fake_png(257, 100)), None);
        assert_eq!(ico_from_png(b"jpeg bytes"), None);
    }

    #[test]
    fn converts_art_the_verbatim_wrap_refuses() {
        // A 512px icon is what SteamGridDB actually serves, and the wrap above
        // declines every one of them. This is the path that has to take it.
        let source = real_png(512, 512);
        assert_eq!(ico_from_png(&source), None, "too big to wrap");

        let ico = ico_from_image(&source).expect("512px converts");
        assert_eq!(icon_sizes(&ico), vec![16, 32, 48, 256]);
    }

    #[test]
    fn converts_a_jpeg_too() {
        let ico = ico_from_image(&real_jpeg(300, 300)).expect("a JPEG converts");
        assert_eq!(icon_sizes(&ico), vec![16, 32, 48, 256]);
    }

    #[test]
    fn never_upscales_past_the_source() {
        // 256 would be invented detail on a 64px source, so it is not offered.
        assert_eq!(icon_sizes_of(&real_png(64, 64)), vec![16, 32, 48]);
        assert_eq!(icon_sizes_of(&real_png(20, 20)), vec![16]);
        // Below every size there is still one icon rather than none.
        assert_eq!(icon_sizes_of(&real_png(8, 8)), vec![16]);
    }

    #[test]
    fn a_wide_source_keeps_its_proportions() {
        // Every frame is a square container; the art inside is letterboxed.
        let ico = ico_from_image(&real_png(600, 900)).expect("a grid converts");
        assert_eq!(icon_sizes(&ico), vec![16, 32, 48, 256]);
    }

    #[test]
    fn an_ico_is_recognised_by_its_bytes_and_not_its_name() {
        // What SteamGridDB's icons tab serves. It reached the cartridge named
        // `icon.jpg`, and trusting that name is what lost the drive its icon.
        let ico = ico_from_image(&real_png(64, 64)).unwrap();
        assert!(is_ico(&ico));

        // Nothing else is one.
        assert!(!is_ico(&real_png(64, 64)));
        assert!(!is_ico(&real_jpeg(64, 64)));
        assert!(!is_ico(&[]));
        assert!(!is_ico(&[0, 0, 1, 0]), "a header with no images");
        assert!(!is_ico(&[0, 0, 1, 0, 0, 0]), "a header claiming none");
        assert!(!is_ico(&[0, 0, 2, 0, 1, 0]), "type 2 is a cursor");
    }

    #[test]
    fn an_ico_is_written_through_untouched_whatever_it_is_called() {
        let scratch = crate::testutil::Scratch::new("ico-passthrough");

        // Real .ico files hold BMP frames, not PNG, so re-encoding one would
        // mean decoding a format this crate does not read. It is copied whole.
        let source = ico_from_image(&real_png(300, 300)).unwrap();
        let misnamed = scratch.join("icon.jpg");
        std::fs::write(&misnamed, &source).unwrap();

        let name = write_autorun(scratch.path(), "ICO", Some(&misnamed))
            .unwrap()
            .expect("an icon");
        assert_eq!(std::fs::read(scratch.join(&name)).unwrap(), source);
    }

    #[test]
    fn new_art_gets_a_new_icon_path_and_the_old_icon_goes() {
        // Explorer's icon cache is keyed by path, so the same name for new
        // art keeps showing the old picture until the cache is deleted by hand.
        let scratch = crate::testutil::Scratch::new("icon-rename");
        // Left by builds that used a fixed name.
        std::fs::write(scratch.join("icon.ico"), b"old").unwrap();
        std::fs::write(scratch.join("cover.ico"), b"older").unwrap();
        // Not ours.
        std::fs::write(scratch.join("icon-mine.ico"), b"x").unwrap();

        let first_art = scratch.join("a.png");
        std::fs::write(&first_art, real_png(64, 64)).unwrap();
        let first = write_autorun(scratch.path(), "A", Some(&first_art))
            .unwrap()
            .unwrap();

        let second_art = scratch.join("b.png");
        std::fs::write(&second_art, real_png(96, 96)).unwrap();
        let second = write_autorun(scratch.path(), "B", Some(&second_art))
            .unwrap()
            .unwrap();

        assert_ne!(first, second);
        assert!(scratch.join(&second).is_file());
        for gone in [first.as_str(), "icon.ico", "cover.ico"] {
            assert!(!scratch.join(gone).exists(), "{gone} survived");
        }
        assert!(scratch.join("icon-mine.ico").is_file());

        // The same art again is the same path, so nothing churns needlessly.
        assert_eq!(
            write_autorun(scratch.path(), "B", Some(&second_art))
                .unwrap()
                .unwrap(),
            second
        );

        // Art dropped: no icon key and no icon file left behind.
        write_autorun(scratch.path(), "C", None).unwrap();
        assert!(!scratch.join(&second).exists());
    }

    #[test]
    fn refuses_what_it_cannot_decode() {
        assert_eq!(ico_from_image(b"not an image"), None);
        assert_eq!(ico_from_image(&[]), None);
    }

    /// Every frame the container advertises, in order. 0 in the entry means 256.
    fn icon_sizes(ico: &[u8]) -> Vec<u32> {
        let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
        (0..count)
            .map(|i| match ico[6 + i * 16] {
                0 => 256,
                edge => u32::from(edge),
            })
            .collect()
    }

    fn icon_sizes_of(png: &[u8]) -> Vec<u32> {
        icon_sizes(&ico_from_image(png).expect("converts"))
    }

    /// A real, decodable PNG - the fake below only carries a header.
    fn real_png(width: u32, height: u32) -> Vec<u8> {
        encode(width, height, image::ImageFormat::Png)
    }

    fn real_jpeg(width: u32, height: u32) -> Vec<u8> {
        encode(width, height, image::ImageFormat::Jpeg)
    }

    fn encode(width: u32, height: u32, format: image::ImageFormat) -> Vec<u8> {
        let image = image::RgbaImage::from_fn(width, height, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255])
        });
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .into_rgb8()
            .write_to(&mut out, format)
            .expect("encodes");
        out.into_inner()
    }

    #[test]
    fn writes_autorun_and_uses_a_supplied_ico() {
        let scratch = crate::testutil::Scratch::new("autorun");

        // No cover: label only.
        assert_eq!(write_autorun(scratch.path(), "PLAIN", None).unwrap(), None);
        let text = std::fs::read_to_string(scratch.join("autorun.inf")).unwrap();
        assert!(text.contains("label=PLAIN"));
        assert!(!text.contains("icon="));

        // A PNG cover.
        let png_path = scratch.join("art.png");
        std::fs::write(&png_path, real_png(512, 512)).unwrap();
        let icon = write_autorun(scratch.path(), "CONVERTED", Some(&png_path))
            .unwrap()
            .expect("a PNG converts");
        assert!(scratch.join(&icon).is_file());

        // A JPEG cover converts as well; this is the case that used to leave
        // the cartridge with Explorer's default icon.
        let jpg_path = scratch.join("art.jpg");
        std::fs::write(&jpg_path, real_jpeg(600, 900)).unwrap();
        let icon = write_autorun(scratch.path(), "JPEG", Some(&jpg_path))
            .unwrap()
            .expect("a JPEG converts");
        let text = std::fs::read_to_string(scratch.join("autorun.inf")).unwrap();
        assert!(text.contains("label=JPEG"));
        assert!(text.contains(&format!("icon={icon}")));

        // Art that is not an image at all: autorun still written, no icon key.
        let junk_path = scratch.join("art.bin");
        std::fs::write(&junk_path, b"not an image").unwrap();
        assert_eq!(
            write_autorun(scratch.path(), "JUNK", Some(&junk_path)).unwrap(),
            None
        );
    }

    /// A PNG header with real dimensions; the pixel data is irrelevant here.
    fn fake_png(width: u32, height: u32) -> Vec<u8> {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&width.to_be_bytes());
        png.extend_from_slice(&height.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);
        png.extend_from_slice(&[0, 0, 0, 0]); // stand-in CRC
        png
    }
}
