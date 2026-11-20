use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::UNIX_EPOCH,
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use wallpaper_core::error::{AppError, ErrorCode};
use wallpaper_media::{
    Availability, CatalogStore, MediaKind, VideoReadEvent, VideoSource, decode_image,
};

const THUMBNAIL_EDGE: u32 = 320;
const THUMBNAIL_CACHE_LIMIT_BYTES: u64 = 64 * 1024 * 1024;
const BITMAP_HEADER_BYTES: usize = 54;
static THUMBNAIL_NONCE: AtomicU64 = AtomicU64::new(0);
static THUMBNAIL_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static CATALOG_MUTATION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub(crate) fn lock_catalog_mutation() -> Result<std::sync::MutexGuard<'static, ()>, AppError> {
    CATALOG_MUTATION_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| internal_error("The wallpaper library is busy."))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryEntryView {
    id: u64,
    display_name: String,
    source_path: String,
    media_kind: MediaKind,
    is_favorite: bool,
    availability: Availability,
    size_bytes: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryPreview {
    width: u32,
    height: u32,
    duration_100ns: Option<i64>,
    size_bytes: u64,
    thumbnail_data_url: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedPreviewMetadata {
    width: u32,
    height: u32,
    duration_100ns: Option<i64>,
}

#[tauri::command]
pub async fn list_library() -> Result<Vec<LibraryEntryView>, AppError> {
    let store = CatalogStore::for_current_user().map_err(|error| error.to_app_error())?;
    tauri::async_runtime::spawn_blocking(move || {
        let library = store.load().map_err(|error| error.to_app_error())?;
        Ok(library
            .entries()
            .iter()
            .map(|entry| LibraryEntryView {
                id: entry.id().get(),
                display_name: entry.display_name().to_owned(),
                source_path: entry.source_path().to_string_lossy().into_owned(),
                media_kind: entry.media_kind(),
                is_favorite: entry.is_favorite(),
                availability: entry.availability(),
                size_bytes: fs::metadata(entry.source_path())
                    .ok()
                    .filter(fs::Metadata::is_file)
                    .map(|metadata| metadata.len()),
            })
            .collect())
    })
    .await
    .map_err(|_| internal_error("The wallpaper library could not be listed."))?
}

#[tauri::command]
pub async fn library_preview(id: u64) -> Result<LibraryPreview, AppError> {
    let store = CatalogStore::for_current_user().map_err(|error| error.to_app_error())?;
    tauri::async_runtime::spawn_blocking(move || build_preview(&store, id))
        .await
        .map_err(|_| internal_error("The wallpaper preview could not be created."))?
}

#[tauri::command]
pub async fn remove_library_entry(id: u64) -> Result<(), AppError> {
    let store = CatalogStore::for_current_user().map_err(|error| error.to_app_error())?;
    tauri::async_runtime::spawn_blocking(move || {
        let _catalog_lock = lock_catalog_mutation()?;
        let mut library = store.load().map_err(|error| error.to_app_error())?;
        if !library.remove(id) {
            return Err(invalid_entry_error());
        }
        store.save(&library).map_err(|error| error.to_app_error())
    })
    .await
    .map_err(|_| internal_error("The wallpaper entry could not be removed."))?
}

#[tauri::command]
pub async fn rename_library_entry(id: u64, display_name: String) -> Result<(), AppError> {
    let store = CatalogStore::for_current_user().map_err(|error| error.to_app_error())?;
    tauri::async_runtime::spawn_blocking(move || {
        let _catalog_lock = lock_catalog_mutation()?;
        let mut library = store.load().map_err(|error| error.to_app_error())?;
        library
            .rename(id, display_name)
            .map_err(|error| error.to_app_error())?;
        store.save(&library).map_err(|error| error.to_app_error())
    })
    .await
    .map_err(|_| internal_error("The wallpaper entry could not be renamed."))?
}

#[tauri::command]
pub async fn set_library_favorite(id: u64, favorite: bool) -> Result<(), AppError> {
    let store = CatalogStore::for_current_user().map_err(|error| error.to_app_error())?;
    tauri::async_runtime::spawn_blocking(move || {
        let _catalog_lock = lock_catalog_mutation()?;
        let mut library = store.load().map_err(|error| error.to_app_error())?;
        library
            .set_favorite(id, favorite)
            .map_err(|error| error.to_app_error())?;
        store.save(&library).map_err(|error| error.to_app_error())
    })
    .await
    .map_err(|_| internal_error("The wallpaper favorite could not be updated."))?
}

#[tauri::command]
pub async fn relocate_library_entry(id: u64, path: String) -> Result<(), AppError> {
    let store = CatalogStore::for_current_user().map_err(|error| error.to_app_error())?;
    tauri::async_runtime::spawn_blocking(move || {
        let _catalog_lock = lock_catalog_mutation()?;
        let mut library = store.load().map_err(|error| error.to_app_error())?;
        let Some(entry) = library.get_by_raw_id(id) else {
            return Err(internal_error("The wallpaper entry no longer exists."));
        };
        let path = PathBuf::from(path);
        validate_media_kind(entry.media_kind(), &path)?;
        library
            .relocate(id, path)
            .map_err(|error| error.to_app_error())?;
        store.save(&library).map_err(|error| error.to_app_error())
    })
    .await
    .map_err(|_| internal_error("The wallpaper file could not be located."))?
}

fn build_preview(store: &CatalogStore, id: u64) -> Result<LibraryPreview, AppError> {
    let library = store.load().map_err(|error| error.to_app_error())?;
    let entry = library
        .get_by_raw_id(id)
        .ok_or_else(|| internal_error("The wallpaper entry no longer exists."))?;
    if entry.availability() != Availability::Available {
        return Err(internal_error("The wallpaper source file is unavailable."));
    }
    let source_path = entry.source_path().to_owned();
    let metadata = fs::metadata(&source_path)
        .map_err(|_| internal_error("The wallpaper source file is unavailable."))?;
    if !metadata.is_file() {
        return Err(internal_error("The wallpaper source file is unavailable."));
    }
    let size_bytes = metadata.len();
    let cache_directory = thumbnail_cache_directory()?;
    let cache_path = thumbnail_cache_path(&cache_directory, &source_path, size_bytes, &metadata);
    let metadata_path = cache_path.with_extension("json");
    let _guard = THUMBNAIL_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| internal_error("The thumbnail cache is unavailable."))?;
    let cached_metadata = if cache_path.is_file() && metadata_path.is_file() {
        let valid_bitmap = fs::metadata(&cache_path)
            .map(|metadata| {
                (BITMAP_HEADER_BYTES as u64..=max_thumbnail_bytes()).contains(&metadata.len())
            })
            .unwrap_or(false)
            && fs::read(&cache_path)
                .map(|bytes| bytes.starts_with(b"BM"))
                .unwrap_or(false);
        valid_bitmap
            .then(|| fs::read(&metadata_path).ok())
            .flatten()
            .and_then(|bytes| serde_json::from_slice::<CachedPreviewMetadata>(&bytes).ok())
            .filter(|cached| {
                cached.width > 0
                    && cached.height > 0
                    && cached.width <= 16_384
                    && cached.height <= 16_384
            })
    } else {
        None
    };
    let preview_metadata = if let Some(cached) = cached_metadata {
        cached
    } else {
        let (width, height, duration_100ns, pixels) =
            decode_entry_preview(entry.media_kind(), &source_path)?;
        let thumbnail = encode_thumbnail_bmp(width, height, &pixels)
            .ok_or_else(|| internal_error("The wallpaper thumbnail could not be created."))?;
        prune_thumbnail_cache(&cache_directory, thumbnail.len() as u64, &cache_path);
        if cache_path.exists() {
            let _ = fs::remove_file(&cache_path);
        }
        write_cache_file(&cache_path, &thumbnail)?;
        let cached = CachedPreviewMetadata {
            width,
            height,
            duration_100ns,
        };
        let bytes = serde_json::to_vec(&cached)
            .map_err(|_| internal_error("The wallpaper metadata could not be saved."))?;
        if metadata_path.exists() {
            let _ = fs::remove_file(&metadata_path);
        }
        write_cache_file(&metadata_path, &bytes)?;
        cached
    };
    let thumbnail = fs::read(&cache_path)
        .map_err(|_| internal_error("The wallpaper thumbnail could not be read."))?;

    Ok(LibraryPreview {
        width: preview_metadata.width,
        height: preview_metadata.height,
        duration_100ns: preview_metadata.duration_100ns,
        size_bytes,
        thumbnail_data_url: format!("data:image/bmp;base64,{}", BASE64.encode(thumbnail)),
    })
}

fn decode_entry_preview(
    kind: MediaKind,
    source_path: &Path,
) -> Result<(u32, u32, Option<i64>, Vec<u8>), AppError> {
    match kind {
        MediaKind::Image => {
            let image = decode_image(source_path)
                .map_err(|_| internal_error("The wallpaper image could not be decoded."))?;
            Ok((image.width(), image.height(), None, image.rgba8().to_vec()))
        }
        MediaKind::Video => {
            let mut source = VideoSource::open(source_path)
                .map_err(|_| internal_error("The wallpaper video could not be decoded."))?;
            let duration = source.metadata().duration_100ns();
            let mut frame = None;
            for _ in 0..64 {
                match source
                    .read_next()
                    .map_err(|_| internal_error("The wallpaper video preview could not be read."))?
                {
                    VideoReadEvent::Frame(frame_value) => {
                        frame = Some(frame_value);
                        break;
                    }
                    VideoReadEvent::StreamTick { .. } => {}
                    VideoReadEvent::EndOfStream => break,
                }
            }
            let frame =
                frame.ok_or_else(|| internal_error("The wallpaper video has no preview frame."))?;
            Ok((
                frame.width(),
                frame.height(),
                Some(duration),
                frame.rgba8().to_vec(),
            ))
        }
    }
}

fn validate_media_kind(kind: MediaKind, path: &Path) -> Result<(), AppError> {
    match kind {
        MediaKind::Image => decode_image(path)
            .map(|_| ())
            .map_err(|_| internal_error("Choose a supported image file for this entry.")),
        MediaKind::Video => VideoSource::open(path)
            .map(|_| ())
            .map_err(|_| internal_error("Choose a supported H.264 MP4 file for this entry.")),
    }
}

fn thumbnail_cache_directory() -> Result<PathBuf, AppError> {
    let local_app_data = env::var_os("LOCALAPPDATA")
        .ok_or_else(|| internal_error("The local thumbnail cache directory is unavailable."))?;
    Ok(PathBuf::from(local_app_data)
        .join("WallpaperLite")
        .join("thumbnails")
        .join("v1"))
}

fn thumbnail_cache_path(
    directory: &Path,
    source_path: &Path,
    size_bytes: u64,
    metadata: &fs::Metadata,
) -> PathBuf {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .unwrap_or_default();
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in source_path
        .to_string_lossy()
        .bytes()
        .chain(size_bytes.to_le_bytes())
        .chain(modified.as_secs().to_le_bytes())
        .chain(modified.subsec_nanos().to_le_bytes())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    directory.join(format!("{hash:016x}.bmp"))
}

fn prune_thumbnail_cache(directory: &Path, new_size: u64, target: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut files = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if path == target || path.extension().is_none_or(|extension| extension != "bmp") {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            metadata.is_file().then(|| {
                (
                    path,
                    metadata.len(),
                    metadata.modified().unwrap_or(UNIX_EPOCH),
                )
            })
        })
        .collect::<Vec<_>>();
    let mut total = files.iter().map(|(_, size, _)| *size).sum::<u64>();
    if total.saturating_add(new_size) <= THUMBNAIL_CACHE_LIMIT_BYTES {
        return;
    }
    files.sort_by_key(|(_, _, modified)| *modified);
    for (path, size, _) in files {
        let metadata_path = path.with_extension("json");
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(metadata_path);
        total = total.saturating_sub(size);
        if total.saturating_add(new_size) <= THUMBNAIL_CACHE_LIMIT_BYTES {
            break;
        }
    }
}

