//! Image decoding and video-source opening boundaries for wallpaper media.

use std::path::Path;

use wallpaper_core::error::{AppError, ErrorCode};

mod editor_settings;
mod frame_queue;
mod import;
mod library;
mod playback;
#[cfg(target_os = "windows")]
mod video;
#[cfg(target_os = "windows")]
mod wic;

pub use editor_settings::{EditorSettingsError, EditorSettingsStore};
pub use frame_queue::{FrameEnqueueOutcome, FrameQueueError, VideoFrameQueue};
pub use import::{ImportFailure, ImportResult, ImportStatus, import_local_media};
pub use library::{
    Availability, CatalogStore, LibraryError, MediaKind, WallpaperEntry, WallpaperId,
    WallpaperLibrary,
};
pub use playback::{
    PlaybackError, PlaybackScheduler, PlaybackState, VideoFpsLimit, VideoFpsLimitError,
    VideoFpsPreset, VideoPlaybackSpeed, VideoPlaybackSpeedError,
};
pub use wallpaper_core::editor_settings::{
    EDITOR_SETTINGS_SCHEMA_VERSION, EditorColorSettings, EditorSettingsValidationError,
    NormalizedCrop, NormalizedPoint, PhysicalPosition, PhysicalSize, PlacementMode, RenderSettings,
    Scale2d, WallpaperEditorSettings,
};

#[cfg(any(target_os = "windows", test))]
const MAX_IMAGE_DIMENSION: u32 = 16_384;
#[cfg(any(target_os = "windows", test))]
const MAX_IMAGE_PIXELS: u64 = 64 * 1024 * 1024;
#[cfg(any(target_os = "windows", test))]
const BYTES_PER_PIXEL: u32 = 4;

/// A decoded image using tightly packed, top-to-bottom RGBA8 pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    width: u32,
    height: u32,
    stride: u32,
    rgba8: Vec<u8>,
}

impl DecodedImage {
    pub const fn width(&self) -> u32 {
        self.width
    }

    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Bytes per row in the tightly packed RGBA8 buffer.
    pub const fn stride(&self) -> u32 {
        self.stride
    }

    pub fn rgba8(&self) -> &[u8] {
        &self.rgba8
    }
}

/// Safe failure categories for media decoding and source opening.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaError {
    UnsupportedPlatform,
    UnsupportedVideoPlatform,
    MediaFoundationStartupFailed { hresult: i32 },
    VideoComInitializationFailed { hresult: i32 },
    VideoSourceOpenFailed { hresult: i32 },
    VideoStreamNotFound { hresult: i32 },
    UnsupportedVideoFormat,
    UnsupportedVideoContainer,
    VideoMetadataUnavailable { hresult: i32 },
    InvalidVideoMetadata,
    InvalidVideoPath,
    VideoSampleDecodeFailed { hresult: i32 },
    VideoSampleBufferAccess { hresult: i32 },
    VideoFormatChanged,
    VideoDecoderReportedError,
    InvalidVideoSeekPosition,
    VideoSeekFailed { hresult: i32 },
    UnsupportedVideoPixelFormat,
    InvalidVideoSample,
    InvalidVideoTimestamp,
    VideoFrameAllocationFailed,
    ComInitialization { hresult: i32 },
    FactoryCreation { hresult: i32 },
    DecoderCreation { hresult: i32 },
    UnsupportedImageFormat,
    ImageHasNoFrames,
    FrameAccess { hresult: i32 },
    InvalidDimensions,
    ImageTooLarge,
    PixelFormatConversion { hresult: i32 },
    PixelCopy { hresult: i32 },
    AllocationFailed,
}

impl MediaError {
    pub const fn to_app_error(self) -> AppError {
        match self {
            Self::UnsupportedPlatform => AppError::new(
                ErrorCode::UnsupportedPlatform,
                "Media decoding is unavailable on this platform.",
            ),
            Self::UnsupportedVideoPlatform => AppError::new(
                ErrorCode::UnsupportedPlatform,
                "Video source opening is unavailable on this platform.",
            ),
            Self::MediaFoundationStartupFailed { .. }
            | Self::VideoComInitializationFailed { .. }
            | Self::VideoSourceOpenFailed { .. }
            | Self::VideoStreamNotFound { .. }
            | Self::UnsupportedVideoFormat
            | Self::UnsupportedVideoContainer
            | Self::VideoMetadataUnavailable { .. }
            | Self::InvalidVideoMetadata
            | Self::InvalidVideoPath => AppError::new(
                ErrorCode::VideoSourceOpenFailed,
                "The video source could not be opened.",
            ),
            Self::VideoSampleDecodeFailed { .. }
            | Self::VideoSampleBufferAccess { .. }
            | Self::VideoFormatChanged
            | Self::VideoDecoderReportedError
            | Self::InvalidVideoSeekPosition
            | Self::VideoSeekFailed { .. }
            | Self::UnsupportedVideoPixelFormat
            | Self::InvalidVideoSample
            | Self::InvalidVideoTimestamp
            | Self::VideoFrameAllocationFailed => AppError::new(
                ErrorCode::VideoDecodeFailed,
                "The video frame could not be decoded.",
            ),
            _ => AppError::new(
                ErrorCode::ImageDecodeFailed,
                "The image could not be decoded.",
            ),
        }
    }

