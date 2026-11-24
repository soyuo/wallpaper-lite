#Requires -Version 5.1
<#
.SYNOPSIS
  Builds the custom wallpaper Lite setup program (apps/installer) with the app and host embedded.

.DESCRIPTION
  Builds wallpaper-host.exe and wallpaper-ui.exe in release mode, copies both into
  apps\installer\payload, then builds wallpaper-setup in release mode. The result is one
  exe that installs, upgrades and (copied as uninstall.exe) uninstalls the app.
  The NSIS flow in build-release.ps1 is unchanged.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$triple = 'x86_64-pc-windows-msvc'
$payload = Join-Path $root 'apps\installer\payload'

function Invoke-Checked([string]$Description, [scriptblock]$Command) {
    Write-Host "==> $Description"
    & $Command
    if ($LASTEXITCODE -ne 0) { throw "$Description failed with exit code $LASTEXITCODE." }
}

Push-Location $root
try {
    Invoke-Checked 'Build wallpaper-host (release)' {
        cargo +stable build --release --locked -p wallpaper-host --target $triple
    }

    Push-Location (Join-Path $root 'apps\ui')
    try {
        Invoke-Checked 'Install UI dependencies' { npm ci }
        # --no-bundle builds wallpaper-ui.exe with the UI embedded, without the NSIS installer.
        Invoke-Checked 'Build wallpaper-ui (release)' { npm run tauri build -- --target $triple --no-bundle }
    } finally {
        Pop-Location
    }

    New-Item -ItemType Directory -Force $payload | Out-Null
    Copy-Item -Force (Join-Path $root "target\$triple\release\wallpaper-host.exe") (Join-Path $payload 'wallpaper-host.exe')
    Copy-Item -Force (Join-Path $root "target\$triple\release\wallpaper-ui.exe") (Join-Path $payload 'wallpaper-ui.exe')

    Invoke-Checked 'Build wallpaper-setup (release)' {
        cargo +stable build --release --locked -p wallpaper-setup --target $triple
    }

    $setup = Get-Item (Join-Path $root "target\$triple\release\wallpaper-setup.exe")
    Write-Host "Setup:   $($setup.FullName)"
    Write-Host ("SHA-256: " + (Get-FileHash $setup.FullName -Algorithm SHA256).Hash)
} finally {
    Pop-Location
}
