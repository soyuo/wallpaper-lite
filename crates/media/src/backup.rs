//! Back up and restore the settings stored in `%APPDATA%\WallpaperLite`.
//!
//! A backup is a folder holding copies of the settings files and a `backup.json`
//! manifest. Only an allow-list of files is ever read or written, so a backup
//! cannot carry paths outside the settings folder. Source images and videos are
//! not copied; the library keeps only their paths.
//!
//! Restore is not atomic. It validates the backup in a staging folder, moves the
//! current settings into `restore-previous`, then moves the staged files into
//! place. If a step fails it moves the previous files back and reports whether
//! that worked. `restore-previous` is replaced by the next restore.

use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use wallpaper_core::error::{AppError, ErrorCode};

use crate::{CatalogStore, DisplayProfileStore, PerformancePolicyStore};

pub const BACKUP_SCHEMA_VERSION: u32 = 1;
pub const PREVIOUS_SETTINGS_DIRECTORY: &str = "restore-previous";

const MANIFEST_FILE: &str = "backup.json";
const STAGING_DIRECTORY: &str = "restore-staging";
const NEXT_PREVIOUS_DIRECTORY: &str = "restore-previous-new";
const UNRESTORED_DIRECTORY: &str = "restore-previous-unrestored";
const BACKUP_FOLDER_PREFIX: &str = "wallpaper-lite-backup-";
const TOP_LEVEL_FILES: [&str; 4] = [
    "library.json",
    "display-profiles.json",
    "performance-policy.json",
    "recent-wallpapers.json",
];
const SETTINGS_DIRECTORIES: [&str; 3] =
    ["editor-presets", "wallpaper-settings", "playback-settings"];
