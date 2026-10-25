# PC GamePak installer (Windows)
#
# Installs two native binaries and a logon task:
#
#   pc-gamepak-watcher.exe   resident, waits for a volume to arrive
#   pc-gamepak.exe  the popup, started by the watcher on insert
#
# The watcher replaces the old resident PowerShell monitor. PowerShell kept the
# .NET runtime and a WMI subscription alive for the whole session to do this job;
# the watcher blocks on the Windows message queue instead, so it uses a couple of
# megabytes and no CPU while it waits.
#
# Experimental AutoPlay support is available as an opt-in registry hook that is
# best effort only: Windows can suppress or ignore it depending on policy,
# device type, and OEM shell behavior.
#
# Run it with no arguments for the menu, or pass -Mode to skip it:
#
#   powershell -ExecutionPolicy Bypass -File windows\install.ps1 -Mode Watcher
#
# -Mode is what makes this usable from a package manager. Scoop and WinGet
# unpack the binaries but neither registers a logon task, so their notes send
# the user here - and `Read-Host` fails outright under -NonInteractive, which
# is how anything scripted runs it.

[CmdletBinding()]
param(
    [ValidateSet("Watcher", "AutoPlay")]
    [string]$Mode
)

$ErrorActionPreference = "Stop"

Write-Host ""
Write-Host "Installing PC GamePak..."
Write-Host ""

########################################
# Paths
########################################

$InstallFolder = Join-Path $env:LOCALAPPDATA "PC-GamePak"
$RepoRoot      = Split-Path -Parent $PSScriptRoot

# Two layouts reach this script. In a clone the binaries are still in their
# cargo target directories; in a release zip - which is what WinGet and Scoop
# unpack - they sit at the root, next to this windows\ folder. Checking the
# package layout first means a package manager's user never sees the "build the
# binaries first" message, which they cannot act on and which used to be the
# only thing this script would say to them.
function Find-Binary {
    param([string]$Name, [string]$BuiltPath)
    $packaged = Join-Path $RepoRoot $Name
    if (Test-Path $packaged) { return $packaged }
    return $BuiltPath
}

$WatcherSource = Find-Binary "pc-gamepak-watcher.exe" `
    (Join-Path $RepoRoot "watcher\target\release\pc-gamepak-watcher.exe")
$LauncherSource = Find-Binary "pc-gamepak.exe" `
    (Join-Path $RepoRoot "tauri-ui\src-tauri\target\release\pc-gamepak.exe")

$WatcherTarget  = Join-Path $InstallFolder "pc-gamepak-watcher.exe"
$LauncherTarget = Join-Path $InstallFolder "pc-gamepak.exe"

$TaskName = "PC GamePak Watcher"

# Put a binary in place even while the old one is running.
#
# A launcher can be open when this runs — a window from the last insert, or a
# `--play` that stays up for the whole of a game to count it — and Windows
# refuses to overwrite a running executable. It does allow renaming one, so the
# old file is moved aside and the new one copied in; the running process keeps
# its renamed file until it exits. Leftovers from earlier installs are removed
# when nothing holds them any more.
function Install-Binary {
    param([string]$Source, [string]$Target)

    Get-ChildItem -Path "$Target.old-*" -ErrorAction SilentlyContinue |
        Remove-Item -Force -ErrorAction SilentlyContinue

    try {
        Copy-Item -Path $Source -Destination $Target -Force
    } catch [System.IO.IOException] {
        $Aside = "$Target.old-$([DateTime]::Now.Ticks)"
        Write-Host "  $(Split-Path $Target -Leaf) is running; moving it aside to replace it"
        Move-Item -Path $Target -Destination $Aside -Force
        Copy-Item -Path $Source -Destination $Target -Force
    }
}

# Whatever happened above, leave a watcher running: the install stops the old
# one first, and a failure part-way used to leave none until the next logon.
function Confirm-WatcherRunning {
    param([string]$Task, [string]$Exe)

    for ($Try = 0; $Try -lt 10; $Try++) {
        if (Get-Process -Name "pc-gamepak-watcher" -ErrorAction SilentlyContinue) { return }
        if ($Try -eq 0 -or $Try -eq 5) {
            Start-ScheduledTask -TaskName $Task -ErrorAction SilentlyContinue
        }
        Start-Sleep -Milliseconds 500
    }
    if (-not (Get-Process -Name "pc-gamepak-watcher" -ErrorAction SilentlyContinue) -and (Test-Path $Exe)) {
        Write-Host "The scheduled task did not start the watcher; starting it directly."
        Start-Process -FilePath $Exe
    }
}

