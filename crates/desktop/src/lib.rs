//! Windows desktop-process initialization and shell integration.

use wallpaper_core::error::{AppError, ErrorCode};
use wallpaper_display::DisplayRect;

#[cfg(target_os = "windows")]
mod lifecycle;
#[cfg(target_os = "windows")]
mod shell;
#[cfg(target_os = "windows")]
mod window;

#[cfg(target_os = "windows")]
pub use lifecycle::ShellLifecycleMonitor;
#[cfg(target_os = "windows")]
pub use shell::discover_desktop_host;
#[cfg(target_os = "windows")]
pub use window::AttachedDesktopWindow;

/// Failure while preparing the Windows desktop process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopError {
    UnsupportedPlatform,
    DpiAwareness { hresult: i32 },
    ShellUnavailable,
    DesktopHostNotFound,
    AmbiguousDesktopHost { candidate_count: usize },
    StaleDesktopHost,
    WindowEnumerationFailed { hresult: i32 },
    WindowClassRegistrationFailed { win32_error: u32 },
    WindowCreationFailed { hresult: i32 },
    WindowPositionFailed { hresult: i32 },
    DesktopBackgroundLayerUnavailable,
    DesktopWindowThreadFailed { os_error_code: Option<i32> },
    DesktopWindowDpiContextUnavailable,
    DesktopWindowDpiContextFailed,
    DesktopWindowBoundsUnavailable,
    WallpaperWindowUnavailable,
    ShellLifecycleMonitorThreadFailed { os_error_code: Option<i32> },
    ShellLifecycleMonitorProcessOpenFailed { hresult: i32 },
}

/// State transitions observed for the Explorer process that owns the desktop host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellLifecycleEvent {
    ExplorerExited {
        shell_process_id: u32,
        host_kind: DesktopHostKind,
    },
    ExplorerRestarted {
        shell_process_id: u32,
        host_kind: DesktopHostKind,
    },
    MonitorFailed {
        stage: &'static str,
        error_code: u32,
    },
}

/// The class of a validated Windows shell desktop window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopHostKind {
    Progman,
    WorkerW,
}

impl DesktopHostKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Progman => "progman",
            Self::WorkerW => "workerw",
        }
    }
}

/// A freshly validated shell window handle. It is borrowed from Explorer and may become stale
/// when the shell changes; call `validate_current` before using it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopHost {
    raw_handle: isize,
    shell_process_id: u32,
    kind: DesktopHostKind,
}

impl DesktopHost {
    pub const fn raw_handle(self) -> isize {
        self.raw_handle
    }

    pub const fn kind(self) -> DesktopHostKind {
        self.kind
    }

    pub fn validate_current(self) -> Result<(), DesktopError> {
        #[cfg(target_os = "windows")]
        {
            shell::validate_current(self)
        }
        #[cfg(not(target_os = "windows"))]
        {
            Err(DesktopError::UnsupportedPlatform)
        }
    }

    /// Create an owned child window in the validated shell background layer below desktop icons.
    pub fn attach_wallpaper_window(self) -> Result<AttachedDesktopWindow, DesktopError> {
        #[cfg(target_os = "windows")]
        {
            window::AttachedDesktopWindow::attach(self)
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = self;
            Err(DesktopError::UnsupportedPlatform)
        }
    }

    /// Create an owned child window positioned over one display's physical desktop bounds.
    pub fn attach_wallpaper_window_for_display(
        self,
        bounds: DisplayRect,
    ) -> Result<AttachedDesktopWindow, DesktopError> {
        #[cfg(target_os = "windows")]
        {
            window::AttachedDesktopWindow::attach_for_display(self, bounds)
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = (self, bounds);
            Err(DesktopError::UnsupportedPlatform)
        }
    }

    pub(crate) const fn new(
        raw_handle: isize,
        shell_process_id: u32,
        kind: DesktopHostKind,
    ) -> Self {
        Self {
            raw_handle,
            shell_process_id,
            kind,
        }
    }
}

impl DesktopError {
    pub const fn to_app_error(self) -> AppError {
        match self {
            Self::UnsupportedPlatform => AppError::new(
                ErrorCode::UnsupportedPlatform,
                "Windows desktop initialization is unavailable on this platform.",
            ),
            Self::DpiAwareness { .. } => AppError::new(
                ErrorCode::WindowsInitializationFailed,
                "Windows desktop initialization failed.",
            ),
            Self::ShellUnavailable
            | Self::DesktopHostNotFound
            | Self::AmbiguousDesktopHost { .. }
            | Self::StaleDesktopHost
            | Self::WindowEnumerationFailed { .. }
            | Self::WindowClassRegistrationFailed { .. }
            | Self::WindowCreationFailed { .. }
            | Self::WindowPositionFailed { .. }
            | Self::DesktopBackgroundLayerUnavailable
            | Self::DesktopWindowThreadFailed { .. }
            | Self::DesktopWindowDpiContextUnavailable
            | Self::DesktopWindowDpiContextFailed
            | Self::DesktopWindowBoundsUnavailable
            | Self::WallpaperWindowUnavailable
            | Self::ShellLifecycleMonitorThreadFailed { .. }
            | Self::ShellLifecycleMonitorProcessOpenFailed { .. } => AppError::new(
                ErrorCode::DesktopHostUnavailable,
                "The Windows desktop window could not be attached.",
            ),
        }
    }

