# Phase 6 Duplicate Display Acceptance

Issue: [#94 — Duplicate one wallpaper across all displays](https://github.com/soyuo/wallpaper-lite/issues/94)

## Result

The host resolves an active Duplicate profile to one shared wallpaper ID and
creates a presentation assignment for every active display. Each target keeps
its own stored FPS override. Profile records are loaded at host startup, so a
host restart reuses the saved mode and source. The Monitor Manager UI that will
edit/apply profiles is still planned under issue #97.

Automated tests use synthetic display snapshots to verify that the primary
Duplicate profile wins source conflicts, all active targets receive that same
wallpaper ID, each target retains its own FPS override, and secondary
assignments fan out only to non-overlapping display bounds. The existing target
startup manager isolates target startup failures and joins target playback
workers before releasing their windows and renderers.

## Windows runtime evidence

- OS: Windows 11 Education 26H2, build 26300.9974, x64.
- GPU: Intel Arc 140T 8 GB, driver 32.0.101.6104.
- Active display: one physical target, 2560 × 1600 at 140% scale.
- A persisted Duplicate profile loaded at host startup. The log recorded
  `presentation_mode=Some(Duplicate)`, one active display/target, and successful
  application of its editor settings. The desktop capture below shows the
  assigned image behind the desktop icons and taskbar.
- The same saved profile was loaded again after restarting the host process.
  The prior test process was force-terminated for cleanup; this does not verify
  graceful shutdown of multiple targets.
- Source fixture SHA-256 before and after: `D1834273D59897DA619B76E1C0ADE794398AE09EFBF6E57896545FD38A4F60A8`.
- Evidence capture: [phase-6-duplicate-single-display.png](assets/phase-6-duplicate-single-display.png)

## Verification

- `cargo +stable test -p wallpaper-host --target x86_64-pc-windows-msvc independent_displays::tests:: -- --test-threads=1` — 5 passed.
- `cargo +stable build -p wallpaper-host --target x86_64-pc-windows-msvc`
  passed.
- `cargo +stable test --workspace --target x86_64-pc-windows-msvc -- --test-threads=1 --quiet`
  passed.
- `cargo +stable check --workspace --target x86_64-pc-windows-msvc` passed.
- Focused Clippy passed for host/desktop/display/media with `--all-targets` and
  for the renderer library, each with `-D warnings`; formatting and diff checks
  passed. Renderer test-target Clippy still reports existing
  `excessive_precision` and `manual_range_contains` lints in renderer tests.

## Limits

Only one physical display is available in this environment. Therefore this run
does not verify simultaneous presentation on two displays, per-display FPS
isolation during live playback, failure isolation between two physical
outputs, or release of multiple outputs when switching away from Duplicate.
The tests verify assignment fan-out and preservation of target-specific FPS
values, but they are not a substitute for two-display runtime evidence. Windows
10 was not tested. No claim of physical dual-display acceptance is made.

For final acceptance, repeat on two active extended displays: save a Duplicate
profile, confirm both show the same wallpaper after Apply and after host
restart, set distinct FPS overrides and confirm one target's cadence does not
affect the other, simulate one target failing, then switch to Independent mode
and confirm obsolete Duplicate targets are released. Hash the source media
before and after.