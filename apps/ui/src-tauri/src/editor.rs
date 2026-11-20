use std::path::PathBuf;

use base64::Engine;
use serde::Serialize;
use wallpaper_core::error::{AppError, ErrorCode};
use wallpaper_media::{
    Availability, CatalogStore, EditorSettingsStore, MediaKind, VideoReadEvent, VideoSource,
    decode_image,
};
use wallpaper_renderer::render_editor_preview;

use crate::library::lock_catalog_mutation;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorMonitor {
    id: String,
    label: String,
    width: u32,
    height: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorPreview {
    width: u32,
    height: u32,
    monitor_width: u32,
    monitor_height: u32,
    data_url: String,
}

#[tauri::command]
pub fn editor_monitors(window: tauri::WebviewWindow) -> Vec<EditorMonitor> {
    window
        .available_monitors()
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .map(|(index, monitor)| {
            let position = monitor.position();
            let size = monitor.size();
            let id = format!(
                "{}:{}:{}:{}",
                position.x, position.y, size.width, size.height
            );
            EditorMonitor {
                label: format!("Display {} — {} × {}", index + 1, size.width, size.height),
                id,
                width: size.width,
                height: size.height,
            }
        })
        .collect()
}

#[tauri::command]
pub async fn editor_preview(
    wallpaper_id: u64,
    monitor_width: u32,
    monitor_height: u32,
) -> Result<EditorPreview, AppError> {
    if !(1..=32_768).contains(&monitor_width) || !(1..=32_768).contains(&monitor_height) {
        return Err(AppError::new(
            ErrorCode::InvalidEditorSettings,
            "The selected display dimensions are invalid.",
        ));
    }
    tauri::async_runtime::spawn_blocking(move || {
        render_preview(wallpaper_id, monitor_width, monitor_height)
    })
    .await
    .map_err(|_| {
        AppError::new(
            ErrorCode::Internal,
            "The wallpaper preview could not be completed.",
        )
    })?
}

fn render_preview(
    wallpaper_id: u64,
    monitor_width: u32,
    monitor_height: u32,
) -> Result<EditorPreview, AppError> {
    let (source_path, media_kind) = {
        let _catalog_lock = lock_catalog_mutation()?;
        let catalog = CatalogStore::for_current_user().map_err(|error| error.to_app_error())?;
        let library = catalog.load().map_err(|error| error.to_app_error())?;
        let entry = library
            .get_by_raw_id(wallpaper_id)
            .ok_or_else(|| invalid_entry("The selected wallpaper is no longer in the library."))?;
        if entry.availability() != Availability::Available {
            return Err(invalid_entry(
                "The selected wallpaper source is unavailable.",
            ));
        }
        (PathBuf::from(entry.source_path()), entry.media_kind())
    };
    let settings = EditorSettingsStore::for_current_user()
        .map_err(|error| error.to_app_error())?
        .load(wallpaper_id)
        .map_err(|error| error.to_app_error())?;

    let (width, height, stride, pixels) = match media_kind {
        MediaKind::Image => {
            let image = decode_image(&source_path).map_err(|error| error.to_app_error())?;
            (
                image.width(),
                image.height(),
                image.stride(),
                image.rgba8().to_vec(),
            )
        }
        MediaKind::Video => {
            let mut source =
                VideoSource::open(&source_path).map_err(|error| error.to_app_error())?;
            let mut decoded = None;
            for _ in 0..64 {
                match source.read_next().map_err(|error| error.to_app_error())? {
                    VideoReadEvent::Frame(frame) => {
                        decoded = Some(frame);
                        break;
                    }
                    VideoReadEvent::StreamTick { .. } => continue,
                    VideoReadEvent::EndOfStream => break,
                }
            }
            let frame = decoded.ok_or_else(|| {
                AppError::new(
                    ErrorCode::VideoDecodeFailed,
                    "A video preview frame is not available.",
                )
            })?;
            (
                frame.width(),
                frame.height(),
                frame.stride(),
                frame.rgba8().to_vec(),
            )
        }
    };

    let rendered = render_editor_preview(
        width,
        height,
        stride,
        &pixels,
        monitor_width,
        monitor_height,
        &settings,
    )
    .map_err(|error| error.to_app_error())?;
    let bitmap = encode_preview_bmp(rendered.width(), rendered.height(), rendered.rgba8())
        .ok_or_else(|| AppError::new(ErrorCode::Internal, "The preview could not be encoded."))?;
    Ok(EditorPreview {
        width: rendered.width(),
        height: rendered.height(),
        monitor_width,
        monitor_height,
        data_url: format!(
            "data:image/bmp;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bitmap)
        ),
    })
}

fn encode_preview_bmp(width: u32, height: u32, rgba8: &[u8]) -> Option<Vec<u8>> {
    let row_bytes = (width as usize).checked_mul(3)?;
    let row_stride = row_bytes.checked_add(3)? & !3;
    let pixel_bytes = row_stride.checked_mul(height as usize)?;
    let file_bytes = 54usize.checked_add(pixel_bytes)?;
    let file_bytes_u32 = u32::try_from(file_bytes).ok()?;
    if width == 0 || height == 0 || rgba8.len() != width as usize * height as usize * 4 {
        return None;
    }
    let mut output = vec![0; file_bytes];
    output[0..2].copy_from_slice(b"BM");
    output[2..6].copy_from_slice(&file_bytes_u32.to_le_bytes());
    output[10..14].copy_from_slice(&54u32.to_le_bytes());
    output[14..18].copy_from_slice(&40u32.to_le_bytes());
    output[18..22].copy_from_slice(&i32::try_from(width).ok()?.to_le_bytes());
    output[22..26].copy_from_slice(&i32::try_from(height).ok()?.to_le_bytes());
    output[26..28].copy_from_slice(&1u16.to_le_bytes());
    output[28..30].copy_from_slice(&24u16.to_le_bytes());
    output[34..38].copy_from_slice(&u32::try_from(pixel_bytes).ok()?.to_le_bytes());
    for y in 0..height as usize {
        let source_y = height as usize - 1 - y;
        for x in 0..width as usize {
            let source = (source_y * width as usize + x) * 4;
            let target = 54 + y * row_stride + x * 3;
            output[target] = rgba8[source + 2];
            output[target + 1] = rgba8[source + 1];
            output[target + 2] = rgba8[source];
        }
    }
    Some(output)
}

fn invalid_entry(message: &'static str) -> AppError {
    AppError::new(ErrorCode::InvalidLibraryEntry, message)
}

#[cfg(test)]
mod tests {
    use super::encode_preview_bmp;

    #[test]
    fn preview_bmp_has_expected_header_and_bgr_pixels() {
        let bitmap = encode_preview_bmp(1, 1, &[10, 20, 30, 255]).unwrap();
        assert_eq!(&bitmap[..2], b"BM");
        assert_eq!(u32::from_le_bytes(bitmap[18..22].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(bitmap[22..26].try_into().unwrap()), 1);
        assert_eq!(&bitmap[54..57], &[30, 20, 10]);
    }
}
