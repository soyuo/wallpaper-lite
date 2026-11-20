//! Content-validated import into the per-user wallpaper catalog.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use serde::Serialize;

use crate::{
    CatalogStore, LibraryError, MediaError, MediaKind, VideoSource, WallpaperId, WallpaperLibrary,
    decode_image,
};

const MAX_IMPORT_CANDIDATES: usize = 4096;
const MAX_DIRECTORY_DEPTH: usize = 32;
const MAX_RESULT_NAME_CHARS: usize = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportStatus {
    Imported,
    AlreadyPresent,
    Rejected,
    Unavailable,
    ScanFailed,
    LimitReached,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportFailure {
    UnsupportedFormat,
    InvalidMedia,
    CodecUnavailable,
    SourceUnavailable,
    DirectoryUnreadable,
    DirectoryDepthExceeded,
    CandidateLimitReached,
    CatalogWriteFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub file_name: String,
    pub status: ImportStatus,
    pub failure: Option<ImportFailure>,
    pub wallpaper_id: Option<WallpaperId>,
}

/// Import selected files and recursively scan selected folders.
///
/// Every source is opened read-only and validated by WIC or Media Foundation using its actual
/// content. Duplicate absolute paths resolve to the existing stable catalog ID. A catalog save
/// failure leaves the previous catalog intact and changes provisional success results to failures.
pub fn import_local_media(
    store: &CatalogStore,
    selections: &[PathBuf],
) -> Result<Vec<ImportResult>, LibraryError> {
    let mut library = store.load()?;
    let mut results = Vec::new();
    let mut candidates = Vec::new();
    for selection in selections {
        collect_selection(selection, &mut candidates, &mut results);
    }
    candidates.sort();

    let mut changed = false;
    let mut seen = HashMap::<PathBuf, WallpaperId>::new();
    for candidate in candidates {
        let file_name = safe_file_name(&candidate);
        match fs::metadata(&candidate) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => {
                results.push(result(
                    file_name,
                    ImportStatus::Rejected,
                    Some(ImportFailure::InvalidMedia),
                    None,
                ));
                continue;
            }
            Err(_) => {
                results.push(result(
                    file_name,
                    ImportStatus::Unavailable,
                    Some(ImportFailure::SourceUnavailable),
                    None,
                ));
                continue;
            }
        };
        let canonical = match fs::canonicalize(&candidate) {
            Ok(path) => path,
            Err(_) => {
                results.push(result(
                    file_name,
                    ImportStatus::Unavailable,
                    Some(ImportFailure::SourceUnavailable),
                    None,
                ));
                continue;
            }
        };
        if let Some(id) = seen
            .get(&canonical)
            .copied()
            .or_else(|| find_existing(&library, &canonical))
        {
            seen.insert(canonical, id);
            results.push(result(
                file_name,
                ImportStatus::AlreadyPresent,
                None,
                Some(id),
            ));
            continue;
        }

        let kind = match probe_media(&candidate) {
            Ok(kind) => kind,
            Err((status, failure)) => {
                results.push(result(file_name, status, Some(failure), None));
                continue;
            }
        };
        match library.add(canonical.clone(), file_name.clone(), kind) {
            Ok(id) => {
                changed = true;
                seen.insert(canonical, id);
                results.push(result(file_name, ImportStatus::Imported, None, Some(id)));
            }
            Err(_) => results.push(result(
                file_name,
                ImportStatus::Unavailable,
                Some(ImportFailure::SourceUnavailable),
                None,
            )),
        }
    }

    if changed && store.save(&library).is_err() {
        for item in &mut results {
            if item.status == ImportStatus::Imported {
                item.status = ImportStatus::Rejected;
                item.failure = Some(ImportFailure::CatalogWriteFailed);
                item.wallpaper_id = None;
            }
        }
    }
    Ok(results)
}

