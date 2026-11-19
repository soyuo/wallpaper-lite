# Phase 1 desktop host acceptance

## Status

Some Windows 11 checks passed on the environment below, but issue #6 is not complete. Windows 10
runtime checks remain blocked because no Windows 10 machine or VM is available. The current host
also has no image/video renderer, and this session could not expose the Start button for a physical
taskbar click. Keep issue #6 open until the outstanding checks run or are explicitly re-scoped.

## Test environment

- OS: Microsoft Windows 11 Education, 26H2, version 10.0.26300, build 26300.9974, x64.
- GPU: Intel Arc 140T GPU (8GB), driver 32.0.101.6104.
- Windows 10: not tested. The configured local VM list is empty; the available ISO is Windows 11.

## Acceptance results

| Criterion | Result | Evidence |
| --- | --- | --- |
| Wallpaper content is displayed behind the icons | Partial on Windows 11 | The shell child was placed below the icon view, and the existing Windows desktop background remained visible without a blank or black obstruction. This verifies layering only; it does not verify wallpaper-lite image/video rendering, which is not implemented in this phase. See [host discovery](../architecture/desktop-host-discovery.md) and [window attachment](../architecture/desktop-window-attachment.md). |
| Icon and taskbar input still work | Partial on Windows 11 | Clicking Recycle Bin selected it in Explorer. The Start button Toggle pattern opened Start and Escape dismissed it, but that only verifies the UI Automation control. A physical taskbar click was not verified: the Start button reported `IsOffscreen=true` with an empty bounds in this session. |
| Explorer restarts without losing the host window | Pass on Windows 11 | Explorer PID 8964 was replaced by PID 28784 while host PID 30908 remained alive. The host logged `wallpaper_window_recovered`; UI Automation found exactly one wallpaper child under `Progman`, owned by PID 30908. See [lifecycle monitor](../architecture/shell-lifecycle-monitor.md). |
| App exit releases the host window and owned resources | Partial on Windows 11 | The recovery worker unit test queues exit/restart events, calls `stop()` twice, and joins successfully. In the duplicate-host check, PID 2704 exited through the IPC server error path (`ERROR_PIPE_BUSY`, OS code 231); afterward the only child belonged to primary PID 5072. The primary was force-terminated after that measurement, so orderly primary shutdown cleanup and a live Explorer exit concurrent with shutdown remain unverified. |
| Windows 10 and 11 results and manual procedures are recorded | Partial | Windows 11 results and the checklist below are recorded. Windows 10 remains unverified. |

The visual result above verifies only that the new child window does not cover the existing Windows
desktop background. Phase 1's child window has no wallpaper renderer, so showing a selected image
or video is not yet testable and belongs to the later rendering phase.

## Automated regression checks

Run from the repository root in a Windows x64 developer shell:

```powershell
cargo +stable test -p wallpaper-desktop -p wallpaper-host --target x86_64-pc-windows-msvc -- --test-threads=1
cargo +stable check -p wallpaper-host --target x86_64-pc-windows-msvc
cargo +stable clippy -p wallpaper-host --target x86_64-pc-windows-msvc -- -D warnings
cargo +stable fmt --all -- --check
```

Results on the test environment: 9 desktop tests and 2 host tests passed; check, Clippy, and
formatting passed. The desktop tests cover host selection and shell lifecycle transitions. Host
tests cover recovery retry backoff and idempotent shutdown.

## Manual Windows checklist

1. Build and launch the host:

   ```powershell
   cargo +stable build -p wallpaper-host --target x86_64-pc-windows-msvc
   Start-Process -FilePath .\target\x86_64-pc-windows-msvc\debug\wallpaper-host.exe -WindowStyle Hidden
   ```

2. In Windows UI Automation or Inspect, confirm `WallpaperLiteDesktopChild` is below the shell
   icon view under `Progman`. Confirm the existing desktop background and icons remain visible.
3. Click an icon and confirm Explorer handles the input. Physically click Start on the taskbar,
   dismiss it with Escape, and confirm the host does not take focus. The UI Automation Toggle check
   is supplementary and does not replace this physical input check.
4. Save work and close open File Explorer windows. In Task Manager, select **Windows Explorer** and
   choose **Restart**. Wait for the taskbar to return and for Explorer to have a new PID. Confirm
   `shell_exited`, `shell_restarted`, and `wallpaper_window_recovered` events in
   `%LOCALAPPDATA%\com.soyuo.wallpaper-lite\logs\`. Confirm exactly one wallpaper child exists
   under the current `Progman` and is owned by the host.
5. With one host already active, launch a second instance to exercise its normal IPC failure exit.
   Confirm the second process exits and only the primary host's wallpaper child remains. Run the
   worker shutdown unit test to verify repeated `stop()` calls. Do not treat force-terminating the
   primary process as proof of Rust cleanup; it is only final test cleanup.
6. Repeat the checklist and automated commands on Windows 10 before closing issue #6 or marking
   Phase 1 complete.

## Remaining verification

- A Windows 10 desktop runtime is required to complete the cross-version acceptance.
- A live Explorer termination concurrent with host shutdown has not been exercised. The queued
  lifecycle-event shutdown test covers the worker race path, but does not replace that live check.
