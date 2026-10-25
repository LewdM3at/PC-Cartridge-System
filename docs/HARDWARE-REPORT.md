# PC GamePak — first run on real Windows hardware

Branch: `claude/nvme-game-launcher-tauri-5g4p6g`
Date started: 2026-08-20
Machine: Windows 11 Pro 10.0.22621

Running log. Appended as work happens, not written up at the end.

---

## Test host

| Item | Value |
|---|---|
| OS | Windows 11 Pro 10.0.22621 |
| Shell session elevation | **Not elevated** (see Open issue 3) |
| rustc used for this run | **1.98.0 (88d9e12ae 2026-08-18)** via rustup |
| Rust also installed | standalone MSI `C:\Program Files\Rust stable MSVC 1.87` — shadows rustup on PATH (see Open issue 1) |
| node | v22.15.1 |
| npm | 11.16.0 |
| WebView2 runtime | 151.0.4129.93 — **present** |
| MSVC toolchain | present and linking (core and watcher link and run) |
| Steam | `c:/program files (x86)/steam`, running at session start |
| `config/libraryfolders.vdf` | present, 3 libraries: `C:\Program Files (x86)\Steam`, `B:\Steam`, `F:\Games\Steam` |

### Fixed disks — none of these is a cartridge candidate

| Disk | Model | Bus | Size | Letter | Label |
|---|---|---|---|---|---|
| 0 | CT1000MX500SSD1 | SATA | 931.5 GB | B: | Milo |
| 1 | Samsung SSD 850 EVO 500GB | SATA | 465.8 GB | E: | Harry |
| 2 | NVMe Samsung SSD 970 | NVMe | 931.5 GB | C: | Idris — **boot + system** |
| 3 | Force MP600 | NVMe | 1863.0 GB | F: | GAMES (108.3 GB free of 1863) |

All four are `DriveType = Fixed`.

### Cartridge candidate — the external NVMe

| Item | Value |
|---|---|
| Device | `\\.\PHYSICALDRIVE4` — disk 4 |
| Model | JMicron Tech SCSI Disk Device |
| Serial | DD56419883914 |
| Size | 256052966400 bytes (238.5 GiB / 256 GB) |
| Media type | External hard disk media |
| Partition style | **MBR** — one partition, offset 1048576, MBR type `FAT32 XINT13` |
| Filesystem | exFAT, label `External`, 238.40 GB free of 238.47 |
| Drive letter | **D:** |
| `GetDriveTypeW("D:\")` | **3 = `DRIVE_FIXED`** — not `DRIVE_REMOVABLE` (see Phase 1) |
| Bridge | USB `VID_152D&PID_A583` = **JMicron JMS583**, USB 3.1 Gen 2 NVMe-to-USB bridge |
| Driver bound | `USB Attached SCSI (UAS)` — **UASP, not BOT** |
| USB parent | `USB\ROOT_HUB30` — a USB 3.0 root hub, but see the correction below |
| Port | `Port_#0005.Hub_#0004` |
| `SafeRemovalRequired` | True |
| `RemovalPolicy` | 3 (removable, surprise removal expected) |

**First real hardware measurement the project has ever had:** the enclosure
negotiates **UASP**, not BOT — Windows bound `uaspstor`, not `USBSTOR`. That is
the good outcome. Negotiated USB link speed not yet read; the wizard health
readout is the next place to look for it (Phase 2).

**Corrected in Phase 3.** `ROOT_HUB30` names a USB 3.0-style root hub, and I
read it as proof of a USB 3 link. It is not. Walking one level further up,
that hub hangs off an `AMD USB 2.0 eXtensible Host Controller`, so the link
negotiated at **480 Mbps**. The hub's name says what the driver stack is, not
what the cable got. Always walk to the host controller.

---

## Phase 1 — prepare the NVMe — **PASS**

The enclosure is **external, in a USB enclosure, not internal** — so
insert-detection has something to detect. JMicron JMS583 bridge, USB 3.1 Gen 2,
on a USB 3.x root hub, `Port_#0005.Hub_#0004`.

It was found in a state the storage stack would not enumerate at all (see the
former Open issue 2, now closed below). **A physical replug cleared it.** After
the replug it came up already initialised and already formatted:

| Item | State found in, after replug |
|---|---|
| Disk number | 4 |
| Partition style | MBR |
| Partitions | 1, spanning the disk |
| Filesystem | exFAT |
| Volume label | `External` |
| Drive letter | D: |
| Contents | empty — `$RECYCLE.BIN` and `System Volume Information` only |
| Free | 238.40 GB of 238.47 GB |

So no `Initialize-Disk` step was needed, and none was run. Nothing destructive
has been done to it.

**Docs gap, worth recording:** the brief expects an uninitialised drive to be set
up as **GPT**; this one arrived as **MBR**. The wizard formats the volume in
Phase 3 but does not repartition, so the cartridge will sit on an MBR disk. That
is fine for exFAT and for the launcher, but it means the partition style a
cartridge ends up with is whatever the drive already had — the wizard has no
opinion about it, and the docs do not say so.

### Finding: `GetDriveType` reports this enclosure as FIXED, and the drive list follows

`GetDriveTypeW("D:\")` returns **3, `DRIVE_FIXED`** — not `DRIVE_REMOVABLE`.
That is normal for a USB NVMe bridge, and `core/src/drives.rs:296` already
anticipates it, accepting both kinds with the comment *"A USB-C NVMe enclosure
usually reports FIXED, not REMOVABLE, so both are offered."* Without that, the
cartridge drive would not appear in the wizard at all. Correct call.

The cost is the other half of the filter. The only drive `list()` excludes is
`%SystemDrive%`, so on this host the wizard drive list will offer, alongside the
cartridge:

- B: Milo (931.5 GB, 159.2 GB free)
- E: Harry (465.8 GB, 438.3 GB free)
- F: GAMES (1863.0 GB, 108.2 GB free)

The brief expected the list to show the NVMe and **not** fixed disks. It cannot,
because on Windows the cartridge is itself a fixed disk by this API. What stands
between a mis-click and erasing B: Milo is the format confirmation gate
(`core/src/format.rs:192`), which refuses until the drive's current name is typed
back exactly. That gate is real and the UI prints the exact string to type
(`create.js:849`). Verified by reading, not yet exercised — Phase 3.

Flagged as a design question rather than fixed, since narrowing the filter would
hide the cartridge. See Open question 5.

### Wart: on Windows the "current label" is a display string

`format::current_label` reads `TargetDrive::label`. On Linux that is the bare
volume label (`CART`); on Windows `drives.rs:317` builds it for display as
`External (D:)`, label plus letter. So the Windows gate demands
`External (D:)` typed back, not `External`.

Harmless in practice — the UI shows the exact string in both the prompt and the
placeholder, so there is nothing to guess — and it makes the gate marginally
harder to satisfy by accident. But the two platforms ask for different things
from the same code path. Left alone; noted for Phase 3, where it can be
confirmed against the real dialog.

---

## Phase 0 — build and unit tests — **PASS**

Ran with the rustup stable toolchain, 1.98.0. Two host problems had to be
cleared first; both are described under Open issues, and neither was a defect in
this repo.

| Check | Result |
|---|---|
| `cargo test --manifest-path core/Cargo.toml` | **PASS** — 149 passed, 0 failed |
| `cargo test --manifest-path watcher/Cargo.toml` | **PASS** — 20 passed, 0 failed (after one fix, below) |
| `cargo clippy --manifest-path core/Cargo.toml --all-targets -- -D warnings` | **PASS** — clean |
| `cargo clippy --manifest-path watcher/Cargo.toml --all-targets -- -D warnings` | **PASS** — clean |
| `npm install` (tauri-ui) | **PASS** — 4 packages audited, 0 vulnerabilities |
| `npm run build` (tauri-ui, wraps `tauri build`) | **PASS** — release build in 1m 49s, **0 warnings** |
| `cargo build --release` (watcher) | **PASS** — 4.08s, 0 warnings |

