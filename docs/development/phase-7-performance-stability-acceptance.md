# Phase 7 Performance and Stability Acceptance

## Result

This report records the evidence requested by issue #111. Results are scoped to
the Windows 11 host and the conditions exercised below. The 15-minute release
soak **failed the long-run playback stability criterion**: the process remained
alive without a reported playback error, but later samples showed no rendered
frames, rising frame drops, and substantial private-byte growth. The cause was
not identified, so this run is not represented as a stability pass.

## Environment

- Windows 11 Education, 26H2, build 26300, x64
- Intel Arc 140T, 8 GB, driver `32.0.101.6104`
- One active display was enumerated; multi-display behavior was not exercised
- During the soak, the system was on AC (`ACLineStatus=1`), with the installed
  battery at 100%; the soak did not test a power-source transition
- Release host, built with:

```powershell
cargo +stable build -p wallpaper-host --release --target x86_64-pc-windows-msvc
```

## Per-criterion results

| Criterion | Result | Evidence and limits |
| --- | --- | --- |
| Fullscreen automatic pause and resume | Pass | The controlled Windows 11 smoke in [`phase-7-automatic-pause-acceptance.md`](phase-7-automatic-pause-acceptance.md) observed `Playing` → `Paused` → `Playing`; media time held across 30 paused samples and all 80 samples had no playback error. During the long soak, fullscreen also changed true for about 15 seconds and the policy changed `PausePlayback` → `Continue`. That soak trigger was not controlled and its source application was not identified. |
| Session-lock and suspend/resume behavior | Unavailable | No live lock/unlock or suspend/resume was performed. Automated policy and signal tests cover those transitions. |
| Battery FPS cap | Pass, limited | The separate unplugged runtime run in [`phase-7-battery-policy-acceptance.md`](phase-7-battery-policy-acceptance.md) observed the temporary 30 FPS cap while on battery. Physical AC reconnection was unavailable; restoring configured FPS is covered by automated tests. The current soak ran on AC. |
| D3D device-loss recovery | Pass, injected only | Windows-target tests cover injected `DXGI_ERROR_DEVICE_REMOVED`, bounded retries, same-HWND image/video restoration, and transient recovery. No physical driver fault was injected; see [`phase-7-device-loss-recovery.md`](phase-7-device-loss-recovery.md). |
| Bounded per-target telemetry and dashboard | Pass | `npm --prefix apps/ui run test:performance` passed 5/5 and `npm --prefix apps/ui run build` passed. The soak queried the protocol v12 metrics snapshot once per minute and recorded playback, source/rendered FPS, drops, latencies, process memory, GPU memory, and policy causes. This run covered one display. |
| Static-image idle rendering | Pass, automated | `presents_once_and_stays_idle_until_image_or_size_changes` passed: one initial presentation and no additional presentation during the 250 ms idle window. This is a renderer test, not a long-duration desktop image soak. |
| 15-minute video playback and resource trend | **Fail** | The release soak completed 900 seconds with 16 samples and the host process stayed alive. See the measurements below. All six samples from 600 through 900 seconds reported 0 rendered FPS; cumulative drops rose from 3,114 at 600 seconds to 6,845 at 900 seconds. Private bytes rose from 134,688,768 at 60 seconds to 245,563,392 at 900 seconds (+110,874,624 bytes / 105.7 MiB). The final sample had `last_error=null`; the host log contained no `ERROR` or `WARN` lines. Absence of a crash/error did not meet the playback stability criterion. |
| Windows 10 and physical multi-display behavior | Unavailable | No Windows 10 system or second active display was available. This report makes no claim for either condition; issues #6 and #98 remain open. |
| Longer playback durations | Unavailable | Only the 15-minute soak below was run. One-hour, eight-hour, and 24-hour behavior has not been measured, and no claim is made for those durations. |

## Long-run sample evidence

The soak used a local 16-minute, 64x48, 30 FPS H.264 Constrained Baseline,
yuv420p clip derived from the one-second fixture
`crates/media/tests/fixtures/short-h264.mp4`. The measured file was kept outside
the repository; its SHA-256 was
`4649858D412CA2EB8636FDBF662A6125E03BAD200C24F103021E6493E9C8D752`. FFmpeg
7.1.1 was used. Recreate an equivalent 960-second input with:

```powershell
New-Item -ItemType Directory -Force "$env:LOCALAPPDATA\Temp\wallpaper-lite-issue111" | Out-Null
ffmpeg -n -stream_loop 959 -i crates/media/tests/fixtures/short-h264.mp4 `
  -t 960 -c copy "$env:LOCALAPPDATA\Temp\wallpaper-lite-issue111\phase7-equivalent-16m.mp4"
```

The remuxed file may have a different container hash; its decoded frame hash
matches the measured clip (`d5b163657b764c0fc9ae2d33c7c9a0298921ad59f5e8d0c02eebb63835023924`).
Use a new output path for each harness run; the script refuses to overwrite an
existing file. It isolates `APPDATA` and `LOCALAPPDATA`, starts its own host,
verifies the named-pipe server PID, requests protocol v12 state/metrics, and
stops only that host in `finally`:

```powershell
.\docs\development\phase-7-soak.ps1 `
  -VideoPath "$env:LOCALAPPDATA\Temp\wallpaper-lite-issue111\phase7-equivalent-16m.mp4" `
  -OutputPath "$env:LOCALAPPDATA\Temp\wallpaper-lite-issue111\phase7-release-soak-15m-repro.csv" `
  -DurationMinutes 15 -SampleIntervalSeconds 60 -BuildProfile release
```

Repeat the four-minute manual-pause procedure with the remuxed equivalent clip
using:

