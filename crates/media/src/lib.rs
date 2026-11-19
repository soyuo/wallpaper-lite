//! Still-image decoding boundaries for wallpaper media.

use std::path::Path;

use wallpaper_core::error::{AppError, ErrorCode};

#[cfg(target_os = "windows")]
mod wic;

const MAX_IMAGE_DIMENSION: u32 = 16_384;
const MAX_IMAGE_PIXELS: u64 = 64 * 1024 * 1024;
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

/// Safe failure categories for still-image decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaError {
    UnsupportedPlatform,
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
                "Image decoding is unavailable on this platform.",
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

/// Decode the first frame of a PNG or JPEG file into an owned RGBA8 pixel buffer.
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
