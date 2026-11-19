#![cfg(target_os = "windows")]

use std::{fs, path::PathBuf};

use wallpaper_media::{MediaError, decode_image};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn decodes_png_to_rgba8_and_preserves_the_source() {
    let path = fixture("pixels.png");
    let source_before = fs::read(&path).expect("PNG fixture should be readable");

    let image = decode_image(&path).expect("WIC should decode the PNG fixture");

    assert_eq!((image.width(), image.height(), image.stride()), (2, 1, 8));
    assert_eq!(
        image.rgba8(),
        &[255, 0, 0, 255, 12, 34, 56, 128],
        "PNG channels and alpha should be preserved in RGBA order"
    );
    assert_eq!(fs::read(&path).unwrap(), source_before);
}

#[test]
fn decodes_jpeg_pixels_to_rgba8() {
    let image = decode_image(fixture("solid.jpg")).expect("WIC should decode the JPEG fixture");

    assert_eq!(
        (image.width(), image.height(), image.stride()),
        (16, 16, 64)
    );
    for rgba in image.rgba8().chunks_exact(4) {
        assert!(
            (i16::from(rgba[0]) - 20).abs() <= 5,
            "unexpected red channel"
        );
        assert!(
            (i16::from(rgba[1]) - 100).abs() <= 5,
            "unexpected green channel"
        );
        assert!(
            (i16::from(rgba[2]) - 200).abs() <= 5,
            "unexpected blue channel"
        );
        assert_eq!(rgba[3], 255);
    }
}

#[test]
fn decodes_a_4k_png_with_checked_dimensions() {
    let image = decode_image(fixture("4k.png")).expect("WIC should decode the 4K PNG fixture");

    assert_eq!((image.width(), image.height()), (3840, 2160));
    assert_eq!(image.stride(), 15_360);
    assert_eq!(image.rgba8().len(), 33_177_600);
    assert_eq!(&image.rgba8()[..4], &[12, 34, 56, 255]);
}

#[test]
fn malformed_input_returns_a_safe_typed_error() {
    let path = fixture("malformed.bin");
    let error = decode_image(&path).expect_err("malformed bytes must be rejected");

    assert!(matches!(
        error,
        MediaError::DecoderCreation { .. }
            | MediaError::FrameAccess { .. }
            | MediaError::PixelFormatConversion { .. }
            | MediaError::PixelCopy { .. }
    ));
    assert!(!error.diagnostic_summary().contains("malformed.bin"));
    assert!(!error.to_app_error().message.contains("malformed.bin"));
}

#[test]
fn rejects_other_wic_supported_formats() {
    let path = fixture("unsupported.bmp");

    let error = decode_image(&path).expect_err("BMP is outside the PNG/JPEG contract");

    assert_eq!(error, MediaError::UnsupportedImageFormat);
    assert!(!error.diagnostic_summary().contains("unsupported.bmp"));
}
