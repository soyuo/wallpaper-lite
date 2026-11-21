//! Atomic persistence for stable-ID keyed display profiles.

use crate::library::atomic_write_bytes;
use std::{env, error::Error, fmt, fs, io::Read, path::PathBuf};
use wallpaper_display::{
    DISPLAY_PROFILE_SCHEMA_VERSION, DisplayProfile, DisplayProfileSet,
    DisplayProfileValidationError,
};

const MAX_DISPLAY_PROFILE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayProfileStore {
    path: PathBuf,
}

impl DisplayProfileStore {
    /// Resolve `%APPDATA%\\WallpaperLite\\display-profiles.json` for the current user.
    pub fn for_current_user() -> Result<Self, DisplayProfileStoreError> {
        let app_data =
            env::var_os("APPDATA").ok_or(DisplayProfileStoreError::DataDirectoryUnavailable)?;
        let app_data = PathBuf::from(app_data);
        if !app_data.is_absolute() {
            return Err(DisplayProfileStoreError::DataDirectoryUnavailable);
        }
        Ok(Self::at_path(
            app_data.join("WallpaperLite").join("display-profiles.json"),
        ))
    }

    /// Construct a store at a specific JSON file path, primarily for isolated tests.
    pub fn at_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Load all known profiles, including records for displays not currently connected.
    pub fn load(&self) -> Result<DisplayProfileSet, DisplayProfileStoreError> {
        self.validate_path()?;
        let metadata = match fs::metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(DisplayProfileSet::default());
            }
            Err(_) => return Err(DisplayProfileStoreError::ReadFailed),
        };
        if !metadata.is_file() || metadata.len() > MAX_DISPLAY_PROFILE_BYTES {
            return Err(DisplayProfileStoreError::InvalidProfiles);
        }
        let mut file =
            fs::File::open(&self.path).map_err(|_| DisplayProfileStoreError::ReadFailed)?;
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.by_ref()
            .take(MAX_DISPLAY_PROFILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| DisplayProfileStoreError::ReadFailed)?;
        if bytes.len() as u64 > MAX_DISPLAY_PROFILE_BYTES {
            return Err(DisplayProfileStoreError::InvalidProfiles);
        }
        parse_profiles(&bytes)
    }

    pub fn save(&self, profiles: &DisplayProfileSet) -> Result<(), DisplayProfileStoreError> {
        self.validate_path()?;
        let bytes = serde_json::to_vec_pretty(profiles)
            .map_err(|_| DisplayProfileStoreError::InvalidProfiles)?;
        self.write_candidate(&bytes)
    }

    /// Validate and save a complete snapshot before touching the existing file.
    pub fn save_profiles(
        &self,
        profiles: Vec<DisplayProfile>,
    ) -> Result<DisplayProfileSet, DisplayProfileStoreError> {
        let profiles = DisplayProfileSet::new(profiles).map_err(DisplayProfileStoreError::from)?;
        self.save(&profiles)?;
        Ok(profiles)
    }

    /// Update one display while preserving profiles for every other, including disconnected, ID.
    pub fn upsert(
        &self,
        profile: DisplayProfile,
    ) -> Result<DisplayProfileSet, DisplayProfileStoreError> {
        let mut profiles = self.load()?;
        profiles
            .upsert(profile)
            .map_err(DisplayProfileStoreError::from)?;
        self.save(&profiles)?;
        Ok(profiles)
    }

    fn validate_path(&self) -> Result<(), DisplayProfileStoreError> {
        if self.path.is_absolute() {
            Ok(())
        } else {
            Err(DisplayProfileStoreError::InvalidProfiles)
        }
    }

    fn write_candidate(&self, bytes: &[u8]) -> Result<(), DisplayProfileStoreError> {
        if bytes.len() as u64 > MAX_DISPLAY_PROFILE_BYTES {
            return Err(DisplayProfileStoreError::InvalidProfiles);
        }
        atomic_write_bytes(&self.path, bytes).map_err(|_| DisplayProfileStoreError::WriteFailed)
    }
}

