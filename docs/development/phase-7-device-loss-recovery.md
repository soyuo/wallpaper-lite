# Phase 7 D3D device-loss recovery

## Result

The host distinguishes common D3D11/DXGI device removal, reset, hung, and
driver-internal HRESULTs from media and ordinary renderer errors. Recovery is
owned by the affected wallpaper target: it drops that target's presenter and
swap-chain resources, recreates the D3D11 device, and restores the last
committed image or video frame, Span bounds, and source-matched editor
settings on the existing child HWND. It does not create another desktop
window. Retries use 100 ms exponential backoff capped at 1 second and stop
after eight attempts. Exhaustion is recorded as a structured
`wallpaper_device_recovery_exhausted` target-local error; the failed target
stays quiescent instead of restarting retry cycles on each incoming frame.

## Automated verification

Windows-target tests cover HRESULT classification, bounded retry scheduling,
restoration of image and video state on the same HWND, deferred recovery while
rendering is paused, and preservation of the playback actor after a transient
device-loss error. The recovery test injects `DXGI_ERROR_DEVICE_REMOVED` into
the worker's recovery path and then uses the available hardware to recreate
the device. This is a controlled software test; it does not remove or reset the
physical GPU.

```powershell
cargo +stable test -p wallpaper-renderer -p wallpaper-host --target x86_64-pc-windows-msvc -- --test-threads=1 --quiet
```

The regression checks assert that the original child HWND remains the sole
window, the last committed content and settings are restored, and retry
scheduling has a finite limit.

## Environment and limits

- Windows 11 Education, build 26300, x64
- Intel Arc 140T, 8 GB
- D3D11 hardware device and interactive Explorer desktop available to tests
- Windows 10 was not available for runtime verification.
- A physical driver/device fault was not induced; hardware recovery is verified
  through an injected removal error and successful device recreation.
- Terminal recovery exhaustion is validated through bounded retry state and
  structured target-local logging, not by disabling the physical GPU.

This report does not claim that Media Foundation's video decoder device is
recreated. It records restoration of the last committed video frame and the
playback actor's behavior while the renderer recovers.
