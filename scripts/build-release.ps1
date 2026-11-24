#Requires -Version 5.1
<#
.SYNOPSIS
  Builds the wallpaper Lite NSIS installer, including the wallpaper host.

.DESCRIPTION
  Builds wallpaper-host.exe in release mode, stages it as the Tauri sidecar
  (installed next to the app as wallpaper-host.exe), then builds the UI and the
  installer. The installer is written to target\x86_64-pc-windows-msvc\release\bundle\nsis.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$triple = 'x86_64-pc-windows-msvc'
$binaries = Join-Path $root 'apps\ui\src-tauri\binaries'

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

    New-Item -ItemType Directory -Force $binaries | Out-Null
    Copy-Item -Force (Join-Path $root "target\$triple\release\wallpaper-host.exe") `
        (Join-Path $binaries "wallpaper-host-$triple.exe")

    Push-Location (Join-Path $root 'apps\ui')
    try {
        Invoke-Checked 'Install UI dependencies' { npm ci }
        Invoke-Checked 'Build the installer' { npm run tauri build -- --target $triple --config src-tauri/tauri.release.conf.json }
    } finally {
        Pop-Location
    }

    $config = Get-Content (Join-Path $root 'apps\ui\src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json
    $installerPath = Join-Path $root "target\$triple\release\bundle\nsis\$($config.productName)_$($config.version)_x64-setup.exe"
    if (-not (Test-Path $installerPath)) { throw "Expected installer was not produced: $installerPath" }
    $installer = Get-Item $installerPath
    Write-Host "Installer: $($installer.FullName)"
    Write-Host ("SHA-256:   " + (Get-FileHash $installer.FullName -Algorithm SHA256).Hash)
} finally {
    Pop-Location
}
