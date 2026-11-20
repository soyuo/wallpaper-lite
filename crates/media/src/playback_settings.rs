//! Versioned per-wallpaper playback settings storage.

use crate::library::{LibraryError, atomic_write_bytes};
use std::{env, error::Error, fmt, fs, io::Read, path::PathBuf};
use wallpaper_core::{
    error::{AppError, ErrorCode},
    playback_settings::{PlaybackSettings, PlaybackSettingsValidationError},
};

const MAX_PLAYBACK_SETTINGS_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackSettingsStore {
    directory: PathBuf,
}

impl PlaybackSettingsStore {
    /// Resolve `%APPDATA%\\WallpaperLite\\playback-settings` for the current user.
    pub fn for_current_user() -> Result<Self, PlaybackSettingsError> {
        let app_data =
            env::var_os("APPDATA").ok_or(PlaybackSettingsError::DataDirectoryUnavailable)?;
        let app_data = PathBuf::from(app_data);
        if !app_data.is_absolute() {
            return Err(PlaybackSettingsError::DataDirectoryUnavailable);
        }
        Ok(Self::at_directory(
            app_data.join("WallpaperLite").join("playback-settings"),
        ))
    }

    pub fn at_directory(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    pub fn load(&self, wallpaper_id: u64) -> Result<PlaybackSettings, PlaybackSettingsError> {
        let path = self.settings_path(wallpaper_id)?;
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return PlaybackSettings::default_for(wallpaper_id)
                    .map_err(PlaybackSettingsError::from_validation);
            }
            Err(_) => return Err(PlaybackSettingsError::ReadFailed),
        };
        if !metadata.is_file() || metadata.len() > MAX_PLAYBACK_SETTINGS_BYTES {
            return Err(PlaybackSettingsError::InvalidSettings);
        }
        let mut file = fs::File::open(path).map_err(|_| PlaybackSettingsError::ReadFailed)?;
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.by_ref()
            .take(MAX_PLAYBACK_SETTINGS_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| PlaybackSettingsError::ReadFailed)?;
        if bytes.len() as u64 > MAX_PLAYBACK_SETTINGS_BYTES {
            return Err(PlaybackSettingsError::InvalidSettings);
        }
        let settings = parse_settings(&bytes)?;
        if settings.wallpaper_id() != wallpaper_id {
            return Err(PlaybackSettingsError::InvalidSettings);
        }
        Ok(settings)
    }

    pub fn save(&self, settings: &PlaybackSettings) -> Result<(), PlaybackSettingsError> {
        settings
            .validate()
            .map_err(PlaybackSettingsError::from_validation)?;
        let contents = serde_json::to_vec_pretty(settings)
            .map_err(|_| PlaybackSettingsError::InvalidSettings)?;
        self.save_candidate(settings.wallpaper_id(), &contents)
    }

    fn save_candidate(
        &self,
        wallpaper_id: u64,
        candidate: &[u8],
    ) -> Result<(), PlaybackSettingsError> {
        let path = self.settings_path(wallpaper_id)?;
        if candidate.len() as u64 > MAX_PLAYBACK_SETTINGS_BYTES {
            return Err(PlaybackSettingsError::InvalidSettings);
        }
        let settings = parse_settings(candidate)?;
        if settings.wallpaper_id() != wallpaper_id {
            return Err(PlaybackSettingsError::InvalidSettings);
        }
        let contents = serde_json::to_vec_pretty(&settings)
            .map_err(|_| PlaybackSettingsError::InvalidSettings)?;
        atomic_write_bytes(&path, &contents).map_err(PlaybackSettingsError::from_library)
    }

    fn settings_path(&self, wallpaper_id: u64) -> Result<PathBuf, PlaybackSettingsError> {
        if wallpaper_id == 0 || !self.directory.is_absolute() {
            return Err(PlaybackSettingsError::InvalidSettings);
        }
        Ok(self.directory.join(format!("{wallpaper_id}.json")))
    }
}

fn parse_settings(bytes: &[u8]) -> Result<PlaybackSettings, PlaybackSettingsError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| PlaybackSettingsError::InvalidSettings)?;
    match value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
    {
        Some(version)
            if version
                != u64::from(
                    wallpaper_core::playback_settings::PLAYBACK_SETTINGS_SCHEMA_VERSION,
                ) =>
        {
            return Err(PlaybackSettingsError::UnsupportedSchema);
        }
        Some(_) => {}
        None => return Err(PlaybackSettingsError::InvalidSettings),
    }
    let settings: PlaybackSettings =
        serde_json::from_value(value).map_err(|_| PlaybackSettingsError::InvalidSettings)?;
    settings
        .validate()
        .map_err(PlaybackSettingsError::from_validation)?;
    Ok(settings)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackSettingsError {
    DataDirectoryUnavailable,
    ReadFailed,
    InvalidSettings,
    UnsupportedSchema,
    WriteFailed,
}

