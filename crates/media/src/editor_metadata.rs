//! Local, versioned editor presets and recently applied wallpaper IDs.

use crate::library::atomic_write_bytes;
use serde::{Deserialize, Serialize};
use std::{env, fs, io::Read, path::PathBuf};
use wallpaper_core::{
    editor_settings::WallpaperEditorSettings,
    error::{AppError, ErrorCode},
};

const PRESET_SCHEMA_VERSION: u32 = 1;
const RECENT_SCHEMA_VERSION: u32 = 1;
const MAX_PRESETS: usize = 32;
const MAX_RECENT_ITEMS: usize = 100;
const MAX_NAME_CHARS: usize = 64;
const MAX_METADATA_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WallpaperEditorPreset {
    name: String,
    settings: WallpaperEditorSettings,
}

impl WallpaperEditorPreset {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn settings(&self) -> &WallpaperEditorSettings {
        &self.settings
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PresetDocument {
    schema_version: u32,
    wallpaper_id: u64,
    presets: Vec<WallpaperEditorPreset>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecentDocument {
    schema_version: u32,
    wallpaper_ids: Vec<u64>,
}

#[derive(Clone, Debug)]
pub struct EditorPresetStore {
    directory: PathBuf,
}

impl EditorPresetStore {
    pub fn for_current_user() -> Result<Self, EditorMetadataError> {
        let app_data =
            env::var_os("APPDATA").ok_or(EditorMetadataError::DataDirectoryUnavailable)?;
        let app_data = PathBuf::from(app_data);
        if !app_data.is_absolute() {
            return Err(EditorMetadataError::DataDirectoryUnavailable);
        }
        Ok(Self::at_directory(
            app_data.join("WallpaperLite").join("editor-presets"),
        ))
    }

    pub fn at_directory(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    pub fn list(
        &self,
        wallpaper_id: u64,
    ) -> Result<Vec<WallpaperEditorPreset>, EditorMetadataError> {
        Ok(self.load_document(wallpaper_id)?.presets)
    }

    pub fn save(
        &self,
        wallpaper_id: u64,
        name: &str,
        settings: &WallpaperEditorSettings,
    ) -> Result<(), EditorMetadataError> {
        let name = validate_name(name)?;
        validate_settings(wallpaper_id, settings)?;
        let mut document = self.load_document(wallpaper_id)?;
        if document
            .presets
            .iter()
            .any(|preset| preset.name.eq_ignore_ascii_case(&name))
        {
            return Err(EditorMetadataError::DuplicatePreset);
        }
        if document.presets.len() >= MAX_PRESETS {
            return Err(EditorMetadataError::LimitExceeded);
        }
        document.presets.push(WallpaperEditorPreset {
            name,
            settings: settings.clone(),
        });
        self.write_document(wallpaper_id, &document)
    }

    pub fn rename(
        &self,
        wallpaper_id: u64,
        current_name: &str,
        new_name: &str,
    ) -> Result<(), EditorMetadataError> {
        let current_name = validate_name(current_name)?;
        let new_name = validate_name(new_name)?;
        let mut document = self.load_document(wallpaper_id)?;
        let Some(index) = document
            .presets
            .iter()
            .position(|preset| preset.name.eq_ignore_ascii_case(&current_name))
        else {
            return Err(EditorMetadataError::PresetNotFound);
        };
        if document
            .presets
            .iter()
            .enumerate()
            .any(|(candidate, preset)| {
                candidate != index && preset.name.eq_ignore_ascii_case(&new_name)
            })
        {
            return Err(EditorMetadataError::DuplicatePreset);
        }
        document.presets[index].name = new_name;
        self.write_document(wallpaper_id, &document)
    }

    pub fn remove(&self, wallpaper_id: u64, name: &str) -> Result<(), EditorMetadataError> {
        let name = validate_name(name)?;
        let mut document = self.load_document(wallpaper_id)?;
        let previous_len = document.presets.len();
        document
            .presets
            .retain(|preset| !preset.name.eq_ignore_ascii_case(&name));
        if document.presets.len() == previous_len {
            return Err(EditorMetadataError::PresetNotFound);
        }
        self.write_document(wallpaper_id, &document)
    }

    fn load_document(&self, wallpaper_id: u64) -> Result<PresetDocument, EditorMetadataError> {
        validate_wallpaper_id(wallpaper_id)?;
        let path = self.preset_path(wallpaper_id)?;
        let bytes = match read_bounded(&path)? {
            Some(bytes) => bytes,
            None => {
                return Ok(PresetDocument {
                    schema_version: PRESET_SCHEMA_VERSION,
                    wallpaper_id,
                    presets: Vec::new(),
                });
            }
        };
        let document: PresetDocument =
            serde_json::from_slice(&bytes).map_err(|_| EditorMetadataError::InvalidMetadata)?;
        if document.schema_version != PRESET_SCHEMA_VERSION
            || document.wallpaper_id != wallpaper_id
            || document.presets.len() > MAX_PRESETS
        {
            return Err(EditorMetadataError::InvalidMetadata);
        }
        let mut seen = Vec::new();
        for preset in &document.presets {
            let name = validate_name(&preset.name)?;
            validate_settings(wallpaper_id, &preset.settings)?;
            if name != preset.name
                || seen
                    .iter()
                    .any(|existing: &String| existing.eq_ignore_ascii_case(&name))
            {
                return Err(EditorMetadataError::InvalidMetadata);
            }
            seen.push(name);
        }
        Ok(document)
    }

    fn write_document(
        &self,
        wallpaper_id: u64,
        document: &PresetDocument,
    ) -> Result<(), EditorMetadataError> {
        let bytes = serde_json::to_vec_pretty(document)
            .map_err(|_| EditorMetadataError::InvalidMetadata)?;
        atomic_write_bytes(&self.preset_path(wallpaper_id)?, &bytes)
            .map_err(|_| EditorMetadataError::WriteFailed)
    }

    fn preset_path(&self, wallpaper_id: u64) -> Result<PathBuf, EditorMetadataError> {
        validate_wallpaper_id(wallpaper_id)?;
        Ok(self.directory.join(format!("{wallpaper_id}.json")))
    }
}

#[derive(Clone, Debug)]
pub struct RecentWallpaperStore {
    path: PathBuf,
}

impl RecentWallpaperStore {
    pub fn for_current_user() -> Result<Self, EditorMetadataError> {
        let app_data =
            env::var_os("APPDATA").ok_or(EditorMetadataError::DataDirectoryUnavailable)?;
        let app_data = PathBuf::from(app_data);
        if !app_data.is_absolute() {
            return Err(EditorMetadataError::DataDirectoryUnavailable);
        }
        Ok(Self::at_path(
            app_data
                .join("WallpaperLite")
                .join("recent-wallpapers.json"),
        ))
    }

    pub fn at_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn list(&self) -> Result<Vec<u64>, EditorMetadataError> {
        let Some(bytes) = read_bounded(&self.path)? else {
            return Ok(Vec::new());
        };
        let document: RecentDocument =
            serde_json::from_slice(&bytes).map_err(|_| EditorMetadataError::InvalidMetadata)?;
        if document.schema_version != RECENT_SCHEMA_VERSION
            || document.wallpaper_ids.len() > MAX_RECENT_ITEMS
            || document.wallpaper_ids.contains(&0)
            || has_duplicates(&document.wallpaper_ids)
        {
            return Err(EditorMetadataError::InvalidMetadata);
        }
        Ok(document.wallpaper_ids)
    }

    /// Move an ID to the front. Call only for an Apply transaction; file order is the persistent recency order.
    pub fn record_applied(&self, wallpaper_id: u64) -> Result<(), EditorMetadataError> {
        validate_wallpaper_id(wallpaper_id)?;
        let mut wallpaper_ids = self.list()?;
        wallpaper_ids.retain(|id| *id != wallpaper_id);
        wallpaper_ids.insert(0, wallpaper_id);
        wallpaper_ids.truncate(MAX_RECENT_ITEMS);
        let document = RecentDocument {
            schema_version: RECENT_SCHEMA_VERSION,
            wallpaper_ids,
        };
        let bytes = serde_json::to_vec_pretty(&document)
            .map_err(|_| EditorMetadataError::InvalidMetadata)?;
        atomic_write_bytes(&self.path, &bytes).map_err(|_| EditorMetadataError::WriteFailed)
    }

    pub fn restore(&self, wallpaper_ids: &[u64]) -> Result<(), EditorMetadataError> {
        if wallpaper_ids.len() > MAX_RECENT_ITEMS
            || wallpaper_ids.contains(&0)
            || has_duplicates(wallpaper_ids)
        {
            return Err(EditorMetadataError::InvalidMetadata);
        }
        let document = RecentDocument {
            schema_version: RECENT_SCHEMA_VERSION,
            wallpaper_ids: wallpaper_ids.to_vec(),
        };
        let bytes = serde_json::to_vec_pretty(&document)
            .map_err(|_| EditorMetadataError::InvalidMetadata)?;
        atomic_write_bytes(&self.path, &bytes).map_err(|_| EditorMetadataError::WriteFailed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorMetadataError {
    DataDirectoryUnavailable,
    InvalidMetadata,
    InvalidName,
    DuplicatePreset,
    PresetNotFound,
    LimitExceeded,
    ReadFailed,
    WriteFailed,
}

impl EditorMetadataError {
    pub const fn to_app_error(self) -> AppError {
        let (code, message) = match self {
            Self::InvalidMetadata
            | Self::InvalidName
            | Self::DuplicatePreset
            | Self::PresetNotFound
            | Self::LimitExceeded => (
                ErrorCode::InvalidEditorSettings,
                "The wallpaper preset or recent-item data is invalid.",
            ),
            Self::DataDirectoryUnavailable | Self::ReadFailed | Self::WriteFailed => (
                ErrorCode::EditorSettingsStorageFailed,
                "Wallpaper presets or recent items could not be loaded or saved.",
            ),
        };
        AppError::new(code, message)
    }
}

fn validate_name(name: &str) -> Result<String, EditorMetadataError> {
    let name = name.trim();
    if name.is_empty()
        || name.chars().count() > MAX_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(EditorMetadataError::InvalidName);
    }
    Ok(name.to_owned())
}

fn validate_settings(
    wallpaper_id: u64,
    settings: &WallpaperEditorSettings,
) -> Result<(), EditorMetadataError> {
    if settings.wallpaper_id() != wallpaper_id || settings.validate().is_err() {
        return Err(EditorMetadataError::InvalidMetadata);
    }
    Ok(())
}

fn validate_wallpaper_id(wallpaper_id: u64) -> Result<(), EditorMetadataError> {
    if wallpaper_id == 0 {
        Err(EditorMetadataError::InvalidMetadata)
    } else {
        Ok(())
    }
}

fn read_bounded(path: &PathBuf) -> Result<Option<Vec<u8>>, EditorMetadataError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(EditorMetadataError::ReadFailed),
    };
    if !metadata.is_file() || metadata.len() > MAX_METADATA_BYTES {
        return Err(EditorMetadataError::InvalidMetadata);
    }
    let mut file = fs::File::open(path).map_err(|_| EditorMetadataError::ReadFailed)?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_METADATA_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| EditorMetadataError::ReadFailed)?;
    if bytes.len() as u64 > MAX_METADATA_BYTES {
        return Err(EditorMetadataError::InvalidMetadata);
    }
    Ok(Some(bytes))
}

fn has_duplicates(values: &[u64]) -> bool {
    let mut seen = Vec::with_capacity(values.len());
    values.iter().any(|value| {
        if seen.contains(value) {
            true
        } else {
            seen.push(*value);
            false
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn temp() -> PathBuf {
        let path = env::temp_dir().join(format!(
            "wallpaper-editor-metadata-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn presets_round_trip_rename_remove_and_are_bound_to_wallpaper() {
        let directory = temp();
        let store = EditorPresetStore::at_directory(&directory);
        let settings = WallpaperEditorSettings::default_for(1).unwrap();
        store.save(1, "  Soft  ", &settings).unwrap();
        assert_eq!(store.list(1).unwrap()[0].name(), "Soft");
        assert_eq!(
            store.save(1, "soft", &settings),
            Err(EditorMetadataError::DuplicatePreset)
        );
        assert_eq!(
            store.save(2, "Other", &settings),
            Err(EditorMetadataError::InvalidMetadata)
        );
        store.rename(1, "soft", "Warm").unwrap();
        assert_eq!(store.list(1).unwrap()[0].name(), "Warm");
        store.remove(1, "warm").unwrap();
        assert!(store.list(1).unwrap().is_empty());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn recents_persist_and_move_successfully_applied_items_to_front() {
        let directory = temp();
        let path = directory.join("recent.json");
        let store = RecentWallpaperStore::at_path(&path);
        store.record_applied(4).unwrap();
        store.record_applied(8).unwrap();
        store.record_applied(4).unwrap();
        assert_eq!(
            RecentWallpaperStore::at_path(path).list().unwrap(),
            vec![4, 8]
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn preset_and_recent_mutations_preserve_source_media() {
        let directory = temp();
        let source = directory.join("source.png");
        let source_bytes = b"source fixture bytes";
        fs::write(&source, source_bytes).unwrap();
        let metadata_directory = directory.join("metadata");
        let settings = WallpaperEditorSettings::default_for(1).unwrap();

        let presets = EditorPresetStore::at_directory(&metadata_directory);
        presets.save(1, "Soft", &settings).unwrap();
        presets.rename(1, "Soft", "Warm").unwrap();
        assert_eq!(presets.list(1).unwrap()[0].name(), "Warm");
        presets.remove(1, "Warm").unwrap();
        assert!(presets.list(1).unwrap().is_empty());

        let recents = RecentWallpaperStore::at_path(metadata_directory.join("recent.json"));
        recents.record_applied(1).unwrap();

        assert_eq!(fs::read(&source).unwrap(), source_bytes);
        let mut metadata_files = fs::read_dir(&metadata_directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        metadata_files.sort();
        assert_eq!(
            metadata_files,
            vec![
                std::ffi::OsString::from("1.json"),
                std::ffi::OsString::from("recent.json")
            ]
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn corrupt_preset_metadata_is_rejected_without_replacing_it() {
        let directory = temp();
        fs::write(directory.join("1.json"), b"not json").unwrap();
        let store = EditorPresetStore::at_directory(&directory);

        assert_eq!(store.list(1), Err(EditorMetadataError::InvalidMetadata));
        assert_eq!(fs::read(directory.join("1.json")).unwrap(), b"not json");
        let _ = fs::remove_dir_all(directory);
    }
}
