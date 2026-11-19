# Desktop wallpaper window attachment

## Parent and z-order

`DesktopHost::attach_wallpaper_window` uses the freshly validated Progman/WorkerW selected by
[`desktop-host-discovery.md`](desktop-host-discovery.md). It creates a process-owned `WS_CHILD`
window whose parent is that shell window, sizes it to the parent's client bounds, and places it at
`HWND_BOTTOM` in the child z-order. The validated `SHELLDLL_DefView` and its icon list remain above
the wallpaper child. The taskbar remains a separate shell window. No undocumented Explorer message,
top-level overlay, or `SetParent` reparenting is used.

Microsoft documents that `CreateWindowExW` creates a child when `WS_CHILD` and a parent HWND are
provided, and that `SetWindowPos` with `HWND_BOTTOM` moves a window to the bottom of its z-order:
[CreateWindowExW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-createwindowexw),
[window features](https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features), and
[SetWindowPos](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowpos).

The child uses `WS_EX_NOACTIVATE`, `SWP_NOACTIVATE`, and returns `MA_NOACTIVATE` for
`WM_MOUSEACTIVATE`, so creating or clicking the surface does not activate it. Its child z-order
keeps the shell icon view above it; the implementation does not rely on `WS_EX_TRANSPARENT` as an
input pass-through mechanism. The window procedure does not draw a background. The renderer and
swap chain remain later work; this issue establishes the desktop surface and ownership only.

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
the owner for the lifetime of its IPC server. Explorer restart recovery remains issue #3/#4 scope;
per-monitor placement and hotplug remain Phase 6 scope.

## Verification

On Windows 11 Education 10.0.26300 x64, the desktop test attaches and drops the child three times,
confirms it has no lower-z-order sibling, validates the parent relationship, confirms the
foreground window did not change, and confirms the HWND is destroyed on drop. The test runs with:

```powershell
cargo +1.85.0 test -p wallpaper-desktop --target x86_64-pc-windows-msvc -- --test-threads=1
```

The separate `wallpaper-host.exe` smoke check on the same Windows 11 machine logged successful
Progman discovery, D3D11 initialization (`feature_level=0xB000`), and wallpaper-window attachment.
While it was running, a desktop screenshot showed the existing wallpaper and icons without a blank
or obstructed area. A mouse click selected the Recycle Bin icon and left Explorer in the foreground.
Toggling the taskbar Start button opened the Start UI; Escape dismissed it, and the wallpaper host
remained alive without taking foreground focus. Closing the attached child and then stopping the
host left no wallpaper child in the shell tree. Windows 10 runtime verification remains pending;
the available machine was Windows 11 Education 10.0.26300 x64 with Intel Arc 140T graphics.
