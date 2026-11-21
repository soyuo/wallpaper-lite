# Phase 6 Display Manager Acceptance

## Environment and scope

- Windows 11 Education, 10.0.26300 build 26300.9974, x64
- Intel Arc 140T, 8 GB, driver 32.0.101.6104
- One active display: 2560×1600, 140% scale, 134.4 effective DPI, approximately 144 Hz
- The smoke used an isolated app-data directory and a catalog entry pointing to
  `crates/media/tests/fixtures/pixels.png`.
- No second physical display was available. Dual-display rendering and target
  isolation remain reserved for issue #98.

## Checks performed

1. Opened the Monitor Manager and confirmed the connected primary display shows
   its stable identity, runtime device name, resolution, refresh rate, scale,
   effective DPI, and primary status.
2. Selected the fixture for the display, set Independent mode and a 45 FPS
   override, and used **Save and apply**. The profile was persisted in schema 1
   and the host confirmed the assignment. The assigned image appeared behind
   the desktop icons.
3. Closed and restarted the host. It loaded the saved profile and applied the
   wallpaper settings again.
4. With the host stopped, the refreshed manager showed the `wallpaper host:
   refresh_unavailable` alert. A candidate change from 45 to 60 FPS did not
   report success: the Tauri apply command returned
   `display_profiles_rollback_failed`, and the profile file remained at 45 FPS.
   The message said the saved assignment was restored but the host could not
   confirm its prior runtime state. Restarting the host again loaded the last
   valid profile. This offline UI and rollback path was repeated after host
   result acknowledgements were added.
5. Compared the fixture before and after the smoke. SHA-256 remained
   `D1834273D59897DA619B76E1C0ADE794398AE09EFBF6E57896545FD38A4F60A8`.

The host was deliberately unavailable for the failure-path check; it was then
restarted to verify profile recovery. This is not a multi-display or graceful
shutdown test. Physical Duplicate/Span/Independent behavior, simultaneous
wallpapers, per-target FPS isolation, and target-local runtime failures are
reserved for issue #98.

## Automated coverage

The display-manager model tests cover Independent assignment and FPS validation,
Duplicate/Span primary-wallpaper fan-out, disconnected-profile retention,
unavailable media rejection, target-specific apply failures, and host refresh
failure reporting. The Windows-target
workspace test run passed 197 tests; the Explorer-window bounds test remains
ignored in the suite and was covered separately by the Phase 6 topology report.
Workspace check and targeted Clippy passed with warnings denied. Formatting and
diff checks passed. The UI suites passed: display manager 6, editor history 5,
playback controls 6; `npm run build` also passed.
