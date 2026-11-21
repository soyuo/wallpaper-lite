# Phase 5 advanced playback acceptance

## Scope and environment

This report combines the integrated UI/host checks for settings restoration,
timeline input, transport, looping, speed/FPS controls, and audio behavior. It
records the tested samples and environment; it does not claim codec-independent
or universally seamless playback.

- Test date: 2026-11-21.
- OS: Windows 11 Education 26H2, build 26300.9974, x64.
- GPU: Intel Arc 140T 8 GB; driver 32.0.101.6104.
- Playback samples: 64×48 H.264 Baseline/Constrained Baseline clips at 30 FPS;
  a 320×180 H.264 Constrained Baseline clip at 120 FPS; and an H.264/AAC clip.
- Windows 10 runtime was unavailable. No Windows 10 result is claimed.

## Results

| Acceptance area | Observed result |
| --- | --- |
| Settings survive app and host restart | With the isolated profile under `%TEMP%\wallpaper-lite-issue81-smoke`, a fresh host and Tauri UI restored the saved Segment range 250–800 ms, 50% speed, 30 FPS cap, 100% volume, and unmuted setting. The video was loaded by **Play on desktop**. This verifies persisted settings restoration; it does not claim automatic playback restart. |
| Timeline and frame controls | The UI accepted `00:00:00.300`; stepping one frame advanced to about `00:00:00.333` for a 30 FPS source. A pointer drag moved a paused seek from 444 ms to 686 ms, and the host-reported time remained paused at the target. Malformed `1:2` was rejected. A 400 ms loop segment was rejected; a 500 ms segment was accepted. |
| Loop disabled | On the 1-second H.264 sample at 100% speed and 60 FPS cap, playback reached `Ended` in about 1.53 seconds of observation, with zero media-time wraps and no playback error. The final reported media position was about 1008 ms. |
| Full loop | On the same sample at 100% speed and 60 FPS cap, 3 media-time wraps were observed in about 3.09 seconds. All state snapshots remained `Playing`; no playback error was reported. |
| Segment loop | On the same sample with `[250, 800)` ms, 100% speed, and 60 FPS cap, 9 wraps were observed in about 5.12 seconds. All state snapshots remained `Playing`, with media time between 259 and 800 ms and no playback error. |
| Segment boundary frames | The dedicated 2-second, 64×48, 30 FPS H.264 Baseline fixture tested `[500, 1000)` ms at 200% and 60 FPS. Across 28.5 seconds it produced 111 wraps; 1,710 Desktop Duplication center-pixel samples were all classified as in-segment red content, with no black, pre-roll green, or post-segment blue samples. This is point sampling of a color-coded test clip, not a universal scanout guarantee. See [segment-loop acceptance](phase-5-segment-loop-acceptance.md). |
| Independent speed and FPS limits | With the 120 FPS H.264 sample, a 30 FPS cap yielded 143 captured frames in 5 seconds (28.6 FPS); a 60 FPS cap yielded 280 (56.0 FPS). At 200% speed with a 30 FPS cap, 144 frames were captured in 5 seconds (28.8 FPS), while media time advanced about 12.20 seconds over 6.1 seconds of wall time. See [Phase 3 video acceptance](phase-3-video-acceptance.md). |
| Audio and mute | On the H.264/AAC sample, 1× playback exposed audio; user mute was reported effective while the media clock continued. At 2× with user mute off, the UI reported forced mute while video continued. Replacing it with a silent H.264 sample reached `Ended` and showed the no-audio status. Volume/mute were verified through Media Engine state, not calibrated acoustic measurements. See [audio acceptance](phase-5-audio-acceptance.md). |
| Source preservation | SHA-256 values after the integrated checks matched the established fixture values: `short-h264.mp4` `7A793A05AC1685CF774F2D1BE2BBBA0F354948DA4408196871BEAC1625315078`; `short-h264-aac.mp4` `403C4AA856AA8401D76E8AB8601C08508CE4AC829316BB496114CBC87D9B1683`; `loop-boundary-h264.mp4` `9C423B84FDE6CE8704217AF714F0B91F8AAD56281925BC26B296165AB32712EE`. |

The loop tests polled the authoritative playback-state command, not the
displayed slider value. The boundary capture used a single center pixel from a
full-frame, color-coded fixture. Queue limits are structural: source channel
capacity 3, scheduler queue capacity 3, and at most one pending decoded frame;
runtime queue-depth telemetry is not exposed. During the separate 30-second
boundary run, private memory was 124.30–124.34 MiB, working set 128.11–128.48
MiB, handles 577–581, and threads 18–19.

## Reproduction outline

1. Use the checked-in short H.264 fixtures and an isolated app-data profile.
   Store non-default playback settings, stop both processes, then start a fresh
   host and Tauri UI against the same profile. Load the video and confirm the
   saved values before changing them.
2. Seek by entering a timecode, step by one source frame, and drag the timeline
   while paused. Confirm the host-reported position and error state.
3. Apply Disabled, Full, and Segment loop modes separately. Start each run
   from Stop and sample `playback_state` frequently enough to count media-time
   decreases; record state, source, settings, sample interval, and wall duration.
4. For the boundary check, play `loop-boundary-h264.mp4` at `[500, 1000)` ms,
   200%, and 60 FPS while sampling the desktop at 60 FPS. Its color regions
   distinguish pre-roll, eligible segment frames, and post-segment frames.
5. Exercise the 120 FPS source with 30/60 FPS caps and 200% speed, then test
   the A/V and silent fixtures for mute, forced-mute, and no-audio reporting.
   Compare source hashes before and after the run.

## Verification

- `cargo +stable test --workspace --target x86_64-pc-windows-msvc -- --test-threads=1 --quiet` — 157 passed across 17 suites.
- `cargo +stable clippy --workspace --target x86_64-pc-windows-msvc -- -D warnings` — passed.
- `cargo +stable fmt --all -- --check` and `git diff --check` — passed.
- From `apps/ui`: `npm run test:playback` — 6 passed; `npm run test:editor` — 5 passed; `npm run build` — passed.

## Limits

Only the listed H.264 Baseline/Constrained Baseline and AAC samples were tested
on this Windows 11 machine and GPU. Other codecs, GPUs, audio hardware, and
Windows 10 remain unverified. The measured boundary result is specific to the
sample, driver, and capture method; it must not be presented as a universal
seamless-loop guarantee.
