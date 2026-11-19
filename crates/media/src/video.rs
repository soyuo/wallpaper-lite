use std::{os::windows::ffi::OsStrExt, path::Path, ptr};

use windows::{
    Win32::{
        Foundation::RPC_E_CHANGED_MODE,
        Media::MediaFoundation::{
            IMFAttributes, IMFSourceReader, MF_E_INVALIDSTREAMNUMBER, MF_MT_FRAME_RATE,
            MF_MT_FRAME_SIZE, MF_MT_SUBTYPE, MF_PD_DURATION, MF_SOURCE_READER_ALL_STREAMS,
            MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_SOURCE_READER_MEDIASOURCE, MF_VERSION,
            MFCreateSourceReaderFromURL, MFSTARTUP_FULL, MFShutdown, MFStartup, MFVideoFormat_H264,
        },
        System::{
            Com::StructuredStorage::{PROPVARIANT, PropVariantClear, PropVariantToInt64},
            Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
        },
    },
    core::PCWSTR,
};

use crate::{
    MAX_IMAGE_DIMENSION, MAX_IMAGE_PIXELS, MediaError, VideoMetadata, VideoStreamSelection,
};

struct ComApartment {
    initialized_here: bool,
}

impl ComApartment {
    fn initialize() -> Result<Self, MediaError> {
        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if result == RPC_E_CHANGED_MODE {
            return Ok(Self {
                initialized_here: false,
            });
        }
        result.ok().map_err(|error| MediaError::ComInitialization {
            hresult: error.code().0,
        })?;
        Ok(Self {
            initialized_here: true,
        })
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.initialized_here {
            unsafe { CoUninitialize() };
        }
    }
}

struct MediaFoundationRuntime {
    _com_apartment: ComApartment,
}

impl MediaFoundationRuntime {
    fn start() -> Result<Self, MediaError> {
        let com_apartment = ComApartment::initialize().map_err(|error| match error {
            MediaError::ComInitialization { hresult } => {
                MediaError::VideoComInitializationFailed { hresult }
            }
            other => other,
        })?;
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }.map_err(|error| {
            MediaError::MediaFoundationStartupFailed {
                hresult: error.code().0,
            }
        })?;
        Ok(Self {
            _com_apartment: com_apartment,
        })
    }
}

impl Drop for MediaFoundationRuntime {
    fn drop(&mut self) {
        let _ = unsafe { MFShutdown() };
    }
}

struct PropVariantOwner(PROPVARIANT);

impl Drop for PropVariantOwner {
    fn drop(&mut self) {
        let _ = unsafe { PropVariantClear(&mut self.0) };
    }
}

/// Keep the source reader alive until before its Media Foundation runtime is shut down.
pub(super) struct VideoSourceHandle {
    _reader: IMFSourceReader,
    _runtime: MediaFoundationRuntime,
}

