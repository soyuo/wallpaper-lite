# Phase 3 Video Playback Acceptance

**Result:** Passed on the available Windows 11 system. The original one-hour soak target was reduced to a 15-minute minimum at the user's direction on 2026-11-20; the completed run lasted 24 minutes. Windows 10 remains unverified and is tracked by [blocked issue #6](https://github.com/soyuo/wallpaper-lite/issues/6).

## Environment

- Windows 11 Education, version 10.0.26300, x64 (build 26300)
- Intel Arc 140T GPU (8 GB), driver 32.0.101.6104
- D3D11 hardware device; primary display capture was 2560×1600
- FFmpeg 7.1.1; verification date: 2026-11-20

## Acceptance results

| Criterion | Result | Evidence |
| --- | --- | --- |
| H.264 video is visible behind desktop icons and advances from decoded frames | Pass | [Desktop Duplication capture](assets/phase-3-video-desktop.png) shows the rendered video with desktop icons and taskbar visible above it. The moving 120 FPS source produced five distinct captured image checksums; the reported media clock advanced monotonically throughout the soak. |
| 30 FPS and 60 FPS caps are independent of source rate and playback speed | Pass | With a 120 FPS source, five-second Desktop Duplication streams contained 143 frames at cap 30 (28.6 FPS), 280 at cap 60 (56.0 FPS), and 144 at 2× speed with cap 30 (28.8 FPS). Each capture contained five distinct image checksums. The capture ran at about 0.82× wall-clock speed, so rates are calculated over its five-second stream interval. Playback media time advanced 6.12 and 6.09 seconds during the two cap captures, and 12.20 seconds during the 2× capture. |
| Play, Pause, Seek, Stop, and documented speed presets work at runtime | Pass | IPC accepted 25%, 50%, 75%, 100%, 125%, 150%, and 200%. Pause held media time unchanged; seek set the paused timeline to 5 seconds; Play advanced from that position; Stop reset it to zero. |
| Continuous playback remains stable for the user-approved soak duration | Pass | A 24-minute run at 0.5× and a 30 FPS cap produced 25 one-minute samples. All 25 reported `Playing`; media time advanced from 1.012 to 721.423 seconds. Private memory was 114.92–123.37 MB overall and 121.35–123.37 MB after the first-minute warm-up; working set was 40.21–130.26 MB; handles 569–582; threads 17–23. No playback `last_error` or host `ERROR` log entry occurred during the run. |
| Frame queues remain bounded under decoder backpressure | Pass | The source frame channel and scheduler target are each three frames. The source worker retains at most one pending frame and does not read another sample while the channel is full. `source_worker_waits_for_frame_queue_capacity_before_reading_to_eos` verifies that EOS is not reported while queued frames remain unconsumed. |
| Windows 10 results are not claimed | Pass | No Windows 10 runtime was available; issue #6 remains blocked. |

## Reproduction notes

The FPS probe was a 60-second, 320×180 H.264 Constrained Baseline video encoded at 120 FPS with five alternating grayscale levels. Desktop output was sampled with FFmpeg Desktop Duplication (`ddagrab`, `dup_frames=false`, `hwdownload`, `showinfo`) while the host played the source. Frame rates above are captured frame count divided by the five-second capture-stream interval; Desktop Duplication took about 6.1 seconds of wall time per capture.

The soak source was a 60-minute, 320×180, 30 FPS H.264 Constrained Baseline clip generated with FFmpeg 7.1.1. The host loaded it through protocol v3, then played at 50% speed and a 30 FPS cap so it remained within the source duration for the entire 24-minute run. The monitor recorded playback state, media time, process working set/private bytes, handle count, and thread count every 60 seconds. The final sample was collected before the user-directed stop; the host process and monitor were then stopped and confirmed absent.

Commands used for code verification:

```powershell
cargo +stable test -p wallpaper-host --target x86_64-pc-windows-msvc -- --test-threads=1
cargo +stable clippy -p wallpaper-host --target x86_64-pc-windows-msvc -- -D warnings
cargo +stable build -p wallpaper-host --target x86_64-pc-windows-msvc
cargo +stable fmt --all -- --check
git diff --check
```

All listed commands passed; the host test suite passed 6 tests.

## Limitations

- This report covers Windows 11 only; Windows 10 remains blocked by issue #6.
- The one-hour target was explicitly shortened to a 15-minute minimum by the user; the actual soak exceeded that minimum at 24 minutes.
- Runtime queue depth is not exposed as a telemetry counter. Queue bounds and full-queue behavior are verified by the channel configuration and regression test.
- Video playback is silent; this test makes no audio or A/V synchronization claim.
- The soak process was force-stopped after the final sample; it does not claim graceful host-shutdown behavior.
