# Changelog

What changed in each release, in the words someone deciding whether to upgrade
would want. The commit log has the detail; this has the consequences.

Versions follow [semantic versioning](https://semver.org/): the minor number
moves when a cartridge or a setting gains something, the patch number when
nothing about either changes.

Every version in the repository is checked to agree —
`node tools/check-versions.mjs` — because for three releases they did not.

## 1.2.0 — 2026-10-21

### Icons you can read in the taskbar

The app and the wizard have new icons, drawn for their size: the six colour
bars of the PC-GAMEPAK mark on a navy tile for the app, and a cartridge with
the bars on its label for the wizard. The old ones were the logo pictures shrunk, and
at taskbar and notification-area sizes their lettering and thin bands turned
to a smudge and the deep red all but disappeared on a dark taskbar. There are
no letters now, and 24 px and smaller are drawn bolder, not shrunk.

### The memory card, as a memory card

The memory card view is now a stage: the card itself hovers in the middle, a
slab of smoked plastic with its grip, label and connector, and every save on it
stands as a game case on a track round it, the chosen one at the front, stepped
forward and lit. Left and right turn the track (as do the d-pad, a scroll
wheel and the arrows either side); a click brings a case to the front. Opening
it slots the card in and fans the saves out of it. Under the stage, a panel
gives the save's state, when it was saved, how long it has been played and
what it takes on each side, with the button that state calls for. Copying,
deleting (still two presses) and opening the folder are as before.

It is CSS 3D, drawn only while something moves, with no new dependency.
Reduced motion puts everything straight in place, and where the webview cannot
really draw 3D (WebKitGTK without a GPU) the saves are laid out as a flat strip.

### The Iced launcher is gone

`iced-ui/` and `docs/ICED.md` are removed. The experiment answered its
question: Iced started faster but was not much lighter as built, and could not
draw skins or the launcher's animations without a skin format of its own.
Tauri is the one launcher.

### GamePak actions: any card or barcode, any game, Play to start

The wizard's **GamePak actions** tab lists every GamePak with what it starts
and the cards and barcodes that start it. **New GamePak** listens for a tap or
a scan as it opens; pick a game from the ones installed here, a Moonlight
stream (the host's games are listed), a file, a program or a page, and save.
**Add card** puts more cards on the same game. The NFC cards tab is gone.

A trigger now opens the launcher at READY for every GamePak, waking its host
if it has one, and **Play** starts it. Settings → **Launch directly on
trigger** starts it on the tap instead.

Barcode scanners work as triggers on Linux: the watcher takes the scanner for
itself and submits each code, `gp_` added where missing. A PN532 on a serial
adapter is read through libnfc when `/etc/nfc/libnfc.conf` names it, and a
card may carry its ID as a Text record (`gp_stardew`), as a phone writes it,
on NTAG or MIFARE Classic.

### Start on tap: the game starts on the gaming PC, the stream on Play

Tap a GamePak card at the couch and the game starts on the gaming PC at once,
with no stream; the couch launcher opens at READY with the game's box art. Play
runs `moonlight stream`, which resumes the running game, and Eject runs
`moonlight quit`. The gaming PC needs nothing but Apollo or Sunshine with the
game in its app list: the couch PC starts the app with Moonlight's own
pairing, and wakes the gaming PC with the MAC address Moonlight stored. Set it
up under **GamePak actions → New GamePak → Game on another PC**, which offers
Moonlight's hosts and their games. See the manual's *Start on tap*.

### Select GamePaks with NFC

A PC/SC-compatible PN532 reader can read an NDEF GamePak URI and select its host
registry entry. Scanning opens the launcher in READY state without starting the
game; press Play to use the existing launch path. Optional registry host settings
send Wake-on-LAN and wait for readiness before READY.

### Emulated cartridges carry their ROM

**Choose ROM…** under *Add a game by hand* makes a cartridge for an emulated
game. The platform is set from the file type where that is unambiguous, the ROM
is copied into the system's folder (`snes/`, `gba/`, `psx/`…), and Play opens it
with the emulator your desktop uses for that file type. The cartridge also
carries a `gamelist.xml` and a `metadata.pegasus.txt`, so Pegasus and
ES-DE — on a PC, or on an Android phone over USB-C — list it with its art when
pointed at the drive.

### Fixes

- **No Explorer window for a cartridge.** Windows' AutoPlay used to open a
  cartridge as a folder beside the launcher, and the watcher closed it a second
  later. It now answers AutoPlay's own "should I?" question, through the
  documented `IQueryCancelAutoPlay`, so the window never opens — for cartridges
  and memory cards only. Every other drive behaves as AutoPlay is set to.
- **Eject after formatting.** The wizard ejects and records the drive where it
  was remounted after a format, not where it was before.
- **Readable dropdowns on a light desktop theme.** They used to draw light text
  on the system's light menu.
- **The launcher opens on Linux after a desktop restart.** The watcher skips
  display variables left over from a session that has gone, instead of handing
  them to the launcher, which then could not open a window.

## 1.1.0 — 2026-10-03

The release where the cartridge stopped being only a way to carry a game and
started carrying everything around it: the saves, the hours, the compiled
shaders. And where the launcher stopped being the only place a cartridge can
open.

### The filesystem default changed to NTFS

exFAT held that place from the start and is the one filesystem that cannot hold
what a cartridge carries. No symlinks, so Steam cannot unpack Proton onto a
cartridge — it dies on the first of 1,892 of them, reporting `AppError_11`,
"Disk write error", which says nothing about why. No executable bit, so a
carried Linux game has to be a shell script. No permissions that survive a
replug.

**Existing cartridges are unaffected**: this changes what the wizard offers to
format a *new* drive as. exFAT is still one dropdown away — and so are five more
now: ext4, XFS, F2FS, HFS+ and APFS, for a cartridge that only ever meets one
kind of machine. The picker says what each costs at the point of choosing: which
desktops read it with nothing installed, whether Proton runs from it, its label
limit, and what would have to be installed first. One list, read from the code
that does the formatting, rather than a copy in the window that had already
drifted. A format this machine has no `mkfs` for is shown anyway, greyed, with
the package to install — so "why is btrfs missing?" has an answer on screen.

**If a Mac has to write to the drive, pick exFAT.** macOS reads NTFS and does
not write it, so an NTFS cartridge can be played from and copied off on a Mac
and cannot take a save back.

A side effect: cartridges are now named `God of War Ragnarok` rather than
truncated to eleven characters, because the volume label limit follows the
filesystem.

### Saves travel with the cartridge

Two mechanisms, and which one applies depends on whether the cartridge carries
the game or points at it.

- **`save=` lines** name where a game keeps its saves, as `{appdata}/Foo/Saves`
  — a role the host resolves, because the three platforms disagree about where
  saves go. Insert and eject copy whichever side changed. If both changed,
  **neither is touched** and the launcher says so; anything replaced is kept
  beside it, three deep.
- **`portable_home=yes`** gives a game the cartridge *carries* its whole home
  directory on the drive. Every save it writes lands there with nothing
  declared, nothing copied, and no conflict possible — there is only ever one
  copy.

A carried game's save is pushed to the cartridge **as soon as the game exits**,
not only on eject. Quitting the game and pulling the drive out is what somebody
who has finished playing does, and it used to cost them the session.

Off until switched on, in Settings. It is the one thing here that writes to a
directory in your home on the say-so of a file on a drive.

### The cartridge counts its own hours

Launches, playtime and last-played go in `.gamepak/stats.json` on the drive, so
the count follows the cartridge between machines rather than staying on the PC
that happened to play it. The open session re-stamps itself once a minute, so a
crash or a pulled drive costs a minute rather than the session — and whichever
machine sees the cartridge next settles whatever the last one abandoned.

For a game the cartridge carries, the launcher started the process and so waits
for it: the session ends when the game exits, not when the window is closed
afterwards. A `steam://` game is somebody else's launcher's child and there is
nothing to wait on, so the window's lifetime is still the bound there.

On by default: one small file, written to the drive you just pressed Play on.

### Compiled shaders, if the cartridge asks

`shader_cache=drive` carries Steam's per-game shader cache, which is what stops a
second machine stuttering through the first hour of a game the cartridge has
already played.

Per cartridge rather than a global setting, because it is a trade against the
drive's own speed: free on a fast NVMe cartridge, and a wait on a cheap stick.
Only whoever made the cartridge knows which it is.

### Where a cartridge opens is now a choice

The launcher is one front-end, not the only one. It ships with the project and is
on by default; everything else is a plugin, off until switched on. More than one
may be on at once.

- **PC GamePak launcher** — the cartridge's own window.
- **Steam Deck row** — cartridge games on the Steam home screen, through the
  [Decky plugin](https://github.com/HarryBMa/pc-gamepak-decky). Switch the
  launcher off on a Deck and no window appears over the top of it.
- **Playnite library** — named, and not built yet.

The whole contract between the launcher and a plugin is one boolean in
`settings.json`. No socket, no daemon, nothing to keep in step.

### And what the launcher does on insert

A new setting with four answers: open the window (the default, and what it has
always done), start the game, post a notification, or do nothing.

**Starting the game only ever starts a game your PC already has** — a
`steam://`-class URI. A program carried *on* the cartridge still waits for a
click, and so does a collection, which has no single game to mean. Nothing on a
cartridge runs without a click, and a setting left switched on is not consent to
run a stranger's binary.

`notify_only` uses the desktop's own notification on Linux and a
notification-area balloon on Windows. If neither can be posted, the window opens
rather than the insert passing in silence.

### Eject now says what is in the way

Before it takes the volume away, it asks the system who is using it and names
them: *"TombRaider.exe (4812) is running from the cartridge"*. Then two choices —
**Force quit and eject**, which asks each process to quit and waits five seconds
before killing anything, because a game asked to quit writes its save first; or
leave the drive mounted. If something survives being killed, the cartridge stays
mounted and says so.

On Linux this sees open files, memory-mapped files, working directories and
programs running from the drive, and counts the processes it was not allowed to
look at rather than implying the drive is idle. On Windows it sees programs
running from the volume.

### A cartridge can be checked against somebody else's

`build-cart` and `verify-cart` print a **digest**: one SHA-256 over every copied
file's path, length and checksum. Verifying answers "did these bytes survive";
the digest answers "is this the same cartridge somebody else built", which no
amount of local verifying can. Put it in a `build-cart` request as
`expectDigest` and the build fails if it does not match.

### Playtime counts the game, not the window, and lives in the conf

The launcher used to count from Play until its window closed. It now watches
for the game itself — a program running from the game's folder, on the
cartridge or wherever Steam installed it — and keeps counting after the window
is closed, leaving when the game does. The way GameplayTimeTracker and
GamingGaiden count.

It pauses when nobody is there: ten minutes (adjustable, or off) without
keyboard, mouse or controller input, and the idle stretch is taken back out.
Controllers count, which the desktop's own idle clock does not.

Every session is kept, and each game's figures — playtime, launches, first and
last played, the machine, and the last thirty sessions — are written into
`cartridge.conf` under that game, touching nothing else in the file.

**How long to beat**, optional and off: the wizard can look each game up on
HowLongToBeat while it writes the cartridge, once, and puts the figures in the
conf beside the playtime. The launcher's ⓘ shows both, and a skin can draw
progress through the main story from them — see
[SKINNING.md](docs/SKINNING.md#play-stats). HowLongToBeat has no official API,
so this may stop finding anything when the site changes; that is a warning,
never a failed write.

### The wizard asks less and gets more right

One screen, in the order the questions come: **Games**, **Drive**, **Options**,
**Artwork**. The choices that belong to one cartridge — copy the game, verify
the copy, eject when done, erase the drive first and to which filesystem — are
switches on that screen now, starting from the defaults in Settings and
forgotten after the write. They used to exist only in Settings, which made
"erase the drive" a preference that stayed on for every cartridge after it.
Erasing always starts off, and Write asks once more, naming the drive.

Settings lost most of its prose and one switch that did nothing ("Add the
cartridge to Steam's library list" — a copied Steam game is always registered,
or Steam could not play it).

Fixed along the way:

- Ticking a second game threw an error, so a collection could not be made.
- A Hero picture chosen on Create was never written to the cartridge.
- Saving Settings switched every front-end back to its default.
- After a write finished, the button still said Write and would write again. It
  now says **Make another**.
- A game added by hand could not be given a drive: leaving that screen threw
  the entry away. It now has **Done**, and picks the likeliest program in the
  folder for you.
- Artwork picked on SteamGridDB could land in the wrong slot. Each pick was
  filed under whichever tab was open when its download *finished*, so choosing
  quickly across Cover, Hero, Logo and Icon put the cover in the logo slot and
  so on. A pick now belongs to the tab it was clicked on, and Write waits for
  downloads still in flight.
- Picking a logo, hero or icon no longer becomes the game's remembered cover.
- The Edit tab and the drive lists name a cartridge by what its
  `cartridge.conf` says, not by the drive's volume label — which only changes
  when a drive is erased, so a drive rewritten from FTL to Cult of the Lamb
  still called itself FTL. Edit also re-reads a cartridge after it has been
  written over.
- The Windows tuning buttons in Settings name the drive they will change, and
  fall back to the cartridge open on Edit, or the only drive plugged in, rather
  than refusing until a drive was chosen on another tab.
- The artwork picker's preview shows all four pictures at once, each in its own
  shape — the hero as a banner, the cover with the logo over it, and the drive
  icon — with the one being chosen outlined. The Hero tab used to put the hero
  where the cover goes, which read as the hero replacing the cover.
- Ctrl+Enter on the Edit tab started a Create write.
- The collection name and the Edit tab's name were plain white boxes.

### Memory cards, and combo drives

A drive with a `memorycard.conf` carries **saves and no games** — the other half
of a cartridge. Plug one in and every save on it is a game case on a shelf;
choose one and it turns round to its back cover, which says which copy is newer,
when it was saved and how much it holds, beside the one button that does what
it needs: copy to this PC, or to the card. Whatever a copy replaces is kept.

A cartridge can be a **combo drive** too — a switch in the wizard — and its own
games' saves go on its card, one button (or `M`) away from Play. The wizard's
new **Memory card** page writes one: add games, and it finds where each saves.
Writing the same game twice replaces it rather than listing it twice.

### Finding where a game saves, and saves that are not folders

With *Look up where games keep their saves* on, the wizard fills save lines in
from [Ludusavi's manifest](https://github.com/mtkennerly/ludusavi-manifest),
matching by Steam app id (including other editions'), by name with accents and
punctuation ignored, and by install folder, and following the manifest's
aliases. A game it does not list is looked for by folder name in the places
games save. On one real library that took the hit rate from 58 to 65 of 86;
the rest save inside their install folder or in their launcher's cloud.

- **`{steamuserdata}`**: Steam Cloud games' saves under
  `userdata/<account>/<app id>`.
- **`{registry}`**: games that save into the Windows registry — Unity's
  `PlayerPrefs` does, so plenty do, Bluey among them. The key travels as an
  exported `.reg` file and is imported back only after every key in it is
  checked to be inside the one declared.
- **Link mode on Windows** falls back to a directory junction when a symlink
  is refused, so it no longer needs Developer Mode or administrator.

### `platform=`

A cartridge, a collection or one game can say what it is for — `SNES`, `GBA`,
`PS1`, about forty names listed in `cartridge.conf.example` — so a front-end
that draws the physical cartridge can pick the right shell. The wizard has a
picker for it on Create and Edit; skins see it as `data-platform`; the Playnite
plugin files the cartridge under Playnite's own platform of that name.

### For front-ends

`--play <n>` and `--safe-eject` run a game or an eject with no window, for a
front-end with its own buttons, and `--memcard` opens a combo cartridge on its
saves. A game running from the cartridge is now found whatever case its path
is in — Windows reports the case on disk, not the case the drive was named in,
and the busy check and `--play` both missed it.

### A new logo, and sharp icons

The striped badge is the app, the red cartridge is the wizard, cut from the new
logo with the upscale's edges cleaned. Each window now sets its taskbar icon
itself at the screen's DPI: Tauri sets only the small one, so the taskbar had
been stretching a 16 px picture.

### An experiment: a native launcher

`iced-ui/` is a second launcher written in [Iced](https://iced.rs), on the same
core, to find out whether a native window is the better long-term answer. It is
not shipped. Launch and eject moved into core to make it possible, which made
the Tauri backend 550 lines shorter. `docs/ICED.md` has the comparison.

### Removed: NFC and tag support

About 1,200 lines of PC/SC reader and line-source code, gone.
[Zaparoo](https://zaparoo.org/) does tokens across nine platforms and does them
better, and a thinner version living inside a cartridge launcher was never going
to catch up. **A drive is the only doorbell now.** If you were using a tag to
launch a cartridge, use Zaparoo.

### Also

- The controller prompts follow the device in your hands rather than the device
  plugged in: pad icons appear on the first button press and the keycaps come
  back on the next keystroke. A controller left on the table no longer decides
  what somebody typing is shown.
- Flathub's actual requirements are documented and the manifest meets them.
- `tools/check-versions.mjs` checks that every one of the fourteen places the
  version is written agrees, and can move them all at once. It also checks that
  this file has a section for the version being released and that its date
  matches the one appstream publishes — two dates for one release, and nothing
  compared them.
- **The Linux release tarball installs.** It shipped without the icon file
  `install.sh` listed as required, so unpacking a release and running the
  installer stopped on its first check — and `install.sh` looked for the
  launcher only where a source build leaves it, so a tarball carrying the
  binary reported it as "not built yet". Both fixed; the icon is in the tarball,
  and packaging now fails if anything the installers read is missing from it.

## 1.0.1 — 2026-09-06

Packaging only. The Flatpak build needed a tag that contained its own packaging,
and autostart moved to the Background portal instead of a systemd user unit,
because a Flatpak cannot enable one.

## 1.0.0 — 2026-09-05

The first release that built on both platforms. Until shortly before it, nothing
on Windows compiled at all: `gamepak-core` called the Win32 volume API without
depending on `windows-sys`, and CI only ran core on Linux, so the failure
surfaced three jobs later and looked like a launcher problem. Core is checked on
both operating systems now.

## 0.1.0 — 2026-09-01

The first tag, cut while the enclosure that corrupted two files was still being
diagnosed. What that diagnosis found is why `verify` is on by default: a bad USB
link does not merely fail a copy, it can damage a game already written and
already verified — and both faults on that machine turned out to be physical,
and neither was the drive.