const fn max_thumbnail_bytes() -> u64 {
    (BITMAP_HEADER_BYTES + (THUMBNAIL_EDGE as usize * THUMBNAIL_EDGE as usize * 3)) as u64
}

fn write_cache_file(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let parent = path
        .parent()
        .ok_or_else(|| internal_error("The thumbnail cache path is invalid."))?;
    fs::create_dir_all(parent)
        .map_err(|_| internal_error("The thumbnail cache could not be created."))?;
    let nonce = THUMBNAIL_NONCE.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(".thumbnail-{}-{nonce}.tmp", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| internal_error("The thumbnail cache could not be written."))?;
    let result = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if result.is_err() {
        let _ = fs::remove_file(temporary);
        return Err(internal_error("The thumbnail cache could not be saved."));
    }
    if fs::rename(&temporary, path).is_err() && !path.is_file() {
        let _ = fs::remove_file(temporary);
        return Err(internal_error("The thumbnail cache could not be saved."));
    }
    let _ = fs::remove_file(temporary);
    Ok(())
}

fn encode_thumbnail_bmp(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    if width == 0
        || height == 0
        || rgba.len()
            != (width as usize)
                .checked_mul(height as usize)?
                .checked_mul(4)?
    {
        return None;
    }
    let scale = (THUMBNAIL_EDGE as f64 / f64::from(width.max(height))).min(1.0);
    let out_width = (f64::from(width) * scale).round().max(1.0) as u32;
    let out_height = (f64::from(height) * scale).round().max(1.0) as u32;
    let row_bytes = (out_width as usize).checked_mul(3)?;
    let row_stride = row_bytes.checked_add(3)? & !3;
    let pixel_bytes = row_stride.checked_mul(out_height as usize)?;
    let file_size = BITMAP_HEADER_BYTES.checked_add(pixel_bytes)?;
    let mut output = Vec::with_capacity(file_size);
    output.extend_from_slice(b"BM");
    output.extend_from_slice(&u32::try_from(file_size).ok()?.to_le_bytes());
    output.extend_from_slice(&[0; 4]);
    output.extend_from_slice(&(BITMAP_HEADER_BYTES as u32).to_le_bytes());
    output.extend_from_slice(&40u32.to_le_bytes());
    output.extend_from_slice(&i32::try_from(out_width).ok()?.to_le_bytes());
    output.extend_from_slice(&i32::try_from(out_height).ok()?.to_le_bytes());
    output.extend_from_slice(&1u16.to_le_bytes());
    output.extend_from_slice(&24u16.to_le_bytes());
    output.extend_from_slice(&0u32.to_le_bytes());
    output.extend_from_slice(&u32::try_from(pixel_bytes).ok()?.to_le_bytes());
    output.extend_from_slice(&[0; 16]);

    for output_y in (0..out_height).rev() {
        let source_y = ((u64::from(output_y) * u64::from(height)) / u64::from(out_height)) as u32;
        let row_start = output.len();
        for output_x in 0..out_width {
            let source_x = ((u64::from(output_x) * u64::from(width)) / u64::from(out_width)) as u32;
            let source_offset = ((source_y as usize * width as usize) + source_x as usize) * 4;
            let alpha = u32::from(rgba[source_offset + 3]);
            for channel in [2, 1, 0] {
                let value = u32::from(rgba[source_offset + channel]);
                output.push(((value * alpha + 255 * (255 - alpha)) / 255) as u8);
            }
        }
        output.resize(row_start + row_stride, 0);
    }
    Some(output)
}

