# Where the project is

A working inventory: what exists, what it does, and what is missing. Kept in the
repository rather than in a chat log so it stays honest.

## What is built

### `core/` — `gamepak-core`, 362 tests

No Tauri, no UI, no display. That is the point: every decision the launcher and
the wizard make is testable on any machine, in CI, without a webview.

| Module | What it does |
|---|---|
| `busy` | Who is still using the volume, before anything unmounts it. `/proc` on Linux — open files, memory-mapped files, working directories and executables, with the processes it was not allowed to read counted rather than hidden; Toolhelp plus `QueryFullProcessImageNameW` on Windows, which sees programs running from the drive. Also the stop sequence: ask, wait, then kill, because a game asked to quit writes its save to the cartridge first. |
| `cartridge` | Reads a cartridge: `cartridge.conf` (single game, or `[collection]` + `[game]` sections) and legacy `autorun.inf` for label and icon only. Inline INI parser, path confinement, cover inlined as a `data:` URI under an 8 MB cap. |
| `create` | The build pipeline: close Steam and drop its stale entry → format → copy → check the launch target → cover art → `cartridge.conf` → `autorun.inf` → trim and report. Game lists from Playnite and Steam, collection naming, per-game covers. |
| `edit` | Rewrites a cartridge's metadata — name, artwork, which games are listed and in what order — without copying or deleting anything. |
| `drives` | Which volumes may be written to — an allowlist of automount locations, never a denylist. Parses `/proc/mounts`; Win32 volume APIs on Windows. |
| `format` | NTFS, exFAT and btrfs, behind four gates: removable allowlist re-derived here, not the system drive, the current label typed back exactly, and explicitly asked for. |
| `home` | A carried game's whole home directory, on the cartridge. `portable_home=yes` and the launcher starts the game with `HOME` (and the XDG, or Windows, equivalents) pointed at `.gamepak/home/`, so every save it writes lands on the drive with nothing declared and nothing copied. The cache stays on the host. Kazeta's mechanism, minus the overlay it does not need. |
| `frontend` | The register of places a cartridge can open: the built-in launcher, the Decky row, a Playnite extension that is named but unwritten. Holds which are on, detects whether each plugin is actually installed, and keeps a front-end this build has never heard of rather than dropping it. The contract with a plugin is one boolean in `settings.json` — no socket, no daemon, nothing to version. |
| `insert` | What plugging a cartridge in should do — a window, the game, a notification, or nothing — and the rule that auto-launch will only ever start a URI the host already has a handler for, never a program carried on the drive. Lives in core so all three things that open a launcher get the same answer. |
| `health` | Negotiated link speed, UASP vs BOT, how full the drive is, and the volume's own name and filesystem. sysfs on Linux; on Windows, the device tree once, lazily, for the transport and the hub, and the hub's own answer for the negotiated speed. |
| `playnite` | Reads a Playnite JSON library export: one list covering Steam, GOG, Epic, Xbox, itch, emulators. Finds Playnite on Windows and through Proton prefixes on Linux. |
| `portable` | Ranks the executables in a copied game folder so Play points at the game rather than its uninstaller. |
| `settings` | What the user has switched on, stored beside the artwork cache. Everything defaults to off. |
| `saves` | Saves that travel with the cartridge. Reads `save=` lines whose paths are written as a `{token}` the host resolves — the three platforms disagree about where saves go — and reconciles the two copies against what *this machine* last saw of each, kept per host on the drive. Only the side that changed is copied; both changing is a conflict that writes nothing; whatever is about to be replaced is moved aside and kept. Symlink mode for people who want one copy rather than two. |
| `shaders` | The compiled shader cache, carried between machines, per cartridge (`shader_cache=drive`). Steam's own `steamapps/shadercache/<appid>`, newest side wins, and no conflict case because it is a cache — losing it costs compile time and never data. Libraries that live on the cartridge are excluded from the host side, or every sync would copy the drive onto itself. |
| `sgdb` | SteamGridDB artwork search, download and cache. Refuses every request until the user opts in and supplies a key. |
| `stats` | Launches, hours and last-played in `.gamepak/stats.json` on the drive, so the count follows the cartridge rather than the PC. The launch is written before the game starts and the open session re-stamps itself every sixty seconds, so a crash costs a minute rather than the session — and the machine that sees the cartridge next settles whatever the last one left open. No write here can fail a launch. |
| `steam` | Steam's own manifests: `libraryfolders.vdf`, `appmanifest_*.acf`, the library cache for covers. Hand-written KeyValues parser. |
| `steamlib` | Copies a Steam game onto a cartridge and registers the drive as a Steam library, so Steam plays from the cartridge. Also unregisters one, and asks a running Steam to shut down so those edits survive its exit. |
| `trim` | Tells the drive which blocks it no longer has to keep. Treats "this enclosure will not" as a fact, not a failure. |
| `tuning` | The Windows settings worth changing per cartridge, the commands they run, and their exact opposites. |
| `verify` | CRC-32, taken as each file is copied and checked by reading the cartridge back. Also a SHA-256 **cartridge digest** over the manifest — one value answering "is this the same cartridge somebody else built", which verifying against your own manifest cannot. Hand-written hash, checked against the published vectors and an independent implementation. **On by default** since it caught real corruption on the first cartridge ever checked on hardware. Leaves a manifest so the same check can be run later without the original; `verify-cart` is the command that does it. |
| `autorun` | Writes `autorun.inf` so Explorer shows the game's name and icon; builds a PNG-in-ICO when the cover allows it. |

