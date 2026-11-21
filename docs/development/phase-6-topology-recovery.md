# Phase 6 Topology Recovery Implementation Evidence

## Implementation

The host monitor window coalesces display, settings, and DPI notifications for
300 ms before publishing a topology revision. The main loop re-enumerates active
targets by stable display ID and reloads the persisted display profiles and
catalog. If a fresh snapshot cannot be read, it retains the last active outputs
and waits for another notification.

Primary-window geometry changes are sent to the recovery worker through its
command queue, so the IPC loop does not wait for swap-chain resize or reattach
work. Span viewport geometry is updated with the same command. Primary profile
changes are serialized separately: a new wallpaper/mode/FPS/source identity is
applied once, while bounds, DPI, refresh, or Span geometry changes alone do not
restart its decoder.

The secondary target manager reconciles stable IDs and target signatures using
keep/start/replace/remove operations. A replacement is started before the last
good target is released. Failed targets are isolated and retried on a later
topology notification; disconnected profiles remain in persistent storage.
Explorer exit/restart events are forwarded to each live recovery owner.

## Automated and local evidence

- `cargo +stable test -p wallpaper-host --target x86_64-pc-windows-msvc -- --test-threads=1`
  passed 24 tests, including stable-ID reconciliation, profile/source changes,
  output-scoped replacement, failed-replacement retention, disconnected-target
  re-addition, Explorer event forwarding, and manager shutdown.
- The ignored desktop bounds test
  `display_child_uses_requested_screen_origin_and_extent` was run explicitly.
  `GetWindowRect` verified both a non-origin location and a negative-origin
  location after moving the child window.
- The monitor-message unit test covers display, settings, and DPI notification
  classification. The debounce timer is reset for each notification and emits
  one topology revision after the quiet interval.

This Windows 11 environment has one active physical display. The code paths and
synthetic tests are present, but real hotplug, mode-change, DPI-change,
multi-display recovery, and independent output behavior have not been exercised
on multiple physical monitors. Those runtime checks belong to the Phase 6
acceptance issue #98; no Windows 10 result is claimed.
