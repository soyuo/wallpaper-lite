# Rust workspace and crate boundaries

The workspace keeps shared domain contracts, Windows integration, rendering,
transport, and process startup in separate crates. Platform behavior should be
owned by the narrowest crate that can enforce its lifecycle and safety rules.

## Current workspace

| Path | Responsibility | Dependencies |
| --- | --- | --- |
| `crates/core` | Platform-independent domain types and shared contracts | None |
| `crates/desktop` | Windows desktop and shell integration | `core` |
| `crates/media` | WIC image decoding, Media Foundation video-source decoding, local catalog/import, and bounded playback queues | `core` |
| `crates/renderer` | Video-capable D3D11 device and static-image presentation | `core`, `media` |
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

`crates/media` owns WIC still-image decoding, Media Foundation video decoding,
the bounded decoded-frame queue, and PTS-based playback scheduling. The renderer
consumes decoded RGBA8 images or timestamped video frames and does not access
source files. See [`video-playback.md`](video-playback.md) for clock and
transport semantics.

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
smoke-checks creation and normal COM release on Windows. The image presenter
creates a swap chain and shader pipeline once; successive same-sized video
frames reuse the source texture, while a dimension change replaces only the
source-sized resources. See [`image-presentation.md`](image-presentation.md)
for its resource lifetime, resize behavior, and startup input.

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
Component (WIC) for PNG, JPEG, and BMP. Image placement, transform, and color
adjustments are implemented in `crates/renderer`. Media Foundation opens H.264
sources and decodes timestamped RGBA8 frames, with software decoding and
D3D11/DXVA-preferred paths. The bounded frame queue, playback scheduler, and
D3D11 video-frame presentation are implemented. Display topology and
performance policy remain later boundaries in `crates/display` and
`crates/performance`.

See [`media-decoding.md`](media-decoding.md) for the still-image pixel contract,
resource limits, and WIC behavior. See [`video-source.md`](video-source.md)
for the Media Foundation source-reader, frame-timing, GPU interop, and lifetime contracts. See
[`video-frame-queue.md`](video-frame-queue.md) for queue ordering, capacity, and late-frame policy.
See [`media-library.md`](media-library.md) for the local catalog schema,
content-validated import, library browsing/search, metadata, missing-file
actions, and bounded cached thumbnails.
Per-wallpaper editor defaults, units, validation, and persistence are described
in [`editor-settings.md`](editor-settings.md). The monitor preview, D3D11
parity contract, placement/crop/color controls, and bounded edit history are
described in [`editor-preview.md`](editor-preview.md); transactional apply and
live application are implemented. Per-wallpaper presets and persisted recent
wallpaper ordering use the same validated settings snapshot and are documented
in [`editor-settings.md`](editor-settings.md).
Per-wallpaper playback preference defaults, bounds, and atomic persistence are
described in [`playback-settings.md`](playback-settings.md). Host runtime
settings support Disabled, Full, and Segment video looping; timeline UI and
per-wallpaper playback controls, timecode seeking, rational-frame stepping,
and Windows Media Foundation audio output are implemented. See
[`video-playback.md`](video-playback.md) for clock synchronization, mute policy,
loop generation behavior, and UI-to-host control flow. Windows 11 timeline and
settings verification is recorded in
[`../development/phase-5-playback-acceptance.md`](../development/phase-5-playback-acceptance.md).
Windows 11 audio, mute, and silent-video verification is recorded in
[`../development/phase-5-audio-acceptance.md`](../development/phase-5-audio-acceptance.md).
Windows 11 segment-loop boundary verification is recorded in
[`../development/phase-5-segment-loop-acceptance.md`](../development/phase-5-segment-loop-acceptance.md).
The integrated Phase 5 verification is summarized in
[`../development/phase-5-advanced-playback-acceptance.md`](../development/phase-5-advanced-playback-acceptance.md).