### `tauri-ui/` — one binary, two windows

`pc-gamepak --drive <path>` is the popup; `pc-gamepak --create` is the wizard.
Exactly one window is ever built, so neither mode costs anything for the other.
53 commands, no command that takes a path to read.

The launcher counts what it starts and, if asked to, carries the saves. Both
are off the same principle: the cartridge is the thing that travels, so the
history and the save belong on it. Counting is on — it writes one file, to the
drive the user just pressed Play on. Syncing saves is **off** until switched on,
because it is the one thing here that writes to a directory in the user's home
on the say-so of a file on a drive.

**Launcher** — the artwork fills a 420 × 560 window, which is the slot the
cartridge is seated in: Eject rides the whole face out and leaves the slot
behind. Title, Play and an eject icon; everything else appears only under the
pointer. A collection grows a rail, and the one Play acts on whatever it has
selected, with `1`–`9` selecting and starting the *n*th. The accent colour is
sampled from the cover. A pad swaps the keycaps for face-button icons and gets a
focus ring that is always drawn. Details behind the ⓘ, leading with link and
free space and folding the paths away. Nothing on a cartridge runs without a
click.

**Wizard** — one screen: Games, Drive, Options, Artwork, Write. Selection is
always multiple: one ticked is a cartridge, more is a multicartridge, which adds
a name and a drag-sortable play order under the Games card. Options are this
cartridge's switches — copy, verify, eject, erase and to which filesystem —
starting from the defaults in Settings; erasing always starts off and is
confirmed by name. The rail previews the launcher and turns the choices into a
numbered plan with a time estimate; the write happens in the same window, as a
log that ticks itself off. Artwork by file picker or SteamGridDB, per-cartridge
Windows tuning.

### `watcher/` — both platforms, 15 tests

**Windows:** a hidden top-level window blocking on `WM_DEVICECHANGE`. No polling,
no timer, about 2 MB resident.

**Triggers:** every trigger submits a `gp_` ID with `pc-gamepak --trigger gp_id`;
the launcher resolves it in the host registry (`core/src/gamepak.rs`) and runs
the registered action. **NFC** is an optional example adapter: a PC/SC monitor
reads Type 2 and Type 4 NDEF cards in a worker thread and passes the bounded
NDEF message to the launcher, which extracts the ID and takes the same path. The wizard registers mounted GamePaks and writes their stable
IDs as NDEF URIs to blank Type 2 / NTAG tags. Writing a tag does not
launch a game. The watcher does not link `gamepak-core`.

**Linux:** blocks in `poll()` on `/proc/self/mountinfo`, which the kernel wakes on
any mount activity. Used only by the rootless install — the system install has
udev do this and keeps nothing resident. Deliberately does not link
`gamepak-core`: core pulls serde and ureq, which is fine for a launcher that runs
for ten seconds and not for a process that is resident all session.

