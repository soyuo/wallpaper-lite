use crate::library::{LibraryError, atomic_write_bytes};
use std::{env, error::Error, fmt, fs, io::Read, path::PathBuf};
use wallpaper_core::{
    editor_settings::{EditorSettingsValidationError, WallpaperEditorSettings},
    error::{AppError, ErrorCode},
};

const MAX_EDITOR_SETTINGS_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorSettingsStore {
    directory: PathBuf,
}

impl EditorSettingsStore {
    /// Resolve `%APPDATA%\\WallpaperLite\\wallpaper-settings` for the current user.
    pub fn for_current_user() -> Result<Self, EditorSettingsError> {
        let app_data =
            env::var_os("APPDATA").ok_or(EditorSettingsError::DataDirectoryUnavailable)?;
        let app_data = PathBuf::from(app_data);
        if !app_data.is_absolute() {
            return Err(EditorSettingsError::DataDirectoryUnavailable);
        }
        Ok(Self::at_directory(
            app_data.join("WallpaperLite").join("wallpaper-settings"),
        ))
    }

    pub fn at_directory(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    pub fn load(&self, wallpaper_id: u64) -> Result<WallpaperEditorSettings, EditorSettingsError> {
        let path = self.settings_path(wallpaper_id)?;
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return WallpaperEditorSettings::default_for(wallpaper_id)
                    .map_err(EditorSettingsError::from_validation);
            }
            Err(_) => return Err(EditorSettingsError::ReadFailed),
        };
        if !metadata.is_file() || metadata.len() > MAX_EDITOR_SETTINGS_BYTES {
            return Err(EditorSettingsError::InvalidSettings);
        }
        let mut file = fs::File::open(path).map_err(|_| EditorSettingsError::ReadFailed)?;
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.by_ref()
            .take(MAX_EDITOR_SETTINGS_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| EditorSettingsError::ReadFailed)?;
        if bytes.len() as u64 > MAX_EDITOR_SETTINGS_BYTES {
            return Err(EditorSettingsError::InvalidSettings);
        }
        let settings: WallpaperEditorSettings =
            serde_json::from_slice(&bytes).map_err(|_| EditorSettingsError::InvalidSettings)?;
        settings
            .validate()
            .map_err(EditorSettingsError::from_validation)?;
        if settings.wallpaper_id() != wallpaper_id {
            return Err(EditorSettingsError::InvalidSettings);
        }
        Ok(settings)
    }

    pub fn save(&self, settings: &WallpaperEditorSettings) -> Result<(), EditorSettingsError> {
        settings
            .validate()
            .map_err(EditorSettingsError::from_validation)?;
        let path = self.settings_path(settings.wallpaper_id())?;
        let contents = serde_json::to_vec_pretty(settings)
            .map_err(|_| EditorSettingsError::InvalidSettings)?;
        atomic_write_bytes(&path, &contents).map_err(EditorSettingsError::from_library)
    }

    fn settings_path(&self, wallpaper_id: u64) -> Result<PathBuf, EditorSettingsError> {
        if wallpaper_id == 0 || !self.directory.is_absolute() {
            return Err(EditorSettingsError::InvalidSettings);
        }
        Ok(self.directory.join(format!("{wallpaper_id}.json")))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorSettingsError {
    DataDirectoryUnavailable,
    ReadFailed,
    InvalidSettings,
    UnsupportedSchema,
    WriteFailed,
}

impl EditorSettingsError {
    pub const fn to_app_error(self) -> AppError {
        match self {
            Self::InvalidSettings | Self::UnsupportedSchema => AppError::new(
                ErrorCode::InvalidEditorSettings,
                "The wallpaper editor settings are invalid.",
            ),
            Self::DataDirectoryUnavailable | Self::ReadFailed | Self::WriteFailed => AppError::new(
                ErrorCode::EditorSettingsStorageFailed,
                "Wallpaper editor settings could not be loaded or saved.",
            ),
        }
    }

    pub const fn diagnostic_summary(self) -> &'static str {
        match self {
            Self::DataDirectoryUnavailable => "settings_stage=resolve_user_data_directory",
            Self::ReadFailed => "settings_stage=read_wallpaper_settings",
            Self::InvalidSettings => "settings_stage=validate_wallpaper_settings",
            Self::UnsupportedSchema => "settings_stage=validate_wallpaper_settings_schema",
            Self::WriteFailed => "settings_stage=write_wallpaper_settings",
        }
    }

    fn from_validation(error: EditorSettingsValidationError) -> Self {
        match error {
            EditorSettingsValidationError::UnsupportedSchema => Self::UnsupportedSchema,
            _ => Self::InvalidSettings,
        }
    }

    fn from_library(_error: LibraryError) -> Self {
        Self::WriteFailed
    }
}

impl fmt::Display for EditorSettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.to_app_error().message)
    }
}

