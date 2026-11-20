use std::{os::windows::ffi::OsStrExt, path::Path, ptr};

use windows::{
    Win32::{
        Foundation::RPC_E_CHANGED_MODE,
        Graphics::Direct3D11::{
            D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
            D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING, ID3D11Device, ID3D11Multithread,
            ID3D11Texture2D,
        },
        Graphics::Dxgi::Common::{
            DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8X8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM,
        },
        Media::MediaFoundation::{
            IMFAttributes, IMFDXGIBuffer, IMFDXGIDeviceManager, IMFMediaBuffer, IMFSourceReader,
            MF_E_INVALIDSTREAMNUMBER, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE,
            MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_PD_DURATION, MF_PD_MIME_TYPE,
            MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_SOURCE_READER_ALL_STREAMS,
            MF_SOURCE_READER_D3D_MANAGER, MF_SOURCE_READER_DISABLE_DXVA,
            MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING,
            MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, MF_SOURCE_READER_FIRST_VIDEO_STREAM,
            MF_SOURCE_READER_MEDIASOURCE, MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED,
            MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READERF_ERROR, MF_SOURCE_READERF_STREAMTICK,
            MF_VERSION, MFCreateAttributes, MFCreateDXGIDeviceManager, MFCreateMediaType,
            MFCreateSourceReaderFromURL, MFMediaType_Video, MFSTARTUP_FULL, MFShutdown, MFStartup,
            MFVideoFormat_H264, MFVideoFormat_RGB32,
        },
        System::{
            Com::StructuredStorage::{
                PROPVARIANT, PropVariantClear, PropVariantToInt64, PropVariantToString,
            },
            Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
        },
    },
    core::{GUID, Interface, PCWSTR},
};

use crate::{
    DecodedVideoFrame, MAX_IMAGE_DIMENSION, MAX_IMAGE_PIXELS, MediaError, VideoDecoderMode,
    VideoMetadata, VideoPixelFormat, VideoReadEvent, VideoStreamSelection,
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
    reader: IMFSourceReader,
    _device_manager: Option<IMFDXGIDeviceManager>,
    d3d11_device: Option<ID3D11Device>,
    _runtime: MediaFoundationRuntime,
    metadata: VideoMetadata,
    last_pts_100ns: Option<i64>,
    pending_end_of_stream: bool,
    ended: bool,
}

pub(super) fn open(
    path: &Path,
    d3d11_device: Option<&ID3D11Device>,
) -> Result<(VideoSourceHandle, VideoMetadata, VideoDecoderMode), MediaError> {
    let mut filename: Vec<u16> = path.as_os_str().encode_wide().collect();
    if filename.contains(&0) {
        return Err(MediaError::InvalidVideoPath);
    }
    filename.push(0);

    let runtime = MediaFoundationRuntime::start()?;
    let mut device_manager = None;
    let mut retained_device = None;
    let mut reader = None;
    let mut mode = VideoDecoderMode::Software;
    if let Some(device) = d3d11_device {
        if enable_d3d11_multithread_protection(device) {
            if let Some((manager, attributes)) = create_hardware_reader_attributes(device) {
                if let Ok(hardware_reader) = unsafe {
                    MFCreateSourceReaderFromURL(PCWSTR(filename.as_ptr()), Some(&attributes))
                } {
                    device_manager = Some(manager);
                    retained_device = Some(device.clone());
                    reader = Some(hardware_reader);
                    mode = VideoDecoderMode::HardwarePreferred;
                }
            }
        }
    }
    if reader.is_none() {
        let software_attributes = create_software_reader_attributes();
        reader = Some(
            unsafe {
                MFCreateSourceReaderFromURL(PCWSTR(filename.as_ptr()), software_attributes.as_ref())
            }
            .map_err(|error| MediaError::VideoSourceOpenFailed {
                hresult: error.code().0,
            })?,
        );
    }
    let mut reader = reader.ok_or(MediaError::VideoSourceOpenFailed {
        hresult: 0x80004005u32 as i32,
    })?;
    validate_mp4_container(&reader)?;
    let first_video = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
    let mut media_type = select_h264_video_stream(&reader)?;

    let output_type =
        unsafe { MFCreateMediaType() }.map_err(|error| MediaError::VideoSourceOpenFailed {
            hresult: error.code().0,
        })?;
    unsafe {
        output_type
            .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
            .and_then(|()| output_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32))
            .map_err(|error| MediaError::VideoSourceOpenFailed {
                hresult: error.code().0,
            })?;
    }
    if let Err(error) = unsafe { reader.SetCurrentMediaType(first_video, None, &output_type) } {
        if mode != VideoDecoderMode::HardwarePreferred {
            return Err(MediaError::VideoSampleDecodeFailed {
                hresult: error.code().0,
            });
        }

        drop(reader);
        device_manager = None;
        retained_device = None;
        mode = VideoDecoderMode::Software;
        let software_attributes = create_software_reader_attributes();
        let software_reader = unsafe {
            MFCreateSourceReaderFromURL(PCWSTR(filename.as_ptr()), software_attributes.as_ref())
        }
        .map_err(|software_error| MediaError::VideoSourceOpenFailed {
            hresult: software_error.code().0,
        })?;
        media_type = select_h264_video_stream(&software_reader)?;
        unsafe {
            software_reader
                .SetCurrentMediaType(first_video, None, &output_type)
                .map_err(|software_error| MediaError::VideoSampleDecodeFailed {
                    hresult: software_error.code().0,
                })?;
        }
        reader = software_reader;
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
            reader,
            _device_manager: device_manager,
            d3d11_device: retained_device,
            _runtime: runtime,
            metadata,
            last_pts_100ns: None,
            pending_end_of_stream: false,
            ended: false,
        },
        metadata,
        mode,
    ))
}

