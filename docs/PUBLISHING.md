# Publishing PC GamePak

## The constraint that decides everything

This is not a self-contained application. Installing it means installing
**system integration**:

| Platform | What goes where |
|---|---|
| Linux | a udev rule in `/usr/lib/udev/rules.d`, two systemd template units, three helpers in `/usr/local/bin` |
| Windows | two binaries under `%LOCALAPPDATA%`, plus a Task Scheduler logon task |

That is the whole feature. A cartridge opens its launcher **because** udev saw a
partition appear. Anything that cannot write those files, or cannot see the
device layer, ships a program that does nothing until the user installs the real
thing by hand.

So every channel below is judged on one question first: *can a package
installed this way actually watch for a drive?*

The wizard alone is a different matter — it is an ordinary desktop app and would
be fine in a sandbox. Splitting the two is possible, and worth considering later,
but shipping "PC GamePak" that cannot notice a cartridge would be a bait and
switch.

## Verdicts

### Yes — start here

| Channel | Why it fits | Effort |
|---|---|---|
| **GitHub Releases** | The source of truth every other channel points at. A `v*` tag builds Linux and Windows artefacts with checksums, and drafts the release with notes from `CHANGELOG.md`. | Done — three released |
| **AUR** (`pc-gamepak`) | Arch, CachyOS, Manjaro — and the Steam Deck crowd, who are the audience. Ships the rootless watcher as a systemd *user* service, because a package cannot bake a username into a system unit. | Low. `packaging/aur/pc-gamepak/` is written |
| **Flatpak** (built from source) | Built, installed and run against a real cartridge — the whole product works sandboxed. The way onto a Steam Deck, which ships Flatpak and no AUR. Not submitted to any store; see below. | Done — `packaging/flatpak/`, cargo sources vendored, builds offline |
| **WinGet** | Built into Windows 11. The installer script does the logon task; the manifest just delivers the files. | Low, once a release exists |
| **Scoop** | User-space, no admin, popular with the same people who own a drawer of NVMe drives. Published in [HarryBMa/scoop-bucket](https://github.com/HarryBMa/scoop-bucket). | Low |

### Later, and only with a reason

| Channel | Verdict |
|---|---|
| **`.deb` artefact** | Cheap and useful: `cargo-deb` produces one, attach it to the release, `sudo dpkg -i` installs the binary and the units. Do this before standing up an APT repository — a repo is weeks of maintenance for the same result. |
| **Chocolatey** | Fine technically; first submission goes through human moderation and the package needs a maintained `.nuspec`. Worth it only once Windows users actually ask. |
| **AppImage** | Tauri already builds one. It covers the *launcher*, not the watcher or the udev rule, so it is a convenience for people who want the wizard without installing anything — not a way to ship the product. |

### No, for now

| Channel | Why not |
|---|---|
| **Snap** | The audience is the Steam Deck, and SteamOS ships Flatpak. snapd is not installed on Arch by default and has to be added by hand, so a Snap asks the exact people this is for to install a second package manager first. Classic confinement's manual review queue on top. Flatpak covers the same ground with none of that. |
| **Homebrew** | macOS is not a supported platform at all: no watcher, no installer, no icon set. On Linux, Homebrew installs into its own prefix and cannot place system units either. A tap would ship something that cannot work on the platform people would `brew install` it from. |
| **Native pacman repo** | Hosting a signed binary repository to serve what the AUR already serves from source. |

## What changed these answers

Every "no" above used to trace back to one root: the Linux side needed root to
install a udev rule. It no longer does.

`linux/install-user.sh` installs everything under `$HOME` and runs the watcher as
a **systemd user service**. The watcher blocks in `poll()` on
`/proc/self/mountinfo` — not on udev, which a sandbox cannot reach — and wakes
when the mount table changes. About 2 MB resident, no CPU while it waits.

The system install stays the recommendation, because zero is a better number than
two megabytes. But the rootless one is what a package format can actually
install, which is what puts Flatpak back on the table.

That was the last theoretical objection, and it has now been measured rather
than reasoned about. See below.

## What Linux actually does, tested

Checked on CachyOS (KDE Plasma 6, Wayland) against a real 128 GB exFAT
cartridge — a ten-game Tomb Raider collection written by the Windows wizard.

| | Result |
|---|---|
| `cargo build --release` for all three crates | Builds clean, warnings only |
| `cargo test --release` in `core` | 218 pass |
| `verify-cart` over the whole cartridge | 107.43 GB read at 345 MB/s, `intact: every file matches the manifest` |
| Launcher on Wayland | Draws the collection, per-game art, Play and Eject |
| Wizard (`--create`) | Opens, and finds the cartridge: `20.4 GB free · exFAT · has a cartridge` |
| Rootless watcher | Detects arrival and removal, opens and closes the launcher, 2.4 MB resident |
| udev route (`install.sh`) | Fires and opens the launcher |

The cartridge written on Windows was read on Linux without conversion, which is
the claim the exFAT choice exists to make.

**The one thing that actually breaks it is automount.** The watcher waits on the
mount table and the udev helper waits for a mount point; neither can do anything
for a drive the desktop never mounts. KDE does not automount removable media
unless it is switched on, and the first plug-in of the test cartridge failed
exactly there:

```
==== 2026-09-05T18:13:49 cartridge detected: sda2 ====
no mount point appeared for /dev/sda2 after 30s; giving up
```

Nothing was wrong with the cartridge — it opened normally once mounted. So the
packaging says so out loud: `udiskie` is an optdepend, and the post-install
message names automount as the thing to check first. It is the Linux equivalent
of a Windows autorun policy, and it will be the top support question.

## Does a sandboxed watcher actually work?

The Flatpak question was whether a mount made on the host after the sandbox
started becomes visible inside it. Tested with bubblewrap 0.11.2 — the same
thing Flatpak runs — with `/run/media` bound, a private mount namespace and an
unprivileged user namespace, against the real cartridge:

```
[1..4]   sees mount     cartridge was mounted before the sandbox started
[5..11]  --             host unmounted it
[12..30] sees mount     host mounted it again, after the sandbox started
```

Propagation works in both directions, within the one-second granularity of the
poll. Then the same test with the real `pc-gamepak-watcher` binary inside the
sandbox rather than a shell loop:

```
watcher starting (mount table)
watching 16 mounted filesystems
cartridge removed: /run/media/playbox/TOMBRAIDERC
cartridge detected at /run/media/playbox/TOMBRAIDERC
launcher started, pid 3
```

So `poll()` on `/proc/self/mountinfo` receives its `POLLPRI` wakeups through
mount propagation, inside a namespace, with no udev and no root. That is the
whole mechanism the product depends on, working in the place it was assumed it
might not.

`steam://` is the other half, and the pieces are all present on a normal
desktop: the scheme resolves to a handler (`steam.desktop`),
`xdg-desktop-portal` and a backend are installed and running, and
`org.freedesktop.portal.OpenURI` answers on the session bus. A Flatpak's
`xdg-open` is a shim onto that portal, so the URI is handed to the host rather
than run in the sandbox. Worth confirming once packaged, since the portal may
ask the user to confirm the handler the first time.

Then the manifest was built and installed rather than left as a guess:
`flatpak-builder` against the GNOME 49 runtime and the Rust SDK extension, with
all 428 crates vendored so the build runs with no network. It compiled first
time. Installed, the launcher opened the real
cartridge from inside the sandbox, and the watcher — run inside the Flatpak —
logged a full plug cycle and started the launcher itself:

```
watcher starting (mount table)
watching 103 mounted filesystems
cartridge removed: /run/media/playbox/TOMBRAIDERC
cartridge detected at /run/media/playbox/TOMBRAIDERC
launcher started, pid 3
```

So the whole product works sandboxed, with no root, no udev rule and no
privileged helper.

Autostart is the one thing a Flatpak cannot do the ordinary way: it gets no
systemd user unit, so the watcher asks for background autostart through
`org.freedesktop.portal.Background` instead of being enabled with
`systemctl --user`. That path is written and gated on `/.flatpak-info`.

`packaging/flatpak/` has to exist in the tagged source the manifest points at.
v1.0.0 predates it; v1.0.1 contains it.

**Not going to Flathub.** The submission was rejected under their generative-AI
policy, and the reviewer asked that it not be resubmitted. The manifest stays
because it builds a working Flatpak from this repository — see
[INSTALL.md](INSTALL.md) — but there is no store listing and none is planned.

## Cutting a release

Three exist — v0.1.0, v1.0.0 and v1.0.1 — so this is a checklist rather than a
plan. The order matters in one place, and it is the place that went wrong: a
checksum names an artefact, so it cannot be written until the artefact exists.

**Before the tag**

1. `node tools/check-versions.mjs --set <version>`. It moves all fourteen places
   the version is written — three `Cargo.toml`s and their lockfiles,
   `package.json` and its lockfile, `tauri.conf.json`, the AUR `PKGBUILD` and
   `.SRCINFO`, the Flatpak manifest's tag, the metainfo's newest `<release>`, and
   the WinGet directory and the three manifests inside it. It also blanks the
   checksums to `SHA256-PENDING-RELEASE`, because they belong to the *previous*
   release and a stale checksum is worse than a missing one: it looks like an
   answer.
2. Write the `CHANGELOG.md` section, in consequences rather than commit
   subjects. The release workflow uses it for the release notes, so this is the
   text people read — it falls back to `--generate-notes` if there is no section
   for the version, which is not a good outcome, only a non-fatal one.
3. Fill in the metainfo `<description>`, which appstream shows in software
   centres. `--set` leaves a `TODO` there rather than inheriting the last
   release's notes under a new number.
4. Change the changelog heading from `unreleased` to the date — the same date
   the metainfo's newest `<release>` carries. `check-versions.mjs` compares the
   two and refuses a release while the heading still says `unreleased`, because
   both dates are published and nothing used to check they agreed.
5. Run the `release` workflow by hand — Actions → release → Run workflow — and
   give it the version. It builds and packages both platforms and stops before
   publishing anything, which is the cheap way to find out that a file the
   packaging step copies does not exist. That has happened twice: once a
   PowerShell script that was never written failed the whole Windows job, and a
   tag produced no Windows artefact at all.

   The version has to be given as the input. The workflow used to take it from
   the ref, so a dry run on a branch built a version named after the branch and
   uploaded nothing — the one failure the dry run could not catch was its own.
6. Merge to `main`. Tags belong on `main`, not on a branch.

**The tag**

7. `git tag v<version> && git push origin v<version>`. The workflow builds both
   platforms, computes the checksums, and creates the release **as a draft**.
   Look at it before publishing: a tag is cheap to delete before anyone has
   downloaded it and expensive afterwards.

**After the artefacts exist**

8. Put the real checksums where `SHA256-PENDING-RELEASE` is. They are in the
   `.sha256` files the workflow uploads beside each artefact — the AUR one is of
   the source tarball GitHub generates for the tag, not of the release archive.
9. `node tools/check-versions.mjs --release`. Same check, and it also fails on
   any remaining placeholder and on a changelog section that is missing or still
   undated. Run it before submitting a manifest anywhere.
10. **Scoop** needs nothing: `checkver` and `autoupdate` in
   [HarryBMa/scoop-bucket](https://github.com/HarryBMa/scoop-bucket) read the
   `.sha256` themselves.
11. **WinGet** needs nothing either: publishing the draft runs
    `.github/workflows/winget.yml`, which opens the pull request on
    microsoft/winget-pkgs. It needs a fork of winget-pkgs on the owner's account
    and a classic `public_repo` token in the `WINGET_TOKEN` secret; a release it
    missed can be sent again from Actions → winget → Run workflow. **AUR** from the tarball with the real checksum — the
    plain name, since the `-git` suffix is what the AUR reserves for a package
    tracking a branch.

## Still outstanding

- **Code signing on Windows.** Unsigned binaries mean a SmartScreen warning on
  every download, and it does not go away until the certificate builds
  reputation. Azure Trusted Signing is the cheap path; self-signing achieves
  nothing here. Costs money rather than work, which is why it has survived three
  releases.
- **`.deb`** attached to releases via `cargo-deb`. Not written.
- **Flatpak**, built from this repository rather than from a store. Snap is not
  planned.