    /// Numeric-only diagnostics; file paths and source data are never included.
    pub fn diagnostic_summary(self) -> String {
        match self {
            Self::UnsupportedPlatform => "stage=image_decode platform=unsupported".into(),
            Self::UnsupportedVideoPlatform => "stage=video_open platform=unsupported".into(),
            Self::MediaFoundationStartupFailed { hresult } => format!(
                "stage=video_open reason=media_foundation_startup_failed hresult=0x{:08X}",
                hresult as u32
            ),
            Self::VideoComInitializationFailed { hresult } => format!(
                "stage=video_open reason=com_initialization_failed hresult=0x{:08X}",
                hresult as u32
            ),
            Self::VideoSourceOpenFailed { hresult } => format!(
                "stage=video_open reason=source_reader_creation_failed hresult=0x{:08X}",
                hresult as u32
            ),
            Self::VideoStreamNotFound { hresult } => format!(
                "stage=video_open reason=video_stream_not_found hresult=0x{:08X}",
                hresult as u32
            ),
            Self::UnsupportedVideoFormat => {
                "stage=video_open reason=unsupported_video_format".into()
            }
            Self::UnsupportedVideoContainer => {
                "stage=video_open reason=unsupported_video_container".into()
            }
            Self::VideoMetadataUnavailable { hresult } => format!(
                "stage=video_open reason=video_metadata_unavailable hresult=0x{:08X}",
                hresult as u32
            ),
            Self::InvalidVideoMetadata => "stage=video_open reason=invalid_video_metadata".into(),
            Self::InvalidVideoPath => "stage=video_open reason=invalid_path".into(),
            Self::VideoSampleDecodeFailed { hresult } => format!(
                "stage=video_decode reason=sample_decode_failed hresult=0x{:08X}",
                hresult as u32
            ),
            Self::VideoSampleBufferAccess { hresult } => format!(
                "stage=video_decode reason=sample_buffer_access_failed hresult=0x{:08X}",
                hresult as u32
            ),
            Self::VideoFormatChanged => "stage=video_decode reason=unexpected_format_change".into(),
            Self::VideoDecoderReportedError => {
                "stage=video_decode reason=reader_reported_error".into()
            }
            Self::InvalidVideoSeekPosition => {
                "stage=video_seek reason=position_out_of_range".into()
            }
            Self::VideoSeekFailed { hresult } => {
                format!("stage=video_seek hresult=0x{:08X}", hresult as u32)
            }
            Self::UnsupportedVideoPixelFormat => {
                "stage=video_decode reason=unsupported_pixel_format".into()
            }
            Self::InvalidVideoSample => "stage=video_decode reason=invalid_sample".into(),
            Self::InvalidVideoTimestamp => "stage=video_decode reason=invalid_timestamp".into(),
            Self::VideoFrameAllocationFailed => {
                "stage=video_decode reason=frame_allocation_failed".into()
            }
            Self::ComInitialization { hresult } => format!(
                "stage=image_decode reason=com_initialization_failed hresult=0x{:08X}",
                hresult as u32
            ),
            Self::FactoryCreation { hresult } => format!(
                "stage=image_decode reason=wic_factory_failed hresult=0x{:08X}",
                hresult as u32
            ),
            Self::DecoderCreation { hresult } => format!(
                "stage=image_decode reason=decoder_creation_failed hresult=0x{:08X}",
                hresult as u32
            ),
            Self::UnsupportedImageFormat => {
                "stage=image_decode reason=unsupported_container_format".into()
            }
            Self::ImageHasNoFrames => "stage=image_decode reason=no_frames".into(),
            Self::FrameAccess { hresult } => format!(
                "stage=image_decode reason=frame_access_failed hresult=0x{:08X}",
                hresult as u32
            ),
            Self::InvalidDimensions => "stage=image_decode reason=invalid_dimensions".into(),
            Self::ImageTooLarge => "stage=image_decode reason=image_too_large".into(),
            Self::PixelFormatConversion { hresult } => format!(
                "stage=image_decode reason=pixel_format_conversion_failed hresult=0x{:08X}",
                hresult as u32
            ),
            Self::PixelCopy { hresult } => format!(
                "stage=image_decode reason=pixel_copy_failed hresult=0x{:08X}",
                hresult as u32
            ),
            Self::AllocationFailed => "stage=image_decode reason=allocation_failed".into(),
        }
    }
}