fn validate_mp4_container(reader: &IMFSourceReader) -> Result<(), MediaError> {
    let mime = unsafe {
        reader.GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_MIME_TYPE)
    }
    .map_err(|error| MediaError::VideoMetadataUnavailable {
        hresult: error.code().0,
    })?;
    let mime = PropVariantOwner(mime);
    let mut wide = [0u16; 128];
    unsafe { PropVariantToString(ptr::from_ref(&mime.0), &mut wide) }.map_err(|error| {
        MediaError::VideoMetadataUnavailable {
            hresult: error.code().0,
        }
    })?;
    let end = wide
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(wide.len());
    let mime = String::from_utf16_lossy(&wide[..end]);
    if !mime.eq_ignore_ascii_case("video/mp4") && !mime.eq_ignore_ascii_case("audio/mp4") {
        return Err(MediaError::UnsupportedVideoContainer);
    }
    Ok(())
}

fn select_h264_video_stream(
    reader: &IMFSourceReader,
) -> Result<windows::Win32::Media::MediaFoundation::IMFMediaType, MediaError> {
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
    Ok(media_type)
}

fn enable_d3d11_multithread_protection(device: &ID3D11Device) -> bool {
    let Ok(context) = (unsafe { device.GetImmediateContext() }) else {
        return false;
    };
    let Ok(multithread) = context.cast::<ID3D11Multithread>() else {
        return false;
    };
    let _previous_protection = unsafe { multithread.SetMultithreadProtected(true) };
    unsafe { multithread.GetMultithreadProtected() }.as_bool()
}

fn create_hardware_reader_attributes(
    device: &ID3D11Device,
) -> Option<(IMFDXGIDeviceManager, IMFAttributes)> {
    let mut reset_token = 0;
    let mut manager = None;
    unsafe { MFCreateDXGIDeviceManager(&mut reset_token, &mut manager).ok()? };
    let manager = manager?;
    unsafe { manager.ResetDevice(device, reset_token).ok()? };
    let mut attributes = None;
    unsafe { MFCreateAttributes(&mut attributes, 4).ok()? };
    let attributes = attributes?;
    unsafe {
        attributes
            .SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)
            .ok()?;
        attributes
            .SetUINT32(&MF_SOURCE_READER_DISABLE_DXVA, 0)
            .ok()?;
        attributes
            .SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)
            .ok()?;
        attributes
            .SetUnknown(&MF_SOURCE_READER_D3D_MANAGER, &manager)
            .ok()?;
    }
    Some((manager, attributes))
}

fn create_software_reader_attributes() -> Option<IMFAttributes> {
    let mut attributes = None;
    unsafe { MFCreateAttributes(&mut attributes, 1).ok()? };
    let attributes = attributes?;
    unsafe {
        attributes
            .SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)
            .ok()?
    };
    Some(attributes)
}