fn parse_profiles(bytes: &[u8]) -> Result<DisplayProfileSet, DisplayProfileStoreError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| DisplayProfileStoreError::InvalidProfiles)?;
    match value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
    {
        Some(version) if version != u64::from(DISPLAY_PROFILE_SCHEMA_VERSION) => {
            return Err(DisplayProfileStoreError::UnsupportedSchema);
        }
        Some(_) => {}
        None => return Err(DisplayProfileStoreError::InvalidProfiles),
    }
    serde_json::from_value(value).map_err(|_| DisplayProfileStoreError::InvalidProfiles)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisplayProfileStoreError {
    DataDirectoryUnavailable,
    ReadFailed,
    WriteFailed,
    InvalidProfiles,
    UnsupportedSchema,
    Validation(DisplayProfileValidationError),
}

impl From<DisplayProfileValidationError> for DisplayProfileStoreError {
    fn from(error: DisplayProfileValidationError) -> Self {
        Self::Validation(error)
    }
}

impl fmt::Display for DisplayProfileStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::DataDirectoryUnavailable => "display profile directory is unavailable",
            Self::ReadFailed => "display profiles could not be read",
            Self::WriteFailed => "display profiles could not be saved",
            Self::InvalidProfiles => "display profile data is invalid",
            Self::UnsupportedSchema => "display profile schema version is unsupported",
            Self::Validation(error) => return error.fmt(formatter),
        };
        formatter.write_str(message)
    }
}

