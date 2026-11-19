# Rust workspace and crate boundaries

The workspace keeps shared domain contracts, Windows integration, rendering,
transport, and process startup in separate crates. Platform behavior should be
owned by the narrowest crate that can enforce its lifecycle and safety rules.

## Current workspace

| Path | Responsibility | Dependencies |
| --- | --- | --- |
| `crates/core` | Platform-independent domain types and shared contracts | None |
| `crates/desktop` | Windows desktop and shell integration | `core` |
| `crates/renderer` | D3D11 device and rendering boundary | `core` |
| `crates/ipc` | UI-to-host protocol and named-pipe transport | `core` |
| `host` | `wallpaper-host.exe` composition root and process lifecycle | `core`, `desktop`, `renderer`, `ipc` |
| `apps/ui` | React/TypeScript frontend | Tauri commands through the host adapter |
| `apps/ui/src-tauri` | Tauri desktop backend and UI process entry point | `ipc` |

The intended dependency direction is:

```text
apps/ui -> Tauri backend -> ipc -> core
host -> desktop -> core
host -> renderer -> core
host -> ipc -> core
```

## Windows host bootstrap

Before serving IPC, `wallpaper-host.exe` calls `wallpaper_desktop::initialize_windows`
to set per-monitor DPI awareness using the Windows 8.1+ API, then creates a
hardware D3D11 device and
immediate context through `wallpaper_renderer::D3d11Device::create_hardware`.
`D3d11Device` owns both COM interfaces; their references are released when the
host exits. The host keeps this owner alive for its process lifetime.

Initialization failures return typed crate errors, map to the shared
`wallpaper_core::error::AppError` code, and log only the failed stage and
numeric HRESULT. A failed bootstrap exits the host cleanly instead of panicking.
The renderer test `creates_and_releases_hardware_device_and_immediate_context`
smoke-checks creation and normal COM release on Windows. This bootstrap does
not create a window, swap chain, shader, or rendering pipeline.

The host owns process startup and coordinates the desktop, renderer, and IPC
crates. The UI does not own Win32 windows or D3D11 resources. Lower-level crates
must not depend on `host`, and the dependency graph must not contain cycles.

The current UI-to-host named-pipe protocol is documented in
[`ipc.md`](ipc.md).

## Deferred boundaries

Media decoding, display topology, performance policy, shaders, and product
features remain in their later roadmap phases. Their intended boundaries are
`crates/media`, `crates/display`, `crates/performance`, and `shaders`; this
workspace only reserves the architecture and does not create empty crates for
future work.
