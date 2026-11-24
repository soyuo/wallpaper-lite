# Phase 0 Windows acceptance

Verified on 2026-11-19 against Phase 0 milestone #1.

## Environment

- OS: Microsoft Windows 11 Education, x64, version 10.0.26300 (build 26300).
- GPU: Intel(R) Arc(TM) 140T GPU (8GB).
- Driver: 32.0.101.6104, dated 2024-11-09.
- Runtime tools: Node.js 24.22.1, npm 11.19.1, Rust 1.85.0 for workspace and renderer checks, and Rust 1.97.1 stable for the Tauri development runtime.

## Results

| Milestone criterion | Result | Evidence |
| --- | --- | --- |
| GUI launches and exits normally | Pass | `npm run tauri dev` opened the `wallpaper-lite` window. Closing the window exited the Tauri dev process with exit code 0. |
| UI-to-host IPC round trip | Pass | The UI's **Check wallpaper host** action displayed `Wallpaper host connected.` when the separate host was running. After stopping the host it displayed `Wallpaper host is not running.`; restarting the host restored the successful response. |
| D3D11 device/context creation | Pass | Host logs recorded `windows_initialized` and `d3d11_device_initialized` with feature level `0xB000`. The renderer hardware device/context creation and owner-drop smoke test passed (1 test). |
| Reproducible procedure and environment documented | Pass | Commands and this machine's OS/GPU/driver details are recorded below. |

## Reproduce

From the repository root, start the separate host in one PowerShell window and wait for the `windows_initialized` and `d3d11_device_initialized` log events:

```powershell
cargo +1.85.0 run -p wallpaper-host --target x86_64-pc-windows-msvc
```

In a second PowerShell window, build and launch the UI:

```powershell
Set-Location apps/ui
npm ci
npm run build
npm run tauri dev
```

In the `wallpaper-lite` window, click **Check wallpaper host** and confirm `Wallpaper host connected.`. For the failure check, stop the host with `Stop-Process -Name wallpaper-host -Force` in another PowerShell window, click the button again, and confirm the UI reports `Wallpaper host is not running.` without hanging or closing. Restart the host, wait for its initialization log events, and confirm the button reconnects. Close the UI window and confirm the Tauri dev process exits normally.

Run the D3D11 creation/drop smoke test from the repository root:

```powershell
cargo +1.85.0 test -p wallpaper-renderer --target x86_64-pc-windows-msvc -- --test-threads=1
```

The host writes startup and renderer initialization events under `%LOCALAPPDATA%\kr.soyuo.wallpaper\logs`.

## Limitations

- Runtime verification was performed on the Windows 11 machine listed above. Windows 10 and other GPU vendors/drivers were not available for this run and remain unverified here.
- Phase 0 verifies D3D11 device/context initialization and ownership cleanup; it does not render wallpaper content. Rendering begins in Phase 2.
