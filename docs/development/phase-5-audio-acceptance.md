# Phase 5 audio playback acceptance

## Environment

- Windows 11 Education, build 26300, x64
- Intel Arc 140T integrated graphics
- Windows 10 runtime verification was not available

## Implementation checks

- The Windows source worker creates a Media Foundation audio-only Media Engine
  only when the selected source contains an audio stream. A silent H.264 source
  remains playable without an audio engine.
- At 100% speed, the Media Engine's media position drives the video scheduler.
  Seek and loop operations update both the source reader and audio engine.
- The persisted 0–100 volume setting is applied to the Media Engine. The
  Windows-target media test reads back the engine volume and mute state, checks
  clock advancement while user-muted, pauses audio at non-1x, and verifies the
  audio clock resumes at 1x.
- At non-1x speed, audio is paused and forced-muted while the speed-scaled video
  scheduler remains Playing. Playback state exposes audio availability and
  effective mute status to the UI.
- Audio engine teardown calls Media Engine `Shutdown`; the source owns the
  engine, so replacement and source drop release it with the Media Foundation
  reader.

## Windows 11 smoke

The isolated Tauri smoke catalog used the two-second H.264/AAC moving-pattern
fixture and a silent H.264 fixture. At 1x, playback state reported
`audioAvailable: true`; applying volume 40 and mute persisted those values and
reported `audioEffectiveMuted: true` while the media clock advanced from
11,626,024 to 14,249,770 ticks in 250 ms and playback remained `Playing`.
At 2x with user mute disabled, the UI reported that audio was muted while video
continued. The full-loop run stayed `Playing` and wrapped twice during a 5 s
state sample: 20,156,219→891,249 ticks and 19,266,220→75,833 ticks. The audio
clock no longer freezes when the decoder reaches EOS ahead of its buffered
final frames.

Replacing the A/V source with the silent H.264 fixture kept playback active and
the UI reported that the video has no audio track. The per-wallpaper settings
file stored volume, mute, speed, and loop mode. The A/V fixture SHA-256 before
and after playback was
`403C4AA856AA8401D76E8AB8601C08508CE4AC829316BB496114CBC87D9B1683`; the silent
fixture SHA-256 remained
`7A793A05AC1685CF774F2D1BE2BBBA0F354948DA4408196871BEAC1625315078`.

A final UI run against those same fixture hashes sampled the timeline at 1x:
`1163, 1667, 134, 134, 631, 631, 1133, 1618, 1618` ms. The `1667 -> 134` ms
transition shows the full-loop wrap. At 2x, the state remained `Playing` and
timeline time advanced from 900 to 1902 ms across 600 ms of wall time; the UI
reported `Audio is muted while video runs at non-1x speed.` Settings persisted
volume 40, user mute off, speed 200, and full looping. Replacing that source
with the one-second silent fixture remained valid and reached `Ended` at
1004 ms with the no-audio-track status.

## Limits

- Verification used the Windows 11 Media Foundation stack and default audio
  endpoint. Windows 10 and systems without the tested codec components were
  not tested.
- Volume and mute were verified through Media Engine state/readback; no
  calibrated acoustic-level measurement was performed.
- Non-1x audio is intentionally muted rather than time-stretched. The UI
  reports this policy.