### `linux/`, `windows/` — installers

udev rule plus two systemd template units on Linux; two binaries and a logon task
on Windows. Both installers uninstall cleanly, including names from before the
project was called PC GamePak.

### `core/src/bin/` — the wizard's job without the wizard

Two small commands over the same `gamepak-core`, for the cases a GUI is the
wrong shape. `build-cart plan|build <request.json>` builds a cartridge from a
JSON `CartridgeRequest`, so eleven cartridges are eleven files rather than an
evening of clicking; `plan` is the default because formatting is not undoable.
`verify-cart <root>` re-checks a cartridge against its manifest and touches
nothing.

### Everything else

CI on every push (core, watcher, launcher, frontend, shell), a release workflow
that builds both platforms from a tag, AUR and Scoop packaging, and
`docs/PUBLISHING.md` for what each channel can and cannot install.

## What is missing

Ranked by how much it matters.

1. **The AUR, and the WinGet token.** 1.1.0 is out on GitHub, WinGet and Scoop.
   The AUR package is written and checksummed for it and waits on AUR
   registrations reopening. Publishing a release is meant to submit to WinGet
   by itself through `winget.yml`; for 1.1.0 that failed because the
   `WINGET_TOKEN` secret could not push to the winget-pkgs fork, and the
   submission was made by hand. Until the secret holds a classic token with
   `public_repo`, the next release will need the same. `tools/check-versions.mjs`
   keeps every version site in step and fails CI when they disagree.

2. **Real hardware: answered, and the history is worth keeping.** The project
   owner reports repeated end-to-end runs since, with the **God of War
   Ragnarök** and **Tomb Raider** cartridges both working — so the open question
   here is no longer whether this writes a usable cartridge. What follows is the
   record of how it got there, because it is the reason `verify` is on by default
   and the reason a user reporting corruption should be asked about their cable
   before their drive.

   The two earliest cartridges:

   | Cartridge | Written | Verified |
   |---|---|---|
   | `TOMB RAIDER` — 10 games, 107.43 GB | 2026-08-31 | **2 files corrupt** |
   | `PLAYSTATION` — 3 games | 2026-09-01 | no manifest — cannot be checked |

   The Tomb Raider cartridge was read back on 2026-09-01 at 420 MB/s and two
   2 GB `.tiger` archives came back the right length with the wrong CRC-32.
   Repeated reads return the same bytes, so the corruption is on the drive, not
   in the read — the bytes never landed. Windows logged 19 `UASPStor` bus resets
   during that write and five more during the verify, on a Realtek RTL9210B-CG
   enclosure. `std::fs::copy` would have reported success for every byte of it.

   That is the case for `verify` existing, and it is why it is now on by
   default.

   At the time this was written, one path was proven good and one was still
   corrupting data. Both have since been run through repeatedly and work.

   On the AMD chipset port, `PLAYSTATION` was rewritten as a single-game Stardew
   Valley cartridge — format to exFAT, copy, register with Steam, verify — and
   the run logged no bus resets and no I/O retries at all. 3833 files, all
   matching; `verify-cart` agreed afterwards; the launcher opens on it. First
   clean end-to-end write this project has had. It also confirmed the new
   default: the request that built it never mentioned verifying and got it
   anyway.

   The Tomb Raider enclosure took longer to pin down. Its two corrupt archives
   were replaced from source and both verified — but the same pass found a third
   file corrupt that had been intact three hours earlier, with a `UASPStor` reset
   logged inside the window where that copy ran. Writing 4 GB had destroyed 2 GB
   of a file nothing was writing to.

   **That was the port, not the enclosure.** On a different port the same
   enclosure behaves. Both faults on this machine turned out to be physical and
   neither was the drive — which is worth remembering when a user reports that
   this tool corrupted their cartridge.

   The cost is on the record: `bigfile.005.tiger` on that cartridge is still
   corrupt, because repairing it meant another write through the port that broke
   it. A bad link does not merely fail a copy, it can damage a game already
   written and already verified.

   What is still not written down here is the *detail* of those later runs — the
   throughputs, whether the running panel behaved across a hundred gigabytes,
   which enclosure and port each used. `docs/HARDWARE-REPORT.md` is where that
   belongs, and it stops at the early runs. Somebody's word that it works is not
   the same artefact as a log, and only one of the two survives being forgotten.

   Related, and now fixed: until PR #10 nothing on Windows compiled at all —
   `gamepak-core` had no `windows-sys` dependency despite calling the Win32
   volume API, and two more crates were missing feature flags. CI ran core on
   Linux only, so the failure surfaced in the launcher job and looked like a
   launcher problem. Core is checked on both operating systems now.