    /// Numeric-only diagnostic; it does not include process or user paths.
    pub fn diagnostic_summary(self) -> String {
        match self {
            Self::UnsupportedPlatform => "stage=windows_initialization platform=unsupported".into(),
            Self::DpiAwareness { hresult } => {
                format!(
                    "stage=set_process_dpi_awareness hresult=0x{:08X}",
                    hresult as u32
                )
            }
            Self::ShellUnavailable => {
                "stage=desktop_host_discovery reason=shell_unavailable".into()
            }
            Self::DesktopHostNotFound => {
                "stage=desktop_host_discovery reason=desktop_view_not_found".into()
            }
            Self::AmbiguousDesktopHost { candidate_count } => format!(
                "stage=desktop_host_discovery reason=ambiguous_candidates candidate_count={candidate_count}"
            ),
            Self::StaleDesktopHost => "stage=desktop_host_discovery reason=stale_window".into(),
            Self::WindowEnumerationFailed { hresult } => format!(
                "stage=desktop_host_discovery reason=window_enumeration_failed hresult=0x{:08X}",
                hresult as u32
            ),
            Self::WindowClassRegistrationFailed { win32_error } => format!(
                "stage=desktop_window_attach reason=class_registration_failed win32_error={win32_error}"
            ),
            Self::WindowCreationFailed { hresult } => format!(
                "stage=desktop_window_attach reason=create_window_failed hresult=0x{:08X}",
                hresult as u32
            ),
            Self::WindowPositionFailed { hresult } => format!(
                "stage=desktop_window_attach reason=position_window_failed hresult=0x{:08X}",
                hresult as u32
            ),
            Self::DesktopBackgroundLayerUnavailable => {
                "stage=desktop_window_attach reason=background_layer_unavailable".into()
            }
            Self::DesktopWindowThreadFailed { os_error_code } => format!(
                "stage=desktop_window_attach reason=window_thread_failed os_error_code={os_error_code:?}"
            ),
            Self::DesktopWindowDpiContextUnavailable => {
                "stage=desktop_window_attach reason=dpi_context_unavailable".into()
            }
            Self::DesktopWindowDpiContextFailed => {
                "stage=desktop_window_attach reason=dpi_context_change_failed".into()
            }
            Self::DesktopWindowBoundsUnavailable => {
                "stage=desktop_window_attach reason=parent_bounds_unavailable".into()
            }
            Self::WallpaperWindowUnavailable => {
                "stage=desktop_window_attach reason=attached_window_unavailable".into()
            }
            Self::ShellLifecycleMonitorThreadFailed { os_error_code } => format!(
                "stage=shell_lifecycle_monitor reason=thread_start_failed os_error_code={os_error_code:?}"
            ),
            Self::ShellLifecycleMonitorProcessOpenFailed { hresult } => format!(
                "stage=shell_lifecycle_monitor reason=process_open_failed hresult=0x{:08X}",
                hresult as u32
            ),
        }
    }
}

/// Set the host process to per-monitor DPI awareness before creating windows.
#[cfg(target_os = "windows")]
pub fn initialize_windows() -> Result<(), DesktopError> {
    use windows::Win32::UI::HiDpi::{PROCESS_PER_MONITOR_DPI_AWARE, SetProcessDpiAwareness};

    // Process DPI awareness must be configured before the host creates windows.
    unsafe { SetProcessDpiAwareness(PROCESS_PER_MONITOR_DPI_AWARE) }.map_err(|error| {
        DesktopError::DpiAwareness {
            hresult: error.code().0,
        }
    })
}

/// Return a typed error on non-Windows targets instead of silently skipping shell discovery.
#[cfg(not(target_os = "windows"))]
pub fn discover_desktop_host() -> Result<DesktopHost, DesktopError> {
    Err(DesktopError::UnsupportedPlatform)
}

/// Placeholder type for non-Windows consumers of the cross-platform desktop API.
#[cfg(not(target_os = "windows"))]
#[derive(Debug)]
pub struct AttachedDesktopWindow;

/// Placeholder type for non-Windows consumers of the cross-platform desktop API.
#[cfg(not(target_os = "windows"))]
#[derive(Debug)]
pub struct ShellLifecycleMonitor;

#[cfg(not(target_os = "windows"))]
impl ShellLifecycleMonitor {
    pub fn start(_host: DesktopHost) -> Result<Self, DesktopError> {
        Err(DesktopError::UnsupportedPlatform)
    }

    pub fn try_recv(&self) -> Result<ShellLifecycleEvent, std::sync::mpsc::TryRecvError> {
        Err(std::sync::mpsc::TryRecvError::Disconnected)
    }
}

/// Return a typed error on non-Windows targets instead of silently skipping setup.
#[cfg(not(target_os = "windows"))]
pub fn initialize_windows() -> Result<(), DesktopError> {
    Err(DesktopError::UnsupportedPlatform)
}