/// Identifies which video stream selection the Media Foundation source reader uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoStreamSelection {
    FirstVideo,
}

/// Whether this reader was configured to prefer DXVA through the supplied D3D11 device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoDecoderMode {
    Software,
    HardwarePreferred,
}

/// Decoded frames are always tightly packed top-to-bottom RGBA8.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoPixelFormat {
    Rgba8,
}

/// One decoded presentation-order video frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedVideoFrame {
    width: u32,
    height: u32,
    stride: u32,
    pixel_format: VideoPixelFormat,
    rgba8: Vec<u8>,
    pts_100ns: i64,
    duration_100ns: i64,
}

impl DecodedVideoFrame {
    pub const fn width(&self) -> u32 {
        self.width
    }
    pub const fn height(&self) -> u32 {
        self.height
    }
    pub const fn stride(&self) -> u32 {
        self.stride
    }
    pub const fn pixel_format(&self) -> VideoPixelFormat {
        self.pixel_format
    }
    pub fn rgba8(&self) -> &[u8] {
        &self.rgba8
    }
    pub const fn pts_100ns(&self) -> i64 {
        self.pts_100ns
    }
    pub const fn duration_100ns(&self) -> i64 {
        self.duration_100ns
    }
}

/// A reader result that preserves timing and source-reader control events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoReadEvent {
    Frame(DecodedVideoFrame),
    StreamTick { timestamp_100ns: i64 },
    EndOfStream,
}

/// Validated metadata for the first video stream in a source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoMetadata {
    width: u32,
    height: u32,
    frame_rate_numerator: u32,
    frame_rate_denominator: u32,
    duration_100ns: i64,
    selected_stream: VideoStreamSelection,
}

impl VideoMetadata {
    pub const fn width(self) -> u32 {
        self.width
    }

    pub const fn height(self) -> u32 {
        self.height
    }

    pub const fn frame_rate_numerator(self) -> u32 {
        self.frame_rate_numerator
    }

    pub const fn frame_rate_denominator(self) -> u32 {
        self.frame_rate_denominator
    }

    /// Total presentation duration in Media Foundation's 100 ns time units.
    pub const fn duration_100ns(self) -> i64 {
        self.duration_100ns
    }

    pub const fn selected_stream(self) -> VideoStreamSelection {
        self.selected_stream
    }
}

/// An open MP4 video source and its Media Foundation reader.
///
/// The reader is released before Media Foundation is shut down. The source is opened read-only.
pub struct VideoSource {
    metadata: VideoMetadata,
    decoder_mode: VideoDecoderMode,
    #[cfg(target_os = "windows")]
    _platform: video::VideoSourceHandle,
}

impl VideoSource {
    /// Opens a source whose first video stream uses the H.264 codec.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, MediaError> {
        #[cfg(target_os = "windows")]
        {
            let (platform, metadata, decoder_mode) = video::open(path.as_ref(), None)?;
            Ok(Self {
                metadata,
                decoder_mode,
                _platform: platform,
            })
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = path;
            Err(MediaError::UnsupportedVideoPlatform)
        }
    }

    pub const fn metadata(&self) -> VideoMetadata {
        self.metadata
    }

    pub const fn decoder_mode(&self) -> VideoDecoderMode {
        self.decoder_mode
    }

    /// Reads until the next frame, stream tick, or end-of-stream marker.
    pub fn read_next(&mut self) -> Result<VideoReadEvent, MediaError> {
        #[cfg(target_os = "windows")]
        {
            video::read_next(&mut self._platform)
        }
        #[cfg(not(target_os = "windows"))]
        {
            Err(MediaError::UnsupportedVideoPlatform)
        }
    }

    /// Seeks the source reader in 100 ns media-time units and resets timestamp validation.
    pub fn seek(&mut self, position_100ns: i64) -> Result<(), MediaError> {
        if position_100ns < 0 || position_100ns > self.metadata.duration_100ns() {
            return Err(MediaError::InvalidVideoSeekPosition);
        }
        #[cfg(target_os = "windows")]
        {
            video::seek(&mut self._platform, position_100ns)
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = position_100ns;
            Err(MediaError::UnsupportedVideoPlatform)
        }
    }

    /// Opens the source with DXVA preferred and Media Foundation's software fallback available.
    #[cfg(target_os = "windows")]
    pub fn open_with_d3d11_device(
        path: impl AsRef<Path>,
        device: &windows::Win32::Graphics::Direct3D11::ID3D11Device,
    ) -> Result<Self, MediaError> {
        let (platform, metadata, decoder_mode) = video::open(path.as_ref(), Some(device))?;
        Ok(Self {
            metadata,
            decoder_mode,
            _platform: platform,
        })
    }
}