3. **Windows code signing.** Unsigned means SmartScreen on every download. Costs
   money rather than work, which is why it sits here rather than in a commit.
4. **`portable_home` has never met a real game.** The mechanism is proven —
   a script writing to `$XDG_DATA_HOME` lands its save on the cartridge — and no
   actual game has been run under it. Two things to expect: a game that keeps
   per-machine settings will start at its defaults the first time on each host,
   and on exFAT any game that creates a symlink inside its config directory will
   fail to, the filesystem having none.
5. **`auto_launch_game` counts launches but not hours.** It only ever starts a
   `steam://`-class URI, which is somebody else's launcher's child and not ours,
   so there is nothing to wait on and no process left to re-stamp the heartbeat.
   The session is closed at once rather than abandoned, so the cartridge is not
   left carrying a record that never advances. Fixing it means watching a process
   this launcher did not start, which is what `busy` now knows how to find.

   A game the cartridge *carries* is a different case and is now measured
   properly: the launcher started it, so it waits for it, and the session ends
   when the game does rather than when the window closes.
6. **`notify_only` on Windows has not been seen working on Windows.** It used
   to do nothing there at all — not the notification, and not the window it was
   documented as falling back to. It now posts a notification-area balloon from
   a message-only window it creates for the purpose, which is the mechanism that
   works from a process with no window and no registered identity; a real toast
   needs an `AppUserModelID` against an installed shortcut, and this project
   installs by unzipping into `%LOCALAPPDATA%`. The Linux path is tested against
   a fake `notify-send`. The Windows path compiles and is unverified on
   hardware — it is about a hundred lines of Win32 that no test here can reach.
   A notification that cannot be posted now opens the window, which is what the
   old comment claimed and the old code did not do.
7. **The unmount guard has never met a real running game.** It is tested
   against child processes this repository spawns — one holding an open file,
   one ignoring the polite signal and needing to be killed — and against this
   process finding its own mapped binary. What it has not seen is Steam holding
   a cartridge, which is the case the manual already describes as sometimes not
   letting go until the drive is replugged. Windows sees executables only; open
   handles there need the Restart Manager, which is not written.
8. **Adding a game to an existing cartridge** still means writing it again.
   Editing covers everything that does not move files; adding one does.
9. **Verifying a cartridge you already have — half done.** `verify-cart <root>`
   reads `.gamepak/manifest.json`, re-reads every file it names and reports what
   does not match; it is read-only and exits non-zero when a cartridge is bad.
   That is the command. It still needs a button: nothing in the launcher or the
   wizard offers to check a cartridge that is sitting in front of you, which is
   where someone would actually look for it.
10. **macOS** is not supported at all — no watcher, no installer, no icons. The
   save-path tokens resolve for it (`~/Library/Application Support`,
   `~/Library/Preferences`), which is the only part of the platform that has
   been written.
11. **The `gamepak-linux.sh` / `gamepak-windows.ps1` menu wrappers.** The README
   pointed at both as the way to install, and neither has ever been in the
   repository — `linux/install.sh`, `linux/install-user.sh` and
   `windows/install.ps1` are the real entry points and the docs now say so. CI's
   `shell scripts` job still globs `./*.sh` expecting them, which is why that
   job is red on `main`: either write the wrappers, or narrow the glob to
   `linux/*.sh`.
12. **Saves and hours have never met a second machine.** The round trip is
   tested — play on A, carry to B, play, carry back — but in one process with
   two scratch directories standing in for two PCs, which is not the same as a
   Deck and a desktop disagreeing about a clock. The conflict path is the one
   to watch: it is meant to refuse, and a refusal nobody notices is a feature
   that quietly does nothing.
