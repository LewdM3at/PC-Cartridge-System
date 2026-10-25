# WinGet packaging

Three manifests per version, submitted together to
[microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs) under
`manifests/h/HarryBMa/PCGamePak/<version>/`. The version folders here mirror
that layout, and keeping this README outside them is deliberate: `winget
validate` parses every file in the directory it is given and fails on anything
that is not a manifest.

Validated with `winget validate --manifest packaging\winget\1.0.0`.

## State of the 1.0.0 manifests

Ready to submit. Everything that can be checked without publishing has been:

- `winget validate` passes.
- `InstallerSha256` is the real hash, taken from a fresh download of the
  published asset and cross-checked against the `.sha256` GitHub serves.
- Both `NestedInstallerFiles` paths were checked against the actual zip; the
  archive nests under `pc-gamepak-1.0.0-windows-x86_64/` and both executables
  are there.
- `wingetcreate show HarryBMa.PCGamePak` returns not-found, so this is a first
  submission and will go through human moderation rather than the automatic
  path later versions get.

The one step left is the submission itself, which opens a public pull request
against microsoft/winget-pkgs from your GitHub account:

```powershell
wingetcreate submit --token <github-pat> packaging\winget\1.0.0
```

The token needs `public_repo`. `wingetcreate token --store` caches it so later
releases do not need it on the command line.

## What WinGet can and cannot do here

It delivers the files and puts `pc-gamepak` and `pc-gamepak-watcher` on PATH.
It does **not** register the watcher's logon task, because a portable package
runs no scripts. So a WinGet install gives you a working wizard immediately and
a launcher that will not open on insert until the user runs the installer once:

```powershell
powershell -ExecutionPolicy Bypass -File install.ps1 -Mode Watcher
```

That script lives inside the installed package, at roughly:

```
%LOCALAPPDATA%\Microsoft\WinGet\Packages\HarryBMa.PCGamePak_<hash>\pc-gamepak-1.0.0-windows-x86_64\windows\install.ps1
```

`-Mode Watcher` matters: without it the script prompts, and a non-interactive
host cannot answer.

## Releasing a new version

Automatic. Publishing a GitHub release runs `.github/workflows/winget.yml`,
which uses [winget-releaser](https://github.com/vedantmgoyal9/winget-releaser)
to copy the previous manifests, swap in the new zip's URL and hash, and open
the pull request on microsoft/winget-pkgs. It needs, once:

- a fork of microsoft/winget-pkgs on the repository owner's account;
- a classic token with `public_repo`, saved as the repository secret
  `WINGET_TOKEN`.

A release it missed (no secret yet, a failed run) can be sent again from
Actions → winget → Run workflow, with the tag.

By hand, from Windows, if the workflow is not an option:

```powershell
wingetcreate update HarryBMa.PCGamePak --version <v> --urls <zip url> --submit
```

The version folders here are what `check-versions.mjs` keeps in step with the
rest of the repository; the submission itself is built from the previous
published manifests, not from them.

The first submission goes through automated validation and then a human
reviewer. Later ones are usually automatic.

## Identifier

`HarryBMa.PCGamePak`. WinGet identifiers are `Publisher.Package` in PascalCase;
the repository name (`pc-gamepak`) is the `Moniker` instead, which is what
`winget install pc-gamepak` matches on.
