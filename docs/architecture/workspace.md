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
| `apps/ui` | Tauri 2 + React/TypeScript application; added in issue #8 | Tauri backend and host through IPC |

The intended dependency direction is:

```text
apps/ui -> Tauri backend -> ipc -> core
host -> desktop -> core
host -> renderer -> core
host -> ipc -> core
```

The host owns process startup and coordinates the desktop, renderer, and IPC
crates. The UI does not own Win32 windows or D3D11 resources. Lower-level crates
must not depend on `host`, and the dependency graph must not contain cycles.

## Deferred boundaries

Media decoding, display topology, performance policy, shaders, and product
features remain in their later roadmap phases. Their intended boundaries are
`crates/media`, `crates/display`, `crates/performance`, and `shaders`; this
workspace only reserves the architecture and does not create empty crates for
future work.
