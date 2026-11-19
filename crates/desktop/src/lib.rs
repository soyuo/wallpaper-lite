//! Windows desktop-process initialization and shell integration.

use wallpaper_core::error::{AppError, ErrorCode};

/// Failure while preparing the Windows desktop process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopError {
    UnsupportedPlatform,
    DpiAwareness { hresult: i32 },
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

/// Return a typed error on non-Windows targets instead of silently skipping setup.
#[cfg(not(target_os = "windows"))]
pub fn initialize_windows() -> Result<(), DesktopError> {
    Err(DesktopError::UnsupportedPlatform)
}
