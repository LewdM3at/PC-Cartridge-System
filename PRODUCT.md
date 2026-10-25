# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

**Primary: the player at the dock.** Someone pushes a cartridge — a small NVMe
stick in a pocketable USB-C enclosure, or any removable drive — into a machine
and expects a window to appear showing what is on it. The scene may be a desk or
a television across a room, with a gamepad rather than a mouse. Their job is one
press: Play. When the two scenes conflict, this one wins.

**Secondary: the maker at the desk.** The same person, in a different mood,
building a cartridge in the wizard: search the installed library, pick one game
or several, pick the drive, choose what goes on it, Write. Deliberately not
controller-driven — making a cartridge is a desk job.

Distribution is public (GitHub, AUR, WinGet, Scoop), so the audience is PC
gamers and enthusiasts on Windows, Linux and Steam Deck, not a private tool.

## Product Purpose

Turn removable storage into physical game cartridges. Plug one in and a launcher
appears with the game's cover art and two buttons. It gives back the
console-cartridge feeling using hardware that already exists, and genuinely
offloads a game off the internal disk — the practical half of the appeal.

Success is a shelf of cartridges that reads as a library you can hold: what is
on each one is obvious from across the room and from the drive's own name in
Explorer. Insert, press, playing.

## Positioning

A cartridge is just a drive with a `cartridge.conf` text file at its root. No
scripts to write, nothing to allowlist, because **nothing on a cartridge is ever
executed automatically** — pressing Play is the gate. An earlier design
auto-executed a `launch.sh` on insert and needed a SHA-256 allowlist to be safe
at all; removing the auto-execution removed the need for the allowlist with it.

The other half a neighbouring product could not truthfully copy: it works
offline. Covers come from whichever cache the game came from; nothing is
fetched. The one network path (SteamGridDB artwork lookup) is off by default and
refused by the backend until the user opts in and supplies their own key.

## Operating Context

