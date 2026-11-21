# Performance policy model

`wallpaper-core` owns a platform-independent, versioned performance policy.
Fullscreen, session-lock, and battery signals each select `Continue`,
`PauseRendering`, or `PausePlayback`. Evaluation is deterministic: the strongest
active action wins, with playback pause stronger than render pause. The user's
manual pause is always preserved as `PausePlayback`, regardless of automatic
signals.

The current defaults pause playback for fullscreen and session lock, and
continue while on battery. The host samples fullscreen, session-lock, battery,
and suspend/resume state on a dedicated Windows message thread. Session and
power notifications are registered on its hidden notification window; a
one-second poll recovers missed changes and checks fullscreen geometry. A
Tokio `watch` channel keeps only the latest snapshot, and duplicate states are
not republished. If the monitor cannot start or a signal query fails, the host
reports the stage and numeric error while continuing wallpaper rendering.
Fullscreen and session-lock actions are evaluated by each playback actor from
its own copy of the latest signal snapshot. A playback pause preserves media
position and resumes only when all blocking signals clear and the user has not
paused or stopped manually. A render pause keeps the presenter and last frame
alive, retains newer frames without presenting them, and redraws the latest
content when rendering resumes. The monitor and actor communicate through a
`watch` receiver, so Windows signal collection does not block playback or IPC.
Battery actions remain outside this runtime integration.

Windows 11 signal detection details are in
[`docs/development/phase-7-signal-detection.md`](../development/phase-7-signal-detection.md);
automatic pause/resume behavior is measured in
[`docs/development/phase-7-automatic-pause-acceptance.md`](../development/phase-7-automatic-pause-acceptance.md).

`wallpaper-media` persists the schema-v1 snapshot at
`%APPDATA%\\WallpaperLite\\performance-policy.json`. Reads and writes are
bounded to 64 KiB. A candidate is parsed and schema-validated before the
existing same-directory atomic replacement helper is called, so invalid JSON,
unknown enum values, and unsupported schema versions cannot replace the last
valid file. A missing file loads the defaults.

The policy contains no wallpaper paths or media bytes. Evaluation is pure and
can be exercised with synthetic overlapping signals on any platform.