impl Error for EditorSettingsError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        EditorColorSettings, NormalizedCrop, NormalizedPoint, PhysicalPosition, PlacementMode,
        RenderSettings, Scale2d,
        library::{CatalogStore, MediaKind, WallpaperLibrary},
    };
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    static TEST_DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let counter = TEST_DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "wallpaper-lite-editor-settings-test-{}-{counter}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
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

    fn settings_for(id: u64) -> WallpaperEditorSettings {
        let render = RenderSettings::new(
            PlacementMode::Fit,
            PhysicalPosition::new(-120, 80).unwrap(),
            Some(crate::PhysicalSize::new(1920, 1080).unwrap()),
            Scale2d::new(1.25, 0.75).unwrap(),
            45.0,
            NormalizedPoint::new(0.25, 0.75).unwrap(),
            NormalizedCrop::new(0.1, 0.2, 0.7, 0.6).unwrap(),
        )
        .unwrap();
        WallpaperEditorSettings::new(
            id,
            render,
            EditorColorSettings::new(1.1, 0.9, 1.2, 1.0, 0.8, 0.2, 1.0, 0.95, 1.1).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn settings_round_trip_without_changing_catalog_or_source() {
        let directory = TestDirectory::new();
        let source = directory.path().join("original.png");
        fs::write(&source, b"original image bytes").unwrap();
        let catalog_store = CatalogStore::at_path(directory.path().join("catalog/library.json"));
        let mut catalog = WallpaperLibrary::default();
        let id = catalog.add(&source, "Original", MediaKind::Image).unwrap();
        catalog_store.save(&catalog).unwrap();
        let catalog_before = fs::read(catalog_store.path()).unwrap();
        let source_before = fs::read(&source).unwrap();

        let settings_store = EditorSettingsStore::at_directory(directory.path().join("settings"));
        let settings = settings_for(id.get());
        settings_store.save(&settings).unwrap();
        let loaded = settings_store.load(id.get()).unwrap();

        assert_eq!(loaded, settings);
        assert_eq!(loaded.wallpaper_id(), id.get());
        assert_eq!(fs::read(catalog_store.path()).unwrap(), catalog_before);
        assert_eq!(fs::read(&source).unwrap(), source_before);
    }

    #[test]
    fn missing_settings_return_valid_defaults_for_the_requested_id() {
        let directory = TestDirectory::new();
        let store = EditorSettingsStore::at_directory(directory.path().join("settings"));
        let loaded = store.load(42).unwrap();
        assert_eq!(loaded.wallpaper_id(), 42);
        assert_eq!(loaded, WallpaperEditorSettings::default_for(42).unwrap());
    }

    #[test]
    fn invalid_update_does_not_replace_last_valid_settings() {
        let directory = TestDirectory::new();
        let store = EditorSettingsStore::at_directory(directory.path().join("settings"));
        let valid = WallpaperEditorSettings::default_for(5).unwrap();
        store.save(&valid).unwrap();
        let path = directory.path().join("settings/5.json");
        let before = fs::read(&path).unwrap();

        let mut invalid_value = serde_json::to_value(&valid).unwrap();
        invalid_value["render"]["crop"]["x"] = serde_json::json!(0.8);
        invalid_value["render"]["crop"]["width"] = serde_json::json!(0.4);
        let invalid: WallpaperEditorSettings = serde_json::from_value(invalid_value).unwrap();
        assert_eq!(
            store.save(&invalid),
            Err(EditorSettingsError::InvalidSettings)
        );
        assert_eq!(fs::read(path).unwrap(), before);
        assert_eq!(store.load(5).unwrap(), valid);
    }

    #[test]
    fn load_rejects_mismatched_identity_and_unsupported_schema() {
        let directory = TestDirectory::new();
        let store = EditorSettingsStore::at_directory(directory.path().join("settings"));
        store
            .save(&WallpaperEditorSettings::default_for(8).unwrap())
            .unwrap();
        let path = directory.path().join("settings/8.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        value["wallpaperId"] = serde_json::json!(9);
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(store.load(8), Err(EditorSettingsError::InvalidSettings));
        value["wallpaperId"] = serde_json::json!(8);
        value["schemaVersion"] = serde_json::json!(99);
        fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(store.load(8), Err(EditorSettingsError::UnsupportedSchema));
        value["schemaVersion"] = serde_json::json!(1);
        value["render"]["crop"]["x"] = serde_json::json!(0.8);
        value["render"]["crop"]["width"] = serde_json::json!(0.4);
        fs::write(
            directory.path().join("settings/8.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        assert_eq!(store.load(8), Err(EditorSettingsError::InvalidSettings));
    }

    #[test]
    fn public_errors_are_safe_and_do_not_include_paths() {
        let error = EditorSettingsError::WriteFailed;
        assert_eq!(
            error.to_app_error().code,
            ErrorCode::EditorSettingsStorageFailed
        );
        assert_eq!(
            error.to_app_error().message,
            "Wallpaper editor settings could not be loaded or saved."
        );
        assert!(
            error
                .diagnostic_summary()
                .contains("stage=write_wallpaper_settings")
        );
    }
}
