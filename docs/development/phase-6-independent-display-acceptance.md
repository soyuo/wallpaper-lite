# Phase 6 Independent Display Acceptance

## Result

The host now resolves schema-v1 profiles by stable display identity at startup.
Primary images are presented by the existing recovery worker; primary videos
use the existing playback actor. The host starts each assigned,
non-overlapping secondary target with its own child
window, D3D11 device, recovery worker, and (for video) playback actor. Per-target
startup errors are isolated; Explorer lifecycle events are forwarded to each
recovery worker, and shutdown stops playback actors before releasing their
recovery/window resources.

The available Windows machine has one active physical display, so this run
verified startup loading for one primary assignment only. It did **not** verify
two different wallpapers simultaneously, independent FPS caps across physical
displays, or multi-target shutdown/recreation on a dual-display system. Those
acceptance checks remain open for a two-display Windows run.

## Environment and procedure

- Windows 11 Education 10.0.26300 build 26300.9974, x64.
- Intel Arc 140T 8 GB, driver 32.0.101.6104.
- One active physical display; Windows 10 and a second display were unavailable.
- Per-monitor-aware primary bounds from host startup: `(0, 0, 2560, 1600)`.
- Source fixture: `crates/media/tests/fixtures/pixels.png`.
- Source SHA-256 before and after: `D1834273D59897DA619B76E1C0ADE794398AE09EFBF6E57896545FD38A4F60A8`.

The smoke used an isolated `%APPDATA%` catalog with the fixture assigned to the
active display through a stable-ID `independent` profile. Starting
`wallpaper-host.exe` without command-line media loaded the assignment; the log
recorded successful editor-settings application. A desktop capture showed the
assigned red/blue image behind the desktop icons. The host was then stopped and
its process absence confirmed. The capture verified the primary-target path;
process termination was forced for test cleanup and is not evidence of graceful
multi-target shutdown.

## Automated checks

- Host tests cover independent profile selection with distinct wallpaper IDs
  and FPS overrides, plus rejection of overlapping/clone target geometry.
- The ignored Windows desktop integration test
  `display_child_uses_requested_screen_origin_and_extent` was run explicitly;
  `GetWindowRect` confirmed a child requested at `(120, 180)-(520, 460)` kept
  that non-origin screen position and extent.
- Existing playback and recovery tests cover bounded frame queues, playback
  worker stop, and recovery-window cleanup.
- Windows-target host tests, host Clippy with warnings denied, and host build
  passed during implementation.

The machine has only one active target. Therefore per-display FPS isolation,
failure isolation between two live renderers, and cleanup/recreation of multiple
targets are not claimed as runtime verified. No source media was modified.

## Reproducible dual-display follow-up

On a Windows 10/11 system with two active extended displays:

1. Assign a different image or video to each stable display ID in Independent
   mode; set distinct video FPS overrides.
2. Start the host and confirm both wallpapers appear on their assigned displays
   behind their icons. Capture each display while sampling its playback state.
3. Make one assignment unavailable, restart the host, and confirm the remaining
   display still renders.
4. Restart Explorer, confirm both targets recover, then stop the host gracefully
   and verify no host-owned child windows or playback workers remain.
5. Hash each source file before and after the run.

This procedure is still required to close the physical dual-display acceptance
items. Topology-change reassignment remains issue #96. Duplicate mode is tracked
in issue #94, with physical multi-display verification pending; Span remains
issue #95.
