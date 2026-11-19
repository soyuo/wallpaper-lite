# Rust workspace and crate boundaries

The workspace keeps shared domain contracts, Windows integration, rendering,
transport, and process startup in separate crates. Platform behavior should be
owned by the narrowest crate that can enforce its lifecycle and safety rules.

## Current workspace

| Path | Responsibility | Dependencies |
| --- | --- | --- |
| `crates/core` | Platform-independent domain types and shared contracts | None |
| `crates/desktop` | Windows desktop and shell integration | `core` |
| `crates/media` | WIC still-image decoding into owned RGBA buffers | `core` |
| `crates/renderer` | D3D11 device and static-image presentation | `core`, `media` |
| `crates/ipc` | UI-to-host protocol and named-pipe transport | `core` |
| `host` | `wallpaper-host.exe` composition root and process lifecycle | `core`, `desktop`, `media`, `renderer`, `ipc` |
| `apps/ui` | React/TypeScript frontend | Tauri commands through the host adapter |
| `apps/ui/src-tauri` | Tauri desktop backend and UI process entry point | `ipc` |

The intended dependency direction is:

```text
apps/ui -> Tauri backend -> ipc -> core
host -> desktop -> core
host -> renderer -> media -> core
host -> renderer -> core
host -> media -> core
host -> ipc -> core
```

`crates/media` owns WIC still-image decoding. The renderer consumes its immutable
RGBA8 result and does not access source files.

## Windows host bootstrap

Before serving IPC, `wallpaper-host.exe` calls `wallpaper_desktop::initialize_windows`
to set per-monitor DPI awareness using the Windows 8.1+ API, then creates a
hardware D3D11 device and
immediate context through `wallpaper_renderer::D3d11Device::create_hardware`.
`D3d11Device` owns both COM interfaces; the recovery worker keeps the device alive
while it owns the desktop child and any image presentation resources.

Initialization failures return typed crate errors, map to the shared
`wallpaper_core::error::AppError` code, and log only the failed stage and
numeric HRESULT. A failed bootstrap exits the host cleanly instead of panicking.
The renderer test `creates_and_releases_hardware_device_and_immediate_context`
smoke-checks creation and normal COM release on Windows. The static image
presenter creates a swap chain and shader pipeline only when an initial image is
supplied. See [`image-presentation.md`](image-presentation.md) for its resource
lifetime, resize behavior, and startup input.

The host owns process startup and coordinates the desktop, renderer, and IPC
crates. The UI does not own Win32 windows or D3D11 resources. Lower-level crates
must not depend on `host`, and the dependency graph must not contain cycles.

The current UI-to-host named-pipe protocol is documented in
[`ipc.md`](ipc.md).

`crates/desktop` validates the shell window before the host starts its IPC server. See
[`desktop-host-discovery.md`](desktop-host-discovery.md) for the candidate checks and retry behavior.
The host then owns a message-pumped child window inside the shell background WorkerW below the
icon view; see
[`desktop-window-attachment.md`](desktop-window-attachment.md) for z-order, DPI, and cleanup behavior.
The host monitors the shell process and desktop-host handle for exit/restart transitions; see
[`shell-lifecycle-monitor.md`](shell-lifecycle-monitor.md). The recovery worker reattaches the
child window after shell restart without restarting the host process.

## Deferred boundaries

Still-image decoding is implemented in `crates/media` using Windows Imaging
Component (WIC) for PNG and JPEG. Placement modes, transform and color shaders,
display topology, and performance policy remain for later roadmap phases; their
intended boundaries include `crates/display`, `crates/performance`, and `shaders`.

See [`media-decoding.md`](media-decoding.md) for the still-image pixel contract,
resource limits, and WIC behavior.
