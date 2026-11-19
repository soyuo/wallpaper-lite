//! Direct3D device ownership. Rendering pipelines are implemented in later phases.

use wallpaper_core::error::{AppError, ErrorCode};

/// Failure while creating the rendering device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RendererError {
    UnsupportedPlatform,
    DeviceCreation { hresult: i32 },
    MissingDeviceInterface,
    MissingImmediateContext,
}

impl RendererError {
    pub const fn to_app_error(self) -> AppError {
        match self {
            Self::UnsupportedPlatform => AppError::new(
                ErrorCode::UnsupportedPlatform,
                "Direct3D initialization is unavailable on this platform.",
            ),
            _ => AppError::new(
                ErrorCode::GraphicsInitializationFailed,
                "The graphics device could not be initialized.",
            ),
        }
    }

    /// Numeric-only diagnostic; it does not include driver paths or user data.
    pub fn diagnostic_summary(self) -> String {
        match self {
            Self::UnsupportedPlatform => "stage=d3d11_create_device platform=unsupported".into(),
            Self::DeviceCreation { hresult } => {
                format!("stage=d3d11_create_device hresult=0x{:08X}", hresult as u32)
            }
            Self::MissingDeviceInterface => {
                "stage=d3d11_create_device detail=missing_device_interface".into()
            }
            Self::MissingImmediateContext => {
                "stage=d3d11_create_device detail=missing_immediate_context".into()
            }
        }
    }
}

/// Owns the D3D11 device and immediate context through their COM lifetimes.
pub struct D3d11Device {
    #[cfg(target_os = "windows")]
    device: windows::Win32::Graphics::Direct3D11::ID3D11Device,
    #[cfg(target_os = "windows")]
    immediate_context: windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext,
    feature_level: u32,
}

impl D3d11Device {
    /// Create a hardware-backed D3D11 device and its immediate context.
    #[cfg(target_os = "windows")]
    pub fn create_hardware() -> Result<Self, RendererError> {
        use windows::Win32::Foundation::HMODULE;
        use windows::Win32::Graphics::Direct3D::{
            D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_10_0,
            D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0,
        };
        use windows::Win32::Graphics::Direct3D11::{
            D3D11_CREATE_DEVICE_FLAG, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
            ID3D11DeviceContext,
        };
        use windows::Win32::Graphics::Dxgi::IDXGIAdapter;

        let feature_levels = [
            D3D_FEATURE_LEVEL_11_0,
            D3D_FEATURE_LEVEL_10_1,
            D3D_FEATURE_LEVEL_10_0,
        ];
        let mut device: Option<ID3D11Device> = None;
        let mut immediate_context: Option<ID3D11DeviceContext> = None;
        let mut feature_level = D3D_FEATURE_LEVEL(0);

        // The windows crate wraps the COM outputs in owning interface values,
        // which release their references automatically when this owner drops.
        unsafe {
            D3D11CreateDevice(
                None::<&IDXGIAdapter>,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_FLAG(0),
                Some(&feature_levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                Some(&mut feature_level),
                Some(&mut immediate_context),
            )
        }
        .map_err(|error| RendererError::DeviceCreation {
            hresult: error.code().0,
        })?;

        let device = device.ok_or(RendererError::MissingDeviceInterface)?;
        let immediate_context = immediate_context.ok_or(RendererError::MissingImmediateContext)?;

        Ok(Self {
            device,
            immediate_context,
            feature_level: feature_level.0 as u32,
        })
    }

    /// Non-Windows callers receive an explicit unsupported-platform result.
    #[cfg(not(target_os = "windows"))]
    pub fn create_hardware() -> Result<Self, RendererError> {
        Err(RendererError::UnsupportedPlatform)
    }

    pub const fn feature_level(&self) -> u32 {
        self.feature_level
    }

    #[cfg(target_os = "windows")]
    pub fn device(&self) -> &windows::Win32::Graphics::Direct3D11::ID3D11Device {
        &self.device
    }

    #[cfg(target_os = "windows")]
    pub fn immediate_context(&self) -> &windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext {
        &self.immediate_context
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::D3d11Device;
    use windows::core::Interface;

    #[test]
    fn creates_and_releases_hardware_device_and_immediate_context() {
        let device = D3d11Device::create_hardware()
            .expect("supported Windows hardware should create a D3D11 device");

        assert_ne!(device.feature_level(), 0);
        assert!(!device.device().as_raw().is_null());
        assert!(!device.immediate_context().as_raw().is_null());

        drop(device);
    }
}
