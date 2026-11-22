# Phase 7 Battery Policy Acceptance

## Result

Battery FPS limiting is implemented and was exercised on a Windows 11 system that was running on battery. The battery cap remained temporary: host playback state reported the effective cap rather than replacing the configured target FPS. Physical AC reconnection was not available during this run; the AC restore path is covered by automated signal and per-target policy tests.

## Environment

- Windows 11 Education, 26H2, build 26300.9974, x64
- Intel Arc 140T, 8 GB, driver 32.0.101.6104
- `GetSystemPowerStatus`: `ACLineStatus=0`, `BatteryFlag=1`, `BatteryLifePercent=87`
- Test video: `crates/media/tests/fixtures/short-h264.mp4`, SHA-256 `7A793A05AC1685CF774F2D1BE2BBBA0F354948DA4408196871BEAC1625315078`

## Live battery run

Using an isolated `%APPDATA%`/`%LOCALAPPDATA%`, the test started `wallpaper-host.exe` with a schema-v1 policy containing `batteryAction=continue` and `batteryFpsLimit=30`. Host IPC acknowledged `ApplyPerformancePolicySettings`; a short H.264 video was loaded and played. The host logged `battery_fps_limit_changed` with `on_battery=true`, `configured_fps=60`, `battery_fps_limit=30`, and `effective_fps=30`. Playback state reported `fps_limit=30` with no `last_error`. After sending `SetFps(45)`, playback state continued reporting the temporary effective limit of 30 FPS. The configured value was not rewritten by the battery cap.

The smoke used the host’s length-prefixed protocol directly, rather than clicking the UI. The policy controls are covered by the production TypeScript build; the actual unplug/replug transition was not performed.

## Automated verification

- `cargo +stable test --workspace --target x86_64-pc-windows-msvc -- --test-threads=1 --quiet`: 208 tests passed.
- `cargo +stable build -p wallpaper-host --target x86_64-pc-windows-msvc`: passed.
- Core tests cover legacy schema-v1 files without `batteryFpsLimit`, invalid bounds and battery-action conflicts, and cap activation only while on battery.
- Store tests cover persistence and preservation of the previous valid file after invalid updates.
- Host tests cover unknown and transient power samples, rapid battery state changes, temporary target-local caps, and restoring each target’s exact configured FPS after the cap clears.
- `npm run test:editor`, `npm run test:displays`, `npm run test:playback`, and `npm run build`: passed (5, 6, 6 tests and production build).
- Workspace Windows-target check and host all-target Clippy with warnings denied: passed; formatting and diff checks passed.

## Limits

This run did not physically reconnect AC power, test Windows 10, or perform a multi-display battery transition. Unknown battery state fails open; after a known reading, the monitor retains the last known value through transient sampling failures. Multi-target behavior is covered by per-actor policy tests, not by a physical multi-display runtime.
