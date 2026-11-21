# Phase 5 video timeline and playback acceptance

## Environment

- Windows 11 Education 26H2, build 26300.9974, x64
- Intel Arc 140T, driver 32.0.101.6104
- Isolated `%APPDATA%` and `%LOCALAPPDATA%` under `%TEMP%\wallpaper-lite-issue81-smoke`
- Video fixture: `crates/media/tests/fixtures/short-h264.mp4` (64 × 48, 1 second)
- Fixture SHA-256 after the smoke run: `7A793A05AC1685CF774F2D1BE2BBBA0F354948DA4408196871BEAC1625315078`
- Windows 10 runtime verification was unavailable.

## Playback UI and persistence

The Tauri UI listed the isolated video catalog entry. **Play on desktop** loaded
it through the host and displayed authoritative playback state, duration,
speed, FPS, timeline, frame-step buttons, and loop controls. The source
reported a rational 30 FPS rate; seeking from `00:00:00.300` and stepping one
frame advanced the displayed media time to approximately `00:00:00.333`.

Entering malformed timecode `1:2` and pressing Enter showed the format error.
Entering `00:00:00.500` and pressing Enter issued a seek; the next UI state was
approximately `00:00:00.525` while playback continued.

The UI rejected a 400 ms segment (`00:00:00.200`–`00:00:00.600`) with a clear
minimum-duration message. A 500 ms segment (`00:00:00.200`–`00:00:00.700`)
was accepted. Speed 50% and FPS cap 30 were applied and saved. The resulting
per-wallpaper settings file contained the segment bounds, speed, and FPS.
The accessible segment range controls were adjusted to 250 ms and 800 ms;
their paired time fields updated, and Apply persisted the new bounds.
While playing, sampled media times wrapped from about 0.63 seconds back to
0.38 seconds and remained in `Playing` state.

After restarting the Tauri UI while leaving the host active, the panel restored
the active `Playing` state and the saved segment, 50% speed, and 30 FPS cap.
When the isolated host was stopped, the UI surfaced `Wallpaper host is not
running.` The media fixture remained tracked and unmodified in the repository.

## Automated checks

- Windows-target workspace tests: 155 passed across core, desktop, host, IPC,
  media, renderer, and UI crates.
- `cargo +stable check --workspace --target x86_64-pc-windows-msvc`: passed.
- Targeted workspace Clippy with `-D warnings`: passed.
- `cargo +stable fmt --all -- --check` and `git diff --check`: passed.
- `npm run test:playback`: 6 passed.
- `npm run test:editor`: 5 passed.
- `npm run build`: passed.

Node tests cover timecode parsing, segment-duration validation, keyboard
timeline commit keys, segment-handle changes and time-field validation,
failed-command results, and rational source-frame-step calculations.
Browser-native range, select, button, and text-entry controls remain
keyboard-operable; the smoke run also exercised Enter on the timecode field.
