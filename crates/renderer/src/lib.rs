//! Direct3D device ownership and still/video-frame presentation.

use wallpaper_core::error::{AppError, ErrorCode};

mod color;
mod placement;
mod preview;

pub use color::ColorAdjustments;
pub use placement::{
    ImagePlacement, ImageTransform, NormalizedCrop, PlacementDimensions, PlacementMode,
    PlacementPoint, PlacementRect, calculate_image_placement,
    calculate_transformed_image_placement,
};
pub use preview::{EDITOR_PREVIEW_MAX_EDGE, RenderedEditorPreview, render_editor_preview};

#[cfg(target_os = "windows")]
mod image;

#[cfg(target_os = "windows")]
pub use image::D3d11ImagePresenter;

/// Failure while creating the rendering device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RendererError {
    UnsupportedPlatform,
    DeviceCreation { hresult: i32 },
    MissingDeviceInterface,
    MissingImmediateContext,
    MultithreadProtectionUnavailable,
    WindowUnavailable,
    WindowSizeUnavailable,
    InvalidPlacementDimensions,
    InvalidVideoFrame,
    InvalidPreviewInput,
    InvalidEditorSettings,
    PreviewAllocationFailed,
    InvalidTransformParameters,
    InvalidCropBounds,
    InvalidColorAdjustments,
    PlacementBufferCreation { hresult: i32 },
    SwapChainCreation { hresult: i32 },
    SwapChainResize { hresult: i32 },
    BackBufferAccess { hresult: i32 },
    RenderTargetCreation { hresult: i32 },
    ImageTextureCreation { hresult: i32 },
    ImageViewCreation { hresult: i32 },
    ShaderCompilation { hresult: i32 },
    ShaderCreation { hresult: i32 },
    SamplerCreation { hresult: i32 },
    RasterizerCreation { hresult: i32 },
    Present { hresult: i32 },
}

impl RendererError {
    pub const fn to_app_error(self) -> AppError {
        match self {
            Self::UnsupportedPlatform => AppError::new(
                ErrorCode::UnsupportedPlatform,
                "Direct3D initialization is unavailable on this platform.",
            ),
            Self::InvalidEditorSettings => AppError::new(
                ErrorCode::InvalidEditorSettings,
                "The wallpaper editor settings are invalid.",
            ),
            _ => AppError::new(
                ErrorCode::GraphicsInitializationFailed,
                "The graphics renderer could not initialize or present the image.",
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
            Self::MultithreadProtectionUnavailable => {
                "stage=d3d11_create_device detail=multithread_protection_unavailable".into()
            }
            Self::WindowUnavailable => "stage=d3d11_present detail=window_unavailable".into(),
            Self::WindowSizeUnavailable => {
                "stage=d3d11_present detail=window_size_unavailable".into()
            }
            Self::InvalidPlacementDimensions => {
                "stage=d3d11_present detail=invalid_placement_dimensions".into()
            }
            Self::InvalidVideoFrame => "stage=d3d11_present detail=invalid_video_frame".into(),
            Self::InvalidPreviewInput => "stage=editor_preview detail=invalid_input".into(),
            Self::InvalidEditorSettings => "stage=editor_preview detail=invalid_settings".into(),
            Self::PreviewAllocationFailed => "stage=editor_preview detail=allocation_failed".into(),
            Self::InvalidTransformParameters => {
                "stage=image_transform detail=invalid_parameters".into()
            }
            Self::InvalidCropBounds => "stage=image_transform detail=invalid_crop_bounds".into(),
            Self::InvalidColorAdjustments => "stage=image_color detail=invalid_adjustments".into(),
            Self::PlacementBufferCreation { hresult } => {
                diagnostic("placement_buffer_create", hresult)
            }
            Self::SwapChainCreation { hresult } => diagnostic("swap_chain_create", hresult),
            Self::SwapChainResize { hresult } => diagnostic("swap_chain_resize", hresult),
            Self::BackBufferAccess { hresult } => diagnostic("back_buffer_access", hresult),
            Self::RenderTargetCreation { hresult } => diagnostic("render_target_create", hresult),
            Self::ImageTextureCreation { hresult } => diagnostic("image_texture_create", hresult),
            Self::ImageViewCreation { hresult } => diagnostic("image_view_create", hresult),
            Self::ShaderCompilation { hresult } => diagnostic("shader_compile", hresult),
            Self::ShaderCreation { hresult } => diagnostic("shader_create", hresult),
            Self::SamplerCreation { hresult } => diagnostic("sampler_create", hresult),
            Self::RasterizerCreation { hresult } => diagnostic("rasterizer_create", hresult),
            Self::Present { hresult } => diagnostic("present", hresult),
        }
    }
}

fn diagnostic(stage: &str, hresult: i32) -> String {
    format!("stage=d3d11_{stage} hresult=0x{:08X}", hresult as u32)
}

/// Owns the D3D11 device and immediate context through their COM lifetimes.
pub struct D3d11Device {
    #[cfg(target_os = "windows")]
    device: windows::Win32::Graphics::Direct3D11::ID3D11Device,
    #[cfg(target_os = "windows")]
    immediate_context: windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext,
    feature_level: u32,
}

/// D3D11 still-image and video-frame presenter is implemented on Windows only.
#[cfg(not(target_os = "windows"))]
pub struct D3d11ImagePresenter<'a> {
    _device: std::marker::PhantomData<&'a D3d11Device>,
}

