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

## Current runtime evidence

Windows 11 Education 10.0.26300 x64 with Intel Arc 140T exposed one active
desktop target in the current session. It reported bounds `(0, 0, 1707, 1067)`
(1707 × 1067 physical pixels), 140% scale (134.4 effective DPI), primary
status, and approximately 144 Hz. The
Windows integration test performs two consecutive snapshots and checks that
IDs and geometry match. Running the test executable in two fresh processes also
returned the same path-derived ID. The provider’s stable ID is derived from the Windows
monitor device path and does not depend on `HMONITOR` lifetime.

This run covered one attached display only. Multiple-display geometry is
covered with synthetic model tests; Independent, Duplicate, Span, and topology
recovery remain separate Phase 6 work. Windows 10 was not available for this
runtime check.

## Platform API references

- [QueryDisplayConfig](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-querydisplayconfig)
- [DISPLAYCONFIG_TARGET_DEVICE_NAME](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/ns-wingdi-displayconfig_target_device_name)
- [EnumDisplayMonitors](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-enumdisplaymonitors)
- [GetMonitorInfoW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getmonitorinfow)
- [GetScaleFactorForMonitor](https://learn.microsoft.com/en-us/windows/win32/api/shellscalingapi/nf-shellscalingapi-getscalefactorformonitor)