function Install-StandardWatcher {
    param(
        [string]$InstallPath,
        [string]$WatcherExe,
        [string]$LauncherExe,
        [string]$Task
    )

    Write-Host "Creating install directory..."
    New-Item -ItemType Directory -Path $InstallPath -Force | Out-Null

    # Stop a running watcher before overwriting it.
    Get-Process -Name "pc-gamepak-watcher" -ErrorAction SilentlyContinue |
        Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 400

    try {
        Write-Host "Installing watcher and launcher..."
        Install-Binary -Source $WatcherExe -Target $WatcherTarget
        Install-Binary -Source $LauncherSource -Target $LauncherTarget

        # Remove tasks from previous versions, including the PowerShell monitor.
        $OldTaskNames = @(
            "PC GamePak Watcher",
            "PC GamePak Monitor",
            "Steam Game Cartridge Monitor"
        )

        foreach ($Old in $OldTaskNames) {
            $Existing = Get-ScheduledTask -TaskName $Old -ErrorAction SilentlyContinue
            if ($null -ne $Existing) {
                Write-Host "Removing previous task: $Old"
                Stop-ScheduledTask -TaskName $Old -ErrorAction SilentlyContinue
                Start-Sleep -Milliseconds 500
                Unregister-ScheduledTask -TaskName $Old -Confirm:$false
            }
        }

        # Name the user on both the trigger and the principal.
        #
        # `-AtLogOn` with no `-User` means *any* user logging on, which is a
        # machine-wide task, and registering one without elevation fails with
        # "Access denied". The watcher is a per-user thing — it opens a window on
        # one desktop — so the trigger fires for this user, the task runs as this
        # user, and the whole install needs no UAC prompt.
        $UserId = "$env:USERDOMAIN\$env:USERNAME"

        $Action  = New-ScheduledTaskAction -Execute $WatcherTarget
        $Trigger = New-ScheduledTaskTrigger -AtLogOn -User $UserId

        $Principal = New-ScheduledTaskPrincipal `
            -UserId $UserId `
            -LogonType Interactive `
            -RunLevel Limited

        # No execution time limit: this is meant to stay running for the session.
        $Settings = New-ScheduledTaskSettingsSet `
            -ExecutionTimeLimit (New-TimeSpan -Seconds 0) `
            -RestartCount 3 `
            -RestartInterval (New-TimeSpan -Minutes 1) `
            -AllowStartIfOnBatteries `
            -DontStopIfGoingOnBatteries

        Register-ScheduledTask `
            -TaskName $TaskName `
            -Action $Action `
            -Trigger $Trigger `
            -Principal $Principal `
            -Settings $Settings `
            -Force `
            -Description "Opens the cartridge launcher when a cartridge is plugged in" | Out-Null

        Write-Host "Starting watcher..."
    } finally {
        Confirm-WatcherRunning -Task $TaskName -Exe $WatcherTarget
    }
}

function Install-ExperimentalAutoplay {
    param(
        [string]$LauncherExe,
        [string]$InstallPath
    )

    Write-Host "Installing launcher to $InstallPath..."
    New-Item -ItemType Directory -Path $InstallPath -Force | Out-Null
    Copy-Item -Path $LauncherSource -Destination $LauncherTarget -Force

    $handlerKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\AutoplayHandlers\Handlers\PCGamePakCartridge"
    $eventKey   = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\AutoplayHandlers\Events\StorageOnArrival\Handlers\PCGamePakCartridge"

    Write-Host "Registering experimental AutoPlay handler..."
    New-Item -Path $handlerKey -Force | Out-Null

    $launcherCommand = '"{0}" --drive "%L"' -f $LauncherExe

    Set-ItemProperty -Path $handlerKey -Name "Action" -Type String -Value "Open with PC GamePak"
    Set-ItemProperty -Path $handlerKey -Name "Provider" -Type String -Value "PC GamePak"
    Set-ItemProperty -Path $handlerKey -Name "DefaultIcon" -Type String -Value "$LauncherExe,0"
    Set-ItemProperty -Path $handlerKey -Name "InitCmdLine" -Type String -Value $launcherCommand
    Set-ItemProperty -Path $handlerKey -Name "InvokeProgID" -Type String -Value "PCGamePak.Cartridge"

    New-Item -Path $eventKey -Force | Out-Null
    New-ItemProperty -Path $eventKey -Name "(default)" -PropertyType String -Value "PCGamePakCartridge" -Force | Out-Null

    Write-Host "Experimental AutoPlay is installed in best-effort mode."
    Write-Host "Windows may suppress it under policy or OEM shell settings."
}

########################################
# Check the binaries have been built
########################################

$Missing = @()
if (-not (Test-Path $WatcherSource))  { $Missing += $WatcherSource }
if (-not (Test-Path $LauncherSource)) { $Missing += $LauncherSource }

if ($Missing.Count -gt 0) {
    Write-Host "Cannot find the binaries." -ForegroundColor Yellow
    Write-Host ""
    Write-Host "In an unpacked release they belong next to the windows folder."
    Write-Host "If any are missing there, the download is incomplete - fetch it again."
    Write-Host ""
    Write-Host "In a clone, build them first:"
    Write-Host ""
    Write-Host "  cargo build --release --manifest-path watcher\Cargo.toml"
    Write-Host "  cargo build --release --manifest-path tauri-ui\src-tauri\Cargo.toml"
    Write-Host ""
    Write-Host "Looked for:" -ForegroundColor Yellow
    $Missing | ForEach-Object { Write-Host "  $_" }
    exit 1
}

########################################
# Mode
########################################

# Asked for only when it was not passed, and only when there is someone to ask.
# A non-interactive host cannot answer, so it takes the default rather than
# failing on a prompt nobody will ever see.
if (-not $Mode) {
    if ($Host.UI.RawUI -and -not [Environment]::UserInteractive) {
        Write-Host "Not running interactively; installing the standard watcher."
        $Mode = "Watcher"
    }
    else {
        Write-Host "Choose an install mode:"
        Write-Host "  1) Standard watcher (default)"
        Write-Host "  2) Experimental AutoPlay handler"
        Write-Host "  3) Cancel"

        try {
            $Choice = Read-Host "Enter choice [1-3]"
        }
        catch {
            # -NonInteractive: there is no console to read from.
            Write-Host "No console to read from; installing the standard watcher."
            $Choice = "1"
        }

        switch ($Choice) {
            "1" { $Mode = "Watcher" }
            "2" { $Mode = "AutoPlay" }
            "3" {
                Write-Host "Install cancelled."
                exit 0
            }
            default {
                Write-Host "Invalid choice. Defaulting to Standard watcher."
                $Mode = "Watcher"
            }
        }
    }
}

switch ($Mode) {
    "AutoPlay" {
        Install-ExperimentalAutoplay -LauncherExe $LauncherTarget -InstallPath $InstallFolder
    }
    default {
        Install-StandardWatcher -InstallPath $InstallFolder -WatcherExe $WatcherSource -LauncherExe $LauncherSource -Task $TaskName
    }
}

########################################
# Done
########################################

Write-Host ""
Write-Host "=========================================="
Write-Host " PC GamePak installed"
Write-Host "=========================================="
Write-Host ""
Write-Host " Put a cartridge.conf at the root of the drive:"
Write-Host ""
Write-Host "   executable=steam://rungameid/1091500"
Write-Host "   title=Cyberpunk 2077"
Write-Host "   cover=cover.jpg"
Write-Host ""
Write-Host " Then plug the cartridge in. Nothing runs on its own:"
Write-Host " the launcher opens and waits for you to press Play."
Write-Host ""
Write-Host " Installed to: $InstallFolder"
Write-Host ""

# Only when someone is there to press a key. -Mode means this was scripted —
# by a package manager, or by an installer that already knows what it wants —
# and PAUSE is a Read-Host underneath, which fails outright with no console
# and takes the exit code with it. The install had already succeeded.
if (-not $Mode -and [Environment]::UserInteractive) {
    try { PAUSE } catch { }
}