const MAX_FILES: usize = 20_000;
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackupFileEntry {
    /// Path relative to the settings folder, separated by `/`.
    pub path: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackupManifest {
    pub schema_version: u32,
    pub app_version: String,
    pub created_unix_secs: u64,
    pub files: Vec<BackupFileEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupOutcome {
    pub folder: PathBuf,
    pub file_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestoreOutcome {
    pub file_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupError {
    InvalidDestination,
    NothingToBackUp,
    /// A settings file is larger than restore accepts, so the backup could never be restored.
    FileTooLarge,
    WriteFailed,
    InvalidBackup,
    UnsupportedVersion,
    InvalidSettings,
    ApplyFailed,
    /// The restore failed and the previous settings could not all be put back.
    RollbackFailed,
}

impl BackupError {
    pub fn to_app_error(self) -> AppError {
        match self {
            Self::InvalidDestination => AppError::new(
                ErrorCode::BackupFailed,
                "The backup folder cannot be used. Choose an existing folder you can write to.",
            ),
            Self::NothingToBackUp => AppError::new(
                ErrorCode::BackupFailed,
                "There are no saved settings to back up yet.",
            ),
            Self::FileTooLarge => AppError::new(
                ErrorCode::BackupFailed,
                "A settings file is too large to back up.",
            ),
            Self::WriteFailed => {
                AppError::new(ErrorCode::BackupFailed, "The backup could not be written.")
            }
            Self::InvalidBackup => AppError::new(
                ErrorCode::InvalidBackup,
                "The selected folder is not a valid wallpaper Lite backup.",
            ),
            Self::UnsupportedVersion => AppError::new(
                ErrorCode::InvalidBackup,
                "This backup was made by an incompatible version of wallpaper Lite.",
            ),
            Self::InvalidSettings => AppError::new(
                ErrorCode::InvalidBackup,
                "The backup contains settings that wallpaper Lite cannot read.",
            ),
            Self::ApplyFailed => AppError::new(
                ErrorCode::RestoreFailed,
                "The backup could not be restored. Your previous settings were put back.",
            ),
            Self::RollbackFailed => AppError::new(
                ErrorCode::RestoreFailed,
                "The restore failed and some previous settings could not be put back. They are kept in the restore-previous-unrestored folder.",
            ),
        }
    }
}

/// Whether `relative` names a file that a backup may contain.
fn is_allowed_path(relative: &str) -> bool {
    if relative.contains('\\') || relative.contains(':') {
        return false;
    }
    let parts: Vec<&str> = relative.split('/').collect();
    if parts
        .iter()
        .any(|part| part.is_empty() || *part == "." || *part == "..")
    {
        return false;
    }
    match parts.as_slice() {
        [file] => TOP_LEVEL_FILES.contains(file),
        [directory, file] => {
            SETTINGS_DIRECTORIES.contains(directory) && is_settings_file_name(file)
        }
        _ => false,
    }
}

/// `<name>.json` where the name is plain ASCII and not a Windows device name, so a
/// backup cannot make restore open `nul` or `con` instead of a file.
fn is_settings_file_name(file: &str) -> bool {
    let Some(stem) = file.strip_suffix(".json") else {
        return false;
    };
    let plain = !stem.is_empty()
        && stem
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-');
    let upper = stem.to_ascii_uppercase();
    let device = matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            upper
                .strip_prefix(prefix)
                .is_some_and(|number| matches!(number.as_bytes(), [b'1'..=b'9']))
        });
    plain && !device
}

fn native_path(root: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

/// Every settings file currently present, as validated relative paths.
fn current_settings(data_directory: &Path) -> Result<Vec<String>, BackupError> {
    let mut files = Vec::new();
    for name in TOP_LEVEL_FILES {
        if native_path(data_directory, name).is_file() {
            files.push(name.to_owned());
        }
    }
    for directory in SETTINGS_DIRECTORIES {
        let entries = match fs::read_dir(data_directory.join(directory)) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => return Err(BackupError::WriteFailed),
        };
        for entry in entries {
            let entry = entry.map_err(|_| BackupError::WriteFailed)?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let relative = format!("{directory}/{name}");
            if is_allowed_path(&relative) && entry.path().is_file() {
                files.push(relative);
            }
        }
    }
    files.sort();
    if files.len() > MAX_FILES {
        return Err(BackupError::WriteFailed);
    }
    Ok(files)
}

/// Refuse to make a backup that `validate_backup` would reject.
fn check_sizes(data_directory: &Path, files: &[String]) -> Result<(), BackupError> {
    for relative in files {
        let length = fs::metadata(native_path(data_directory, relative))
            .map_err(|_| BackupError::WriteFailed)?
            .len();
        if length > MAX_FILE_BYTES {
            return Err(BackupError::FileTooLarge);
        }
    }
    Ok(())
}

fn copy_file(from: &Path, to: &Path) -> io::Result<u64> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(from, to)
}

/// `YYYYMMDD-HHMMSS` in UTC for a Unix time.
fn timestamp_label(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let secs_of_day = unix_secs % 86_400;
    // Civil-from-days (proleptic Gregorian calendar).
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        secs_of_day / 3_600,
        secs_of_day % 3_600 / 60,
        secs_of_day % 60
    )
}

/// Copy the current settings into a new folder inside `destination`.
pub fn create_backup(
    data_directory: &Path,
    destination: &Path,
    app_version: &str,
    now_unix_secs: u64,
) -> Result<BackupOutcome, BackupError> {
    if !destination.is_absolute() || !destination.is_dir() {
        return Err(BackupError::InvalidDestination);
    }
    let files = current_settings(data_directory)?;
    if files.is_empty() {
        return Err(BackupError::NothingToBackUp);
    }
    check_sizes(data_directory, &files)?;

    let label = format!("{BACKUP_FOLDER_PREFIX}{}", timestamp_label(now_unix_secs));
    let mut folder = destination.join(&label);
    let mut suffix = 1_u32;
    // `create_dir` fails if the folder exists, so an earlier backup is never reused.
    while let Err(error) = fs::create_dir(&folder) {
        if error.kind() != io::ErrorKind::AlreadyExists || suffix >= 1_000 {
            return Err(BackupError::InvalidDestination);
        }
        suffix += 1;
        folder = destination.join(format!("{label}-{suffix}"));
    }

    let result = write_backup(data_directory, &folder, &files, app_version, now_unix_secs);
    if result.is_err() {
        let _ = fs::remove_dir_all(&folder);
    }
    result.map(|file_count| BackupOutcome { folder, file_count })
}