### Bug found and fixed: a watcher test could never pass on Windows

`cargo test` on the watcher failed one of 20:

```
test tags::tests::a_directory_named_the_long_way_round_is_still_that_tag ... FAILED
panicked at src\tags.rs:179:39:
tag directory: Os { code: 123, kind: InvalidFilename,
  message: "Felaktig syntax för filnamn, katalognamn eller volymetikett." }
```

Cause: the test created a tag directory literally named `04:a2:24:b2`. A colon
cannot appear in a Windows filename, so `create_dir_all` returned
`ERROR_INVALID_NAME` (123). The test asserted correct behaviour — a tag
directory named with punctuation still resolves from the plain UID — but chose a
directory name that cannot exist on this platform. It had only ever run on
Linux, where the name is legal.

Not a production defect. `normalise` and `resolve_in` are correct; nothing that
ships was wrong.

Fix: name the directory `04-a2-24-b2`. Same behaviour under test, legal on both
platforms. The colon form is a *lookup* rather than a directory name, and is
already covered as one by `a_tag_resolves_to_its_directory` and
`a_uid_is_reduced_to_its_hex_digits`.

Commit `9cbbc69`. Re-verified: 20 passed, 0 failed.

### Build artifacts

| Artifact | Path | Size |
|---|---|---|
| Launcher and wizard | `tauri-ui/src-tauri/target/release/pc-gamepak.exe` | 7,148,544 bytes |
| Watcher | `watcher/target/release/pc-gamepak-watcher.exe` | 255,488 bytes |
| NSIS installer | `tauri-ui/src-tauri/target/release/bundle/nsis/PC GamePak_0.1.0_x64-setup.exe` | 2,284,089 bytes |
| MSI installer | `tauri-ui/src-tauri/target/release/bundle/msi/PC GamePak_0.1.0_x64_en-US.msi` | 3,026,944 bytes |

`npm run build` is `tauri build`, so it produces the binary *and* both
installers; there is no separate frontend build step. Note the binary lives
under `tauri-ui/src-tauri/target/release/`, not a top-level `target/release/`.
Tauri downloaded NSIS 3.11 and WiX 3.14 during bundling, so the first build on
a fresh machine needs network access.

Both binaries are unsigned, so SmartScreen prompts are expected and are not
defects.

---

## Phase 2 — the wizard, nothing destructive — **PARTIAL**

Run: `tauri-ui\src-tauri\target\release\pc-gamepak.exe --create`. SmartScreen
prompted, as expected for an unsigned binary; clicked through.

| Check | Result |
|---|---|
| Window opens | **PASS** |
| Steam games listed | **PASS** |
| Playnite games listed | **FAIL** — bug found and fixed, below |
| Drive list shows the cartridge | **PASS** — `External (D:)` present |
| Drive list excludes C: | **PASS** |
| Drive list excludes other fixed disks | **FAIL by design** — see Open question 5 |
| Health readout: link speed, UASP vs BOT | **FAIL — not present in the wizard at all** |
| Single-game cartridge via the `+` button | **FAIL** — bug found and fixed, below |
| Artwork lookup | **FAIL** — 404 on every cover; bug found and fixed, below |
| Steam libraries on B: and F: | **PASS** — all three libraries read, 42 games |
| Nothing written | **PASS** — Write not pressed |

### Bug found and fixed: Playnite games never appear, and nothing says why

Reported from the running wizard as *"only steam games are listed, no other"*.
Playnite is installed on this host (`%APPDATA%\Playnite`, `%LOCALAPPDATA%\Playnite`),
so this was not an absent-library case. Two faults compounding:

**1. Every extension's `config.json` counted as a library export.**
`playnite::find_exports` took every `.json` one level inside `ExtensionsData/*`.
Playnite keeps each installed extension's settings in `config.json`, so on this
host that scan returned seven files, all settings, none an export:

```
ExtensionsData/00000002-…/config.json
ExtensionsData/85dd7072-…/config.json
ExtensionsData/aebe8b7c-…/config.json
ExtensionsData/c2f038e5-…/config.json
ExtensionsData/cb91dfc9-…/config.json
ExtensionsData/cb91dfc9-…/tagnames-swedish.json
ExtensionsData/e3c26a3d-…/config.json
```

Because that list was not empty, the branch that says *"Playnite is installed at
… but has no JSON library export. Install a JSON library exporter extension and
run it."* could never run. Instead the code took the newest candidate and tried
to parse it as a library, which failed against an unrelated settings file.

Worth being clear that needing an exporter extension is **correct and
documented**: Playnite keeps its library in `games.db`, a LiteDB file with no
usable Rust reader, so a JSON export is the only way in. This host has no
exporter installed. The bug was never about that — it was that the wizard could
not say so.

**2. The reason was discarded even when it was correct.** `create::list_games`
collected a problem per library and then dropped the whole list whenever any
games were found. Steam almost always answers, so the Playnite failure never
reached the window under any circumstances.

Fixes, in commit `63c986c`:

- `config.json` is no longer a candidate export.
- New `playnite::import_newest_in` tries the remaining candidates newest-first
  until one parses, so a tag list or cache written after a real export cannot
  hide it. `NotFound` when none parse, which is what the user needs told.
- `list_games` returns `GameList { games, problems }`. The wizard shows the
  problems and opens the manual Playnite path field.

Three tests added; core is 152 passed, 0 failed. Clippy clean on core and on the
Tauri backend, and the release binary and both installers rebuilt.

**Still to confirm on this host:** with no exporter installed, the wizard should
now say so in as many words. Install any JSON library exporter extension in
Playnite and re-run to see real Playnite games listed — untested, because there
is no export on this machine to test against.

### Bug found and fixed: adding one game leaves the wizard with no way forward

Reported as *"I can only make a cartridge with >1 games?"* — near enough. One
game is genuinely a dead end, and this is why.

Each row in the game list has a `+` that adds the game to the bundle. Bundle
mode starts at **two** games (`create.js:446`), which is the right threshold: one
game is not a collection and should not be asked for a collection title and
cover. But `toggleBundleGame` only ever wrote to `bundleGames`; it never set
`selectedGame`, which is what `intent()` reads.

So with exactly one game added:

- the bundle panel lists it,
- the row is drawn as selected (`aria-selected` counts bundle membership),
- `isBundleMode()` is false, so the collection branch does not run,
- `selectedGame` is null, so `intent()` returns null,
- **Write is disabled, and nothing on screen says why.**

Clicking the row instead of the `+` works, which is why this is easy to miss:
the two controls sit on the same row and only one of them leads anywhere.

Fix in commit `8f7a367`: when the bundle holds exactly one game, that game
becomes the selection, so one `+` makes a single-game cartridge and a second
hands over to the collection UI. Removing a game leaves the selection alone — a
game picked by clicking its row was never the bundle's to unpick.

### Bug found and fixed: every cover lookup 404s

Reported from the running wizard:

```
Artwork lookup failed: SteamGridDB returned HTTP 404 for
https://www.steamgriddb.com/api/v2/covers/game/4997889
```

There is no `/covers` route. The v2 API serves `grids`, `heroes`, `logos` and
`icons`, and a cover is a grid in one of the portrait sizes. So
`sgdb.rs:301` asked for a route that has never existed, and did it for **every
game, on every cover lookup** — and `Cover/Poster` is the option the dialog
selects by default, so this is what a first-time user meets.

There was a fallback to portrait grids, but it could not fire: it ran only when
the response parsed and came back with an empty list, and a 404 is neither.