pub(super) fn read_next(handle: &mut VideoSourceHandle) -> Result<VideoReadEvent, MediaError> {
    if handle.ended {
        return Ok(VideoReadEvent::EndOfStream);
    }
    if handle.pending_end_of_stream {
        handle.ended = true;
        return Ok(VideoReadEvent::EndOfStream);
    }

    let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
    for _ in 0..16 {
        let mut flags = 0u32;
        let mut timestamp = 0i64;
        let mut sample = None;
        unsafe {
            handle
                .reader
                .ReadSample(
                    stream,
                    0,
                    None,
                    Some(&mut flags),
                    Some(&mut timestamp),
                    Some(&mut sample),
                )
                .map_err(|error| MediaError::VideoSampleDecodeFailed {
                    hresult: error.code().0,
                })?;
        }
        if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
            return Err(MediaError::VideoDecoderReportedError);
        }
        if flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 != 0 {
            validate_current_output_type(handle)?;
            if sample.is_none() {
                continue;
            }
        }
        if flags & MF_SOURCE_READERF_STREAMTICK.0 as u32 != 0 {
            if timestamp < 0 {
                return Err(MediaError::InvalidVideoTimestamp);
            }
            return Ok(VideoReadEvent::StreamTick {
                timestamp_100ns: timestamp,
            });
        }
        let is_eos = flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0;
        if let Some(sample) = sample {
            let frame = decode_sample(handle, &sample, timestamp)?;
            handle.pending_end_of_stream = is_eos;
            return Ok(VideoReadEvent::Frame(frame));
        }
        if is_eos {
            handle.ended = true;
            return Ok(VideoReadEvent::EndOfStream);
        }
    }
    Err(MediaError::VideoDecoderReportedError)
}

pub(super) fn seek(handle: &mut VideoSourceHandle, position_100ns: i64) -> Result<(), MediaError> {
    let position = PROPVARIANT::from(position_100ns);
    let time_format = GUID::zeroed();
    unsafe {
        handle
            .reader
            .SetCurrentPosition(&time_format, &position)
            .map_err(|error| MediaError::VideoSeekFailed {
                hresult: error.code().0,
            })?;
    }
    handle.last_pts_100ns = None;
    handle.pending_end_of_stream = false;
    handle.ended = false;
    Ok(())
}

fn validate_current_output_type(handle: &VideoSourceHandle) -> Result<(), MediaError> {
    let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
    let media_type = unsafe { handle.reader.GetCurrentMediaType(stream) }.map_err(|error| {
        MediaError::VideoSampleDecodeFailed {
            hresult: error.code().0,
        }
    })?;
    let subtype = unsafe { media_type.GetGUID(&MF_MT_SUBTYPE) }.map_err(|error| {
        MediaError::VideoSampleDecodeFailed {
            hresult: error.code().0,
        }
    })?;
    let packed_size = unsafe { media_type.GetUINT64(&MF_MT_FRAME_SIZE) }.map_err(|error| {
        MediaError::VideoSampleDecodeFailed {
            hresult: error.code().0,
        }
    })?;
    validate_output_format(
        subtype,
        packed_size,
        handle.metadata.width(),
        handle.metadata.height(),
    )
}

fn validate_output_format(
    subtype: GUID,
    packed_size: u64,
    expected_width: u32,
    expected_height: u32,
) -> Result<(), MediaError> {
    if subtype != MFVideoFormat_RGB32
        || (packed_size >> 32) as u32 != expected_width
        || packed_size as u32 != expected_height
    {
        return Err(MediaError::VideoFormatChanged);
    }
    Ok(())
}

fn decode_sample(
    handle: &mut VideoSourceHandle,
    sample: &windows::Win32::Media::MediaFoundation::IMFSample,
    timestamp_100ns: i64,
) -> Result<DecodedVideoFrame, MediaError> {
    if timestamp_100ns < 0
        || handle
            .last_pts_100ns
            .is_some_and(|last| timestamp_100ns <= last)
    {
        return Err(MediaError::InvalidVideoTimestamp);
    }
    let duration_100ns = unsafe { sample.GetSampleDuration() }
        .ok()
        .filter(|duration| *duration > 0)
        .unwrap_or_else(|| frame_duration_100ns(handle.metadata));
    if duration_100ns <= 0 {
        return Err(MediaError::InvalidVideoSample);
    }

    let width = handle.metadata.width();
    let height = handle.metadata.height();
    let (rgba_stride, byte_len) =
        crate::checked_pixel_layout(width, height).map_err(|_| MediaError::InvalidVideoSample)?;
    let rgba8 = if let (Some(device), Ok(buffer)) = (handle.d3d11_device.as_ref(), unsafe {
        sample.GetBufferByIndex(0)
    }) {
        if let Ok(dxgi_buffer) = buffer.cast::<IMFDXGIBuffer>() {
            copy_dxgi_sample_to_rgba8(&dxgi_buffer, device, width, height, byte_len)?
        } else {
            copy_locked_sample_to_rgba8(
                sample,
                &handle.reader,
                width,
                height,
                rgba_stride,
                byte_len,
            )?
        }
    } else {
        copy_locked_sample_to_rgba8(sample, &handle.reader, width, height, rgba_stride, byte_len)?
    };

    handle.last_pts_100ns = Some(timestamp_100ns);
    Ok(DecodedVideoFrame {
        width,
        height,
        stride: rgba_stride,
        pixel_format: VideoPixelFormat::Rgba8,
        rgba8,
        pts_100ns: timestamp_100ns,
        duration_100ns,
    })
}

