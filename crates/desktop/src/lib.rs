//! Windows desktop-process initialization and shell integration.

use wallpaper_core::error::{AppError, ErrorCode};

#[cfg(target_os = "windows")]
mod shell;

#[cfg(target_os = "windows")]
pub use shell::discover_desktop_host;

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
            | Self::WindowEnumerationFailed { .. } => AppError::new(
                ErrorCode::DesktopHostUnavailable,
                "The Windows desktop host is not available.",
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

/// Return a typed error on non-Windows targets instead of silently skipping setup.
#[cfg(not(target_os = "windows"))]
pub fn initialize_windows() -> Result<(), DesktopError> {
    Err(DesktopError::UnsupportedPlatform)
}