Fixed in commit `d42f06a`:

- `ArtworkType::Cover` asks for `/grids/game/<id>?dimensions=600x900,342x482,660x930`.
- A 404 from any request is read as an empty result rather than an error, so a
  game with no artwork of one kind falls back to another instead of failing.

Two tests added, one of which asserts that every artwork type targets a route
the API actually has.

**Not yet re-tested against the live API** — it needs the tester's key, and the
old binary was still running and holding the executable when this was written.

### Steam libraries: all three are read, and one entry was not a game

Asked whether the games on B: and F: were being seen. They are. Every library in
`libraryfolders.vdf` is walked:

| Library | Games | Size |
|---|---|---|
| `C:\Program Files (x86)\Steam\steamapps` | 0 | 0 GB |
| `B:\Steam\steamapps` | 3 | 199 GB |
| `F:\Games\Steam\steamapps` | 39 | 586 GB |

B: is Diablo IV (170 GB), HELLDIVERS 2 (23 GB) and LEGO Harry Potter Years 1-4
(6 GB); F: holds the other 39.

The C: entry used to be 1: `228980 Steamworks Common Redistributables`, listed
in the wizard as something a cartridge could be made from. Steam writes it an
`appmanifest` exactly as it does a game and it is fully installed, so nothing in
`game_from_manifest` told them apart, and a cartridge made from it would have a
Play button that starts nothing.

Filtered in the same commit, by id for the redistributables and the Linux
runtimes, and by name for Proton and Steam Linux Runtime, which take a new app
id per release. The list is now 42 games, all of them games.

### Gap: 44 installed games on this host cannot go on a cartridge at all

Asked about `B:\Games`, which holds installed games the same way the folders
under `F:\Games` do. Neither is visible to the wizard, and neither can be
copied.

| Location | Games |
|---|---|
| `B:\Games` | 12 |
| `F:\Games\Epic` | 10 |
| `F:\Games\Origin` | 3 |
| `F:\Games\Blizzard` | 2 |
| `F:\Games\Ubisoft` | 1 |
| `F:\Games\Other\Cracked Games` | 16 |
| **Total** | **44** |

For comparison the wizard currently offers 42, all of them Steam. So roughly
half this machine's library is unreachable.

The cause is one early return in `portable_source` (`core/src/create.rs:1053`):

```rust
let Some(playnite_id) = request.playnite_id.as_deref() else {
    return Ok(None);
};
```

A folder is copyable only if Playnite told the wizard where it is. Everything
else needed is already built and already generic:

- `portable::find_executables(dir, title, play_action)` ranks the candidate
  executables in **any** directory, with scoring that already demotes
  `launcher`, `server`, `editor` and `benchmark`.
- `copy_portable_game` copies **any** source directory to `Games/<name>` on the
  cartridge, sums it for verification, checks free space, and rewrites the Play
  target to a path inside the cartridge.

Only the source is missing. The UI does not hide this: in manual mode the Copy
checkbox is disabled and the hint reads *"Only available for a game picked from
the list."* So a manual entry produces a cartridge that points at
`B:\Games\...`, which works on this PC only — the opposite of what a cartridge
is for.

Two ways out, neither taken yet:

1. **Install a Playnite JSON exporter.** Playnite already has the Epic, Origin,
   Ubisoft and Battle.net library integrations installed, so an export would
   reach 16 of the 44 with install directories attached, and they would become
   copyable with no code change. The 12 in `B:\Games` and the 16 under `Other`
   would each have to be added to Playnite by hand first.
2. **Let the wizard take a folder.** Add a source directory to
   `CartridgeRequest`, return it from `portable_source`, and put a folder picker
   beside the manual entry — `tauri-plugin-dialog` is already a dependency and
   already used by `pick_cover_image`. The executable picker that exists for
   Playnite games (`executable_choices`) would then work for these too, since it
   is only `find_executables` behind a command that currently takes a Playnite
   id.

**Decision: (2), built.** Commit `65236fa`.

- `CartridgeRequest` and `BundleGameRequest` carry a `source_dir`, and
  `portable_source` prefers it over Playnite.
- The by-hand entry grows a folder picker. The folder name is offered as the
  title, the folder size is shown, and the executable picker that already
  existed for Playnite games now works on any directory.
- Write no longer demands a launch target when the copy is about to supply one.
- `check_source_dir` refuses a whole drive (whose copy would take every other
  game and the recycle bin with it) and anything inside `%SystemRoot%`;
  `copy_portable_game` separately refuses a source that is already on the
  cartridge, which would not terminate. Both are checked in core, not in the
  window, because a command is reachable whatever the interface allowed.
- `source_dir` is per game in a bundle. `for_game` fills the rest of the request
  with `..self.clone()`, so an inherited source would have copied one folder once
  per game under each game's name.

Four tests added; core is 159 passed, 0 failed, clippy clean on core and the
Tauri backend, release binary and both installers rebuilt.

**Untested on hardware.** Nothing has been copied to D: yet — that is Phase 3,
which is destructive and waits on confirmation.

### Bug: there is no health readout in the wizard

`cartridge_health` is registered as a command and implemented, but it is invoked
only from `tauri-ui/app/src/main.js:215` — the **launcher**. The string
`cartridge_health` does not appear in `create.js` at all.

So the Phase 2 expectation of a health readout showing negotiated link speed and
UASP vs BOT while choosing a drive cannot be met: the wizard has never had one.
The information exists and the launcher shows it, but only once a cartridge has
been made and inserted.

Not fixed. Adding a health panel to the drive step is a feature, not a
correction, and it wants a design decision about when to run it — `health::inspect`
shells out to PowerShell on Windows, and doing that for every drive in the list
on every rescan is not free. See Open question 6.

This means the negotiated USB link speed is **still unmeasured**. It should
appear in the launcher in Phase 5.

### Artwork search needs a key, and says so

SteamGridDB search is off until switched on: `sgdb::api_key_from` refuses unless
`steamgriddb_enabled` is set and a key is present, and settings default to off.
So a fresh install has no artwork search, by design, and the dialog reports
`SteamGridDB unavailable. You can still paste a URL.`

Reported as *"search etc is a bit broken"*. The in-list game search
(`create.js:178`) is a plain case-insensitive substring match on name and source
and looks correct by inspection. Which of the two was meant is not yet pinned
down — **needs one more detail from the tester** before it can be called a bug.

---

## Open issues

### Open issue 1 — two Rust installations, and the old one wins on PATH

The machine has both:

- `C:\Program Files\Rust stable MSVC 1.87` — standalone MSI, **rustc 1.87.0**, first on PATH.
- rustup at `C:\Users\Harry\.cargo\bin\rustup.exe`, toolchain `stable-x86_64-pc-windows-msvc` = **rustc 1.98.0**.

A plain `cargo test` picks 1.87 and dies during resolution, because the
committed lockfiles pin `icu_* 2.3.0`, which declare `rust-version = 1.88`:

```
error: rustc 1.87.0 is not supported by the following packages:
  icu_collections@2.3.0 requires rustc 1.88
  ...
```

The `icu_*` tree is transitive: ureq, then url, then idna, then idna_adapter,
then icu_*. Nothing in this repo asks for it directly.

Everything in this report was run with `%USERPROFILE%\.cargo\bin` prepended to
PATH, which selects 1.98 and builds fine. **Nothing in the repo needs changing
for this.** It is a host PATH-ordering problem.

Worth deciding — your call, not done:

- Uninstall "Rust 1.87 (MSVC 64-bit)" from Add/Remove Programs, so the rustup
  shims are the only Rust on PATH. Recommended; two Rusts on one PATH will keep
  biting.
- And/or commit a `rust-toolchain.toml` pinning `stable`, so the real floor is
  stated in the repo instead of being discovered like this. The repo currently
  has no toolchain file and no stated MSRV.