fn copy_locked_sample_to_rgba8(
    sample: &windows::Win32::Media::MediaFoundation::IMFSample,
    reader: &IMFSourceReader,
    width: u32,
    height: u32,
    rgba_stride: u32,
    byte_len: usize,
) -> Result<Vec<u8>, MediaError> {
    let buffer = unsafe { sample.ConvertToContiguousBuffer() }.map_err(|error| {
        MediaError::VideoSampleBufferAccess {
            hresult: error.code().0,
        }
    })?;
    let mut default_stride =
        i32::try_from(rgba_stride).map_err(|_| MediaError::InvalidVideoSample)?;
    if let Ok(current_type) =
        unsafe { reader.GetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32) }
    {
        if let Ok(value) = unsafe { current_type.GetUINT32(&MF_MT_DEFAULT_STRIDE) } {
            default_stride = value as i32;
        }
    }
    let abs_stride = default_stride
        .checked_abs()
        .ok_or(MediaError::InvalidVideoSample)? as usize;
    let row_bytes = usize::try_from(width).map_err(|_| MediaError::InvalidVideoSample)? * 4;
    if abs_stride < row_bytes {
        return Err(MediaError::InvalidVideoSample);
    }
    let needed_len = abs_stride
        .checked_mul(height as usize)
        .ok_or(MediaError::InvalidVideoSample)?;
    let mut data = ptr::null_mut();
    let mut max_length = 0u32;
    let mut current_length = 0u32;
    unsafe { buffer.Lock(&mut data, Some(&mut max_length), Some(&mut current_length)) }.map_err(
        |error| MediaError::VideoSampleBufferAccess {
            hresult: error.code().0,
        },
    )?;
    let lock = MediaBufferLock { buffer: &buffer };
    if data.is_null()
        || current_length as usize > max_length as usize
        || (current_length as usize) < needed_len
    {
        drop(lock);
        return Err(MediaError::InvalidVideoSample);
    }
    let mut rgba8 = allocate_frame_buffer(byte_len)?;
    for y in 0..height as usize {
        let source_y = if default_stride < 0 {
            height as usize - 1 - y
        } else {
            y
        };
        let row = unsafe { std::slice::from_raw_parts(data.add(source_y * abs_stride), row_bytes) };
        for (source, destination) in row.chunks_exact(4).zip(
            rgba8[y * rgba_stride as usize..(y + 1) * rgba_stride as usize].chunks_exact_mut(4),
        ) {
            destination.copy_from_slice(&[source[2], source[1], source[0], 255]);
        }
    }
    drop(lock);
    Ok(rgba8)
}