fn internal_error(message: &'static str) -> AppError {
    AppError::new(ErrorCode::Internal, message)
}

fn invalid_entry_error() -> AppError {
    AppError::new(
        ErrorCode::InvalidLibraryEntry,
        "The wallpaper library entry no longer exists.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_is_bounded_top_quality_bmp_with_expected_dimensions() {
        let pixels = [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ];
        let bitmap = encode_thumbnail_bmp(2, 2, &pixels).expect("thumbnail should encode");
        assert_eq!(&bitmap[..2], b"BM");
        assert_eq!(u32::from_le_bytes(bitmap[18..22].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(bitmap[22..26].try_into().unwrap()), 2);
        assert_eq!(
            u32::from_le_bytes(bitmap[2..6].try_into().unwrap()) as usize,
            bitmap.len()
        );
    }

    #[test]
    fn thumbnail_downscales_large_sources_to_the_fixed_edge_limit() {
        let pixels = vec![255; 640 * 320 * 4];
        let bitmap = encode_thumbnail_bmp(640, 320, &pixels).expect("thumbnail should encode");
        assert_eq!(
            u32::from_le_bytes(bitmap[18..22].try_into().unwrap()),
            THUMBNAIL_EDGE
        );
        assert_eq!(u32::from_le_bytes(bitmap[22..26].try_into().unwrap()), 160);
        assert!(
            bitmap.len()
                <= BITMAP_HEADER_BYTES + (THUMBNAIL_EDGE as usize * THUMBNAIL_EDGE as usize * 3)
        );
    }

    #[test]
    fn thumbnail_rejects_inconsistent_source_buffer() {
        assert!(encode_thumbnail_bmp(2, 2, &[0; 15]).is_none());
    }
}
