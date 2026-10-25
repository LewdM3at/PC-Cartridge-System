# PC GamePak Uninstaller

[CmdletBinding()]
param(
    # Also delete settings.json and the artwork cache.
    [switch]$Purge
)

$ErrorActionPreference = "Stop"


Write-Host ""
Write-Host "Uninstalling PC GamePak..."
Write-Host ""


########################################
# Paths
########################################

$InstallFolders = @(
    (Join-Path $env:LOCALAPPDATA "SteamGameCartridge")
    (Join-Path $env:LOCALAPPDATA "PC-GamePak")
)

$TaskNames = @(
    "Steam Game Cartridge Monitor",
    "PC GamePak Monitor",
    "PC GamePak Watcher"
)

$AutoplayHandlerKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\AutoplayHandlers\Handlers\PCGamePakCartridge"
$AutoplayEventKey   = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\AutoplayHandlers\Events\StorageOnArrival\Handlers\PCGamePakCartridge"


########################################
# Remove scheduled tasks
########################################

Write-Host "Removing scheduled tasks..."

foreach ($TaskName in $TaskNames) {
    $Task = Get-ScheduledTask `
        -TaskName $TaskName `
        -ErrorAction SilentlyContinue

    if ($null -ne $Task) {
        Write-Host "Stopping monitor: $TaskName"

        Stop-ScheduledTask `
            -TaskName $TaskName `
            -ErrorAction SilentlyContinue

        Unregister-ScheduledTask `
            -TaskName $TaskName `
            -Confirm:$false
    }
    else {
        Write-Host "Scheduled task not found: $TaskName"
    }
}


########################################
# Remove AutoPlay experiment
########################################

Write-Host "Removing experimental AutoPlay handler..."

if (Test-Path $AutoplayEventKey) {
    Remove-Item -Path $AutoplayEventKey -Recurse -Force -ErrorAction SilentlyContinue
}

if (Test-Path $AutoplayHandlerKey) {
    Remove-Item -Path $AutoplayHandlerKey -Recurse -Force -ErrorAction SilentlyContinue
}


########################################
# Remove installed files
########################################

Write-Host "Stopping the watcher..."

Get-Process -Name "pc-gamepak-watcher" -ErrorAction SilentlyContinue |
    Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 400

Write-Host "Removing installed files..."

# What the installer put there, and nothing else.
#
# This folder holds two other things: settings.json, which carries the
# SteamGridDB key someone had to go and generate, and sgdb-cache, which is every
# piece of artwork ever downloaded plus the record of which one was chosen for
# each game. Neither is ours. Removing the program used to take both with it —
# tens of megabytes of someone else's work, deleted without being mentioned.
#
# -Purge asks for the folder to go entirely.
$Ours = @("pc-gamepak.exe", "pc-gamepak-watcher.exe", "watcher.log")

foreach ($InstallFolder in $InstallFolders) {
    if (-not (Test-Path $InstallFolder)) {
        Write-Host "Install directory not found: $InstallFolder"
        continue
    }

    if ($Purge) {
        Write-Host "Removing $InstallFolder and everything in it"
        Remove-Item -Path $InstallFolder -Recurse -Force
        continue
    }

    Write-Host "Removing the program from $InstallFolder"
    foreach ($Name in $Ours) {
        $Path = Join-Path $InstallFolder $Name
        if (Test-Path $Path) { Remove-Item -Path $Path -Force -ErrorAction SilentlyContinue }
    }
    # Old binaries install.ps1 moved aside because they were running.
    Get-ChildItem -Path $InstallFolder -Filter "*.exe.old-*" -ErrorAction SilentlyContinue |
        Remove-Item -Force -ErrorAction SilentlyContinue

    # Gone entirely if nothing of the user's was in it.
    if (-not (Get-ChildItem $InstallFolder -Force -ErrorAction SilentlyContinue)) {
        Remove-Item -Path $InstallFolder -Force -ErrorAction SilentlyContinue
    }
    else {
        Write-Host "Kept your settings and artwork cache. -Purge removes those too."
    }
}


########################################
# Done
########################################

Write-Host ""
Write-Host "=========================================="
Write-Host " PC GamePak removed"
Write-Host "=========================================="
Write-Host ""