#[cfg(target_os = "windows")]
use std::fs;
use std::path::PathBuf;

#[cfg(target_os = "windows")]
use wallpaper_media::VideoStreamSelection;
use wallpaper_media::{MediaError, VideoSource};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[cfg(target_os = "windows")]
fn open_error(path: impl AsRef<std::path::Path>) -> MediaError {
    match VideoSource::open(path) {
        Ok(_) => panic!("video source should not open"),
        Err(error) => error,
    }
}

#[cfg(target_os = "windows")]
#[test]
fn opens_h264_mp4_and_reports_metadata_without_modifying_source() {
    let path = fixture("short-h264.mp4");
    let original_bytes = fs::read(&path).unwrap();

    {
        let source = VideoSource::open(&path).expect("H.264 fixture should open");
        let metadata = source.metadata();

        assert_eq!((metadata.width(), metadata.height()), (64, 48));
        assert_eq!(
            (
                metadata.frame_rate_numerator(),
                metadata.frame_rate_denominator()
            ),
            (30, 1)
        );
        assert_eq!(metadata.duration_100ns(), 10_000_000);
        assert_eq!(metadata.selected_stream(), VideoStreamSelection::FirstVideo);
    }

    assert_eq!(fs::read(&path).unwrap(), original_bytes);
}

#[cfg(target_os = "windows")]
#[test]
fn rejects_audio_only_mp4_with_a_typed_error_and_safe_diagnostic() {
    let path = fixture("audio-only.mp4");
    let error = open_error(&path);

    assert!(matches!(error, MediaError::VideoStreamNotFound { .. }));
    assert_eq!(
        error.to_app_error().code,
        wallpaper_core::error::ErrorCode::VideoSourceOpenFailed
    );
    assert!(!error.diagnostic_summary().contains("audio-only.mp4"));
}

#[cfg(target_os = "windows")]
#[test]
fn rejects_corrupt_video_source_without_disclosing_its_path() {
    let path = fixture("malformed.bin");
    let error = open_error(&path);

    assert!(matches!(
        error,
        MediaError::VideoSourceOpenFailed { .. }
            | MediaError::VideoStreamNotFound { .. }
            | MediaError::VideoMetadataUnavailable { .. }
    ));
    assert!(!error.diagnostic_summary().contains("malformed.bin"));
}

#[cfg(target_os = "windows")]
#[test]
fn rejects_non_h264_video_as_unsupported() {
    let path = fixture("unsupported-mpeg4.mp4");
    let error = open_error(&path);

    assert_eq!(error, MediaError::UnsupportedVideoFormat);
    assert!(!error.diagnostic_summary().contains("unsupported-mpeg4.mp4"));
}

#[cfg(target_os = "windows")]
#[test]
fn rejects_embedded_nul_in_path_before_calling_media_foundation() {
    let error = open_error(std::path::Path::new("invalid\0video.mp4"));
    assert_eq!(error, MediaError::InvalidVideoPath);
}

#[cfg(target_os = "windows")]
#[test]
fn open_failures_and_multiple_sources_do_not_break_later_initialization() {
    let path = fixture("short-h264.mp4");
    assert!(VideoSource::open(fixture("malformed.bin")).is_err());

    let first = VideoSource::open(&path).expect("first source should open");
    let second = VideoSource::open(&path).expect("second source should open concurrently");
    drop(first);
    assert_eq!(second.metadata().width(), 64);
    drop(second);

    assert!(VideoSource::open(path).is_ok());
}

#[cfg(not(target_os = "windows"))]
#[test]
fn video_source_reports_unsupported_platform() {
    assert!(matches!(
        VideoSource::open(fixture("short-h264.mp4")),
        Err(MediaError::UnsupportedVideoPlatform)
    ));
}