```powershell
.\docs\development\phase-7-soak.ps1 `
  -VideoPath "$env:LOCALAPPDATA\Temp\wallpaper-lite-issue111\phase7-equivalent-16m.mp4" `
  -OutputPath "$env:LOCALAPPDATA\Temp\wallpaper-lite-issue111\phase7-controlled-pause-repro.csv" `
  -DurationMinutes 4 -SampleIntervalSeconds 30 `
  -PauseResumeAtSeconds 60 -PauseResumeDurationSeconds 10 -BuildProfile release
```

The requested pause time must align with the sample interval. The harness
records its actual trigger time, ensures a terminal sample after the pause,
and reports `completion_status` so an incomplete run is explicit. The command
uses the remuxed equivalent fixture; the reported four-minute diagnostic above
used a freshly re-encoded copy.

| Elapsed (s) | State | Fullscreen | Source FPS | Rendered FPS | Drops total | Private bytes |
| ---: | --- | --- | ---: | ---: | ---: | ---: |
| 60 | Playing | false | 29.97 | 29.97 | 46 | 134,688,768 |
| 120 | Playing | false | 34.83 | 30.85 | 87 | 134,991,872 |
| 180 | Playing | false | 30.75 | 29.76 | 180 | 135,049,216 |
| 240 | Playing | false | 31.40 | 17.66 | 729 | 135,581,696 |
| 300 | Playing | false | 29.97 | 29.97 | 928 | 135,102,464 |
| 360 | Playing | false | 30.77 | 27.79 | 1,122 | 135,188,480 |
| 420 | Playing | false | 29.76 | 29.76 | 1,313 | 135,426,048 |
| 480 | Paused | true | 2.98 | 0.00 | 2,195 | 137,936,896 |
| 540 | Playing | false | 29.86 | 29.86 | 2,464 | 138,113,024 |
| 600 | Playing | false | 1.00 | 0.00 | 3,114 | 138,776,576 |
| 660 | Playing | false | 12.97 | 0.00 | 3,484 | 145,424,384 |
| 720 | Playing | false | 14.94 | 0.00 | 4,331 | 161,157,120 |
| 780 | Playing | false | 13.95 | 0.00 | 5,054 | 186,994,688 |
| 840 | Playing | false | 12.00 | 0.00 | 6,074 | 230,649,856 |
| 900 | Playing | false | 7.99 | 0.00 | 6,845 | 245,563,392 |

Across samples from 60 through 900 seconds, working set ranged from
59,854,848 to 139,354,112 bytes, handles from 610 to 627, threads from 21 to
24, and GPU-local usage from 68,329,472 to 98,889,728 bytes. The CSV also
records per-sample values for these counters. The source was 30 FPS while the
reported configured `fps_limit` was 60.

The logs recorded `PausePlayback` at `2026-11-22T08:04:50.514330Z` with
`fullscreen=true`, then `Continue` at `2026-11-22T08:05:05.516468Z` after the
signal cleared. The final media time was 885.027 seconds, consistent with the
observed 15-second policy pause. The later throughput loss and private-byte
growth followed this event, but timing alone does not establish causation.

A separate four-minute release diagnostic used a freshly re-encoded 16-minute
H.264 clip and issued a manual 10-second Pause/Play at 60 seconds. Playback
reported Paused and then Playing as requested, but that run was already below
real-time by its 30-second sample (9.95 source FPS, 0 rendered FPS), before the
manual pause. It ended at 15.88 source FPS, 0 rendered FPS, 3,142 dropped
frames, and 181,747,712 private bytes, with no host warnings or errors. This
follow-up does not isolate a pause/resume defect; it shows that the poor
throughput was also observed in a separate run, and the cause remains
unresolved.

## Reproduction checks

The following checks passed during this issue’s evidence run:

```powershell
cargo +stable test -p wallpaper-host --target x86_64-pc-windows-msvc -- --test-threads=1 --quiet
cargo +stable test -p wallpaper-host device_ --target x86_64-pc-windows-msvc -- --test-threads=1
cargo +stable test -p wallpaper-renderer presents_once_and_stays_idle_until_image_or_size_changes --target x86_64-pc-windows-msvc -- --test-threads=1
npm --prefix apps/ui run test:performance
npm --prefix apps/ui run build
cargo +stable fmt --all --check
git diff --check
```

The host suite passed 43 tests; the focused device-loss selection passed 5;
the renderer idle test passed 1; performance dashboard tests passed 5. One
live Explorer-window integration test remains intentionally ignored. The
soak harness boundary smoke (60-second duration, one-second sampling, pause at
58 seconds for 10 seconds) completed with 60 samples and a terminal sample at
68.1 seconds; `completion_status` was `completed`. An existing-file output
collision preserved the source clip hash, and a pause at 31 seconds with a
30-second sample interval was rejected before launch. The
release build emitted two existing dead-code warnings in
`host/src/playback.rs` for `start_with_policy_and_display_refresh` and
`new_with_policy_and_display_refresh`.

## Limits and follow-up

This report records the observed 15-minute failure instead of extrapolating
from the earlier Phase 3 soak, which predates Phase 7. The host stayed alive,
but later playback did not keep up with the 30 FPS source (the configured
`fps_limit` was 60). No root cause is claimed. Session-lock/suspend, AC
reconnection, Windows 10, physical GPU
failure, and multi-display runtime behavior remain unverified. The installed
battery is `B3122MM`; at the time of the current system probe it was connected
to AC at 100%. The earlier contrary statement in
[`phase-7-signal-detection.md`](phase-7-signal-detection.md) has been corrected.
