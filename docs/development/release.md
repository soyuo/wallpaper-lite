# Release build and installer

This document describes how to build the Windows installer and what it does.
Results of running the installer on real Windows machines are recorded in
[Verification status](#verification-status); anything not listed there has not
been verified.

## What the installer contains

- `wallpaper-ui.exe`, the Tauri settings app.
- `wallpaper-host.exe`, the wallpaper host, installed in the same folder. The app
  looks for the host next to its own executable (`host_process.rs`), so the two
  must stay together.
- The Microsoft Edge WebView2 bootstrapper, downloaded by the installer when the
  runtime is missing (Tauri's default `downloadBootstrapper` mode).

The installer is a per-user NSIS installer (`installMode: currentUser`). It
installs under `%LOCALAPPDATA%\wallpaper-lite` and needs no administrator rights.
It offers English and Korean based on the system language.

## Build

Requirements are the same as for development (see [setup.md](setup.md)), plus
network access on the first build so Tauri can download the NSIS toolset.

From the repository root:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build-release.ps1
```

The script:

1. Builds `wallpaper-host` in release mode for `x86_64-pc-windows-msvc`.
2. Copies it to `apps/ui/src-tauri/binaries/wallpaper-host-x86_64-pc-windows-msvc.exe`
   (git-ignored). Tauri strips the target suffix when it installs the sidecar.
3. Runs `npm ci` and `npm run tauri build` with
   `apps/ui/src-tauri/tauri.release.conf.json`.
4. Prints the installer path and its SHA-256.

The installer is written to `target\x86_64-pc-windows-msvc\release\bundle\nsis`.

The bundle settings live in `tauri.release.conf.json` instead of `tauri.conf.json`
because `externalBin` requires the staged host file to exist, which would break
`cargo build` and `npm run tauri dev` for contributors who have not built a
release. Development builds are unchanged.

## What install, uninstall and upgrade do

`apps/ui/src-tauri/installer-hooks.nsh` adds these steps to Tauri's installer:

- **Install and upgrade:** stops a running `wallpaper-host.exe` and
  `wallpaper-ui.exe` so their files can be replaced.
- **Uninstall:** stops both processes, removes the launch-at-login value
  (`HKCU\...\Run\wallpaper-lite`) and its Task Manager approval value, then
  removes the app files. Stopping the host removes its wallpaper window, so
  Windows shows the normal desktop background again.
- **Uninstall data choice:** the uninstaller's "Delete the application data"
  option removes `%APPDATA%\com.soyuo.wallpaper-lite` (Tauri's default) and, by
  the hook above, `%APPDATA%\WallpaperLite`, where the library, editor settings,
  display profiles and performance policy are stored. Leaving it unticked keeps
  them for a later reinstall.
- Log files in `%LOCALAPPDATA%\com.soyuo.wallpaper-lite\logs` are covered by the
  same Tauri option.
- Source images and videos are never moved or deleted.

## Not included

- Code signing. The installer is unsigned, so Windows SmartScreen warns on first
  run. Signing needs a certificate that this repository does not hold.
- Automatic updates.

## Verification status

Only checks that were actually run are listed. Environment: Windows 11 Education
10.0.26300, x64, release build from `scriptsuild-release.ps1`.

### Run

- The script builds `wallpaper-lite_0.1.0_x64-setup.exe` (about 3.1 MB).
- Silent install (`/S /D=<temporary folder>`) exited with code 0 and installed
  `wallpaper-ui.exe`, `wallpaper-host.exe` and `uninstall.exe` side by side, wrote
  the per-user uninstall entry (publisher and version set), and did not start the
  app.
- With a launch-at-login value and a disabled Task Manager approval value present,
  silent uninstall (`/S`, no data deletion) exited with code 0 and left nothing
  behind: no install folder, no uninstall entry, no `Run` value, no
  `StartupApproved` value, no Start menu shortcut. The existing
  `%APPDATA%\WallpaperLite` data was kept.
- The generated NSIS script contains the sidecar installed as `wallpaper-host.exe`,
  its removal on uninstall, and the custom hooks.

### Not verified

- A clean Windows 10 machine, and a clean Windows 11 machine without the
  development toolchain or WebView2 already present.
- Install and uninstall while `wallpaper-host.exe` and `wallpaper-ui.exe` are
  running, including that the desktop background returns after the host is stopped.
- The interactive uninstaller's "Delete the application data" option removing
  `%APPDATA%\WallpaperLite`, and an upgrade over an older installation keeping it.
- Launching the installed app, reboot with launch at login enabled, and the tray
  on the installed build.
- The Korean installer language and the WebView2 bootstrapper download.

These are tracked as unchecked items on issue #137 and must be run on real
machines before a release.