### Open issue 2 (CLOSED — physical replug) — the cartridge drive was not enumerated by the storage stack

`\\.\PHYSICALDRIVE4` is present and healthy at the PnP and WMI layers, but the
Windows storage stack does not list it:

- `Get-CimInstance Win32_DiskDrive` — lists it, `Status = OK`.
- `Get-PnpDevice` — lists it and its UAS parent, `Status = OK`, `Present = True`, `Problem = CM_PROB_NONE`.
- `Get-Disk` / `MSFT_Disk` — **does not list it.** Only disks 0-3, before and after `Update-HostStorageCache`.

An uninitialised disk normally still appears in `Get-Disk` as
`PartitionStyle = RAW`. This one does not appear at all, so `Initialize-Disk`
has nothing to address.

Possible causes, untested: the NVMe inside the enclosure is not responding to
the bridge; the storage service will only resolve it with elevation; or a stale
enumeration that a physical replug would clear.

**Resolved by unplugging the enclosure and plugging it back in.** It then
enumerated as disk 4 with drive letter D:, already MBR and already exFAT. So it
was a stale enumeration, not a dead NVMe and not an elevation problem.

Worth keeping in the report because the failure mode is nasty: every layer that
reports health said the device was fine — `Win32_DiskDrive` `Status = OK`,
`Get-PnpDevice` `Present = True` `Problem = CM_PROB_NONE` — while the layer that
matters listed nothing, and `Update-HostStorageCache` did not shake it loose. If
a user hits this, the wizard will simply show no cartridge drive and give no
reason. **Replug first** is the answer, and it is not written down anywhere.

### Open issue 3 — the session is not elevated

`IsInRole(Administrator) = False`. Needed for: initialising the disk and
creating a partition, formatting, `FSCTL_LOCK_VOLUME` and
`FSCTL_DISMOUNT_VOLUME` on eject, Defender exclusions, and the watcher install
in `windows/install.ps1`.

### Open issue 4 (host, now cleared) — corrupt cargo registry cache

Under rustc 1.98 the first builds still failed, differently:

```
error: invalid key
 --> ...\registry\src\index.crates.io-...\hashbrown-0.17.1\Cargo.toml:1:1
error: failed to download `hashbrown v0.17.1`
```

The cached `Cargo.toml` was 3847 bytes of **NUL**, with a bogus mtime of
`Jul 24 2006`. A scan of `~/.cargo/registry/src` found **167 of 254** extracted
crates with zeroed manifests — the signature of an unclean shutdown or power
loss while cargo was writing, and nothing to do with this project.

Cleared by deleting `~/.cargo/registry/src` (187.9 MB) and
`~/.cargo/registry/cache` (31.0 MB) and letting cargo re-fetch, plus
`cargo clean` on all three crates to drop 744.6 MB of artifacts built by the old
1.87 toolchain, which produced `error[E0786]: found invalid metadata files for
crate ...`.

Worth noting for the host: **C: has 23.3 GB free of 930.5 GB.** A read-only
`chkdsk C: /scan` would be a reasonable precaution given the corruption pattern.
Not run — needs elevation.

### Open question 5 — the drive list offers every non-system fixed disk

Raised by Phase 1. `drives::list()` on Windows takes both `DRIVE_REMOVABLE` and
`DRIVE_FIXED` — it has to, because a USB NVMe enclosure reports FIXED — and then
excludes only `%SystemDrive%`. On a machine with several data disks, all of them
appear in the wizard as things it will happily format.

Not fixed, because the obvious narrowing breaks the product: filter to
`DRIVE_REMOVABLE` and the cartridge disappears.

Options, none applied:

1. Leave it. The typed-label gate is the guard, and it is a good one.
2. Rank and mark: query `MSFT_Disk` for `BusType = USB` and sort those first,
   or badge the rest as "internal disk". Cosmetic, no behaviour change, keeps
   every drive reachable.
3. Refuse to *format* a non-USB disk while still listing it, so a fixed disk can
   receive a cartridge but never be erased by the wizard.

Your call. (2) is the cheap one; (3) is the one that would have prevented the
worst outcome on this host.

### Open question 6 — should the wizard show cartridge health?

Raised by Phase 2. The brief expects the drive step to show negotiated link
speed and UASP vs BOT. It does not, and never has: `cartridge_health` is wired
to the launcher only.

The cost is that the wizard cannot warn about the failure this project most
wants to catch — a cartridge on a BOT bridge, or negotiated at USB 2.0 — at the
one moment the user could still choose a different enclosure or port. They find
out after the copy instead.

The cost of adding it is that `health::inspect` shells out to PowerShell on
Windows. Running it per drive per rescan is too slow; running it once for the
selected drive, asynchronously, the way the launcher already does, is not.

Not implemented — it is a feature. Say if you want it and it is a small job.

---

## Phase 3 — format and copy (attempted by the user, incomplete)

You ran the wizard against **God of War - Ragnarok** while I was not driving it,
so this is a post-mortem of what the disk shows rather than a watched run.

### What the wizard did

| | |
|---|---|
| Source | `B:\Games\God of War - Ragnarok` — 6,451 files, **159.34 GB** |
| Chosen via | the folder picker built in `65236fa` (this game is in no launcher) |
| Target | **D:**, JMicron JMS583, 238.47 GB |
| Format | exFAT, volume label **`God of War`** — the wizard formatted and labelled it |
| Layout | `D:\Games\God of War - Ragnarok\...` — matches `copy_portable_game` |
| Copy started | 22:44:43 |
| Copy stopped | 22:48:00 |
| Copied | 2,098 files, **3.48 GB** of 159.34 GB — **2.2%** |

So the format step **PASSED**: the drive came back exFAT, correctly labelled,
and the copy wrote into `Games/<safe folder name>` exactly where the code says
it should.

The copy **FAILED**, in two distinct ways.

### Finding 1 — the copy died mid-file and left the wreckage behind

The last file written is truncated:

```
B:\Games\God of War - Ragnarok\exec\sound\pc_le_svartalfheim1a_svaxpl.0.audiopack   819,490,938 bytes
D:\Games\God of War - Ragnarok\exec\sound\pc_le_svartalfheim1a_svaxpl.0.audiopack   678,428,672 bytes
```

A returned error would have unwound out of `copy_and_digest` after a complete
`write_all`; a truncated file means the process stopped between chunks. Nothing
appears in the Windows Application event log for that minute, and a
`pc-gamepak.exe` was started fresh at 22:50:08 — consistent with the window
being closed or killed at 22:48.

Either way the cartridge is now holding 3.48 GB of a game that is not there,
including one file that is the right name and the wrong length. There is no
`cartridge.conf`, because that is written after the copy returns. **Nothing
cleans this up**, and nothing on a later run would notice: a re-copy would
overwrite file by file, and any file the second run did not reach would survive
as garbage. CRC verification would catch the truncated file, but only for files
the manifest lists.

Not fixed yet — see the question at the end, because the right answer depends on
whether the copy should be resumable.

### Finding 2 — the cartridge is plugged into a USB 2.0 port

This is the real number the brief asked for, and it is bad.

| Measurement | Result |
|---|---|
| Wizard copy, B: to D: | 3.478 GB in 197 s = **18.1 MB/s** |
| Windows `Copy-Item`, same file, B: to D: | **31.3 MB/s** |
| USB negotiated link | **480 Mbps (USB 2.0)** |

The device tree says it plainly:

```
USB\VID_152D&PID_A583\MSFT30DD56419883914   (JMS583, UAS)
  parent: USB\ROOT_HUB30&239a9e6d&0&0
  parent: AMD USB 2.0 eXtensible Host Controller - 1.20   <-- here
```

