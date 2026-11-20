use std::{os::windows::ffi::OsStrExt, path::Path, ptr};

use windows::{
    Win32::{
        Foundation::{GENERIC_READ, RPC_E_CHANGED_MODE},
        Graphics::Imaging::{
            CLSID_WICImagingFactory, GUID_ContainerFormatBmp, GUID_ContainerFormatJpeg,
            GUID_ContainerFormatPng, GUID_WICPixelFormat32bppRGBA, IWICBitmapFrameDecode,
            IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom,
            WICDecodeMetadataCacheOnDemand,
        },
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize,
        },
    },
    core::PCWSTR,
};

use crate::{DecodedImage, MediaError, checked_pixel_layout};

struct ComApartment {
    initialized_here: bool,
}

impl ComApartment {
    fn initialize() -> Result<Self, MediaError> {
        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if result == RPC_E_CHANGED_MODE {
            // COM is already initialized on this thread with another apartment model.
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

pub(super) fn decode_image(path: &Path) -> Result<DecodedImage, MediaError> {
    let _apartment = ComApartment::initialize()?;
    let factory: IWICImagingFactory =
        unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }.map_err(
            |error| MediaError::FactoryCreation {
                hresult: error.code().0,
            },
        )?;

    let filename = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let decoder = unsafe {
        factory.CreateDecoderFromFilename(
            PCWSTR(filename.as_ptr()),
            None,
            GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        )
    }
    .map_err(|error| MediaError::DecoderCreation {
        hresult: error.code().0,
    })?;

    let container =
        unsafe { decoder.GetContainerFormat() }.map_err(|error| MediaError::DecoderCreation {
            hresult: error.code().0,
        })?;
    if container != GUID_ContainerFormatPng
        && container != GUID_ContainerFormatJpeg
        && container != GUID_ContainerFormatBmp
    {
        return Err(MediaError::UnsupportedImageFormat);
    }

    let frame_count =
        unsafe { decoder.GetFrameCount() }.map_err(|error| MediaError::FrameAccess {
            hresult: error.code().0,
        })?;
    if frame_count == 0 {
        return Err(MediaError::ImageHasNoFrames);
    }
    let frame: IWICBitmapFrameDecode =
        unsafe { decoder.GetFrame(0) }.map_err(|error| MediaError::FrameAccess {
            hresult: error.code().0,
        })?;

    let (mut width, mut height) = (0, 0);
    unsafe { frame.GetSize(&mut width, &mut height) }.map_err(|error| MediaError::FrameAccess {
        hresult: error.code().0,
    })?;
    let (stride, byte_len) = checked_pixel_layout(width, height)?;
    let mut rgba8 = Vec::new();
    rgba8
        .try_reserve_exact(byte_len)
        .map_err(|_| MediaError::AllocationFailed)?;
    rgba8.resize(byte_len, 0);

    let converter = unsafe { factory.CreateFormatConverter() }.map_err(|error| {
        MediaError::PixelFormatConversion {
            hresult: error.code().0,
        }
    })?;
    unsafe {
        converter.Initialize(
            &frame,
            &GUID_WICPixelFormat32bppRGBA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )
    }
    .map_err(|error| MediaError::PixelFormatConversion {
        hresult: error.code().0,
    })?;
    unsafe { converter.CopyPixels(ptr::null(), stride, &mut rgba8) }.map_err(|error| {
        MediaError::PixelCopy {
            hresult: error.code().0,
        }
    })?;

    Ok(DecodedImage {
        width,
        height,
        stride,
        rgba8,
    })
}