impl Error for DisplayProfileStoreError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use wallpaper_display::{DisplayId, DisplayPresentationMode};

    static TEST_DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let directory = env::temp_dir().join(format!(
                "wallpaper-lite-display-profiles-{}-{}",
                std::process::id(),
                TEST_DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&directory).expect("create isolated test directory");
            Self(directory)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn profile(path: &str, wallpaper_id: u64, fps: Option<u16>) -> DisplayProfile {
        DisplayProfile::new(
            DisplayId::from_device_path(path).expect("valid display path"),
            wallpaper_id,
            DisplayPresentationMode::Independent,
            fps,
        )
        .expect("valid display profile")
    }

    #[test]
    fn profiles_round_trip_across_store_instances_with_schema_version() {
        let directory = TestDirectory::new();
        let path = directory.path("display-profiles.json");
        let store = DisplayProfileStore::at_path(&path);
        let profiles = vec![
            profile(r"\\?\DISPLAY#B", 22, Some(120)),
            profile(r"\\?\DISPLAY#A", 11, None),
        ];

        let saved = store.save_profiles(profiles).expect("save profiles");
        let json = fs::read_to_string(&path).expect("read persisted JSON");
        let loaded = DisplayProfileStore::at_path(path)
            .load()
            .expect("load through new store instance");

        assert!(json.contains("\"schemaVersion\": 1"));
        assert_eq!(loaded, saved);
        assert_eq!(loaded.profiles().len(), 2);
        assert_eq!(loaded.profiles()[0].display_id().as_str(), r"\\?\display#a");
        assert_eq!(loaded.profiles()[1].fps_override(), Some(120));
    }

    #[test]
    fn upsert_keeps_disconnected_display_records() {
        let directory = TestDirectory::new();
        let store = DisplayProfileStore::at_path(directory.path("display-profiles.json"));
        store
            .save_profiles(vec![
                profile(r"\\?\DISPLAY#A", 11, None),
                profile(r"\\?\DISPLAY#B", 22, Some(60)),
            ])
            .expect("save initial topology profiles");

        // Only display A is currently connected; updating it must retain display B.
        let updated = store
            .upsert(profile(r"\\?\DISPLAY#A", 33, Some(30)))
            .expect("upsert connected display");
        let returned = DisplayId::from_device_path(r"\\?\DISPLAY#B").unwrap();

        assert_eq!(updated.profiles().len(), 2);
        assert_eq!(updated.get(&returned).unwrap().wallpaper_id(), 22);
        assert_eq!(store.load().unwrap(), updated);
    }

    #[test]
    fn invalid_update_does_not_replace_last_valid_file_or_source_media() {
        let directory = TestDirectory::new();
        let store = DisplayProfileStore::at_path(directory.path("display-profiles.json"));
        let source = directory.path("source.png");
        let source_bytes = b"immutable source media";
        fs::write(&source, source_bytes).unwrap();
        store
            .save_profiles(vec![profile(r"\\?\DISPLAY#A", 11, Some(60))])
            .unwrap();
        let previous = fs::read(&store.path).unwrap();

        assert_eq!(
            DisplayProfile::new(
                DisplayId::from_device_path(r"\\?\DISPLAY#A").unwrap(),
                0,
                DisplayPresentationMode::Independent,
                None,
            ),
            Err(DisplayProfileValidationError::InvalidWallpaperId)
        );
        assert_eq!(
            DisplayProfile::new(
                DisplayId::from_device_path(r"\\?\DISPLAY#A").unwrap(),
                12,
                DisplayPresentationMode::Independent,
                Some(121),
            ),
            Err(DisplayProfileValidationError::InvalidFpsOverride)
        );
        let invalid = store.save_profiles(vec![
            profile(r"\\?\DISPLAY#A", 12, None),
            profile(r"\\?\display#a", 13, None),
        ]);
        let invalid_mode = br#"{"schemaVersion":1,"profiles":[{"displayId":"\\\\?\\display#a","wallpaperId":12,"presentationMode":"mirror","fpsOverride":60}]}"#;

        assert_eq!(
            invalid,
            Err(DisplayProfileStoreError::Validation(
                DisplayProfileValidationError::DuplicateDisplayId
            ))
        );
        assert_eq!(
            parse_profiles(invalid_mode),
            Err(DisplayProfileStoreError::InvalidProfiles)
        );
        assert_eq!(fs::read(&store.path).unwrap(), previous);
        assert_eq!(fs::read(source).unwrap(), source_bytes);
        assert_eq!(store.load().unwrap().profiles()[0].wallpaper_id(), 11);
    }

    #[test]
    fn invalid_ids_modes_wallpaper_ids_and_fps_are_rejected() {
        assert!(DisplayId::from_device_path("\n").is_err());
        assert_eq!(
            DisplayProfile::new(
                DisplayId::from_device_path(r"\\?\DISPLAY#A").unwrap(),
                0,
                DisplayPresentationMode::Independent,
                None,
            ),
            Err(DisplayProfileValidationError::InvalidWallpaperId)
        );
        for fps in [Some(0), Some(121)] {
            assert_eq!(
                DisplayProfile::new(
                    DisplayId::from_device_path(r"\\?\DISPLAY#A").unwrap(),
                    1,
                    DisplayPresentationMode::Independent,
                    fps,
                ),
                Err(DisplayProfileValidationError::InvalidFpsOverride)
            );
        }

        let unknown_mode = br#"{"schemaVersion":1,"profiles":[{"displayId":"\\\\?\\display#a","wallpaperId":1,"presentationMode":"mirror","fpsOverride":60}]}"#;
        assert_eq!(
            parse_profiles(unknown_mode),
            Err(DisplayProfileStoreError::InvalidProfiles)
        );
    }

    #[test]
    fn corrupt_or_unknown_schema_data_is_not_overwritten_by_upsert() {
        let directory = TestDirectory::new();
        let path = directory.path("display-profiles.json");
        let store = DisplayProfileStore::at_path(&path);
        let corrupt = br#"{"schemaVersion":1,"profiles":["bad"]}"#;
        fs::write(&path, corrupt).unwrap();

        assert_eq!(
            store.upsert(profile(r"\\?\DISPLAY#A", 1, None)),
            Err(DisplayProfileStoreError::InvalidProfiles)
        );
        assert_eq!(fs::read(&path).unwrap(), corrupt);

        fs::write(&path, br#"{"schemaVersion":2,"profiles":[]}"#).unwrap();
        assert_eq!(
            store.load(),
            Err(DisplayProfileStoreError::UnsupportedSchema)
        );
    }
}