pub(super) fn open(path: &Path) -> Result<(VideoSourceHandle, VideoMetadata), MediaError> {
    let mut filename: Vec<u16> = path.as_os_str().encode_wide().collect();
    if filename.contains(&0) {
        return Err(MediaError::InvalidVideoPath);
    }
    filename.push(0);

    let runtime = MediaFoundationRuntime::start()?;
    let reader =
        unsafe { MFCreateSourceReaderFromURL(PCWSTR(filename.as_ptr()), None::<&IMFAttributes>) }
            .map_err(|error| MediaError::VideoSourceOpenFailed {
            hresult: error.code().0,
        })?;

    let all_streams = MF_SOURCE_READER_ALL_STREAMS.0 as u32;
    let first_video = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
    unsafe {
        reader
            .SetStreamSelection(all_streams, false)
            .map_err(|error| MediaError::VideoSourceOpenFailed {
                hresult: error.code().0,
            })?;
        reader
            .SetStreamSelection(first_video, true)
            .map_err(|error| map_stream_error(error.code().0))?;
    }

    let media_type = unsafe { reader.GetNativeMediaType(first_video, 0) }
        .map_err(|error| map_stream_error(error.code().0))?;
    let subtype = unsafe { media_type.GetGUID(&MF_MT_SUBTYPE) }.map_err(|error| {
        MediaError::VideoMetadataUnavailable {
            hresult: error.code().0,
        }
    })?;
    if subtype != MFVideoFormat_H264 {
        return Err(MediaError::UnsupportedVideoFormat);
    }

    let packed_size = unsafe { media_type.GetUINT64(&MF_MT_FRAME_SIZE) }.map_err(|error| {
        MediaError::VideoMetadataUnavailable {
            hresult: error.code().0,
        }
    })?;
    let packed_rate = unsafe { media_type.GetUINT64(&MF_MT_FRAME_RATE) }.map_err(|error| {
        MediaError::VideoMetadataUnavailable {
            hresult: error.code().0,
        }
    })?;
    let width = (packed_size >> 32) as u32;
    let height = packed_size as u32;
    let frame_rate_numerator = (packed_rate >> 32) as u32;
    let frame_rate_denominator = packed_rate as u32;
    validate_video_metadata(width, height, frame_rate_numerator, frame_rate_denominator)?;

    let duration = unsafe {
        reader.GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
    }
    .map_err(|error| MediaError::VideoMetadataUnavailable {
        hresult: error.code().0,
    })?;
    let duration = PropVariantOwner(duration);
    let duration_100ns =
        unsafe { PropVariantToInt64(ptr::from_ref(&duration.0)) }.map_err(|error| {
            MediaError::VideoMetadataUnavailable {
                hresult: error.code().0,
            }
        })?;
    if duration_100ns <= 0 {
        return Err(MediaError::InvalidVideoMetadata);
    }

    let metadata = VideoMetadata {
        width,
        height,
        frame_rate_numerator,
        frame_rate_denominator,
        duration_100ns,
        selected_stream: VideoStreamSelection::FirstVideo,
    };

    Ok((
        VideoSourceHandle {
            _reader: reader,
            _runtime: runtime,
        },
        metadata,
    ))
}

fn map_stream_error(hresult: i32) -> MediaError {
    if hresult == MF_E_INVALIDSTREAMNUMBER.0 {
        MediaError::VideoStreamNotFound { hresult }
    } else {
        MediaError::VideoSourceOpenFailed { hresult }
    }
}

fn validate_video_metadata(
    width: u32,
    height: u32,
    frame_rate_numerator: u32,
    frame_rate_denominator: u32,
) -> Result<(), MediaError> {
    if width == 0
        || height == 0
        || width > MAX_IMAGE_DIMENSION
        || height > MAX_IMAGE_DIMENSION
        || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS
        || frame_rate_numerator == 0
        || frame_rate_denominator == 0
    {
        return Err(MediaError::InvalidVideoMetadata);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_video_metadata;
    use crate::MediaError;

    #[test]
    fn accepts_bounded_video_metadata() {
        assert_eq!(validate_video_metadata(3840, 2160, 60, 1), Ok(()));
    }

    #[test]
    fn rejects_zero_or_excessive_video_dimensions() {
        assert_eq!(
            validate_video_metadata(0, 48, 30, 1),
            Err(MediaError::InvalidVideoMetadata)
        );
        assert_eq!(
            validate_video_metadata(16_385, 48, 30, 1),
            Err(MediaError::InvalidVideoMetadata)
        );
        assert_eq!(
            validate_video_metadata(16_384, 4_097, 30, 1),
            Err(MediaError::InvalidVideoMetadata)
        );
    }

    #[test]
    fn rejects_zero_frame_rate_components() {
        assert_eq!(
            validate_video_metadata(64, 48, 0, 1),
            Err(MediaError::InvalidVideoMetadata)
        );
        assert_eq!(
            validate_video_metadata(64, 48, 30, 0),
            Err(MediaError::InvalidVideoMetadata)
        );
    }
}