AMD chipsets expose a separate USB 2.0 xHCI controller, and the enclosure is on
it. 480 Mbps caps out around 35 MB/s in practice, which is exactly the 31.3 MB/s
Windows itself achieved. UASP is negotiated — that part is fine — but UASP over
USB 2.0 buys nothing.

**At this speed the copy would have taken 1 hour 25 minutes**, and the whole
point of the project (a game that loads from the cartridge as fast as from the
internal disk) cannot be tested from this port.

This host has four faster controllers available:

```
AMD USB 3.10 eXtensible Host Controller  (x2)
AMD USB 3.20 eXtensible Host Controller
ASMedia USB 3.20 eXtensible Host Controller
```

**Action for you: move the enclosure to a different physical port** — a blue or
teal one, or the rear USB-C — and I will re-measure before we retry the copy.
The port is a hardware fact I cannot change from here.

### Finding 3 — our copy runs at 58% of what the port allows

18.1 MB/s against Windows' 31.3 MB/s on the same drives and the same file.

`copy_and_digest` reads 1 MB, CRC32s it a byte at a time through a table, then
`write_all`s it, strictly serially — the disk is idle while the CRC runs and the
CRC is idle while the disk runs. It also calls `sync_all()` per file, which on a
678 MB file forces a full flush before the next file starts.

Worth fixing (overlap the hash with the write, or hash the read buffer while the
previous chunk is in flight), but **not now**: at 31 MB/s the port is the wall,
not us. Re-measure on a USB 3 port first, then decide whether this is still
worth the complexity.

### Open question 8 — what should a failed or abandoned copy leave behind?

The wizard currently leaves a partial game on the cartridge with no record that
it is partial. Three ways out:

1. **Clean up on failure.** Delete the destination folder if the copy does not
   complete. Simple, and safe because the folder is one we created — but a
   1-hour copy that dies at 95% is thrown away entirely.
2. **Mark it.** Write a marker file at the start of the copy and delete it at the
   end. Any run that finds the marker knows the folder is incomplete and can
   offer to resume or to start over. Cheap, and it makes the state legible.
3. **Resume.** Skip files whose size and CRC already match, re-copy the rest.
   Most useful on a 159 GB game over a slow link, and the most work.

A hard kill (task manager, power loss) defeats (1) — the process never gets to
run its cleanup — which is an argument for (2) as the floor whatever else we do.

My recommendation is **(2) now, (3) later**, because (2) is small, survives a
kill, and is the thing (3) would need anyway.

Your call.

---

## Phase status

| Phase | Status |
|---|---|
| 0 — build and unit tests | **PASS** — 172 tests, clippy clean, both binaries built (1 bug found and fixed) |
| 1 — prepare the NVMe | **PASS** — external USB enclosure, D:, UASP; needed a replug (former Open issue 2) |
| 2 — wizard, non-destructive | **PARTIAL** — window, Steam list and drive list pass; Playnite bug fixed; no health readout in the wizard |
| 3 — format and copy | **FAIL** — format passed; copy died at 2.2% and the cartridge is on a USB 2.0 port (18.1 MB/s) |
| 4 — cartridge contents | Not started |
| 5 — insert detection, Play, Eject | Not started |
| 6 — rewrite with Steam running | Not started |
| 7 — collections | Not started |
| 8 — tags without a reader | Not started |
| 9 — tuning, edit, controller, headroom | Not started |
| 10 — report | In progress (this file) |

---

## 2026-09-01 — two cartridges written, one verified, corruption found

First real read-back of a cartridge written by this code. It failed, which is
the most useful result the check could have produced.

### The cartridges

| Drive | Label | Written | Contents |
|---|---|---|---|
| Disk 4 | `PLAYSTATION` (G:) | 2026-09-01 10:31 | 3 games, exFAT, 119.22 GB |
| Disk 5 | `TOMB RAIDER` (D:) | 2026-08-31 20:00 | 10 games, 107.43 GB, exFAT |

Both in Realtek RTL9210B-CG USB enclosures. **Both enclosures report the same
serial, `1342925150003`** — worth knowing before anything identifies a cartridge
by drive serial.

### The verify

`verify-cart D:\` — 107.43 GB in 256 s, 420 MB/s (CRC-32 is the bottleneck, not
the link; the raw read runs at ~800 MB/s).

```
2 problems:
  2 corrupt

  steamapps/common/Shadow of the Tomb Raider/bigfile.000.tiger
  steamapps/common/Tomb Raider/bigfile.002.tiger
```

Both are ~2 GB game archives, both the right length, both the wrong CRC-32.
Seven files on that cartridge are ~2 GB `.tiger` archives and exactly two are
bad; nothing small was touched. That is a link failing under sustained transfer,
not random bit rot.

Hashed each corrupt file twice with SHA-256: identical across reads. The bytes
on the drive are stable, so this is not a read artefact — the data was written
wrong on 2026-08-31 and has been wrong since. `copy_and_digest` sums the source
as it passes, so the manifest CRC describes what was read off `F:`; the mismatch
is on the write side. Sources for both files are still in
`F:\Games\Steam\steamapps\common`, so a repair is a two-file re-copy.

### What the event log recorded

The 2026-09-01 write to `PLAYSTATION`:

```
10:32:05  UASPStor 129  Reset to device \Device\RaidPort9 issued
   ... 19 resets, roughly one every 20 s, for 7 minutes ...
10:39:59  disk 51  x6   error on \Device\Harddisk5\DR6 during a paging operation
```

Plus dozens of `disk 153` I/O retries throughout. The verify run four minutes
ago logged five more resets on a *different* port at 796 MB/s. So the enclosure
resets the bus on every port tried, just at different rates.

### Ports

Chased a "the drive is dead" symptom that was neither the drive nor, in the end,
a single cause:

| Port | Controller | Result |
|---|---|---|
| `Port_#0002.Hub_#0007` | ASMedia ASM4242 USB4 (`VEN_1B21&DEV_2426`) | 37 MB/s, two `VID_0000&PID_0002` Code 43 ghosts, port-path instance IDs |
| `Port_#0001.Hub_#0007` | same controller | 796 MB/s |
| `Port_#0001.Hub_#0004` | AMD chipset (`VEN_1022&DEV_43FD`) | 805 MB/s, no resets logged |

So `Port_#0002` is genuinely bad, and the controller is not — an earlier reading
that blamed the ASM4242 was wrong. Note a device that fails its descriptor
request produces **no volume at all**, so the watcher sees nothing and the user
gets silence rather than an error. Worth surfacing.

Host is now a Gigabyte X870E AORUS ELITE WIFI7, BIOS FB1 (2026-05-21) — the
ASUS-subsystem device IDs elsewhere in this log are from the previous board.

### Changed as a result

- `verify` is **on by default** (`settings.rs`, `create.js`, and the serde
  default on `CartridgeRequest::verify_copy`, so JSON requests get it too).
- `core/src/bin/verify-cart.rs` added: re-check a cartridge on demand.

### Open issue 9 — the enclosure resets the bus on every port

Not settled: cable, RTL9210B-CG bridge, or the drive. Order to try — swap the
cable first (it is the variable that differs between the two enclosures; the
`PLAYSTATION` unit logged no resets today), re-run `verify-cart`, and only
re-copy the two corrupt files once the resets stop. Writing 4 GB back through a
link that is still resetting is how you get a third corrupt file.

`PLAYSTATION` has no manifest at all — it was written before verify was on by
default, during the 19-reset session. It is the cartridge most likely to be bad
and the one that cannot be checked. Rewrite it.

---

## 2026-09-01, later — first clean end-to-end write

Open issue 9 closed: **it was the cable.** With it replaced, both enclosures sit
on ports that behave, and a full build ran without a single `UASPStor 129` reset
or `disk 153` retry in the event log — the first time that has happened.

