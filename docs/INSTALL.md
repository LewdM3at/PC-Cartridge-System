# Installing PC GamePak on Linux

There is no AUR package: the AUR closed new account registration after a
security incident, so `pc-gamepak` is written and tested but has nowhere to be
published to. There is no store listing for the Flatpak either — it is built
from this repository.

Both are ready to build by hand, and this page is how. Every command here has
been run on a real machine against a real cartridge.

Pick one:

| | Who it is for | What it installs |
|---|---|---|
| [pacman package](#arch-cachyos-manjaro-steam-deck) | Arch and its derivatives | A real package, `pacman -R`-able |
| [Flatpak](#flatpak) | Anything else, and Steam Deck | A sandboxed app, no root |
| [Scripts](#any-distribution-the-scripts) | Anything with systemd | Files placed by hand |

**Whichever you pick, read [automount](#automount-read-this) at the end.** It is
the one thing that decides whether plugging a cartridge in does anything at all.

---

## Arch, CachyOS, Manjaro, Steam Deck

The PKGBUILD in this repository is the same one that would be on the AUR. It
builds from the tagged release and checks its signature, so it needs no trust in
this working tree.

```bash
git clone https://github.com/HarryBMa/pc-gamepak.git
cd pc-gamepak/packaging/aur/pc-gamepak
makepkg -si
```

That produces `pc-gamepak 1.0.0-1` and installs it with pacman. Then, once:

```bash
systemctl --user enable --now pc-gamepak-watcher.service
```

You get `/usr/bin/pc-gamepak` (launcher and wizard), `pc-gamepak-watcher`,
`pc-gamepak-verify`, `pc-gamepak-build` and `pc-gamepak-eject`, plus a desktop
entry that opens the wizard.

To remove it: `sudo pacman -R pc-gamepak`.

### On a Steam Deck

SteamOS mounts `/` read-only and wipes it on update, so a pacman package is the
wrong shape there even though the Deck is Arch. Use the Flatpak.

---

## Flatpak

Works on any distribution, needs no root, and is the right answer on a Steam
Deck. Built from this repository — there is no store listing, and the remote
added below is only where the GNOME runtime comes from.

```bash
# The runtime and the Rust SDK extension, once
flatpak remote-add --user --if-not-exists flathub \
    https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak install --user flathub org.gnome.Platform//49 org.gnome.Sdk//49 \
    org.freedesktop.Sdk.Extension.rust-stable//25.08

git clone https://github.com/HarryBMa/pc-gamepak.git
cd pc-gamepak/packaging/flatpak
flatpak-builder --user --force-clean --install build \
    io.github.HarryBMa.PCGamePak.yml
```

The crates are vendored in `cargo-sources.json`, so the build itself needs no
network.

Then:

```bash
flatpak run io.github.HarryBMa.PCGamePak            # the wizard
flatpak run io.github.HarryBMa.PCGamePak --watch    # the watcher
```

The manifest tracks the release tag. To build the current `main` instead, change
the `tag:` in its `sources:` to `branch: main`.

To remove it: `flatpak uninstall --user io.github.HarryBMa.PCGamePak`.

**The watcher has to be started by hand.** A Flatpak gets no systemd user
service, so `--watch` does not survive a logout yet; it needs to ask for
background autostart through the Background portal, which is not written. Until
it is, either run the command above at login or use one of the other two
installs.

---

## Any distribution: the scripts

The oldest route, and the only one that uses udev, so nothing is resident.

```bash
git clone https://github.com/HarryBMa/pc-gamepak.git
cd pc-gamepak
cd tauri-ui/src-tauri && cargo build --release && cd ../..
sudo linux/install.sh
```

`install.sh` writes a udev rule and two systemd units with your username in
them, and puts the binaries in `/usr/local/bin`. `linux/uninstall.sh` takes it
all back out.

For an install with no root at all, `linux/install-user.sh` puts everything
under `~/.local` and runs the watcher as a systemd user service — the same
arrangement the pacman package uses.

The frontend is static files, so `cargo build --release` is the whole build.
There is no npm step.

Or skip the build: the `.tar.gz` on
[Releases](https://github.com/HarryBMa/pc-gamepak/releases) carries both
binaries already, and the same two scripts run out of it unchanged.

```bash
tar -xzf pc-gamepak-*-linux-x86_64.tar.gz
cd pc-gamepak-*-linux-x86_64
sudo linux/install.sh      # or: linux/install-user.sh, no root
```

---

## Automount: read this

Nothing in PC GamePak mounts a drive. The watcher waits on the mount table and
the udev helper waits for a mount point to appear, so **a cartridge your desktop
never mounts is a cartridge nothing will notice.**

This is the most common reason it looks broken. On a KDE machine here, the first
cartridge did nothing at all, and the log said exactly why:

```
cartridge detected: sda2
no mount point appeared for /dev/sda2 after 30s; giving up
```

There was nothing wrong with the cartridge. Plasma had simply not mounted it.

- **GNOME** mounts removable drives on its own. Nothing to do.
- **KDE** asks first, unless you turn automount on in
  *System Settings → Removable Storage → Removable Devices*.
- **A bare window manager** does nothing at all.

The pacman package handles this for you: it ships
`pc-gamepak-automount.service`, and the watcher `Wants=` it, so enabling the
watcher is enough. Install `udiskie` and it works:

```bash
sudo pacman -S --asdeps udiskie
```

On other distributions, run `udiskie` at login yourself, or turn on your
desktop's own automount.

To check what is happening, the watcher writes a short log:

```bash
tail -f ~/.local/state/pc-gamepak/watcher.log
```

---

## Windows

WinGet, Scoop, or the installer from
[Releases](https://github.com/HarryBMa/pc-gamepak/releases):

```powershell
winget install HarryBMa.PCGamePak
```
