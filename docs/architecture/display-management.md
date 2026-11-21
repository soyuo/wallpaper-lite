# Display enumeration and identity

`wallpaper-display` exposes `enumerate_displays() -> Result<DisplaySnapshot,
DisplayError>` for the active Windows desktop targets. The shared model keeps
stable display identity separate from runtime handles and from wallpaper
assignments.

The Windows provider uses `GetDisplayConfigBufferSizes` and
`QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS)` to find active source/target paths.
It obtains each source GDI name and target monitor device path through
`DisplayConfigGetDeviceInfo`. The target device path is the persistent
`DisplayId`; it is normalized for Windows case-insensitive matching. `HMONITOR`
is used only during the current enumeration and is never stored as an ID.

`EnumDisplayMonitors` and `GetMonitorInfoW` provide the monitor rectangle in
virtual-screen coordinates, including negative origins, and primary status.
`GetScaleFactorForMonitor` provides the configured scale percentage; effective
DPI is reported as the corresponding floating-point value `96 * scale_percent / 100`.
The active DisplayConfig target path supplies the refresh-rate rational. A
runtime GDI name such as `\\.\DISPLAY1` is retained for diagnostics and display
labels, but it is not treated as a persistent identity.

The path query retries up to three times if the topology changes between
buffer sizing and enumeration. API failures carry a typed stage and numeric
code; error messages do not include device paths. GDI pseudo-monitors without
active paths are omitted. If an active target has no matching `HMONITOR`, the
provider returns a typed topology-change error instead of a partial snapshot.

## Persisted display profiles

`DisplayProfileSet` is a schema-versioned snapshot keyed by normalized
`DisplayId`. Each profile stores a nonzero wallpaper ID, a presentation mode
(`independent`, `duplicate`, or `span`), and an optional FPS override from 1 to
120. Unknown modes, unsupported schemas, duplicate display IDs, invalid IDs,
and invalid FPS values are rejected during deserialization or validation.

`DisplayProfileStore::for_current_user()` writes
`%APPDATA%\\WallpaperLite\\display-profiles.json` through the shared atomic
file replacement helper. `upsert()` updates a single identity while retaining
all other records, including displays absent from the current enumeration.
Profiles are not filtered by current topology, so a returning display matches
its prior stable ID. This store persists assignment intent only; applying a
profile to desktop windows is handled by the Windows host at startup. Profile
storage never opens or modifies source media.

## Startup mode resolution

The host enables per-monitor DPI awareness before enumerating displays, then
matches profiles by stable display ID and resolves each wallpaper ID against
the catalog. Missing, unavailable, or invalid targets are skipped without
preventing other targets from starting. Clone/overlapping geometry is skipped
to avoid overlapping child surfaces.

If an active profile selects Duplicate mode, the primary display's Duplicate
profile supplies the shared wallpaper when present. Otherwise, the first active
Duplicate profile in stable display-ID order supplies it. The selected source
is assigned to every active display, while each display retains its own FPS
override. Conflicting wallpaper IDs in Duplicate profiles are logged and
resolved to that shared source. Without an active Duplicate profile, the host
uses the persisted Independent assignments.

The primary display reuses the existing host playback actor and window. Its
child is confined to primary display bounds when secondary targets are active.
Each assigned secondary display owns a child window at that display's physical
desktop bounds, its own D3D11 device and recovery worker, and a separate video
playback worker with bounded frame queues when its assignment is video. Its FPS
override is applied only to that playback worker. Target startup failures are
isolated. Explorer exit/restart events are forwarded to every recovery worker,
and shutdown joins playback workers before releasing their windows and
renderer resources.

The host loads the saved mode when it starts. Restarting after profile changes
releases the previous target set during shutdown and creates targets for the
new mode. Live profile editing is part of the Monitor Manager work in issue #97.
Topology changes are not re-enumerated at runtime; that remains issue #96. Span
mode remains later work.
## Current runtime evidence

Windows 11 Education 10.0.26300 x64 with Intel Arc 140T exposed one active
desktop target in the current session. With the caller set to per-monitor DPI
awareness before enumeration, it reported physical bounds `(0, 0, 2560, 1600)`,
140% scale (134.4 effective DPI), primary status, and approximately 144 Hz.
The Windows integration test now sets the same awareness before comparing two
consecutive snapshots. Running the test executable in two fresh processes also
returned the same path-derived ID. The provider’s stable ID is derived from the
Windows monitor device path and does not depend on `HMONITOR` lifetime.

This enumeration run covered one attached display only. Multiple-display
geometry is covered with synthetic model tests. The host applies a persisted
primary Independent profile in the Phase 6 runtime report; a simultaneous
two-physical-display run is unavailable in this environment. A persisted
Duplicate profile reload on the single display is documented in the Duplicate
acceptance report; simultaneous two-display Duplicate behavior remains
unverified. Span and topology recovery remain separate Phase 6 work. Windows 10
was not available for these runtime checks.

## Platform API references

- [QueryDisplayConfig](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-querydisplayconfig)
- [DISPLAYCONFIG_TARGET_DEVICE_NAME](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/ns-wingdi-displayconfig_target_device_name)
- [EnumDisplayMonitors](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-enumdisplaymonitors)
- [GetMonitorInfoW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getmonitorinfow)
- [GetScaleFactorForMonitor](https://learn.microsoft.com/en-us/windows/win32/api/shellscalingapi/nf-shellscalingapi-getscalefactorformonitor)