#[cfg(not(target_os = "windows"))]
impl<'a> D3d11ImagePresenter<'a> {
    pub fn new(
        _device: &'a D3d11Device,
        _window_handle: isize,
        _image: &wallpaper_media::DecodedImage,
    ) -> Result<Self, RendererError> {
        Err(RendererError::UnsupportedPlatform)
    }

    pub fn new_with_placement(
        _device: &'a D3d11Device,
        _window_handle: isize,
        _image: &wallpaper_media::DecodedImage,
        _placement_mode: PlacementMode,
    ) -> Result<Self, RendererError> {
        Err(RendererError::UnsupportedPlatform)
    }

    pub fn new_with_transform(
        _device: &'a D3d11Device,
        _window_handle: isize,
        _image: &wallpaper_media::DecodedImage,
        _placement_mode: PlacementMode,
        _transform: ImageTransform,
    ) -> Result<Self, RendererError> {
        Err(RendererError::UnsupportedPlatform)
    }

    pub fn new_with_color_adjustments(
        _device: &'a D3d11Device,
        _window_handle: isize,
        _image: &wallpaper_media::DecodedImage,
        _color_adjustments: ColorAdjustments,
    ) -> Result<Self, RendererError> {
        Err(RendererError::UnsupportedPlatform)
    }

    pub fn new_with_video_frame(
        _device: &'a D3d11Device,
        _window_handle: isize,
        _frame: &wallpaper_media::DecodedVideoFrame,
    ) -> Result<Self, RendererError> {
        Err(RendererError::UnsupportedPlatform)
    }

    pub fn set_image(
        &mut self,
        _image: &wallpaper_media::DecodedImage,
    ) -> Result<(), RendererError> {
        Err(RendererError::UnsupportedPlatform)
    }

    pub fn set_video_frame(
        &mut self,
        _frame: &wallpaper_media::DecodedVideoFrame,
    ) -> Result<(), RendererError> {
        Err(RendererError::UnsupportedPlatform)
    }

    pub fn refresh_if_resized(&mut self) -> Result<bool, RendererError> {
        Err(RendererError::UnsupportedPlatform)
    }

    pub fn set_placement_mode(
        &mut self,
        _placement_mode: PlacementMode,
    ) -> Result<(), RendererError> {
        Err(RendererError::UnsupportedPlatform)
    }

    pub fn set_transform(&mut self, _transform: ImageTransform) -> Result<(), RendererError> {
        Err(RendererError::UnsupportedPlatform)
    }

    pub fn set_color_adjustments(
        &mut self,
        _color_adjustments: ColorAdjustments,
    ) -> Result<(), RendererError> {
        Err(RendererError::UnsupportedPlatform)
    }

    pub fn set_editor_color_settings(
        &mut self,
        _settings: wallpaper_core::editor_settings::EditorColorSettings,
    ) -> Result<(), RendererError> {
        Err(RendererError::UnsupportedPlatform)
    }

    pub fn transform(&self) -> ImageTransform {
        ImageTransform::default()
    }

    pub const fn placement_mode(&self) -> PlacementMode {
        PlacementMode::Cover
    }

    pub fn color_adjustments(&self) -> ColorAdjustments {
        ColorAdjustments::default()
    }

    pub const fn presented_frames(&self) -> u64 {
        0
    }
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
            D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
            ID3D11DeviceContext, ID3D11Multithread,
        };
        use windows::Win32::Graphics::Dxgi::IDXGIAdapter;
        use windows::core::Interface;

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
                D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
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

        // The Source Reader's DXVA readback shares this device's immediate context.
        let multithread = immediate_context
            .cast::<ID3D11Multithread>()
            .map_err(|error| RendererError::DeviceCreation {
                hresult: error.code().0,
            })?;
        let _previous_protection = unsafe { multithread.SetMultithreadProtected(true) };
        if !unsafe { multithread.GetMultithreadProtected() }.as_bool() {
            return Err(RendererError::MultithreadProtectionUnavailable);
        }

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
    use windows::Win32::Graphics::Direct3D11::ID3D11Multithread;
    use windows::core::Interface;

    #[test]
    fn creates_and_releases_hardware_device_and_immediate_context() {
        let device = D3d11Device::create_hardware()
            .expect("supported Windows hardware should create a D3D11 device");

        assert_ne!(device.feature_level(), 0);
        assert!(!device.device().as_raw().is_null());
        assert!(!device.immediate_context().as_raw().is_null());
        let multithread = device
            .immediate_context()
            .cast::<ID3D11Multithread>()
            .expect("D3D11 immediate context should expose multithread controls");
        assert!(unsafe { multithread.GetMultithreadProtected() }.as_bool());

        drop(device);
    }
}