fn write_backup(
    data_directory: &Path,
    folder: &Path,
    files: &[String],
    app_version: &str,
    now_unix_secs: u64,
) -> Result<usize, BackupError> {
    let mut entries = Vec::with_capacity(files.len());
    for relative in files {
        let from = native_path(data_directory, relative);
        let to = native_path(folder, relative);
        let bytes = copy_file(&from, &to).map_err(|_| BackupError::WriteFailed)?;
        entries.push(BackupFileEntry {
            path: relative.clone(),
            bytes,
        });
    }
    let manifest = BackupManifest {
        schema_version: BACKUP_SCHEMA_VERSION,
        app_version: app_version.to_owned(),
        created_unix_secs: now_unix_secs,
        files: entries,
    };
    let contents = serde_json::to_vec_pretty(&manifest).map_err(|_| BackupError::WriteFailed)?;
    // The manifest is written last, so a folder without one is an unfinished backup.
    fs::write(folder.join(MANIFEST_FILE), contents).map_err(|_| BackupError::WriteFailed)?;
    Ok(manifest.files.len())
}

/// Check a backup folder without touching the current settings.
pub fn validate_backup(source: &Path) -> Result<BackupManifest, BackupError> {
    let manifest_path = source.join(MANIFEST_FILE);
    let metadata = fs::metadata(&manifest_path).map_err(|_| BackupError::InvalidBackup)?;
    if !metadata.is_file() || metadata.len() > MAX_MANIFEST_BYTES {
        return Err(BackupError::InvalidBackup);
    }
    let bytes = fs::read(&manifest_path).map_err(|_| BackupError::InvalidBackup)?;
    let manifest: BackupManifest =
        serde_json::from_slice(&bytes).map_err(|_| BackupError::InvalidBackup)?;
    if manifest.schema_version != BACKUP_SCHEMA_VERSION {
        return Err(BackupError::UnsupportedVersion);
    }
    if manifest.files.is_empty() || manifest.files.len() > MAX_FILES {
        return Err(BackupError::InvalidBackup);
    }

    let mut seen = HashSet::new();
    for entry in &manifest.files {
        if !is_allowed_path(&entry.path) || !seen.insert(entry.path.as_str()) {
            return Err(BackupError::InvalidBackup);
        }
        let path = native_path(source, &entry.path);
        // A linked folder would let a backup read files from outside itself.
        if let Some(parent) = path.parent().filter(|parent| *parent != source) {
            let parent_metadata =
                fs::symlink_metadata(parent).map_err(|_| BackupError::InvalidBackup)?;
            if !parent_metadata.is_dir() || parent_metadata.file_type().is_symlink() {
                return Err(BackupError::InvalidBackup);
            }
        }
        let metadata = fs::symlink_metadata(&path).map_err(|_| BackupError::InvalidBackup)?;
        if !metadata.is_file() || metadata.len() != entry.bytes || entry.bytes > MAX_FILE_BYTES {
            return Err(BackupError::InvalidBackup);
        }
        let contents = fs::read(&path).map_err(|_| BackupError::InvalidBackup)?;
        serde_json::from_slice::<serde_json::Value>(&contents)
            .map_err(|_| BackupError::InvalidSettings)?;
    }
    Ok(manifest)
}

/// Replace the current settings with a validated backup.
pub fn restore_backup(data_directory: &Path, source: &Path) -> Result<RestoreOutcome, BackupError> {
    let manifest = validate_backup(source)?;
    let staging = data_directory.join(STAGING_DIRECTORY);
    let previous = data_directory.join(PREVIOUS_SETTINGS_DIRECTORY);
    let next_previous = data_directory.join(NEXT_PREVIOUS_DIRECTORY);
    let _ = fs::remove_dir_all(&staging);
    let _ = fs::remove_dir_all(&next_previous);

    let outcome = restore_from(&manifest, data_directory, source, &staging, &next_previous);
    let _ = fs::remove_dir_all(&staging);
    match outcome {
        Ok(outcome) => {
            // The new safety copy replaces the old one only now that the restore worked.
            let _ = fs::remove_dir_all(&previous);
            fs::rename(&next_previous, &previous).map_err(|_| BackupError::ApplyFailed)?;
            Ok(outcome)
        }
        Err(BackupError::RollbackFailed) => {
            // Keep the older generation and leave the new copy where the error says.
            let _ = fs::rename(&next_previous, data_directory.join(UNRESTORED_DIRECTORY));
            Err(BackupError::RollbackFailed)
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&next_previous);
            Err(error)
        }
    }
}

