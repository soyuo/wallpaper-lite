# Performance policy model

`wallpaper-core` owns a platform-independent, versioned performance policy.
Fullscreen, session-lock, and battery signals each select `Continue`,
`PauseRendering`, or `PausePlayback`. Evaluation is deterministic: the strongest
active action wins, with playback pause stronger than render pause. The user's
manual pause is always preserved as `PausePlayback`, regardless of automatic
signals.

The defaults pause playback for fullscreen and session lock and continue while
on battery. Users can select a battery pause action or an optional 1–120 FPS
cap. Each playback actor applies the temporary cap as the minimum of the
battery cap and that target's configured FPS. When AC power returns, it restores
the configured limit without rewriting saved playback settings. A battery cap
cannot be combined with a battery pause action.

The host samples fullscreen, session-lock, battery, and suspend/resume state on
a dedicated Windows message thread. Session and power notifications are
registered on its hidden notification window; a one-second poll recovers
missed changes and checks fullscreen geometry. A Tokio `watch` channel keeps
only the latest snapshot, and duplicate states are not republished. If the
monitor cannot start or a signal query fails, the host reports the stage and
numeric error while continuing wallpaper rendering. Each playback actor
evaluates the latest shared policy against its own signal snapshot, so one
target's effective FPS or pause state does not alter another target's saved
settings. Policy updates reach active actors without restarting playback.

A playback pause preserves media position and resumes only when all blocking
signals clear and the user has not paused or stopped manually. A render pause
keeps the presenter and last frame alive, retains newer frames without
presenting them, and redraws the latest content when rendering resumes. A
never-known battery state fails open; a transient failed sample retains the
last known state. The monitor and playback actors communicate through `watch`
receivers, so Windows signal collection does not block playback or IPC.

Windows 11 signal detection is recorded in
[`docs/development/phase-7-signal-detection.md`](../development/phase-7-signal-detection.md);
automatic pause/resume behavior is in
[`docs/development/phase-7-automatic-pause-acceptance.md`](../development/phase-7-automatic-pause-acceptance.md);
battery policy and its transition coverage are in
[`docs/development/phase-7-battery-policy-acceptance.md`](../development/phase-7-battery-policy-acceptance.md).

`wallpaper-media` persists the schema-v1 snapshot at
`%APPDATA%\WallpaperLite\performance-policy.json`. Reads and writes are
bounded to 64 KiB. A candidate is parsed and schema-validated before the
existing same-directory atomic replacement helper is called, so invalid JSON,
unknown enum values, and unsupported schema versions cannot replace the last
valid file. The optional battery cap defaults to `null` when reading existing
schema-v1 files that predate the field. A missing file loads the defaults.

The policy contains no wallpaper paths or media bytes. Evaluation is pure and
can be exercised with synthetic overlapping signals on any platform.
