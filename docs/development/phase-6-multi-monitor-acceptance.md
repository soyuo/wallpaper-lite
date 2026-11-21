# Phase 6 Multi-Monitor Acceptance

## Status

The Phase 6 implementation issues #91–#97 are merged. Automated and
single-display checks are recorded below. The end-to-end acceptance issue #98
remains blocked because this machine has only one active physical display.
Independent, Duplicate, and Span behavior across two or more physical displays,
per-display FPS isolation, and multi-display hotplug recovery have not been
verified here. This report records available evidence; it does not claim those
acceptance criteria passed.

## Environment

- Windows 11 Education, 10.0.26300 build 26300.9974, x64
- Intel Arc 140T, 8 GB; driver 32.0.101.6104
- One active display: 2560×1600 physical pixels, 140% scale, 134.4 effective
  DPI, approximately 144 Hz
- Windows 10 and a second physical display were unavailable
- Still-image fixture: `crates/media/tests/fixtures/pixels.png`, SHA-256
  `D1834273D59897DA619B76E1C0ADE794398AE09EFBF6E57896545FD38A4F60A`

## Checks completed

- Display enumeration returned the same path-derived stable identity across
  separate process runs. The physical enumeration test covered one connected
  display only.
- The ignored Windows child-window bounds test verified that a child can be
  placed at a non-origin rectangle `(120,180)-(520,460)` and a negative-origin
  rectangle `(-320,40)-(280,340)`. This checks coordinate conversion, not a
  second physical monitor.
- Display profile model/store tests cover stable-ID persistence, disconnected
  record retention, Independent/Duplicate/Span selection, target-specific FPS
  values, and invalid-profile rejection. Synthetic geometry tests cover Span
  placement across mixed target sizes and negative origins.
- On the available display, the Monitor Manager loaded an Independent profile
  at 45 FPS, displayed the assigned fixture behind desktop icons, and restored
  it after a host restart. An attempted update with the host unavailable did
  not report success; the stored profile remained at 45 FPS and the UI showed
  the host failure. The fixture hash remained unchanged.
- Windows-target workspace tests passed: 197 tests. Workspace check passed.
  Focused host/UI all-target Clippy and IPC library Clippy passed with
  `-D warnings`; broad IPC all-target Clippy still reports the existing
  `clippy::items-after-test-module` lint in `crates/ipc/src/error.rs`.
- UI checks passed: `npm run test:displays` (6), `npm run test:editor` (5),
  `npm run test:playback` (6), and `npm run build`. Formatting and diff checks
  passed.

## Physical multi-display verification still required

On a system with at least two active displays:

1. Record each display's stable identity, physical bounds, scale, and refresh
   rate. Assign visibly different images in Independent mode and distinct FPS
   caps; verify each target independently for at least 30 seconds.
2. Apply Duplicate mode and confirm each display shows the same source while
   using its own geometry and FPS setting.
3. Apply Span mode and verify the image is continuous across the combined
   virtual rectangle, including mixed resolutions and a display left of the
   primary origin.
4. Disconnect and reconnect a display, then change resolution and scale. Check
   that stable-ID assignments return, each target has exactly one wallpaper
   child, and failures on one target do not stop healthy targets.
5. Restart the host and Explorer. Confirm each target recovers once, then stop
   the host and confirm every target window, playback worker, and renderer is
   released. Compare source hashes before and after.

Windows 10 behavior is also unverified and is tracked separately from the
currently blocked physical multi-display run.
