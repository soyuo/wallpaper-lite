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
installs under `%LOCALAPPDATA%\wallpaper Lite` and needs no administrator rights.
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

## Product name

The product is called **wallpaper Lite**. It is set once, as `productName` in
`apps/ui/src-tauri/tauri.conf.json`, and Tauri uses it for the installer file name,
the install folder, the Start menu shortcut and the entry in Apps & features. The
window title, sidebar, tray tooltip and messages use the same name.

The app identifier is `kr.soyuo.wallpaper` (reverse domain of soyuo.kr), set as
`identifier` in `tauri.conf.json`. Tauri derives from it the WebView2 data folder and
the single-instance lock. According to the installer script Tauri generates (read, not
run), the installer's "Delete the application data" option removes
`%APPDATA%\kr.soyuo.wallpaper` and `%LOCALAPPDATA%\kr.soyuo.wallpaper`. The app and
host also write their logs to `%LOCALAPPDATA%\kr.soyuo.wallpaper\logs`, and the host's
single-instance mutex is `Local\kr.soyuo.wallpaper.host`. The host also takes the old
mutex name `Local\com.soyuo.wallpaper-lite.host`, so a host from a build with the
earlier identifier `com.soyuo.wallpaper-lite` is still noticed and a second host does
not start next to it; that second name can be dropped once no such build is expected to
be running. Such a build leaves its logs and WebView data under
`%LOCALAPPDATA%\com.soyuo.wallpaper-lite`. They are not moved or deleted, and nothing
removes them later, so delete that folder by hand if you do not need the old logs.
Unit tests check that the log folder and mutex names match `identifier`.

These names keep their old spelling on purpose, so settings and the launch-at-login
entry of an existing install are still found: the settings folder
`%APPDATA%\WallpaperLite`, the `wallpaper-lite` launch-at-login value, the pipe names,
the executable names `wallpaper-ui.exe` and `wallpaper-host.exe`, and the repository
and crate names.
A copy installed under the earlier name `wallpaper-lite` is a different product to the
Tauri installer, so the installer hook removes it first (silently, keeping the user's
data) and carries its launch-at-login choice over to the new install. See
`installer-hooks.nsh`.

## What install, uninstall and upgrade do

`apps/ui/src-tauri/installer-hooks.nsh` adds these steps to Tauri's installer:

- **Install and upgrade:** stops a running `wallpaper-ui.exe` and then
  `wallpaper-host.exe` so their files can be replaced. Both are stopped with
  `taskkill /F` by image name, so a development build of either that is running
  at the same time is stopped too, and the host gets no chance to run its own
  shutdown path. Stopping it removes its wallpaper window.
- **Uninstall:** stops both processes, removes the launch-at-login value
  (`HKCU\...\Run\wallpaper-lite`) and its Task Manager approval value, then
  removes the app files. An upgrade keeps launch at login and user data. Stopping the host removes its wallpaper window, so
  Windows shows the normal desktop background again.
