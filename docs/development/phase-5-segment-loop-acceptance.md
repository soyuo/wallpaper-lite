# Phase 5 segment-loop boundary acceptance

## Scope

This report verifies segment-loop output for one supported H.264 fixture on the
available Windows 11 system. It does not claim universally seamless looping or
Windows 10 coverage.

## Environment and fixture

- Windows 11 Education, version 10.0.26300, build 26300, x64.
- Intel Arc 140T GPU, driver 32.0.101.6104.
- FFmpeg 7.1.1.
- `crates/media/tests/fixtures/loop-boundary-h264.mp4`: 2 seconds, 64×48,
  30 FPS, H.264 Constrained Baseline, 60-frame GOP. Frames before 500 ms are
  green, frames in `[500, 1000)` ms are red, and frames at or after 1000 ms are
  blue. The full-frame color changes make a leaked keyframe pre-roll frame,
  post-segment frame, or black frame distinguishable at the display sample.
- Fixture SHA-256:
  `9C423B84FDE6CE8704217AF714F0B91F8AAD56281925BC26B296165AB32712EE`.
- The desktop capture used the 2560×1600 primary display and sampled its
  center pixel at 60 FPS.

## Procedure

1. Start the fixture in the host and apply Segment loop `[500, 1000)` ms,
   playback speed 200%, 60 FPS, and mute enabled.
2. Poll the host playback-state command every 35 ms for 28.5 seconds. Count a
   wrap when media time decreases by more than 50 ms.
3. Capture the center pixel of the desktop at 60 FPS with Desktop Duplication
   for the same 28.5-second interval. Classify a frame as segment content when
   red is more than twice each of green and blue; classify black, green, and
   blue samples separately.
4. During a separate 30-second playback interval, sample host private memory,
   working set, handle count, and thread count once per second.

## Results

- The host remained `Playing` in all 523 state samples. The state reported no
  `lastError`, and 111 media-time wraps were observed in 28.5 seconds. The
  500 ms segment at 200% speed has a nominal 250 ms loop interval.
- Desktop Duplication produced 1,710 samples at 60 FPS. All 1,710 center-pixel
  samples were red segment content; none were black, green pre-roll, blue
  post-segment, or another color class. This is a point sample of a full-frame
  color-coded fixture, not a per-pixel scanout guarantee.
- Across the separate 30-sample resource interval, private memory stayed at
  124.30–124.34 MiB, working set at 128.11–128.48 MiB, handles at 577–581, and
  threads at 18–19.
- Queue bounds are structural: the source-frame channel holds at most 3
  frames, the playback scheduler queue holds 3, and the source worker retains
  at most 1 additional pending frame when the channel is full. Seek increments
  the source generation, clears pending data, and the playback actor rejects
  messages from older generations.
- `source_worker_repeats_segment_seeks_without_old_or_pre_start_frames` uses
  this fixture across repeated keyframe seeks and verifies that the first
  accepted frame is inside the segment and has red segment content. It passed
  on the Windows target.

The measurements cover this H.264 Baseline fixture and this Windows 11 GPU
path. Other codecs, GOP layouts, drivers, and Windows versions may have
different boundary timing; no claim of seamless output for every source is
made.