fn copy_dxgi_sample_to_rgba8(
    buffer: &IMFDXGIBuffer,
    device: &ID3D11Device,
    width: u32,
    height: u32,
    byte_len: usize,
) -> Result<Vec<u8>, MediaError> {
    let mut raw_texture = ptr::null_mut();
    unsafe { buffer.GetResource(&ID3D11Texture2D::IID, &mut raw_texture) }.map_err(|error| {
        MediaError::VideoSampleBufferAccess {
            hresult: error.code().0,
        }
    })?;
    if raw_texture.is_null() {
        return Err(MediaError::InvalidVideoSample);
    }
    let texture = unsafe { ID3D11Texture2D::from_raw(raw_texture) };
    let mut source_desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { texture.GetDesc(&mut source_desc) };
    if source_desc.Width < width || source_desc.Height < height || source_desc.SampleDesc.Count != 1
    {
        return Err(MediaError::InvalidVideoSample);
    }
    if source_desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM
        && source_desc.Format != DXGI_FORMAT_B8G8R8X8_UNORM
        && source_desc.Format != DXGI_FORMAT_R8G8B8A8_UNORM
    {
        return Err(MediaError::UnsupportedVideoPixelFormat);
    }

    let mut staging_desc = source_desc;
    staging_desc.Width = width;
    staging_desc.Height = height;
    staging_desc.MipLevels = 1;
    staging_desc.ArraySize = 1;
    staging_desc.SampleDesc.Count = 1;
    staging_desc.SampleDesc.Quality = 0;
    staging_desc.Usage = D3D11_USAGE_STAGING;
    staging_desc.BindFlags = 0;
    staging_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
    staging_desc.MiscFlags = 0;
    let mut staging = None;
    unsafe { device.CreateTexture2D(&staging_desc, None, Some(&mut staging)) }.map_err(
        |error| MediaError::VideoSampleBufferAccess {
            hresult: error.code().0,
        },
    )?;
    let staging = staging.ok_or(MediaError::InvalidVideoSample)?;
    let subresource = unsafe { buffer.GetSubresourceIndex() }.map_err(|error| {
        MediaError::VideoSampleBufferAccess {
            hresult: error.code().0,
        }
    })?;
    let source_box = D3D11_BOX {
        left: 0,
        top: 0,
        front: 0,
        right: width,
        bottom: height,
        back: 1,
    };
    let context = unsafe { device.GetImmediateContext() }.map_err(|error| {
        MediaError::VideoSampleBufferAccess {
            hresult: error.code().0,
        }
    })?;
    unsafe {
        context.CopySubresourceRegion(
            &staging,
            0,
            0,
            0,
            0,
            &texture,
            subresource,
            Some(&source_box),
        )
    };
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe { context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) }.map_err(|error| {
        MediaError::VideoSampleBufferAccess {
            hresult: error.code().0,
        }
    })?;
    let map_guard = D3dTextureMap {
        context: &context,
        texture: &staging,
    };
    let row_bytes = width as usize * 4;
    if mapped.pData.is_null() || (mapped.RowPitch as usize) < row_bytes {
        drop(map_guard);
        return Err(MediaError::InvalidVideoSample);
    }
    let mut rgba8 = allocate_frame_buffer(byte_len)?;
    for y in 0..height as usize {
        let row = unsafe {
            std::slice::from_raw_parts(
                mapped.pData.cast::<u8>().add(y * mapped.RowPitch as usize),
                row_bytes,
            )
        };
        for (source, destination) in row
            .chunks_exact(4)
            .zip(rgba8[y * row_bytes..(y + 1) * row_bytes].chunks_exact_mut(4))
        {
            match source_desc.Format {
                DXGI_FORMAT_B8G8R8A8_UNORM | DXGI_FORMAT_B8G8R8X8_UNORM => {
                    destination.copy_from_slice(&[source[2], source[1], source[0], 255])
                }
                DXGI_FORMAT_R8G8B8A8_UNORM => {
                    destination.copy_from_slice(&[source[0], source[1], source[2], 255])
                }
                _ => unreachable!(),
            }
        }
    }
    drop(map_guard);
    Ok(rgba8)
}

fn allocate_frame_buffer(byte_len: usize) -> Result<Vec<u8>, MediaError> {
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(byte_len)
        .map_err(|_| MediaError::VideoFrameAllocationFailed)?;
    pixels.resize(byte_len, 255);
    Ok(pixels)
}

struct MediaBufferLock<'a> {
    buffer: &'a IMFMediaBuffer,
}
impl Drop for MediaBufferLock<'_> {
    fn drop(&mut self) {
        let _ = unsafe { self.buffer.Unlock() };
    }
}

struct D3dTextureMap<'a> {
    context: &'a windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext,
    texture: &'a ID3D11Texture2D,
}
impl Drop for D3dTextureMap<'_> {
    fn drop(&mut self) {
        unsafe { self.context.Unmap(self.texture, 0) };
    }
}

fn frame_duration_100ns(metadata: VideoMetadata) -> i64 {
    let numerator = i128::from(metadata.frame_rate_denominator()) * 10_000_000;
    let denominator = i128::from(metadata.frame_rate_numerator());
    ((numerator + denominator / 2) / denominator)
        .try_into()
        .unwrap_or(0)
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
    use super::{validate_output_format, validate_video_metadata};
    use crate::MediaError;
    use windows::Win32::Media::MediaFoundation::{MFVideoFormat_H264, MFVideoFormat_RGB32};

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

    #[test]
    fn rejects_video_output_format_changes_with_a_typed_error() {
        let packed_size = (64u64 << 32) | 48;
        assert_eq!(
            validate_output_format(MFVideoFormat_H264, packed_size, 64, 48),
            Err(MediaError::VideoFormatChanged)
        );
        assert_eq!(
            validate_output_format(MFVideoFormat_RGB32, (128u64 << 32) | 48, 64, 48),
            Err(MediaError::VideoFormatChanged)
        );
    }
}
