# Phase 7 runtime signal detection

## Result

The wallpaper host starts a dedicated Windows signal-monitor thread before the
playback actor, so the initial policy state can be applied before video
commands are accepted. A hidden notification window receives
session and power messages. The monitor samples foreground fullscreen state,
session lock state, and AC/battery state once per second to recover missed
notifications. Suspend and resume messages update the system power state.

Snapshots are typed and carry a monotonically increasing revision. A bounded
Tokio `watch` channel publishes only distinct latest states, so the playback
actor never waits on Windows signal collection. The host logs state changes and
detector errors. Failure to start the monitor leaves wallpaper rendering
running.

## Runtime smoke

- OS: Windows 11 Education 26H2, build 10.0.26300.9974, x64
- GPU: Intel Arc 140T 8 GB, driver 32.0.101.6104
- Host: Windows-target debug build, attached to the existing Progman desktop
  host; D3D11 feature level `0xB000`
- At idle, the initial snapshot reported fullscreen `false`, session locked
  `false`, on battery `false`, and power state `Running`; all detector error
  fields were empty.
- A temporary foreground borderless window covering the primary monitor caused
  fullscreen to transition `false → true`; closing it returned the signal to
  `false`. Session and power values remained stable. The test window was closed
  immediately after the transition check.
- The host and test window were stopped after the smoke; no host process
  remained. This was test cleanup, not a graceful host-shutdown acceptance run.

Unit tests cover duplicate-state coalescing, fullscreen/session/battery and
suspend/resume transitions, bounded watch publication, and detector errors
preserving the last known state. This machine has no battery, so no physical AC
to battery transition was tested. Session lock/unlock and system suspend/resume
were not triggered on the interactive machine. Windows 10 runtime behavior was
not tested.

## Limits

Fullscreen combines the shell's exclusive-fullscreen signal with a visible
foreground window covering its nearest monitor. The shell signal may not
identify every application-specific presentation mode. This issue detects and
publishes signals. Playback and render policy behavior is verified separately
in [`phase-7-automatic-pause-acceptance.md`](phase-7-automatic-pause-acceptance.md).
