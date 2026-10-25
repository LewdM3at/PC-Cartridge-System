# PC GamePak - Safe Eject Helper
# Usage: eject.ps1 [-DriveLetter X]
# Safely unmounts a drive volume and flushes write caches before physical removal.

param(
    [string]$DriveLetter
)


function Invoke-SafeEject {
    param(
        [string]$Letter
    )

    # Normalise: accept "D", "D:", "D:\" - we need the single letter.
    $Letter = $Letter.TrimEnd('\').TrimEnd(':').ToUpper()

    if ($Letter.Length -ne 1 -or $Letter -notmatch '[A-Z]') {
        Write-Host "Invalid drive letter: $Letter" -ForegroundColor Red
        return $false
    }

    $DrivePath = "${Letter}:"

    Write-Host "Preparing to eject $DrivePath ..."

    # --- Primary method: Win32_Volume.Dismount ---
    try {

        $vol = Get-WmiObject `
            -Class Win32_Volume `
            -Filter "DriveLetter='${DrivePath}'" `
            -ErrorAction Stop

        if ($null -eq $vol) {
            Write-Host "No volume found for $DrivePath" -ForegroundColor Yellow
            return $false
        }

        # Flush ($true = Force, $false = do not permanently remove)
        $result = $vol.Dismount($true, $false)

        if ($result.ReturnValue -eq 0) {
            Write-Host "Volume $DrivePath safely ejected." -ForegroundColor Green
            return $true
        }
        else {
            Write-Host "Win32_Volume.Dismount returned $($result.ReturnValue). Trying fallback..." -ForegroundColor Yellow
        }

    }
    catch {
        Write-Host "Win32_Volume method failed: $($_.Exception.Message)" -ForegroundColor Yellow
        Write-Host "Trying fallback..."
    }

    # --- Fallback: mountvol /P ---
    try {

        $mountvol = Start-Process `
            -FilePath "mountvol.exe" `
            -ArgumentList "${DrivePath} /P" `
            -Wait `
            -PassThru `
            -WindowStyle Hidden

        if ($mountvol.ExitCode -eq 0) {
            Write-Host "Volume $DrivePath safely ejected via mountvol." -ForegroundColor Green
            return $true
        }
        else {
            Write-Host "mountvol exited with code $($mountvol.ExitCode)." -ForegroundColor Red
        }

    }
    catch {
        Write-Host "mountvol fallback failed: $($_.Exception.Message)" -ForegroundColor Red
    }

    return $false
}


# ---- Interactive mode when run directly ----

if ([string]::IsNullOrWhiteSpace($DriveLetter)) {

    Write-Host ""
    Write-Host "  Safely eject a PC GamePak cartridge" -ForegroundColor Cyan
    Write-Host ""

    $DriveLetter = Read-Host "  Enter drive letter to eject (e.g. D)"

}

Invoke-SafeEject -Letter $DriveLetter