- **Uninstall data choice:** the uninstaller's "Delete the application data"
  option removes `%APPDATA%\kr.soyuo.wallpaper` (Tauri's default) and, by
  the hook above, `%APPDATA%\WallpaperLite`, where the library, editor settings,
  display profiles and performance policy are stored. Leaving it unticked keeps
  them for a later reinstall.
- Log files in `%LOCALAPPDATA%\kr.soyuo.wallpaper\logs` are covered by the
  same Tauri option.
- Source images and videos are never moved or deleted.

## Icon

The app, the window, the tray and the installer use the icon built from
`apps/ui/src-tauri/icons/icon.svg` (see the README for how to regenerate it).
`tauri.release.conf.json` sets `installerIcon` to the same `icon.ico`, because the
installer otherwise gets the default NSIS icon. Tauri 2 has no setting for the
uninstaller's icon, so `uninstall.exe` in the install folder keeps the default NSIS
icon. Apps & features uses the app's own icon.

## Not included

- Code signing. The installer is unsigned, so Windows SmartScreen warns on first
  run. Signing needs a certificate that this repository does not hold.
- Automatic updates.

## Verification status

Only checks that were actually run are listed. Environment: Windows 11 Education
10.0.26300, x64, release build from `scripts\build-release.ps1`.

### Run

- The script builds `wallpaper Lite_0.1.0_x64-setup.exe` (about 3.1 MB).
- Silent install (`/S /D=<temporary folder>`) exited with code 0 and installed
  `wallpaper-ui.exe`, `wallpaper-host.exe` and `uninstall.exe` side by side, wrote
  the per-user uninstall entry (publisher and version set), and did not start the
  app.
- With a launch-at-login value and a disabled Task Manager approval value present,
  silent uninstall (`/S`, no data deletion) exited with code 0 and left nothing
  behind: no install folder, no uninstall entry, no `Run` value, no
  `StartupApproved` value, no Start menu shortcut. The existing
  `%APPDATA%\WallpaperLite` data was kept.
- Installing the same installer over an existing install (the upgrade path, silent)
  exited with code 0, kept a launch-at-login value that was present, and kept the
  user data; the following uninstall then removed the value.
- Icon check on the release build: `wallpaper-ui.exe` and the installer executable
  carry the new icon (checked by reading the icon Explorer shows for each file), and
  the installed `wallpaper-ui.exe` does too. `uninstall.exe` still shows the default
  NSIS icon. The install and uninstall ran silently with exit code 0 and removed the
  Start menu shortcut and the install folder.
- Name check after the rename to wallpaper Lite. Commands, from the repository root:
  `powershell -File scripts\build-release.ps1`, then
  `& "wallpaper Lite_0.1.0_x64-setup.exe" /S` (no `/D`) and
  `& "%LOCALAPPDATA%\wallpaper Lite\uninstall.exe" /S`. The install went to
  `%LOCALAPPDATA%\wallpaper Lite`, created the Start menu shortcut
  `wallpaper Lite.lnk` and an uninstall entry whose display name is `wallpaper Lite`.
  A `wallpaper-lite` launch-at-login value set beforehand was removed by the silent
  uninstall, which also removed the folder, shortcut and uninstall entry and left the
  existing `%APPDATA%\WallpaperLite` data in place.
- Identifier check after the change to `kr.soyuo.wallpaper`: the generated NSIS script
  defines `BUNDLEID "kr.soyuo.wallpaper"`; `wallpaper-ui.exe` and `wallpaper-host.exe`
  contain the new string and not `com.soyuo.wallpaper-lite`; a silent install and
  uninstall (`/S`) exited with code 0, removed the folder, the uninstall entry and a
  `wallpaper-lite` launch-at-login value set beforehand, and kept the `%APPDATA%\WallpaperLite`
  data. That the app and host actually write their logs to the new folder, and that the
  uninstaller's "Delete the application data" option removes the new identifier's folders,
  were not run.
- Moving from the old name. A build made before the rename was installed with its own
  installer (`wallpaper-lite_0.1.0_x64-setup.exe /S`, which created the uninstall entry
  `wallpaper-lite`), and a `wallpaper-lite` launch-at-login value was set. Running the
  new installer with `/S` exited with code 0, removed the old uninstall entry, installed
  `wallpaper Lite` with its shortcut, rewrote the launch-at-login value to
  `"%LOCALAPPDATA%\wallpaper Lite\wallpaper-ui.exe" --autostart`, and kept the user
  data (file count unchanged). Uninstalling the new build then removed everything it
  had installed, the shortcut, the uninstall entry and the launch-at-login value. Not
  tested: an old install whose app or host was running at the time, and an old install
  made to a custom folder.
- Installed app run (Windows 11 Education 10.0.26300, silent install to the default
  folder, then `wallpaper-ui.exe` started from it; the screen was not captured, only
  processes, window lists, logs and the control pipe were read):
  - Starting the app started `wallpaper-host.exe` by itself; the control pipe
    `\\.\pipe\wallpaper-lite` appeared and the main window title was `wallpaper Lite`.
  - The host log recorded, in order, `startup`, `windows_initialized`,
    `desktop_host_discovered` (`host_kind="progman"`), `d3d11_device_initialized`,
    `wallpaper_window_attached`, `shell_lifecycle_monitor_started`,
    `desktop_recovery_started` and `signal_monitor_started`.
  - A second `wallpaper-host.exe` exited at once with code 3 and logged
    `already_running`. A second `wallpaper-ui.exe` exited with code 0 and one
    `wallpaper-ui.exe` stayed running.
  - A `shutdown` request sent over the pipe was answered `shutting_down`; the host then
    exited, the pipe disappeared and the log recorded `shutdown_requested`. Explorer kept
    running.
  - Stopping only the UI left the host running, as designed.
  - Starting with `--autostart` left the main window hidden (`IsWindowVisible` false), created
    the tray window (`tray_icon_app`) and started the host.
  - Logs were written to `%LOCALAPPDATA%\kr.soyuo.wallpaper\logs` (`host.<date>.log`, `ui.log`).
  - Afterwards the app was stopped, the silent uninstall exited with code 0 and left no
    process, folder or launch-at-login value, and `%APPDATA%\WallpaperLite` was unchanged.
- The generated NSIS script contains the sidecar installed as `wallpaper-host.exe`,
  its removal on uninstall, and the custom hooks.

### Not verified

- A clean Windows 10 machine, and a clean Windows 11 machine without the
  development toolchain or WebView2 already present.
- What is seen on screen: the wallpaper behind the desktop icons, icon and taskbar input,
  the tray icon and its menu, closing the window while the wallpaper keeps playing,
  quitting from the tray, the Dashboard and Settings pages, and that the desktop
  background looks restored after the host stops. Only the host's log and process state
  were read.
- Explorer being restarted while the host runs.
- Reboot or sign-in with launch at login enabled.
- Install and uninstall while `wallpaper-host.exe` and `wallpaper-ui.exe` are running.
- The interactive uninstaller's "Delete the application data" option, and an upgrade over
  an older version (only a same-version reinstall and a move from the old product name were
  run).
- The Korean installer language and the WebView2 bootstrapper download.
- Several monitors, and video playback with real media on the installed build.

These are tracked as unchecked items on the issues listed in the pull requests (#6, #98,
#134 to #137, #142 to #145, #148, #153 and #155) and must be run on real machines before a
release.