- **Insert:** the OS automounts a drive; a watcher notices (udev or `poll()` on
  `/proc/self/mountinfo` on Linux, `WM_DEVICECHANGE` on Windows) and opens the
  launcher. A drive is the only doorbell — tokens are
  [Zaparoo](https://zaparoo.org/)'s job, not this one's.
- **Launcher:** 420 × 560 window, the 3:4 of a cover, artwork filling it. The
  window is the slot and the cover is the cartridge in it: Eject rides the whole
  face out. Title, Play and an eject icon on top; the corner buttons appear under
  the pointer, everything else behind the ⓘ. Accent colour sampled from the cover
  at load. A collection grows a rail that picks which game the one Play acts on,
  cross-fading the art to it; `1`–`9` select and start the first nine. Keys:
  `Enter`, `1`–`9`, `E`, `I`, `Esc`. Gamepad: d-pad/stick to move, A press, B
  back, Y details, Start play — polling loop exists only while a pad is connected
  and the window is open, and the keycaps become face-button icons while it is.
- **Wizard:** searchable library (Playnite JSON export first, Steam manifests
  always, the only source on Linux), drive picker, options, Write. Selection is
  always multiple — one game ticked is a cartridge, several a multicartridge, and
  the rail titles itself to match. A collection gets a second step for the two
  things it cannot infer, a suggested name and chosen artwork; a single game
  never sees it. Options are grouped by what they touch, with the destructive one
  alone, and become a numbered plan with a time estimate. Editing an existing
  cartridge rewrites metadata only and moves no files.
- **Eject:** powers the drive down; asks twice when the game itself lives on the
  cartridge.
- Cartridges spend most of their life unplugged. A missing folder is the normal
  state, not stale cruft.

## Capabilities and Constraints

- **Stack:** Rust workspace — `core/` (`gamepak-core`, 156 tests, no UI
  dependency), `watcher/` (28 tests, ~2 MB resident on Windows), `tauri-ui/`
  (Tauri 2, one binary, two window modes: `--drive <path>` popup and `--create`
  wizard, 24 commands). Frontend is plain HTML/CSS/JS under `tauri-ui/app/` with
  bundled fonts — no framework, no bundler.
- **Security model is a design constraint.** The webview has no filesystem access
  and no command that takes a path; the cover is read in Rust from a path
  confined to the cartridge and passed in as a `data:` URI under an 8 MB cap.
  CSP is `default-src 'self'`. Fonts bundled. Titles come off an untrusted volume
  and are inserted with `textContent`, never as markup. Any design that needs a
  new asset source needs a Rust command, not a fetch.
- **Formatting is gated four ways:** removable-drive allowlist re-derived in the
  backend, not the system drive, the drive's current label typed back exactly,
  and explicitly asked for. Write stays disabled until it matches, and the
  backend re-checks all of it.
- exFAT is the default filesystem; btrfs is an enthusiast option that Windows
  cannot read without a third-party driver. exFAT caps drive labels at 11
  characters.
- Steam cartridges register in `libraryfolders.vdf`; Steam must be closed first
  and is asked to shut down, never killed.
- Optional copy verification: CRC-32 taken while writing, checked by reading the
  cartridge back, manifest left at `.gamepak/manifest.json`. An integrity check,
  not a signature.
- **Terminology:** cartridge, dock, insert, Play, Eject, collection (several
  games on one cartridge), the wizard, the launcher.
- **Undecided / not yet true:** no tagged release exists; **no cartridge has ever
  been written by this code on real hardware** — every path is unit-tested and
  the frontend screenshotted only. macOS is unsupported. Windows code signing,
  on-demand re-verification of an existing cartridge, programming a tag from the
  wizard, and adding a game to an existing cartridge without a full rewrite are
  all missing. Do not describe any of these as working.

## Brand Commitments

Binding: the name **PC GamePak** and the cartridge vocabulary above. The
launcher's 420 × 560 3:4 window is a product fact, tied to cover-art proportion.

Not binding, and open to replacement by later visual work: the current type
(bundled Archivo variable + Spline Sans Mono), the accent-sampled-from-cover
rule, and the existing surface treatment — now the slot-and-cartridge reading of
the launcher window, and the tick-count selection model in the wizard. They are
the incumbent implementation, evidence rather than commitment.

## Evidence on Hand

- README.md — the full product account, written and current.
- `docs/STATUS.md` — honest inventory of what is built and what is missing.
- Screenshots of every shipped surface: `docs/launcher.png`,
  `launcher-bundle.png`, `launcher-details.png`, `launcher-health.png`,
  `wizard.png`, `wizard-bundle.png`, `wizard-edit.png`, `wizard-format.png`,
  `wizard-portable.png`, `wizard-settings.png`; icon at `docs/icon.png`.
- Demo art for UI work: `tauri-ui/app/src/demo/`.
- `docs/HARDWARE-REPORT.md`, `docs/PUBLISHING.md`, `cartridge.conf.example`.
- MIT licence, Ko-fi support link, CI badges — all real.
- **No** users, testimonials, download counts, reviews, benchmarks or hardware
  field reports exist. Nothing may claim them.

## Product Principles

1. **Nothing runs without a click.** Every design decision inherits this; no
   affordance may imply automatic execution.
2. **The cover is the interface.** The artwork is the product's identity on
   screen; chrome sits on top of it and stays out of its way.
3. **One press from insert to playing.** Depth is allowed only behind the gear.
4. **Offline by default, and honest about the one exception.** Network features
   announce themselves and stay off until asked for.
5. **Destructive actions are gated in the backend, not just the window.** The UI
   states the consequence in the user's own words; the gate exists regardless.

## Accessibility & Inclusion

The launcher must stay readable at television distance and be fully operable by
gamepad or keyboard — no mouse-only path to Play, Eject or Details. Focus must be
visible on a pad. No formal standard is claimed yet; the cover-sampled accent
means contrast varies per cartridge and must be handled, not assumed.