13. **The settings the design asks for that no command answers.** Per-source
   toggles with game counts, the artwork cache's size and an Empty button, a
   copy-speed default, and the launcher-on-the-cartridge options are all drawn
   in the design and absent here. The dialog is grouped the way the design asks
   and reports what was actually scanned instead of offering switches that would
   do nothing.

## Stretch goals

Not on the list above because nothing is missing without them; worth doing
when the rest is settled.

- **A 3D cartridge for every platform.** `platform=` says what a cartridge is
  for so that something can draw the physical thing: a model per platform — a
  PC game box, a SNES cartridge, a Game Boy cartridge and so on — with the
  cartridge's own front, side and back artwork on it. It would serve the
  launcher, the unboxing and the memory card's shelf, which already turns CSS
  boxes made from a cover. The models and their artwork slots are ours to make:
  LaunchBox's 3D box and cart pictures come from its own database, for
  LaunchBox, and are not ours to ship. Means a back and spine image per
  cartridge as well as the four it carries now, and a way for the wizard to
  find or make them.

## What Kazeta settles, and what it does not

[Kazeta](https://github.com/kazetaos/kazeta) and
[kazeta-creator](https://github.com/kazetaos/kazeta-creator) (both MIT) are the
closest prior art there is, and reading them changes what is worth building
here. Recorded now rather than rediscovered later; the manual's *Thanks* section
credits them properly.

Kazeta is an operating system, not a program: an Arch image that boots greetd
into gamescope with no desktop, finds a `*.kzi` within two levels of `/media` or
`/run/media`, and shows a BIOS screen when there is no cart. That single fact —
**it owns the session** — is what lets it do three things this project cannot,
and the honest reading of each is different.

| | Kazeta | Here |
|---|---|---|
| Saves | The cart is an overlayfs lowerdir, a host directory the upperdir, the result is the game's `$HOME`. No save paths are known or needed. | `save=` lines declaring where each game keeps its saves, resolved per platform |
| Where saves live | On the host, moved to external "memory cards" as `<cart-id>.tar` | On the cartridge |
| Playtime | ISO-8601 start/end pairs in `.kazeta/var/playtime.log`, re-stamped every 60s | Launch count and seconds in `.gamepak/stats.json` |
| A cart | One immutable erofs image (`.kzp`) with a hash, or a directory | A directory with a text file in it |
| Runtimes | Proton as a `.kzr` image mounted *under* the game | Steam's own Proton, on the host |
| Reach | One OS, which you install instead of yours | Windows and Linux you already run |

**1. The overlay is better than declared save paths, where it is available —
and now half of it is taken.** It cannot miss a path, needs no per-game
knowledge, and captures a game that writes somewhere nobody documented.

Reading Kazeta closely is what made that adoptable, because the overlayfs is not
where its save capture comes from. The overlay is there so a *read-only* cart can
be written to at all; the capture is one line, `export HOME=…`, pointing the game
at the writable layer. A PC GamePak cartridge is already writable, so the same
result needs no mount, no root, no user namespace and no dependency — just the
environment the launcher hands the child process.

That is `home`, and for a carried game it is strictly better than a `save=` line:
nothing to research, nothing to declare, nothing copied, and no conflict possible
because there is only ever one copy. Demonstrated with a game that knows nothing
about any of this — a shell script writing to `$XDG_DATA_HOME/Tunic` — landing its
save on the cartridge with no `save=` line anywhere.

What stays out of reach is the other half: a game the cartridge only *points at*.
A `steam://` cartridge is started by Steam, in Steam's environment, and nothing
here can set it. Kazeta does not have that problem because it starts everything
itself, being the operating system. So the token vocabulary stays, for exactly
the cartridges that need it.

**2. Proton as a mounted image is the answer to the exFAT symlink problem.**
This project's own finding — Steam unpacking Proton onto a cartridge dies on the
first of 1892 symlinks, because exFAT has none — is a problem Kazeta does not
have, because a runtime never gets unpacked anywhere. Worth remembering before
building anything clever about shader caches or per-game Proton pinning.

**3. `kazeta-creator`'s recipe model is the best idea in either repository.**
`contentdb.yaml` holds recipes, not games: where to get the files, how to
unpack, what to run, which runtime, and the xxh3 hash the finished cart must
match. The community shares recipes, everyone builds a byte-identical cart, and
the same file is a compatibility list as a side effect.

**Half of this is now taken.** `build-cart` and `verify-cart` print a SHA-256
digest over the manifest, and a request can declare `expectDigest` and fail the
build when it does not match — so `verify-cart` now answers across machines, not
only against the manifest one write produced. What is *not* taken is the part
that makes Kazeta's version work: a shared file of recipes anybody can build
from. That is a community, not a feature, and this project has no claim on
having one.

**4. The playtime heartbeat, also taken.** Kazeta re-stamps `playtime_end` every
sixty seconds while a game runs, so a crash costs a minute. `stats` did not, and
"the launch survives, the hours do not" was written down here as honest
undercounting — which it was, and a minute is more honest for one small write a
minute. An open session now lives in `stats.json`, and whichever machine sees
the cartridge next settles whatever the last one abandoned.

**What none of this changes.** Kazeta replaces the operating system, so it cannot
be the thing you plug a cartridge into on a work laptop, a Windows gaming PC, or
a Mac — which is the whole premise here, and is not a premise Kazeta is competing
for. The formats are close enough to read each other (`.kzi` is `Key=value`
with `Name`, `Id`, `Exec`, `Icon`, `Runtime`, `GamescopeOptions`), so a
`.kzi` arm in `cartridge` would let a Kazeta cart open in this launcher, and a
`.kzi` written beside `cartridge.conf` would let one drive do both. Neither is
built and neither is decided.

## What the Moonlight forks got right

[StreamLight](https://github.com/FoggyBytes/StreamLight) and
[ArtMoon](https://github.com/onaiaku/ArtMoon) (both GPL-3.0, so ideas only —
nothing can be copied into an MIT project) are gamepad-first Moonlight forks.
They solve a different problem, and they have thought harder than this has about
what a game menu feels like from a sofa.

**Taken: prompts follow the device in your hands.** `is-gamepad` went on when a
pad *connected* and came off only when the last one disconnected, so a desktop
with a controller attached — or a Deck in desktop mode with a keyboard — showed
face-button icons to somebody typing. It now goes on at the first button press
and comes off at the next keystroke. Small, and it was wrong before.

**Not taken, and deliberately: a prompt bar, and brand-specific glyphs.** Their
bottom bar names what each button does on the current screen; this launcher has
four actions on four face buttons and draws each prompt on the button it belongs
to, so there is no bar to put anything in. They detect the pad's make and show
Xbox, PlayStation or Nintendo lettering; this shows the action's own icon
instead, which reads correctly on all three and cannot be detected wrongly. Both
of those are defensible the way they are.

**Where their model would actually help: the wizard.** It has ten settings tabs,
several dialogs, dropdowns and text fields, and no gamepad support at all —
which is fine on a desk and the whole problem on a Deck. `NavigableDialog` and
`NavigableItemDelegate` in ArtMoon are the shape of the answer: pad reachability
as a property of the widget, not something retrofitted per dialog. Not written
here, and the largest piece of Phase 5's ten-foot UI that is still missing.

## The rootless Linux install

Built. `linux/install-user.sh` puts everything under `$HOME` and runs the watcher
as a systemd user service; `linux/uninstall-user.sh` takes it back out. You pick
between this and the system install by which script you run — the `gamepak-*`
menu wrappers the README used to point at were never written.

| Install | Trigger | Resident | Needs root |
|---|---|---|---|
| **System** (AUR, `.deb`, `install.sh`) | udev rule | nothing | yes, once |
| **Rootless** (`install-user.sh`, and what a Flatpak would use) | mount-table watcher, systemd user service | one process, ~2 MB | no |

Both run the same launcher and the same detection rules; only the trigger
differs. The rootless one is arguably more accurate: it wakes when the cartridge
is mounted and readable, where udev fires when the kernel first sees the
partition and its helper then polls `findmnt` for up to sixty seconds waiting for
the desktop to catch up.

Verified on Linux with a loop-mounted image: insert opens the launcher, eject
closes that launcher and leaves any other cartridge's window alone, and a
re-insert after a genuine eject opens a new one rather than being debounced away.
Not yet verified on real removable hardware, or inside a Flatpak sandbox.
