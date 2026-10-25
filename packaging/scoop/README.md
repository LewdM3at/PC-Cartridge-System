# Scoop packaging

The manifest is **not here**. Scoop installs from buckets — git repositories
full of manifests — so the only copy that matters is the one Scoop reads:

**[HarryBMa/scoop-bucket](https://github.com/HarryBMa/scoop-bucket)** →
`bucket/pc-gamepak.json`

A second copy in this repository was deleted rather than kept in step. Two files
that must say the same thing eventually do not, and the one that goes stale is
always the one nobody installs from.

```powershell
scoop bucket add harrybma https://github.com/HarryBMa/scoop-bucket
scoop install pc-gamepak
```

## What the manifest does

`extract_dir` strips the version folder, so both executables land at the app
root and both get shims. `windows\install.ps1` comes along with them, which is
what the notes point at.

`checkver` and `autoupdate` are configured, so a new tagged release needs no
manual hashing: `autoupdate.hash.url` reads the `.sha256` the release workflow
uploads beside the zip. Verified by forcing an update against v1.0.0 —

```
Searching hash for pc-gamepak-1.0.0-windows-x86_64.zip in …zip.sha256
Found: 82fcf997…d153 using Extract Mode
```

To bump it by hand from a clone of the bucket:

```powershell
.\bin\checkver.ps1 -App pc-gamepak -Dir bucket -Update
```

Adding `.github/workflows/excavator.yml` from
[ScoopInstaller/GithubActions](https://github.com/ScoopInstaller/GithubActions)
does it on a schedule instead.

The bucket pins `*.json text eol=crlf`, because `checkver -u` rewrites the whole
file with CRLF endings and a manifest stored with LF turns every autoupdate into
a diff of all forty-six lines.

## What Scoop cannot do

Register the watcher's logon task — it runs no install scripts. So a Scoop
install gives a working wizard immediately and a launcher that will not open on
insert until the user runs `windows\install.ps1 -Mode Watcher` once. The notes
in the manifest say so, and `$dir` resolves to the real path when Scoop prints
them.

## Why a personal bucket

Scoop's main bucket takes widely-used software and Extras takes established
GUI applications. A 1.0 is neither yet. Moving to Extras later is a pull request
with the same file.