fn restore_from(
    manifest: &BackupManifest,
    data_directory: &Path,
    source: &Path,
    staging: &Path,
    previous: &Path,
) -> Result<RestoreOutcome, BackupError> {
    for entry in &manifest.files {
        copy_file(
            &native_path(source, &entry.path),
            &native_path(staging, &entry.path),
        )
        .map_err(|_| BackupError::ApplyFailed)?;
    }
    validate_typed_settings(staging)?;

    fs::create_dir_all(previous).map_err(|_| BackupError::ApplyFailed)?;
    let existing = current_settings(data_directory).map_err(|_| BackupError::ApplyFailed)?;
    let mut moved_away: Vec<&str> = Vec::new();
    let mut placed: Vec<&str> = Vec::new();

    let applied = (|| -> io::Result<()> {
        for relative in &existing {
            let to = native_path(previous, relative);
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(native_path(data_directory, relative), to)?;
            moved_away.push(relative);
        }
        for entry in &manifest.files {
            let to = native_path(data_directory, &entry.path);
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(native_path(staging, &entry.path), &to)?;
            placed.push(&entry.path);
        }
        Ok(())
    })();

    if applied.is_ok() {
        return Ok(RestoreOutcome {
            file_count: manifest.files.len(),
        });
    }

    // Put things back: remove what was placed, then return what was moved away.
    let mut rolled_back = true;
    for relative in &placed {
        rolled_back &= fs::remove_file(native_path(data_directory, relative)).is_ok();
    }
    for relative in &moved_away {
        let from = native_path(previous, relative);
        let to = native_path(data_directory, relative);
        rolled_back &= to
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::rename(from, to))
            .is_ok();
    }
    Err(if rolled_back {
        BackupError::ApplyFailed
    } else {
        BackupError::RollbackFailed
    })
}

