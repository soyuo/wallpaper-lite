# Phase 7 automatic pause and resume acceptance

## Result

Fullscreen and session-lock policies are evaluated independently by each
playback actor from the latest signal snapshot. The default policy pauses
playback for either signal. Playback time is retained while paused and resumes
only after every blocking cause clears and the user's current play intent allows
it. A manual Pause or Stop during an automatic pause prevents automatic resume;
Play during a policy block is deferred until the block clears.

When the selected policy pauses rendering, the recovery worker retains its
presenter and current backbuffer. It accepts newer decoded content without
presenting it, then redraws the latest frame after rendering resumes. Display
geometry may be recorded during the pause, but HWND/swap-chain resizing and
span updates are deferred until resume so they cannot present during the pause.
Each playback worker owns separate policy state. Battery behavior is deferred to
issue #107, and unavailable fullscreen/session signals fail open.

## Automated verification

The Windows-target host suite passed 32 tests:

```powershell
cargo +stable test -p wallpaper-host --target x86_64-pc-windows-msvc -- --test-threads=1 --quiet
```

Policy tests cover overlapping automatic causes, manual Pause and Stop,
deferred Play, render-only pause, per-worker state isolation, and unknown
signals. Workspace Windows-target checking, host all-target Clippy with
warnings denied, formatting, and `git diff --check` passed.

## Windows 11 fullscreen smoke

Environment: Windows 11 Education 26H2, build 26300.9974, x64; Intel Arc 140T
8 GB; driver 32.0.101.6104. The smoke used an isolated profile and
`crates/media/tests/fixtures/short-h264.mp4` (SHA-256
`7A793A05AC1685CF774F2D1BE2BBBA0F354948DA4408196871BEAC1625315078`). A
temporary borderless window covering the display activated the default
fullscreen pause policy, then was closed to clear the cause. The host's
`playback_state` was sampled 80 times at approximately 80 ms intervals:

- State changed from `Playing` to `Paused` at about 2.86 seconds, then back to
  `Playing` at about 5.85 seconds.
- Media time was identical across all 30 samples taken while paused.
- 50 samples reported `Playing`; every sample had `lastError = null`.
- Host logs recorded `automatic_playback_policy_changed` for `PausePlayback`
  and then `Continue`.

The test exercised an overlapping fullscreen foreground window only; the
interactive machine was not session-locked, and no suspend/resume or physical
battery transition was attempted. Windows 10 runtime behavior was not tested.
The monitor detector's session transitions and overlapping policy causes are
covered by automated tests; this report does not claim a live session-lock
smoke test.
