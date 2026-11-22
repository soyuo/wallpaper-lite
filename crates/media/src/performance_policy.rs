//! Versioned, atomic persistence for the user's performance policy.

use crate::library::atomic_write_bytes;
use std::{env, error::Error, fmt, fs, io::Read, path::PathBuf};
use wallpaper_core::performance_policy::{
    PERFORMANCE_POLICY_SCHEMA_VERSION, PerformancePolicySettings,
};

const MAX_POLICY_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerformancePolicyStore {
    path: PathBuf,
}

impl PerformancePolicyStore {
    /// Resolve `%APPDATA%\\WallpaperLite\\performance-policy.json` for the current user.
    pub fn for_current_user() -> Result<Self, PerformancePolicyStoreError> {
        let app_data =
            env::var_os("APPDATA").ok_or(PerformancePolicyStoreError::DataDirectoryUnavailable)?;
        let app_data = PathBuf::from(app_data);
        if !app_data.is_absolute() {
            return Err(PerformancePolicyStoreError::DataDirectoryUnavailable);
        }
        Ok(Self::at_path(
            app_data
                .join("WallpaperLite")
                .join("performance-policy.json"),
        ))
    }

    pub fn at_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn load(&self) -> Result<PerformancePolicySettings, PerformancePolicyStoreError> {
        self.validate_path()?;
        let metadata = match fs::metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(PerformancePolicySettings::default());
            }
            Err(_) => return Err(PerformancePolicyStoreError::ReadFailed),
        };
        if !metadata.is_file() || metadata.len() > MAX_POLICY_BYTES {
            return Err(PerformancePolicyStoreError::InvalidSettings);
        }
        let mut file =
            fs::File::open(&self.path).map_err(|_| PerformancePolicyStoreError::ReadFailed)?;
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.by_ref()
            .take(MAX_POLICY_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| PerformancePolicyStoreError::ReadFailed)?;
        if bytes.len() as u64 > MAX_POLICY_BYTES {
            return Err(PerformancePolicyStoreError::InvalidSettings);
        }
        parse_policy(&bytes)
    }

    pub fn save(
        &self,
        settings: &PerformancePolicySettings,
    ) -> Result<(), PerformancePolicyStoreError> {
        let bytes = serde_json::to_vec(settings)
            .map_err(|_| PerformancePolicyStoreError::InvalidSettings)?;
        self.write_candidate(&bytes)
    }

    fn validate_path(&self) -> Result<(), PerformancePolicyStoreError> {
        if self.path.is_absolute() {
            Ok(())
        } else {
            Err(PerformancePolicyStoreError::InvalidSettings)
        }
    }

    fn write_candidate(&self, bytes: &[u8]) -> Result<(), PerformancePolicyStoreError> {
        if bytes.len() as u64 > MAX_POLICY_BYTES {
            return Err(PerformancePolicyStoreError::InvalidSettings);
        }
        let candidate = parse_policy(bytes)?;
        let validated = serde_json::to_vec(&candidate)
            .map_err(|_| PerformancePolicyStoreError::InvalidSettings)?;
        atomic_write_bytes(&self.path, &validated)
            .map_err(|_| PerformancePolicyStoreError::WriteFailed)
    }
}

fn parse_policy(bytes: &[u8]) -> Result<PerformancePolicySettings, PerformancePolicyStoreError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| PerformancePolicyStoreError::InvalidSettings)?;
    match value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
    {
        Some(version) if version != u64::from(PERFORMANCE_POLICY_SCHEMA_VERSION) => {
            return Err(PerformancePolicyStoreError::UnsupportedSchema);
        }
        Some(_) => {}
        None => return Err(PerformancePolicyStoreError::InvalidSettings),
    }
    serde_json::from_value(value).map_err(|_| PerformancePolicyStoreError::InvalidSettings)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PerformancePolicyStoreError {
    DataDirectoryUnavailable,
    ReadFailed,
    WriteFailed,
    InvalidSettings,
    UnsupportedSchema,
}

impl fmt::Display for PerformancePolicyStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::DataDirectoryUnavailable => "performance policy directory is unavailable",
            Self::ReadFailed => "performance policy could not be read",
            Self::WriteFailed => "performance policy could not be saved",
            Self::InvalidSettings => "performance policy data is invalid",
            Self::UnsupportedSchema => "performance policy schema version is unsupported",
        })
    }
}

impl Error for PerformancePolicyStoreError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use wallpaper_core::performance_policy::PolicyAction;

    static TEST_DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = env::temp_dir().join(format!(
                "wallpaper-lite-performance-policy-{}-{}",
                std::process::id(),
                TEST_DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).expect("create isolated test directory");
            Self(path)
        }

        fn file(&self) -> PathBuf {
            self.0.join("performance-policy.json")
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn missing_policy_defaults_and_saved_settings_reload() {
        let directory = TestDirectory::new();
        let store = PerformancePolicyStore::at_path(directory.file());
        assert_eq!(store.load().unwrap(), PerformancePolicySettings::default());

        let settings = PerformancePolicySettings::new(
            PolicyAction::PauseRendering,
            PolicyAction::PausePlayback,
            PolicyAction::Continue,
        );
        store.save(&settings).unwrap();
        assert_eq!(store.load().unwrap(), settings);
    }

    #[test]
    fn rejected_candidate_preserves_last_valid_file() {
        let directory = TestDirectory::new();
        let store = PerformancePolicyStore::at_path(directory.file());
        let valid = PerformancePolicySettings::default();
        store.save(&valid).unwrap();
        let original_bytes = fs::read(store.path.clone()).unwrap();
        let invalid = br#"{"schemaVersion":1,"fullscreenAction":"pause_forever","sessionLockAction":"continue","batteryAction":"continue"}"#;

        assert_eq!(
            store.write_candidate(invalid),
            Err(PerformancePolicyStoreError::InvalidSettings)
        );
        assert_eq!(fs::read(store.path).unwrap(), original_bytes);
    }

    #[test]
    fn unsupported_schema_is_not_overwritten_by_a_failed_candidate() {
        let directory = TestDirectory::new();
        let store = PerformancePolicyStore::at_path(directory.file());
        let valid = PerformancePolicySettings::default();
        store.save(&valid).unwrap();
        let original_bytes = fs::read(&store.path).unwrap();
        let unsupported = br#"{"schemaVersion":2,"fullscreenAction":"continue","sessionLockAction":"continue","batteryAction":"continue"}"#;

        assert_eq!(
            store.write_candidate(unsupported),
            Err(PerformancePolicyStoreError::UnsupportedSchema)
        );
        assert_eq!(fs::read(store.path).unwrap(), original_bytes);
    }

    #[test]
    fn battery_cap_persists_and_invalid_update_preserves_last_good_file() {
        let directory = TestDirectory::new();
        let store = PerformancePolicyStore::at_path(directory.file());
        let settings = PerformancePolicySettings::default()
            .with_battery_fps_limit(Some(30))
            .unwrap();
        store.save(&settings).unwrap();
        assert_eq!(store.load().unwrap(), settings);
        let original_bytes = fs::read(&store.path).unwrap();

        let invalid = br#"{"schemaVersion":1,"fullscreenAction":"pause_playback","sessionLockAction":"pause_playback","batteryAction":"continue","batteryFpsLimit":0}"#;
        assert_eq!(
            store.write_candidate(invalid),
            Err(PerformancePolicyStoreError::InvalidSettings)
        );
        assert_eq!(fs::read(store.path).unwrap(), original_bytes);
    }
}
