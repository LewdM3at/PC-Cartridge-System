# Getting cartridges into other frontends

A design note, not a plan of record. Nothing here is built yet.

## What exists

One thing: `steamlib::register_library()` writes the cartridge into Steam's
`libraryfolders.vdf`. Steam then treats the drive as a library, so Steam games
copied onto a cartridge appear as installed when it is plugged in and
uninstalled when it is not.

That is genuinely useful and it is why Deck Shelves can *nearly* do a cartridge
shelf today: a shelf backed by a collection plus the `installed` filter fills
and empties with the drive.

Two things it does not do:

- **Non-Steam games are invisible.** A cartridge holding a GOG game, an
  emulator or a portable binary reaches no frontend at all.
- **There is no per-cartridge identity.** Steam knows the games are installed;
  nothing knows *which cartridge* they came from, so a frontend cannot say
  "these four are on the drive in your hand".

## What a frontend actually needs

1. Know a cartridge arrived, and that it went away.
2. Know what is on it — titles, art, and what Play would run.
3. Be able to start one.

Today a frontend can infer (1) and part of (2) by watching Steam's library
list. Nothing serves them directly.

## Four ways to close it

### A. Steam library registration — exists

Free, Steam-native, and the games really do appear and disappear with the
drive. Limited to Steam games, and carries no cartridge identity.

### B. Non-Steam shortcuts (`shortcuts.vdf`)

Write every cartridge game into Steam as a non-Steam shortcut. This is how
Heroic, Lutris and EmuDeck reach the Deck's UI, and it would put *any*
cartridge game in front of Deck Shelves, Big Picture and the Steam library at
once.

There is a real synergy: Steam wants artwork in
`userdata/<id>/config/grid/`, and a cartridge is already carrying grid, hero,
logo and icon for every game. The pictures exist; they would just be copied.

The cost is that it writes into Steam's own data:

- `shortcuts.vdf` is binary VDF, and Steam rewrites it from memory on exit —
  so edits made while Steam is running are lost. The wizard already has this
  problem for library registration and already tells the user to close Steam.
- **Ejecting has to clean up**, or the library fills with dead tiles pointing
  at a drive that is not there. That is worse than not appearing at all.
- App IDs for shortcuts are a hash of exe+name. Get it wrong and the artwork
  attaches to nothing.

Highest value, highest blast radius.

### C. A state file — recommended first

The watcher already knows exactly what is mounted and what is on it. It just
never says so. Have it write one file:

```
$XDG_RUNTIME_DIR/pc-gamepak/cartridges.json     Linux
%LOCALAPPDATA%\PC-GamePak\cartridges.json       Windows
```

```json
{
  "version": 1,
  "cartridges": [
    {
      "id": "0A3F-19C2",
      "title": "God of War Collection",
      "mount": "/run/media/harry/GOW",
      "steam_library": true,
      "games": [
        {
          "title": "God of War (2018)",
          "executable": "steam://rungameid/310970",
          "size_bytes": 64200000000,
          "art": { "grid": ".gamepak/gow-grid.jpg", "hero": ".gamepak/gow-hero.jpg" }
        }
      ]
    }
  ]
}
```

Why this first:

- **It is small.** The watcher has the data in hand at the moment it decides to
  open the launcher. Writing it is a serialisation, not a feature.
- **It couples to nothing.** A Decky plugin, a Playnite extension, a Rofi
  script and a Deck Shelves filter can all read one file. No D-Bus, no port, no
  daemon protocol to version.
- **It is watchable.** `inotify` on Linux, `ReadDirectoryChangesW` on Windows —
  a frontend gets told, it does not have to poll.
- **It is honest about what it is.** A cache of "what is plugged in right now",
  deleted on exit. Nothing depends on it being durable.

The `id` should be the filesystem UUID or volume serial, not the mount point,
so a cartridge keeps its identity across replugs and across machines.

### D. D-Bus or a socket

Push-based and idiomatic on Linux. Costs a dependency and a protocol in a
process whose whole design goal is to be 2 MB and asleep, and buys little over
a watched file. Not worth it yet.

## Deck Shelves specifically

Deck Shelves filters on installed / not installed, app status, Remote Play
location, playtime, and a dozen more — but **not on which library folder a
game is installed in**. That single missing predicate is what stands between
today's behaviour and a real cartridge shelf.

With it, a shelf is "games installed on this removable library" and it appears
and empties with the drive, using only (A), which already works.

Worth noting the filter is not really about cartridges: **every Steam Deck
owner with an SD card wants a "games on my SD card" shelf**, and it is the same
one-line predicate. That is the version to propose upstream, with cartridges as
a happy consequence.

## Other frontends

| | How it would get in |
|---|---|
| **Playnite** | Built: the extension in [pc-gamepak-plugins](https://github.com/HarryBMa/pc-gamepak-plugins/tree/main/playnite) (Id `PCGamePak`) keeps a cartridge slot as the first library tile. It polls drives for `cartridge.conf` itself rather than waiting for the state file, and Play runs the launcher with no window (`--play 0`) for a single game, or opens its window (`--show`) to choose from a collection. Eject is `--safe-eject`. Off until `frontends.playnite` is switched on |
| **Steam Big Picture** | Via (B). Nothing else reaches it |
| **EmulationStation / Batocera** | Reads directories of ROMs. A cartridge holding ROMs could be a mount point it already scans — likely no work at all |
| **Zaparoo** | Already supports USB sticks as tokens. A cartridge could carry a Zaparoo token file, so tapping *or* plugging works |

## Suggested order

1. **State file (C).** Small, unblocks everyone else, commits to nothing.
2. **Deck Shelves library-folder filter**, upstream. Uses (A) and (C) needs
   nothing from it.
3. **A Decky plugin or Playnite extension** against the state file, to prove
   the format is usable by someone other than its author.
4. **`shortcuts.vdf` (B)** last, and only with eject cleanup designed first.

## Open questions

- [ ] Does the state file belong to the watcher or the launcher? The watcher
      knows about mounts; the launcher knows about parsing. Probably the
      watcher, calling into `core`.
- [ ] What happens on an unclean shutdown — stale file listing a cartridge that
      is gone. Runtime directory on Linux handles it; Windows does not.
- [ ] Should the wizard write a Steam *collection* per cartridge, so Deck
      Shelves works today without an upstream change? It would make the
      "nearly" above into a "yes", at the cost of writing into Steam's leveldb.
