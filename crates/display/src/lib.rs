//! Stable display identities and physical geometry for multi-monitor rendering.

use std::collections::HashSet;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

#[cfg(target_os = "windows")]
mod windows;

mod profiles;

pub use profiles::{
    DISPLAY_PROFILE_SCHEMA_VERSION, DisplayPresentationMode, DisplayProfile, DisplayProfileSet,
    DisplayProfileValidationError,
};

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct DisplayId(String);

impl<'de> Deserialize<'de> for DisplayId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let path = String::deserialize(deserializer)?;
        Self::from_device_path(path).map_err(serde::de::Error::custom)
    }
}

impl DisplayId {
    /// Creates a stable identifier from a Windows monitor device path.
    ///
    /// The path is normalized to ASCII lowercase because Windows device paths
    /// are case-insensitive. Transient `HMONITOR` values must never be stored
    /// as display identities.
    pub fn from_device_path(path: impl AsRef<str>) -> Result<Self, DisplayError> {
        let path = path.as_ref().trim();
        if path.is_empty() || path.len() > 512 || path.chars().any(char::is_control) {
            return Err(DisplayError::InvalidDevicePath);
        }

        Ok(Self(path.to_ascii_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct DisplayRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl DisplayRect {
    pub fn new(left: i32, top: i32, right: i32, bottom: i32) -> Result<Self, DisplayError> {
        let width = i64::from(right) - i64::from(left);
        let height = i64::from(bottom) - i64::from(top);
        if width <= 0 || height <= 0 || width > i64::from(u32::MAX) || height > i64::from(u32::MAX)
        {
            return Err(DisplayError::InvalidGeometry);
        }

        Ok(Self {
            left,
            top,
            right,
            bottom,
        })
    }

    pub const fn left(self) -> i32 {
        self.left
    }

    pub const fn top(self) -> i32 {
        self.top
    }

    pub const fn right(self) -> i32 {
        self.right
    }

    pub const fn bottom(self) -> i32 {
        self.bottom
    }

    pub const fn width(self) -> u32 {
        (self.right as i64 - self.left as i64) as u32
    }

    pub const fn height(self) -> u32 {
        (self.bottom as i64 - self.top as i64) as u32
    }
}

/// The shared virtual desktop and one physical output's viewport within it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpanBounds {
    virtual_desktop: DisplayRect,
    target: DisplayRect,
}

impl SpanBounds {
    pub fn new(virtual_desktop: DisplayRect, target: DisplayRect) -> Result<Self, DisplayError> {
        if target.left() < virtual_desktop.left()
            || target.top() < virtual_desktop.top()
            || target.right() > virtual_desktop.right()
            || target.bottom() > virtual_desktop.bottom()
        {
            return Err(DisplayError::InvalidGeometry);
        }
        Ok(Self {
            virtual_desktop,
            target,
        })
    }

    pub const fn virtual_desktop(self) -> DisplayRect {
        self.virtual_desktop
    }

    pub const fn target(self) -> DisplayRect {
        self.target
    }
}

/// Return the physical bounding rectangle containing all active display rectangles.
pub fn virtual_desktop_bounds(displays: &[DisplayRect]) -> Result<DisplayRect, DisplayError> {
    let Some(first) = displays.first() else {
        return Err(DisplayError::InvalidGeometry);
    };
    let mut left = first.left();
    let mut top = first.top();
    let mut right = first.right();
    let mut bottom = first.bottom();
    for display in &displays[1..] {
        left = left.min(display.left());
        top = top.min(display.top());
        right = right.max(display.right());
        bottom = bottom.max(display.bottom());
    }
    let width = i64::from(right) - i64::from(left);
    let height = i64::from(bottom) - i64::from(top);
    if width > i64::from(u32::MAX) || height > i64::from(u32::MAX) {
        return Err(DisplayError::InvalidGeometry);
    }
    DisplayRect::new(left, top, right, bottom)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct RefreshRate {
    numerator: u32,
    denominator: u32,
}

impl RefreshRate {
    pub fn new(numerator: u32, denominator: u32) -> Result<Self, DisplayError> {
        if numerator == 0 || denominator == 0 {
            return Err(DisplayError::InvalidRefreshRate);
        }

        Ok(Self {
            numerator,
            denominator,
        })
    }

    pub const fn numerator(self) -> u32 {
        self.numerator
    }

    pub const fn denominator(self) -> u32 {
        self.denominator
    }

    pub fn hertz(self) -> f64 {
        f64::from(self.numerator) / f64::from(self.denominator)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct DisplayResolution {
    width: u32,
    height: u32,
}

impl DisplayResolution {
    const fn from_bounds(bounds: DisplayRect) -> Self {
        Self {
            width: bounds.width(),
            height: bounds.height(),
        }
    }

    pub const fn width(self) -> u32 {
        self.width
    }

    pub const fn height(self) -> u32 {
        self.height
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DisplayInfo {
    id: DisplayId,
    name: String,
    device_name: String,
    bounds: DisplayRect,
    resolution: DisplayResolution,
    dpi_scale_percent: u32,
    effective_dpi: f64,
    refresh_rate: Option<RefreshRate>,
    primary: bool,
}

impl DisplayInfo {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: DisplayId,
        name: impl Into<String>,
        device_name: impl Into<String>,
        bounds: DisplayRect,
        dpi_scale_percent: u32,
        refresh_rate: Option<RefreshRate>,
        primary: bool,
    ) -> Result<Self, DisplayError> {
        if !(50..=800).contains(&dpi_scale_percent) {
            return Err(DisplayError::InvalidDpiScale);
        }

        Ok(Self {
            id,
            name: name.into(),
            device_name: device_name.into(),
            bounds,
            resolution: DisplayResolution::from_bounds(bounds),
            dpi_scale_percent,
            effective_dpi: 96.0 * f64::from(dpi_scale_percent) / 100.0,
            refresh_rate,
            primary,
        })
    }

    pub fn id(&self) -> &DisplayId {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Runtime Windows GDI name, such as `\\.\DISPLAY1`; not a persistent ID.
    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    pub const fn bounds(&self) -> DisplayRect {
        self.bounds
    }

    /// Physical desktop resolution in pixels, derived from the monitor bounds.
    pub const fn resolution(&self) -> DisplayResolution {
        self.resolution
    }

    pub const fn dpi_scale_percent(&self) -> u32 {
        self.dpi_scale_percent
    }

    pub const fn effective_dpi(&self) -> f64 {
        self.effective_dpi
    }

    pub const fn refresh_rate(&self) -> Option<RefreshRate> {
        self.refresh_rate
    }

    pub const fn is_primary(&self) -> bool {
        self.primary
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct DisplaySnapshot {
    displays: Vec<DisplayInfo>,
}

impl DisplaySnapshot {
    pub fn new(mut displays: Vec<DisplayInfo>) -> Result<Self, DisplayError> {
        let mut ids = HashSet::with_capacity(displays.len());
        if displays
            .iter()
            .any(|display| !ids.insert(display.id().clone()))
        {
            return Err(DisplayError::DuplicateDisplayId);
        }

        displays.sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
        Ok(Self { displays })
    }

    pub fn displays(&self) -> &[DisplayInfo] {
        &self.displays
    }

    pub fn into_displays(self) -> Vec<DisplayInfo> {
        self.displays
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisplayStage {
    DisplayConfigBufferSizes,
    DisplayConfigPaths,
    DisplayConfigSourceName,
    DisplayConfigTargetName,
    MonitorEnumeration,
    MonitorInformation,
    MonitorScaleFactor,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DisplayError {
    UnsupportedPlatform,
    InvalidDevicePath,
    InvalidGeometry,
    InvalidDpiScale,
    InvalidRefreshRate,
    DuplicateDisplayId,
    WindowsApi { stage: DisplayStage, code: i32 },
    DisplayTopologyChanged,
}

impl fmt::Display for DisplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::UnsupportedPlatform => "display enumeration is only supported on Windows",
            Self::InvalidDevicePath => "display device path is invalid",
            Self::InvalidGeometry => "display geometry is invalid",
            Self::InvalidDpiScale => "display DPI scale is invalid",
            Self::InvalidRefreshRate => "display refresh rate is invalid",
            Self::DuplicateDisplayId => "display enumeration returned a duplicate identity",
            Self::WindowsApi { .. } => "Windows display enumeration failed",
            Self::DisplayTopologyChanged => "display topology changed during enumeration",
        };

        formatter.write_str(message)
    }
}

impl std::error::Error for DisplayError {}

/// Enumerates the currently active desktop display targets.
///
/// Callers must configure per-monitor DPI awareness before enumeration so GDI monitor
/// rectangles are returned in physical desktop pixels rather than virtualized coordinates.
#[cfg(target_os = "windows")]
pub fn enumerate_displays() -> Result<DisplaySnapshot, DisplayError> {
    windows::enumerate_displays()
}

/// Non-Windows builds retain the shared display contracts for tests and tools.
#[cfg(not(target_os = "windows"))]
pub fn enumerate_displays() -> Result<DisplaySnapshot, DisplayError> {
    Err(DisplayError::UnsupportedPlatform)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(path: &str, bounds: DisplayRect) -> DisplayInfo {
        DisplayInfo::new(
            DisplayId::from_device_path(path).unwrap(),
            "Test display",
            "\\\\.\\DISPLAY1",
            bounds,
            150,
            Some(RefreshRate::new(60_000, 1_000).unwrap()),
            false,
        )
        .unwrap()
    }

    #[test]
    fn stable_ids_normalize_case_and_reject_empty_paths() {
        let first = DisplayId::from_device_path(r"\\?\DISPLAY#ABC").unwrap();
        let second = DisplayId::from_device_path(r"\\?\display#abc").unwrap();

        assert_eq!(first, second);
        assert_eq!(first.as_str(), r"\\?\display#abc");
        assert_eq!(
            DisplayId::from_device_path("  "),
            Err(DisplayError::InvalidDevicePath)
        );
    }

    #[test]
    fn virtual_bounds_include_negative_origins_and_monitor_gaps() {
        let left = DisplayRect::new(-1_920, 0, 0, 1_080).unwrap();
        let upper = DisplayRect::new(0, -1_080, 1_920, 0).unwrap();
        let virtual_bounds = virtual_desktop_bounds(&[left, upper]).unwrap();

        assert_eq!(
            virtual_bounds,
            DisplayRect::new(-1_920, -1_080, 1_920, 1_080).unwrap()
        );
        assert!(SpanBounds::new(virtual_bounds, left).is_ok());
        assert!(SpanBounds::new(virtual_bounds, upper).is_ok());
        assert!(virtual_desktop_bounds(&[]).is_err());
    }

    #[test]
    fn rectangles_keep_negative_virtual_origins_and_physical_extent() {
        let bounds = DisplayRect::new(-1920, -200, 0, 880).unwrap();

        assert_eq!((bounds.left(), bounds.top()), (-1920, -200));
        assert_eq!((bounds.width(), bounds.height()), (1920, 1080));
        assert_eq!(
            DisplayRect::new(10, 10, 10, 100),
            Err(DisplayError::InvalidGeometry)
        );
    }

    #[test]
    fn snapshot_sorts_stable_ids_and_rejects_duplicates() {
        let right = display(
            r"\\?\DISPLAY#B",
            DisplayRect::new(0, 0, 1920, 1080).unwrap(),
        );
        let left = display(
            r"\\?\DISPLAY#A",
            DisplayRect::new(-1920, 0, 0, 1080).unwrap(),
        );
        let snapshot = DisplaySnapshot::new(vec![right.clone(), left]).unwrap();

        assert_eq!(snapshot.displays()[0].id().as_str(), r"\\?\display#a");
        assert_eq!(snapshot.displays()[1].resolution().width(), 1920);
        assert_eq!(snapshot.displays()[1].resolution().height(), 1080);
        assert_eq!(snapshot.displays()[1].effective_dpi(), 144.0);
        assert_eq!(snapshot.displays()[1].refresh_rate().unwrap().hertz(), 60.0);
        assert_eq!(
            DisplaySnapshot::new(vec![right.clone(), right]),
            Err(DisplayError::DuplicateDisplayId)
        );
    }

    #[test]
    fn invalid_display_scale_is_rejected() {
        let id = DisplayId::from_device_path(r"\\?\DISPLAY#A").unwrap();
        assert_eq!(
            DisplayInfo::new(
                id,
                "Display",
                "\\\\.\\DISPLAY1",
                DisplayRect::new(0, 0, 1920, 1080).unwrap(),
                0,
                None,
                true,
            ),
            Err(DisplayError::InvalidDpiScale)
        );
    }
}