/// Load the staged files through the real stores, so a backup that is valid JSON
/// but not valid settings is rejected before anything is replaced.
fn validate_typed_settings(staging: &Path) -> Result<(), BackupError> {
    if staging.join("library.json").is_file() {
        CatalogStore::at_path(staging.join("library.json"))
            .load()
            .map_err(|_| BackupError::InvalidSettings)?;
    }
    if staging.join("display-profiles.json").is_file() {
        DisplayProfileStore::at_path(staging.join("display-profiles.json"))
            .load()
            .map_err(|_| BackupError::InvalidSettings)?;
    }
    if staging.join("performance-policy.json").is_file() {
        PerformancePolicyStore::at_path(staging.join("performance-policy.json"))
            .load()
            .map_err(|_| BackupError::InvalidSettings)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::{
        BackupError, BackupManifest, create_backup, is_allowed_path, restore_backup,
        timestamp_label, validate_backup,
    };

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "wallpaper-lite-backup-test-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn dir(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::create_dir_all(&path).unwrap();
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A library file the real store accepts.
    fn valid_library() -> String {
        serde_json::to_string(&crate::WallpaperLibrary::default()).unwrap()
    }

    fn write(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn read(root: &Path, relative: &str) -> Option<String> {
        fs::read_to_string(root.join(relative)).ok()
    }

    #[test]
    fn timestamps_use_utc_calendar_dates() {
        assert_eq!(timestamp_label(0), "19700101-000000");
        assert_eq!(timestamp_label(951_782_400 + 3_661), "20000229-010101");
        assert_eq!(timestamp_label(1_700_000_000), "20231114-221320");
    }

    #[test]
    fn only_known_settings_files_are_allowed() {
        for allowed in [
            "library.json",
            "display-profiles.json",
            "editor-presets/a.json",
            "playback-settings/12.json",
        ] {
            assert!(is_allowed_path(allowed), "{allowed}");
        }
        for rejected in [
            "",
            "../library.json",
            "editor-presets/../library.json",
            "/library.json",
            "C:/Windows/win.ini",
            "library.json/",
            "unknown.json",
            "editor-presets/a.txt",
            "editor-presets/.json",
            "editor-presets/sub/a.json",
            "editor-presets/nul.json",
            "editor-presets/CON.json",
            "playback-settings/com1.json",
            "playback-settings/a b.json",
            "playback-settings/a.json ",
            "playback-settings/\u{00e9}.json",
            "editor-presets\\a.json",
            "restore-previous/library.json",
        ] {
            assert!(!is_allowed_path(rejected), "{rejected}");
        }
    }

    #[test]
    fn backup_copies_settings_and_restore_brings_them_back() {
        let scratch = Scratch::new();
        let data = scratch.dir("data");
        let destination = scratch.dir("backups");
        write(&data, "library.json", &valid_library());
        write(&data, "playback-settings/3.json", r#"{"a":1}"#);
        write(&data, "editor-presets/p.json", r#"{"b":2}"#);
        write(&data, "library.json.tmp123", "leftover temp file");
        write(&data, "notes.txt", "not a setting");

        let made = create_backup(&data, &destination, "9.9.9", 1_700_000_000).unwrap();
        assert_eq!(made.file_count, 3);
        assert!(
            made.folder
                .ends_with("wallpaper-lite-backup-20231114-221320")
        );
        assert!(read(&made.folder, "notes.txt").is_none());
        assert!(read(&made.folder, "library.json.tmp123").is_none());
        let manifest: BackupManifest =
            serde_json::from_str(&read(&made.folder, "backup.json").unwrap()).unwrap();
        assert_eq!(manifest.app_version, "9.9.9");

        // Change the settings, then restore.
        write(
            &data,
            "library.json",
            r#"{"schemaVersion":1,"entries":[],"x":1}"#,
        );
        write(&data, "playback-settings/extra.json", "{}");
        let outcome = restore_backup(&data, &made.folder).unwrap();
        assert_eq!(outcome.file_count, 3);
        assert_eq!(
            read(&data, "library.json").as_deref(),
            Some(valid_library().as_str())
        );
        assert_eq!(
            read(&data, "playback-settings/3.json").as_deref(),
            Some(r#"{"a":1}"#)
        );
        // Settings absent from the backup are gone, but kept in restore-previous.
        assert!(read(&data, "playback-settings/extra.json").is_none());
        assert_eq!(
            read(&data, "restore-previous/playback-settings/extra.json").as_deref(),
            Some("{}")
        );
        assert!(
            read(&data, "restore-previous/library.json")
                .unwrap()
                .contains("\"x\":1")
        );
        assert!(!data.join("restore-staging").exists());
        assert_eq!(read(&data, "notes.txt").as_deref(), Some("not a setting"));
    }

    #[test]
    fn a_settings_file_too_large_to_restore_is_refused_at_backup_time() {
        let scratch = Scratch::new();
        let data = scratch.dir("data");
        let destination = scratch.dir("backups");
        write(&data, "library.json", "{}");
        let padding = "x".repeat(super::MAX_FILE_BYTES as usize + 1);
        write(&data, "recent-wallpapers.json", &padding);
        assert_eq!(
            create_backup(&data, &destination, "1", 1).unwrap_err(),
            BackupError::FileTooLarge
        );
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
    }

    #[test]
    fn each_restore_replaces_the_previous_copy_with_the_settings_it_displaced() {
        let scratch = Scratch::new();
        let (data, folder) = made_backup(&scratch);
        write(&data, "library.json", r#"{"generation":"first"}"#);
        restore_backup(&data, &folder).unwrap();
        assert!(
            read(&data, "restore-previous/library.json")
                .unwrap()
                .contains("first")
        );

        write(&data, "library.json", &valid_library());
        write(
            &data,
            "playback-settings/3.json",
            r#"{"generation":"second"}"#,
        );
        restore_backup(&data, &folder).unwrap();
        assert_eq!(
            read(&data, "restore-previous/playback-settings/3.json").as_deref(),
            Some(r#"{"generation":"second"}"#)
        );
        assert!(!data.join("restore-previous-new").exists());
    }

    #[test]
    fn a_second_backup_in_the_same_second_gets_its_own_folder() {
        let scratch = Scratch::new();
        let data = scratch.dir("data");
        let destination = scratch.dir("backups");
        write(&data, "library.json", "{}");
        let first = create_backup(&data, &destination, "1", 100).unwrap();
        let second = create_backup(&data, &destination, "1", 100).unwrap();
        assert_ne!(first.folder, second.folder);
    }

    #[test]
    fn backup_rejects_a_missing_destination_and_empty_settings() {
        let scratch = Scratch::new();
        let data = scratch.dir("data");
        let destination = scratch.dir("backups");
        assert_eq!(
            create_backup(&data, &destination, "1", 1).unwrap_err(),
            BackupError::NothingToBackUp
        );
        write(&data, "library.json", "{}");
        assert_eq!(
            create_backup(&data, &scratch.0.join("missing"), "1", 1).unwrap_err(),
            BackupError::InvalidDestination
        );
        assert_eq!(
            create_backup(&data, Path::new("relative"), "1", 1).unwrap_err(),
            BackupError::InvalidDestination
        );
    }

    fn made_backup(scratch: &Scratch) -> (PathBuf, PathBuf) {
        let data = scratch.dir("data");
        let destination = scratch.dir("backups");
        write(&data, "library.json", &valid_library());
        write(&data, "playback-settings/3.json", r#"{"a":1}"#);
        let made = create_backup(&data, &destination, "1", 5).unwrap();
        (data, made.folder)
    }

    #[test]
    fn damaged_backups_are_rejected_and_leave_current_settings_alone() {
        let scratch = Scratch::new();
        let (data, folder) = made_backup(&scratch);
        write(&data, "library.json", r#"{"current":true}"#);

        // Truncated file: size no longer matches the manifest.
        write(&folder, "playback-settings/3.json", "{");
        assert_eq!(
            restore_backup(&data, &folder).unwrap_err(),
            BackupError::InvalidBackup
        );

        // Same size but not JSON.
        write(&folder, "playback-settings/3.json", "{a:1}");
        let mut manifest: BackupManifest =
            serde_json::from_str(&read(&folder, "backup.json").unwrap()).unwrap();
        manifest.files.iter_mut().for_each(|file| {
            file.bytes = fs::metadata(folder.join(&file.path)).unwrap().len();
        });
        fs::write(
            folder.join("backup.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert_eq!(
            restore_backup(&data, &folder).unwrap_err(),
            BackupError::InvalidSettings
        );

        // Missing manifest.
        fs::remove_file(folder.join("backup.json")).unwrap();
        assert_eq!(
            validate_backup(&folder).unwrap_err(),
            BackupError::InvalidBackup
        );

        assert_eq!(
            read(&data, "library.json").as_deref(),
            Some(r#"{"current":true}"#)
        );
        assert!(!data.join("restore-previous").exists());
    }

    #[test]
    fn manifests_with_other_versions_or_escaping_paths_are_rejected() {
        let scratch = Scratch::new();
        let (data, folder) = made_backup(&scratch);
        let original = read(&folder, "backup.json").unwrap();

        fs::write(
            folder.join("backup.json"),
            original.replace("\"schemaVersion\": 1", "\"schemaVersion\": 2"),
        )
        .unwrap();
        assert_eq!(
            restore_backup(&data, &folder).unwrap_err(),
            BackupError::UnsupportedVersion
        );

        fs::write(
            folder.join("backup.json"),
            original.replace("library.json", "../library.json"),
        )
        .unwrap();
        assert_eq!(
            restore_backup(&data, &folder).unwrap_err(),
            BackupError::InvalidBackup
        );

        fs::write(folder.join("backup.json"), "not json").unwrap();
        assert_eq!(
            restore_backup(&data, &folder).unwrap_err(),
            BackupError::InvalidBackup
        );
    }

    #[test]
    fn valid_json_that_is_not_a_library_is_rejected_before_anything_changes() {
        let scratch = Scratch::new();
        let data = scratch.dir("data");
        let destination = scratch.dir("backups");
        write(&data, "library.json", &valid_library());
        let made = create_backup(&data, &destination, "1", 5).unwrap();

        // Replace the backed-up library with JSON of the wrong shape.
        write(&made.folder, "library.json", r#"{"entries":"nope"}"#);
        let mut manifest: BackupManifest =
            serde_json::from_str(&read(&made.folder, "backup.json").unwrap()).unwrap();
        manifest.files[0].bytes = fs::metadata(made.folder.join("library.json"))
            .unwrap()
            .len();
        fs::write(
            made.folder.join("backup.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();

        write(
            &data,
            "library.json",
            r#"{"schemaVersion":1,"entries":[],"keep":true}"#,
        );
        assert_eq!(
            restore_backup(&data, &made.folder).unwrap_err(),
            BackupError::InvalidSettings
        );
        assert!(read(&data, "library.json").unwrap().contains("keep"));
        assert!(!data.join("restore-staging").exists());
    }
}