### The run

`PLAYSTATION` (3 games, 99 GB, no manifest, written during the 19-reset session)
was rewritten as a single-game cartridge, driven by `build-cart`:

```json
{ "drivePath": "G:\\", "title": "Stardew Valley",
  "executable": "steam://rungameid/413150", "appId": "413150",
  "formatDrive": true, "formatFilesystem": "exfat", "formatLabel": "STARDEW",
  "formatConfirmation": "PLAYSTATION (G:)", "copyGame": true, "writeIcon": true }
```

`verifyCopy` is deliberately absent: this run was also the test that the new
serde default reaches the real pipeline. It did.

```
formatted: true (exfat)        gameCopied: true, 750,467,660 bytes
registeredWithSteam: true      usedPercent: 1
verified: "Checked all 3833 files against what was written; every one matches."
verifiedOk: true               warnings: []
```

Then, independently, `verify-cart G:\` — 3833 files, 0.75 GB at 443 MB/s,
"intact: every file matches the manifest."

The cartridge came out with `cartridge.conf` in single-game form (no
`[collection]` section), `.gamepak/manifest.json`, and `autorun.inf` plus
`cover.ico` both hidden. `pc-gamepak --drive G:\` opens on it: window titled
`PC GamePak`, 28.1 MB resident, closes cleanly on window close.

### Phase status, updated

| Phase | Status |
|---|---|
| 0 — build and unit tests | **PASS** — 194 tests, clippy clean, fmt clean |
| 1 — prepare the NVMe | **PASS** |
| 3 — format and copy | **PASS** — formats, copies, registers with Steam, verifies |
| 4 — cartridge contents | **PASS** — conf, manifest, autorun and icon all correct |
| 5 — insert detection, Play, Eject | **PARTIAL** — launcher opens on a real cartridge; insert detection and Play not yet driven |

### Still open

- The clean run was 0.75 GB. The write that corrupted data was 107 GB. Nothing
  has yet been watched through a long sustained transfer on the new cable, and
  that is where the old failure lived — so this is evidence, not proof.
- `TOMB RAIDER` still has its two corrupt `.tiger` archives. Now that the link
  is quiet, re-copying them from `F:\Games\Steam` should stick.
- The format gate wants the label exactly as `format_plan` reports it, which
  includes the drive letter — `PLAYSTATION (G:)`, not `PLAYSTATION`. The wizard
  shows that string so it is consistent there, but anything scripting
  `build-cart` has to know it.

---

## 2026-09-01, later still — the repair created a new corruption

Reopening Open issue 9. The previous entry closed it as "it was the cable". That
was too fast: it was the cable *for the port G: is on*. The other enclosure is
still destroying data.

### What was done

The two corrupt archives on `TOMB RAIDER` were replaced from
`F:\Games\Steam\steamapps\common`, 4 GB total, at 6.7 s and 3.4 s. Sizes matched
the manifest before and after.

### What the re-verify found

```
read 107.43 GB in 181s (594 MB/s)
1 problems:
  1 corrupt
  steamapps/common/Shadow of the Tomb Raider/bigfile.005.tiger
```

The two repaired files now pass. **`bigfile.005.tiger` did not, and it passed
three hours earlier** — the 17:17 verify reported exactly two bad files and this
was not one of them. `verify()` walks every file in the manifest, so it was not
skipped.

Checked before drawing conclusions:

- Two SHA-256 reads of the file agree, so the bytes on the drive are stable and
  this is not a read artefact.
- Its hash differs from the source file on `F:`, so the cartridge copy really is
  wrong rather than the manifest being stale.
- The event log has `UASPStor 129` plus three `disk 153` retries at 19:23:53,
  inside the window where the repair copy and verify ran.

So writing 4 GB to this enclosure corrupted 2 GB of a file that was not being
written to. The plausible mechanism is a reset landing mid-write and damaging
the exFAT allocation, so freshly allocated clusters overlapped an existing
file's chain — which is worse than a bad byte, because it means a write to a
cartridge can damage a game that was already on it and verified.

### Where each drive sits

| Drive | Port | Controller | Behaviour |
|---|---|---|---|
| G: `STARDEW` | `Port_#0001.Hub_#0004` | AMD chipset | clean write, clean verify, no resets |
| D: `TOMB RAIDER` | `Port_#0001.Hub_#0007` | ASMedia ASM4242 | reset during a 4 GB copy, one new corrupt file |

### Not done, deliberately

`bigfile.005.tiger` was left corrupt. Copying it would mean another write
through the same enclosure, and the last one is what caused this. The next step
is to move that enclosure to a chipset port — or swap the enclosure — and only
then rewrite the cartridge whole rather than patching files into it.

---

## 2026-09-01, resolved — Open issue 9 closed: it was the port

Reported by the machine's owner: the enclosure that corrupted `bigfile.005.tiger`
works correctly on a different port. Not the cable, not the bridge chip, not the
drive — the port.

That closes both hardware faults chased today, and both were physical:

| Symptom | Cause |
|---|---|
| Drive did not enumerate at all; Code 43 ghosts, 37 MB/s | `Port_#0002.Hub_#0007` — bad port |
| 19 bus resets over a 107 GB write; three files corrupted | the port the second enclosure was on |

**Neither was the drive, and neither was this software.** Worth keeping in mind
the first time a user reports that PC GamePak corrupted their cartridge: the
tool was right, the hardware was not, and the only reason anyone can tell the
difference is that `verify` wrote down what it copied.

### What it cost, and what that implies

`bigfile.005.tiger` on `TOMB RAIDER` is still corrupt. It stays that way here as
a record: it was destroyed by a write to two *other* files, which means a bad
link can damage a game that was already on the cartridge and had already passed
its check. Verifying what a copy just wrote is not sufficient on its own; that is
the argument for `verify-cart` being able to re-check a whole cartridge on demand
and, eventually, for the launcher offering it.

### Packaging verified against this release

While the hardware question was settling, both Windows channels were tested
against the published v0.1.0 artefact rather than in theory:

