#![cfg(target_os = "windows")]

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use wallpaper_media::{
    CatalogStore, ImportFailure, ImportStatus, WallpaperLibrary, import_local_media,
};

static DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let counter = DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "wallpaper-lite-import-test-{}-{counter}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("test directory should be created");
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn imports_mixed_folder_by_content_keeps_sources_and_deduplicates_by_canonical_path() {
    let directory = TestDirectory::new();
    let root = directory.path().join("selected");
    let nested = root.join("nested");
    fs::create_dir_all(&nested).unwrap();

    // Deliberately mismatch extensions: the bytes, not the suffix, select the decoder.
    let png_as_jpeg = root.join("actual-png.jpg");
    let mp4_as_binary = nested.join("actual-mp4.bin");
    let bmp = nested.join("actual-bmp.data");
    let invalid = root.join("invalid.png");
    let unsupported_container = root.join("h264-avi.mp4");
    let unsupported_codec = root.join("unsupported-codec.mp4");
    fs::copy(fixture("pixels.png"), &png_as_jpeg).unwrap();
    fs::copy(fixture("short-h264.mp4"), &mp4_as_binary).unwrap();
    fs::copy(fixture("tiny.bmp"), &bmp).unwrap();
    fs::copy(fixture("unsupported.gif"), &invalid).unwrap();
    fs::copy(fixture("h264-avi.avi"), &unsupported_container).unwrap();
    fs::copy(fixture("unsupported-mpeg4.mp4"), &unsupported_codec).unwrap();
    let source_bytes = [
        fs::read(&png_as_jpeg).unwrap(),
        fs::read(&mp4_as_binary).unwrap(),
        fs::read(&bmp).unwrap(),
        fs::read(&invalid).unwrap(),
        fs::read(&unsupported_container).unwrap(),
        fs::read(&unsupported_codec).unwrap(),
    ];

    let store = CatalogStore::at_path(directory.path().join("WallpaperLite/library.json"));
    let results = import_local_media(&store, &[root.clone(), png_as_jpeg.clone()]).unwrap();

    assert_eq!(results.len(), 7);
    assert_eq!(
        results
            .iter()
            .filter(|result| result.status == ImportStatus::Imported)
            .count(),
        3
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| result.status == ImportStatus::AlreadyPresent)
            .count(),
        1
    );
    assert!(results.iter().any(|result| {
        result.file_name == "invalid.png"
            && result.status == ImportStatus::Rejected
            && result.failure == Some(ImportFailure::UnsupportedFormat)
    }));
    assert!(results.iter().any(|result| {
        result.file_name == "h264-avi.mp4"
            && result.failure == Some(ImportFailure::UnsupportedFormat)
    }));
    assert!(results.iter().any(|result| {
        result.file_name == "unsupported-codec.mp4"
            && result.failure == Some(ImportFailure::CodecUnavailable)
    }));

    let library = store.load().unwrap();
    assert_eq!(library.entries().len(), 3);
    assert_eq!(fs::read(&png_as_jpeg).unwrap(), source_bytes[0]);
    assert_eq!(fs::read(&mp4_as_binary).unwrap(), source_bytes[1]);
    assert_eq!(fs::read(&bmp).unwrap(), source_bytes[2]);
    assert_eq!(fs::read(&invalid).unwrap(), source_bytes[3]);
    assert_eq!(fs::read(&unsupported_container).unwrap(), source_bytes[4]);
    assert_eq!(fs::read(&unsupported_codec).unwrap(), source_bytes[5]);

    let repeated = import_local_media(&store, &[root]).unwrap();
    assert_eq!(
        repeated
            .iter()
            .filter(|result| result.status == ImportStatus::AlreadyPresent)
            .count(),
        3
    );
    assert_eq!(store.load().unwrap(), library);
}

#[test]
fn reports_unavailable_and_corrupt_candidates_without_creating_entries() {
    let directory = TestDirectory::new();
    let missing = directory.path().join("missing.png");
    let corrupt = directory.path().join("corrupt.mp4");
    fs::write(&corrupt, b"not media").unwrap();
    let store = CatalogStore::at_path(directory.path().join("library.json"));

    let results = import_local_media(&store, &[missing, corrupt]).unwrap();
    assert_eq!(results.len(), 2);
    assert!(results.iter().any(|result| {
        result.status == ImportStatus::Unavailable
            && result.failure == Some(ImportFailure::SourceUnavailable)
    }));
    assert!(results.iter().any(|result| {
        result.status == ImportStatus::Rejected
            && result.failure == Some(ImportFailure::InvalidMedia)
    }));
    assert_eq!(store.load().unwrap(), WallpaperLibrary::default());
}