impl PlaybackSettingsError {
    pub const fn to_app_error(self) -> AppError {
        match self {
            Self::InvalidSettings | Self::UnsupportedSchema => AppError::new(
                ErrorCode::InvalidPlaybackSettings,
                "The wallpaper playback settings are invalid.",
            ),
            Self::DataDirectoryUnavailable | Self::ReadFailed | Self::WriteFailed => AppError::new(
                ErrorCode::PlaybackSettingsStorageFailed,
                "Wallpaper playback settings could not be loaded or saved.",
            ),
        }
    }

    pub const fn diagnostic_summary(self) -> &'static str {
        match self {
            Self::DataDirectoryUnavailable => "playback_settings_stage=resolve_user_data_directory",
            Self::ReadFailed => "playback_settings_stage=read",
            Self::InvalidSettings => "playback_settings_stage=validate",
            Self::UnsupportedSchema => "playback_settings_stage=validate_schema",
            Self::WriteFailed => "playback_settings_stage=write",
        }
    }

    fn from_validation(error: PlaybackSettingsValidationError) -> Self {
        match error {
            PlaybackSettingsValidationError::UnsupportedSchema => Self::UnsupportedSchema,
            _ => Self::InvalidSettings,
        }
    }

    fn from_library(_error: LibraryError) -> Self {
        Self::WriteFailed
    }
}

impl fmt::Display for PlaybackSettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.to_app_error().message)
    }
}

impl Error for PlaybackSettingsError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };
    use wallpaper_core::playback_settings::{LoopMode, SegmentRange};

    static TEST_DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let counter = TEST_DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "wallpaper-lite-playback-settings-test-{}-{counter}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn missing_file_returns_defaults_and_settings_round_trip() {
        let directory = TestDirectory::new();
        let store = PlaybackSettingsStore::at_directory(directory.0.join("settings"));
        assert_eq!(
            store.load(9).unwrap(),
            PlaybackSettings::default_for(9).unwrap()
        );

        let settings = PlaybackSettings::default_for(9)
            .unwrap()
            .with_loop(
                LoopMode::Segment,
                Some(SegmentRange::new(1_500, 12_000).unwrap()),
            )
            .unwrap()
            .with_speed_percent(125)
            .unwrap()
            .with_fps_limit(30)
            .unwrap()
            .with_audio(65, true)
            .unwrap();
        store.save(&settings).unwrap();
        assert_eq!(store.load(9).unwrap(), settings);
    }

    #[test]
    fn invalid_update_preserves_last_valid_file_and_source_media() {
        let directory = TestDirectory::new();
        let source = directory.0.join("source.mp4");
        fs::write(&source, b"immutable video bytes").unwrap();
        let store = PlaybackSettingsStore::at_directory(directory.0.join("settings"));
        let valid = PlaybackSettings::default_for(5).unwrap();
        store.save(&valid).unwrap();
        let settings_path = directory.0.join("settings/5.json");
        let previous_settings = fs::read(&settings_path).unwrap();
        let previous_source = fs::read(&source).unwrap();
        let mut candidate: serde_json::Value = serde_json::from_slice(&previous_settings).unwrap();
        candidate["speedPercent"] = serde_json::json!(110);
        assert_eq!(
            store.save_candidate(5, &serde_json::to_vec(&candidate).unwrap()),
            Err(PlaybackSettingsError::InvalidSettings)
        );
        assert_eq!(fs::read(&settings_path).unwrap(), previous_settings);
        assert_eq!(fs::read(&source).unwrap(), previous_source);
        assert_eq!(store.load(5).unwrap(), valid);
    }

    #[test]
    fn rejects_corrupt_mismatched_oversized_and_unsupported_files() {
        let directory = TestDirectory::new();
        let store = PlaybackSettingsStore::at_directory(directory.0.join("settings"));
        let path = directory.0.join("settings/8.json");
        store
            .save(&PlaybackSettings::default_for(8).unwrap())
            .unwrap();
        let valid_bytes = fs::read(&path).unwrap();

        fs::write(&path, b"not-json").unwrap();
        assert_eq!(store.load(8), Err(PlaybackSettingsError::InvalidSettings));
        fs::write(&path, &valid_bytes).unwrap();

        let mut value: serde_json::Value = serde_json::from_slice(&valid_bytes).unwrap();
        value["wallpaperId"] = serde_json::json!(9);
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(store.load(8), Err(PlaybackSettingsError::InvalidSettings));

        value["wallpaperId"] = serde_json::json!(8);
        value["schemaVersion"] = serde_json::json!(99);
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(store.load(8), Err(PlaybackSettingsError::UnsupportedSchema));

        value.as_object_mut().unwrap().remove("schemaVersion");
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(store.load(8), Err(PlaybackSettingsError::InvalidSettings));

        fs::write(&path, vec![b'x'; 65 * 1024]).unwrap();
        assert_eq!(store.load(8), Err(PlaybackSettingsError::InvalidSettings));
    }

    #[test]
    fn storage_errors_are_safe_and_do_not_include_paths() {
        let error = PlaybackSettingsError::WriteFailed;
        assert_eq!(
            error.to_app_error().code,
            ErrorCode::PlaybackSettingsStorageFailed
        );
        assert!(!error.to_app_error().message.contains("C:\\Users"));
        assert_eq!(error.diagnostic_summary(), "playback_settings_stage=write");
    }
}