- **Scoop** — installed on this machine from the Scoop manifest, which now
  lives in [HarryBMa/scoop-bucket](https://github.com/HarryBMa/scoop-bucket).
  Scoop's hash check passed, `extract_dir` stripped the version folder, both
  executables landed at the app root and got shims, `windows\install.ps1` came
  with them. Uninstalled cleanly afterwards.
- **WinGet** — `winget validate` passes; the zip's internal paths match the
  manifest's `NestedInstallerFiles`; the hash matches a fresh download.
  `wingetcreate show` confirms the package is not yet in winget-pkgs, so the
  first submission goes through moderation.

---

## 2026-09-12 — 1.1.0 on hardware: first NTFS cartridge, insert detection

`claude/pc-gamepak-architecture-8evhux` fast-forwarded into `main` (13 commits:
saves, stats, insert actions, busy-check, shaders, front-end register, NTFS by
default, 1.1.0). This is the first time any of it has run on Windows.

### It did not build on Windows

CI checks core on Linux only, so `busy.rs` — the Windows half of "what is
holding this drive" — had never been compiled for the platform it is for.
Fixed in `aa781dc`:

| Fault | Effect |
|---|---|
| `Win32_System_Diagnostics_ToolHelp` and `Win32_System_Threading` missing from windows-sys features | core did not compile |
| `.is_null()` on handles, which are `isize` in windows-sys 0.52 | core did not compile |
| `Process32NextW` called without `Process32FirstW` | would have compiled into a process walk that returns nothing — "no one is holding the drive" with a game running from it |
| `home` and `shaders` tests expected `a/b` where `Path::join` makes `a\b` | 2 of 343 tests failed |

The third is the one worth remembering. It passes review, it passes type
checking once the first two are fixed, and it is wrong in the direction that
lets an eject proceed.

After the fixes: core **343 passed**, watcher **6 passed**, clippy and fmt
clean. `npm run build` produced `PC GamePak_1.1.0_x64-setup.exe` and the MSI.

Open issue 1 is still open on this host: plain `cargo` resolves to the 1.87 MSI
and fails. `rust-toolchain.toml` now pins **1.88.0**, and everything above ran
with `%USERPROFILE%\.cargo\bin` first on PATH. (`core/Cargo.toml`'s comment on
the `image` pin still says the toolchain file holds 1.87; it is stale.)

### The host, since last time

| Item | Value |
|---|---|
| Cartridge enclosure | Realtek RTL9210B-CG, 119.2 GB, **GPT**, found as empty NTFS `New Volume` (D:) |
| Port | `Port_#0001.Hub_#0004` — the AMD chipset port recorded as clean on 2026-09-01 |
| Host controller | **AMD USB 3.20 eXtensible Host Controller** (`VEN_1022&DEV_43FD`) — walked to, not inferred from the hub name |
| Driver | `UASPStor` |
| `TOMB RAIDER` | not connected; `bigfile.005.tiger` still unchecked |
| C: free | **8.4 GB** of 930 GB — down from 23.3 GB |

### Two old questions, answered by the code that is now here

- **Open question 5 (the drive list offers internal disks): settled.**
  `build-cart drives` offers exactly one drive, D:. B:, E: and F: are gone,
  because `drives.rs` now asks each volume's disk for its `BusType` and keeps
  USB. The cartridge is still found, which is the thing narrowing the filter
  used to cost.
- **The 44 unreachable games: reached.** `build-cart games` lists **86**, with
  `Folder`, `Epic`, `Ubisoft`, `Blizzard` and `Xbox` sources beside Steam. The
  Playnite problem is still reported in as many words, as fixed on 2026-08-20.

### Phase 3 and 4 — FTL, NTFS — **PASS**

Driven by `build-cart`:

```json
{ "drivePath": "D:\\", "title": "FTL: Faster Than Light",
  "executable": "steam://rungameid/212680", "appId": "212680",
  "formatDrive": true, "formatFilesystem": "ntfs", "formatLabel": "FTL",
  "formatConfirmation": "New Volume (D:)", "copyGame": true,
  "closeSteam": true, "writeIcon": true }
```

The first attempt, without `closeSteam`, refused before touching the drive:
*"Steam is running, and it rewrites its library list from memory when it exits,
so anything changed now would be undone."* Right call — the 2026-09-01 run
closed Steam without being asked. A script driving `build-cart` now has to say so.

| Result | |
|---|---|
| Format | NTFS, label `FTL` |
| Copied | 287,269,659 bytes, 13 files, into `SteamLibrary/steamapps/common` |
| Registered with Steam | yes — `D:\SteamLibrary` in `libraryfolders.vdf` |
| Verify during build | *"Checked all 13 files against what was written; every one matches."* |
| Whole build, format included | **10.7 s** |
| `verify-cart D:\` afterwards | intact, **619 MB/s**, same digest `fcce34e2…` |
| `UASPStor` / `disk` / `Ntfs` events in the window | **none** |
| Root | `cartridge.conf`, `.gamepak/`, `SteamLibrary/`, hidden `autorun.inf` |

Small and fast enough that it proves the pipeline, not the link. The failures of
2026-09-01 lived in sustained 100 GB writes; this was not one.

**Bug, not fixed: progress passes 100%.** The copy step printed `100.1%` and
`100.2%`, and the Steam registration step carried `100.2%` over. Whatever the
denominator is, it is smaller than what gets copied.

`libraryfolders.vdf` also still lists `H:\SteamLibrary`, a cartridge that is not
plugged in. Whether Steam or unregistering should clear that is its own
question; noted, not chased.

### Phase 5 — insert detection **PASS**, Play **not tested**, Eject **not tested**

The installed watcher (scheduled task, `%LOCALAPPDATA%\PC-GamePak`, pre-1.1.0)
was stopped and the 1.1.0 watcher run in its place with `PC_GAMEPAK_LAUNCHER`
pointing at the new launcher. A replug was simulated with `Disable-PnpDevice` /
`Enable-PnpDevice` on the enclosure's USB node — a real removal and arrival as
far as the storage stack is concerned.

| Check | Result |
|---|---|
| D: gone after disable | yes |
| D: back with `cartridge.conf` after enable | 5.9 s |
| Watcher log | `D: opened the launcher` |
| Launcher | 1.1.0, window `PC GamePak`, **27.4 MB** resident |
| Window | title, Play, eject; placeholder art because `build-cart` fetched no cover |

The installed watcher was put back afterwards.

**Play was not tested, deliberately.** The launcher window was open, but the
desktop was in active use at the time, and Windows' foreground lock meant a
synthetic click could not be relied on to reach the launcher rather than
whatever the user had in front of them. Injecting input into someone's live
session to click a button is not a test worth that risk, so it stopped there.
Play and Eject want one click each from a person.

Cosmetic: the placeholder monogram reads **FFT** for *FTL: Faster Than Light* —
the initials of `FTL:`, `Faster`, `Than`, taking the first letter of the
abbreviation as though it were a word.

### Bug: the Windows health readout cannot warn about anything

The launcher's health panel exists to catch the two failures this project has
actually hit — a USB 2.0 link and a BOT bridge. On Windows it can catch neither.

`health::probe` (`core/src/health.rs:300`):

- returns `mbps: None` unconditionally, so `link_mbps` is never set, and
- reads `Get-PhysicalDisk ... BusType`, which is `USB` for every enclosure, and
  `windows_transport` maps `USB` to an empty string.

So `advise()`, which is correct and tested, is only ever handed nothing on
Windows. The 2026-08-20 cartridge on the AMD USB 2.0 controller at 18 MB/s would
have shown a clean panel.

What the device tree does give, measured on this enclosure:

| Source | Value | Usable |
|---|---|---|
| `DEVPKEY_Device_Service` on the USB node | `UASPStor` (BOT would be `USBSTOR`) | **yes** — exactly the UASP/BOT answer |
| Parent walk to the host controller | `AMD USB 3.20 eXtensible Host Controller` | **yes, as a ceiling** — it names a USB 2.0 controller as one |
| `{3464F7A4-2444-40B1-980A-E0903CB6D912} 10` | 3 | **no** — also 3 on a Generic USB 2.0 hub and on a Bluetooth adapter. Looks like a speed enum, is not one |

The exact negotiated speed needs `IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX_V2`
on the parent hub. The service name and the controller walk are enough to catch
BOT and a USB 2.0 port, which are the cases that have cost data here. Not
implemented yet.

### Watcher findings, from its log

- **The tray icon fails when started by the scheduled task.** `could not add the
  tray icon; carrying on without it` on 2026-09-10 20:28 and 2026-09-11 19:35,
  both task starts. Started from a shell today it added the icon fine. Likely
  the task running before Explorer's notification area exists.
- **Timestamps are UTC and do not say so.** `18:13:41 D: opened the launcher`
  was 20:13:41 local. Anyone reading the log against the event log, which is
  local, is two hours out.

### `notify_only` does nothing on Windows

The launcher decides the insert reaction (`core/src/insert.rs`), and for
`Notify` on Windows `notify()` prints *"notify_only is not implemented on
Windows"* to a stderr nobody sees. The setting can be chosen and has no effect.
`copilot/phase-1-ecosystem-integration` has a tray balloon for exactly this in
`watcher/src/notifications.rs` — the watcher owns the tray icon and is resident,
which is what a Windows notification needs. Its settings and insert logic are
superseded by what merged; that file is not.

### Phase status

| Phase | Status |
|---|---|
| 0 — build and unit tests | **PASS** — 349 tests on Windows, after 4 Windows-only faults fixed |
| 1 — prepare the NVMe | **PASS** |
| 2 — wizard, non-destructive | drive list now USB-only; game list 86 incl. folders |
| 3 — format and copy | **PASS** — NTFS, 287 MB, verified, no bus events |
| 4 — cartridge contents | **PASS** |
| 5 — insert detection, Play, Eject | **PARTIAL** — insert **PASS**; Play and Eject need a hand click |
| 6 — rewrite with Steam running | refuses without `closeSteam`; with it, closes Steam and registers. Not yet a *rewrite* of an existing cartridge |
| 7–9 | Not started |

---

## 2026-09-12, later — the bugs from this morning, fixed

Every bug the entry above recorded, fixed and, where the hardware allowed it,
checked on it.

### The Windows health readout now measures the link

`health::probe` walks the device tree from the drive letter (PowerShell, once,
when the details are opened), takes the transport from the USB node's service,
and asks the hub the drive is plugged into for the negotiated speed with
`IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX_V2` and `_EX`. A USB 2.0 host
controller caps the answer at 480 Mbps when the hub will not say. A USB 3
controller does not: the port can still have fallen back.

`build-cart health D:\`, new, prints what the launcher would:

```json
{ "link": "10 Gbps", "linkMbps": 10000, "transport": "UASP",
  "label": "FTL (D:)", "filesystem": "NTFS", "warnings": [] }
```

About 2 s, nearly all of it PowerShell starting. C: and F: report `NVMe` and no
link, as they should.

To be sure the hub query reads speeds rather than printing one number, it was
pointed at every occupied port on the same root hub:

| Port | Device | Negotiated |
|---|---|---|
| 1 | Realtek RTL9210B-CG enclosure | **10 Gbps** |
| 5 | VIA hub, USB 3 half | 5 Gbps |
| 10 | VIA hub, USB 2.0 half | **480 Mbps** |
| 11 | Realtek Bluetooth | **12 Mbps** |

Four ports, four different answers, and the two that can only be one thing —
the USB 2.0 half of a hub, and a Bluetooth radio — are exactly that. Compare the
device property that looked like a speed this morning and read 3 on all of them.

SuperSpeedPlus is reported as 10 Gbps whether it is 10 or 20, because the hub
does not distinguish them. SuperSpeed of an unknown kind is reported as unknown,
not 5 Gbps, so a 10 Gbps link is never warned about as half speed.

### `notify_only` works on Windows

The launcher sends the notification text to the watcher's window with
`WM_COPYDATA`; the watcher posts it as a balloon from its tray icon and answers
whether it could. If it could not — no watcher, or no icon — the launcher opens
its window, which is what the settings dialog always said would happen, and not
what the code did.

| Check | Result |
|---|---|
| `onCartridgeInsert: notify_only`, 1.1.0 watcher running, `pc-gamepak --drive D:\` | watcher log `posted an insert notification`; launcher exited 0 with no window |
| Same, watcher stopped | window `PC GamePak` opened |

The settings file and the installed watcher were restored afterwards.

A simulated replug to drive the watcher end to end failed this time —
`Disable-PnpDevice` returned *Generic failure*, which is what it does while
something holds the volume — so the notification was triggered by starting the
launcher the way the watcher does. The watcher's own log shows the drive was
replugged by hand twice around then, so it may simply have been busy.

### The rest

| Bug | Fix | Checked |
|---|---|---|
| Copy progress reached 100.2% | Total is the measured tree, not `SizeOnDisk` (FTL: 286,586,003 in the manifest, 287,269,333 on disk). `build-cart` also caps at 100%, like the wizard | by the numbers; not re-copied |
| Tray icon missing all session after a logon start | `TaskbarCreated` is listened for from the start, not only after a first success, plus a 3 s retry for up to two minutes | compiles and starts cleanly; the logon race itself was not reproduced |
| Watcher log times were UTC without saying so | `2026-09-12T18:46:56Z`, the launcher log's format | yes, in the log |
| Placeholder read **FFT** for *FTL: Faster Than Light* | Initials come from the name before a subtitle, and a name that is already an abbreviation is kept | against real titles: FTL, GOW, HI, LHP, H2 |
| `core/Cargo.toml` said the toolchain is 1.87 | Says 1.88.0, and why the `image` pin is still exact | — |

Not a bug, after looking: `H:\SteamLibrary` staying in `libraryfolders.vdf`.
Libraries are only unregistered by an explicit action, on purpose, so a
cartridge that is simply unplugged is still a library when it comes back.

Tests: core **346**, watcher **7**; clippy clean on core, watcher and the Tauri
backend; fmt clean.

Still not done: Play and Eject by a person's hand.

---

## 2026-09-12/13 — Play started the PC's copy, not the cartridge's

Play and Eject worked by hand. Asked to confirm the game was really running from
the cartridge, it was not:

```
FTLGame.exe   F:\Games\Steam\steamapps\common\FTL Faster Than Light\FTLGame.exe
parent        steam.exe
DLLs          steam_wrapper.dll, BASS.dll, steam_api.dll — all from F:
```

Steam lists an app as installed in exactly one library. FTL was still installed
on F:, so `libraryfolders.vdf` had `212680` under `F:\Games\Steam` and the
cartridge's `D:\SteamLibrary` registered with `apps {}` — present, owning
nothing. The copy on D: was never going to be launched, however many times Play
was pressed; the earlier Play presses at 20:38 and 20:51 had also run from F:.

This is true of **every** Steam cartridge made from a game still installed on the
PC. Stardew on 2026-09-01 only looked right because Stardew is installed nowhere
else.

### Manual proof that the cartridge copy runs once the PC stops claiming it

1. Steam shut down with `-shutdown`; both `libraryfolders.vdf` backed up.
2. `F:\…\appmanifest_212680.acf` renamed aside. Game files on F: untouched.
3. Steam started. With no manifest on F:, it moved `212680` into
   `D:\SteamLibrary`'s `apps` by itself.
4. `steam://rungameid/212680`:

```
FTLGame.exe   D:\SteamLibrary\steamapps\common\FTL Faster Than Light\FTLGame.exe
DLLs          every one from D:\SteamLibrary\…
parent        steam.exe
```

Then put back: FTL closed, Steam shut down, the manifest renamed back. Steam was
then restarted by an installed `pc-gamepak --drive D:\ --play 0` (10:59:14), and
on that start it logged `Loaded 0 apps from install folder "D:\SteamLibrary\…"`
and rewrote its list with F: owning FTL again — the original state. Both copies
intact.

### Built: move instead of copy

`CartridgeRequest::move_game`, and **Move instead of copy** in the wizard rail
(Steam games only, per write, never saved, starts unticked). After the cartridge
has been read back and every file matched, the build deletes the PC's manifest,
moves the app's entry in both `libraryfolders.vdf` to the cartridge's library,
and deletes the install folder — in that order, so stopping anywhere leaves
Steam believing the truth. Refused outright without copy and verify; skipped
with a warning when the verify fails, Steam is running, the game is running, the
install is not laid out as `…/steamapps/common/<dir>`, or the cartridge does not
hold the copy.

`build-cart plan` with `"moveGame": true` prints, for this host:

```
move plan: delete F:\Games\Steam\steamapps\common\FTL Faster Than Light (287 MB) once the cartridge verifies
```

The library-list edit walks the file's structure rather than searching for
`"3"`, because in this host's real file the same quoted digits are values
(`"totalsize" "0"`, `"update_clean_bytes_tally"`) before they are entry keys.

**Not yet run for real.** It deletes a game from this PC, so it waits for a go on
which game. Tests: core 349 on a clean checkout of this change (3 new), clippy
and fmt clean.
