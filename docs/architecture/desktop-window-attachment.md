# Desktop wallpaper window attachment

## Parent and z-order

`DesktopHost::attach_wallpaper_window` validates the Progman/WorkerW shell window selected by
[`desktop-host-discovery.md`](desktop-host-discovery.md), then finds its visible, shell-owned,
full-client-size `WorkerW` child below `SHELLDLL_DefView` in sibling z-order. The wallpaper surface
is created directly inside this background layer and placed at `HWND_BOTTOM`. The shell icon view
remains above it, and the taskbar remains a separate shell window. If the shell does not expose a
matching background layer, attachment fails instead of putting rendered pixels over desktop icons.
When Explorer has no shell-owned `WorkerW` child, the host requests the background layer with
Explorer's private `0x052C` message (`wParam = 13`, `lParam = 1`) through `SendMessageTimeoutW`
with a one-second hung-window timeout. This message is undocumented and may vary across Windows
versions. The host sends it only when no same-process `WorkerW` child exists, then re-enumerates and
validates the new layer. If the layer does not appear, attachment fails closed. No top-level overlay
or runtime `SetParent` reparenting is used.

Microsoft documents that `CreateWindowExW` creates a child when `WS_CHILD` and a parent HWND are
provided, and that `SetWindowPos` with `HWND_BOTTOM` moves a window to the bottom of its z-order:
[CreateWindowExW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-createwindowexw),
[window features](https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features), and
[SetWindowPos](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowpos).

The child uses `WS_EX_NOACTIVATE`, `SWP_NOACTIVATE`, and returns `MA_NOACTIVATE` for
`WM_MOUSEACTIVATE`, so creating or clicking the surface does not activate it. `WS_EX_TRANSPARENT` is
omitted because it changes shell sibling paint ordering and can place the rendered surface over the
icon list. The window procedure does not draw a background. The renderer and swap chain are
documented in [`image-presentation.md`](image-presentation.md).

## DPI context

The host process remains per-monitor DPI aware as established during Phase 0. When the optional
Windows 10 version 1607+ APIs are available, the window thread temporarily adopts the parent
window's DPI awareness context while it calls `GetClientRect`, `CreateWindowExW`, and
`SetWindowPos`, then restores its previous context. The APIs are resolved dynamically so older
Windows 10 builds do not require imports they do not provide. Microsoft documents the cross-process
child-window DPI behavior and the thread context APIs in [High DPI Desktop Application Development](https://learn.microsoft.com/en-us/windows/win32/hidpi/high-dpi-desktop-application-development-on-windows),
[GetWindowDpiAwarenessContext](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowdpiawarenesscontext),
and [SetThreadDpiAwarenessContext](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setthreaddpiawarenesscontext).

If registration, window creation, sizing, z-ordering, or revalidation fails, the partially created
window is destroyed before the error returns. Failures map to the safe shared
`desktop_host_unavailable` code; diagnostics contain only the failure stage and numeric system
code.

## Lifetime

The HWND is created and serviced on a dedicated message-pump thread so the host's current-thread
Tokio runtime can continue serving IPC. `AttachedDesktopWindow` owns the thread and HWND. Dropping
it posts `WM_QUIT`, destroys the HWND on its owning thread, and joins that thread. The host retains
the owner for the lifetime of its IPC server. On Explorer restart, the recovery worker discovers
the replacement shell, requests its background layer if needed, then recreates the child and image
presenter. Per-monitor placement and hotplug remain Phase 6 scope.

## Verification

On Windows 11 Education 10.0.26300 x64, the desktop test attaches and drops the child three times,
confirms it has no lower-z-order sibling, validates the parent relationship, confirms the
foreground window did not change, and confirms the HWND is destroyed on drop. The test runs with:

```powershell
cargo +stable test -p wallpaper-desktop --target x86_64-pc-windows-msvc -- --test-threads=1
```

The separate `wallpaper-host.exe --image crates/media/tests/fixtures/pixels.png` smoke check on the
same Windows 11 machine attached its child to the full-screen `WorkerW` below
`SHELLDLL_DefView`. A desktop capture showed the red/blue decoded image behind all three desktop
icons. The child's extended style did not include `WS_EX_TRANSPARENT`. A single click on Recycle Bin
changed the desktop list view's selected item from `-1` to `0` while the image remained visible.
The host logged successful Progman discovery, D3D11 initialization (`feature_level=0xB000`), and
wallpaper image presentation.
Toggling the taskbar Start button opened the Start UI; Escape dismissed it, and the wallpaper host
remained alive without taking foreground focus. Closing the attached child and then stopping the
host left no wallpaper child in the shell tree. Windows 10 runtime verification remains pending;
the available machine was Windows 11 Education 10.0.26300 x64 with Intel Arc 140T graphics.

After Explorer was restarted, PID 14264 was replaced by PID 1968 while host PID 26700 stayed alive.
The host created the replacement shell's full-screen `WorkerW` (HWND 591416), attached its child
(HWND 591408) beneath that layer, and logged `wallpaper_window_recovered` followed by
`wallpaper_image_restored`. A post-recovery desktop capture showed the red test image behind all
three icons. Windows 10 behavior for the private WorkerW creation message remains unverified.
