# Performance policy model

`wallpaper-core` owns a platform-independent, versioned performance policy.
Fullscreen, session-lock, and battery signals each select `Continue`,
`PauseRendering`, or `PausePlayback`. Evaluation is deterministic: the strongest
active action wins, with playback pause stronger than render pause. The user's
manual pause is always preserved as `PausePlayback`, regardless of automatic
signals.

The current defaults pause playback for fullscreen and session lock, and
continue while on battery. This module only defines settings and evaluates a
snapshot; it does not query Windows or change playback. Signal collection and
applying decisions are separate host work.

`wallpaper-media` persists the schema-v1 snapshot at
`%APPDATA%\\WallpaperLite\\performance-policy.json`. Reads and writes are
bounded to 64 KiB. A candidate is parsed and schema-validated before the
existing same-directory atomic replacement helper is called, so invalid JSON,
unknown enum values, and unsupported schema versions cannot replace the last
valid file. A missing file loads the defaults.

The policy contains no wallpaper paths or media bytes. Evaluation is pure and
can be exercised with synthetic overlapping signals on any platform.