/// Decode the first frame of a PNG, JPEG, or BMP file into an owned RGBA8 pixel buffer.
///
/// Dimensions are limited to 16,384 per axis and 67,108,864 pixels before allocating the buffer.
/// The source file is opened for reading only and is never modified.
pub fn decode_image(path: impl AsRef<Path>) -> Result<DecodedImage, MediaError> {
    #[cfg(target_os = "windows")]
    {
        wic::decode_image(path.as_ref())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        Err(MediaError::UnsupportedPlatform)
    }
}

#[cfg(any(target_os = "windows", test))]
fn checked_pixel_layout(width: u32, height: u32) -> Result<(u32, usize), MediaError> {
    if width == 0 || height == 0 {
        return Err(MediaError::InvalidDimensions);
    }
    if width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
        return Err(MediaError::ImageTooLarge);
    }

    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(MediaError::ImageTooLarge)?;
    if pixels > MAX_IMAGE_PIXELS {
        return Err(MediaError::ImageTooLarge);
    }

    let stride = width
        .checked_mul(BYTES_PER_PIXEL)
        .ok_or(MediaError::ImageTooLarge)?;
    let byte_len = u64::from(stride)
        .checked_mul(u64::from(height))
        .ok_or(MediaError::ImageTooLarge)?;
    let byte_len = usize::try_from(byte_len).map_err(|_| MediaError::ImageTooLarge)?;

    Ok((stride, byte_len))
}

#[cfg(test)]
mod tests {
    use super::{DecodedImage, MediaError, checked_pixel_layout};

    #[test]
    fn rgba_layout_is_checked_and_tightly_packed() {
        assert_eq!(checked_pixel_layout(3840, 2160), Ok((15_360, 33_177_600)));
    }

    #[test]
    fn rejects_empty_dimensions() {
        assert_eq!(
            checked_pixel_layout(0, 1),
            Err(MediaError::InvalidDimensions)
        );
        assert_eq!(
            checked_pixel_layout(1, 0),
            Err(MediaError::InvalidDimensions)
        );
    }

    #[test]
    fn rejects_dimensions_over_the_resource_limit() {
        assert_eq!(
            checked_pixel_layout(16_385, 1),
            Err(MediaError::ImageTooLarge)
        );
        assert_eq!(
            checked_pixel_layout(16_384, 16_384),
            Err(MediaError::ImageTooLarge)
        );
    }

    #[test]
    fn public_error_diagnostics_do_not_disclose_paths() {
        let error = MediaError::DecoderCreation { hresult: -1 };
        assert_eq!(
            error.to_app_error().code,
            wallpaper_core::error::ErrorCode::ImageDecodeFailed
        );
        assert!(!error.diagnostic_summary().contains("C:\\Users"));
    }

    #[test]
    fn video_errors_use_a_safe_video_code_and_path_free_diagnostics() {
        let error = MediaError::VideoSourceOpenFailed { hresult: -1 };
        assert_eq!(
            error.to_app_error().code,
            wallpaper_core::error::ErrorCode::VideoSourceOpenFailed
        );
        assert!(!error.diagnostic_summary().contains("C:\\Users"));
    }

    #[test]
    fn video_com_initialization_uses_video_error_code_and_stage() {
        let error = MediaError::VideoComInitializationFailed { hresult: -1 };
        assert_eq!(
            error.to_app_error().code,
            wallpaper_core::error::ErrorCode::VideoSourceOpenFailed
        );
        assert!(error.diagnostic_summary().contains("stage=video_open"));
    }

    #[test]
    fn unsupported_video_platform_uses_video_diagnostic_stage() {
        let error = MediaError::UnsupportedVideoPlatform;
        assert_eq!(
            error.to_app_error().code,
            wallpaper_core::error::ErrorCode::UnsupportedPlatform
        );
        assert!(error.diagnostic_summary().contains("stage=video_open"));
    }

    #[test]
    fn decoded_image_exposes_owned_pixel_metadata() {
        let image = DecodedImage {
            width: 1,
            height: 1,
            stride: 4,
            rgba8: vec![255, 0, 0, 255],
        };
        assert_eq!((image.width(), image.height(), image.stride()), (1, 1, 4));
        assert_eq!(image.rgba8(), &[255, 0, 0, 255]);
    }
}