fn collect_selection(
    selection: &Path,
    candidates: &mut Vec<PathBuf>,
    results: &mut Vec<ImportResult>,
) {
    let selection_name = safe_file_name(selection);
    let metadata = match fs::metadata(selection) {
        Ok(metadata) => metadata,
        Err(_) => {
            results.push(result(
                selection_name,
                ImportStatus::Unavailable,
                Some(ImportFailure::SourceUnavailable),
                None,
            ));
            return;
        }
    };
    if metadata.is_file() {
        if candidates.len() >= MAX_IMPORT_CANDIDATES {
            results.push(result(
                selection_name,
                ImportStatus::LimitReached,
                Some(ImportFailure::CandidateLimitReached),
                None,
            ));
        } else {
            candidates.push(selection.to_path_buf());
        }
        return;
    }
    if !metadata.is_dir() {
        results.push(result(
            selection_name,
            ImportStatus::Rejected,
            Some(ImportFailure::InvalidMedia),
            None,
        ));
        return;
    }

    let candidates_before = candidates.len();
    let results_before = results.len();
    let mut directories = vec![(selection.to_path_buf(), 0usize)];
    let mut visited = HashSet::new();
    let mut limit_reported = false;
    while let Some((directory, depth)) = directories.pop() {
        if depth > MAX_DIRECTORY_DEPTH {
            results.push(result(
                safe_file_name(&directory),
                ImportStatus::LimitReached,
                Some(ImportFailure::DirectoryDepthExceeded),
                None,
            ));
            continue;
        }
        let key = fs::canonicalize(&directory).unwrap_or_else(|_| directory.clone());
        if !visited.insert(key) {
            continue;
        }
        let mut entries = match fs::read_dir(&directory) {
            Ok(entries) => entries
                .filter_map(|entry| match entry {
                    Ok(entry) => Some(entry),
                    Err(_) => {
                        results.push(result(
                            safe_file_name(&directory),
                            ImportStatus::ScanFailed,
                            Some(ImportFailure::DirectoryUnreadable),
                            None,
                        ));
                        None
                    }
                })
                .collect::<Vec<_>>(),
            Err(_) => {
                results.push(result(
                    safe_file_name(&directory),
                    ImportStatus::ScanFailed,
                    Some(ImportFailure::DirectoryUnreadable),
                    None,
                ));
                continue;
            }
        };
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if candidates.len() >= MAX_IMPORT_CANDIDATES {
                if !limit_reported {
                    results.push(result(
                        selection_name.clone(),
                        ImportStatus::LimitReached,
                        Some(ImportFailure::CandidateLimitReached),
                        None,
                    ));
                    limit_reported = true;
                }
                break;
            }
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                results.push(result(
                    safe_file_name(&path),
                    ImportStatus::Unavailable,
                    Some(ImportFailure::SourceUnavailable),
                    None,
                ));
                continue;
            };
            if file_type.is_dir() {
                directories.push((path, depth + 1));
            } else if file_type.is_file() {
                candidates.push(path);
            } else {
                results.push(result(
                    safe_file_name(&path),
                    ImportStatus::Rejected,
                    Some(ImportFailure::InvalidMedia),
                    None,
                ));
            }
        }
    }
    if candidates.len() == candidates_before && results.len() == results_before {
        results.push(result(
            selection_name,
            ImportStatus::Rejected,
            Some(ImportFailure::UnsupportedFormat),
            None,
        ));
    }
}

fn probe_media(path: &Path) -> Result<MediaKind, (ImportStatus, ImportFailure)> {
    if fs::File::open(path).is_err() {
        return Err((ImportStatus::Unavailable, ImportFailure::SourceUnavailable));
    }
    let image_error = match decode_image(path) {
        Ok(_) => return Ok(MediaKind::Image),
        Err(error) => error,
    };
    match VideoSource::open(path) {
        Ok(source) => {
            drop(source);
            Ok(MediaKind::Video)
        }
        Err(MediaError::UnsupportedVideoContainer) => {
            Err((ImportStatus::Rejected, ImportFailure::UnsupportedFormat))
        }
        Err(MediaError::UnsupportedVideoFormat) => {
            Err((ImportStatus::Rejected, ImportFailure::CodecUnavailable))
        }
        Err(MediaError::VideoSampleDecodeFailed { hresult }) if hresult as u32 == 0xC00D5212 => {
            Err((ImportStatus::Unavailable, ImportFailure::CodecUnavailable))
        }
        Err(
            MediaError::MediaFoundationStartupFailed { .. }
            | MediaError::VideoComInitializationFailed { .. }
            | MediaError::UnsupportedVideoPlatform,
        ) => Err((ImportStatus::Unavailable, ImportFailure::CodecUnavailable)),
        Err(_) if image_error == MediaError::UnsupportedImageFormat => {
            Err((ImportStatus::Rejected, ImportFailure::UnsupportedFormat))
        }
        Err(_) => Err((ImportStatus::Rejected, ImportFailure::InvalidMedia)),
    }
}

fn find_existing(library: &WallpaperLibrary, canonical_path: &Path) -> Option<WallpaperId> {
    library.entries().iter().find_map(|entry| {
        let existing = fs::canonicalize(entry.source_path())
            .unwrap_or_else(|_| entry.source_path().to_path_buf());
        paths_equal(&existing, canonical_path).then_some(entry.id())
    })
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn safe_file_name(path: &Path) -> String {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let safe = name
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_RESULT_NAME_CHARS)
        .collect::<String>();
    if safe.is_empty() {
        "Selected item".to_owned()
    } else {
        safe
    }
}

fn result(
    file_name: String,
    status: ImportStatus,
    failure: Option<ImportFailure>,
    wallpaper_id: Option<WallpaperId>,
) -> ImportResult {
    ImportResult {
        file_name,
        status,
        failure,
        wallpaper_id,
    }
}
