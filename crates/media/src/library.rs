//! Versioned local catalog for wallpaper media references.

use std::{
    collections::HashSet,
    env, fmt,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};
use wallpaper_core::error::{AppError, ErrorCode};

const LIBRARY_SCHEMA_VERSION: u32 = 1;
const MAX_DISPLAY_NAME_CHARS: usize = 256;
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Stable, monotonically allocated catalog identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WallpaperId(u64);

impl WallpaperId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Broad media kind recorded by the catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Image,
    Video,
}

/// Availability of the referenced source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Available,
    FileMissing,
    Unavailable,
}

/// A non-owning reference to a user's source media.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WallpaperEntry {
    id: WallpaperId,
    source_path: PathBuf,
    display_name: String,
    media_kind: MediaKind,
    availability: Availability,
}

impl WallpaperEntry {
    pub const fn id(&self) -> WallpaperId {
        self.id
    }

    pub fn source_path(&self) -> &Path {
        &self.source_path
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub const fn media_kind(&self) -> MediaKind {
        self.media_kind
    }

    pub const fn availability(&self) -> Availability {
        self.availability
    }
}

/// The complete persisted media catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WallpaperLibrary {
    schema_version: u32,
    next_id: u64,
    entries: Vec<WallpaperEntry>,
}

impl Default for WallpaperLibrary {
    fn default() -> Self {
        Self {
            schema_version: LIBRARY_SCHEMA_VERSION,
            next_id: 1,
            entries: Vec::new(),
        }
    }
}

impl WallpaperLibrary {
    pub fn entries(&self) -> &[WallpaperEntry] {
        &self.entries
    }

    pub fn get(&self, id: WallpaperId) -> Option<&WallpaperEntry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    pub fn get_by_raw_id(&self, id: u64) -> Option<&WallpaperEntry> {
        self.entries.iter().find(|entry| entry.id.get() == id)
    }

    pub fn remove(&mut self, id: u64) -> bool {
        let Some(index) = self.entries.iter().position(|entry| entry.id.get() == id) else {
            return false;
        };
        self.entries.remove(index);
        true
    }

    pub fn relocate(
        &mut self,
        id: u64,
        source_path: impl Into<PathBuf>,
    ) -> Result<bool, LibraryError> {
        let source_path = source_path.into();
        validate_source_path(&source_path)?;
        let metadata = fs::metadata(&source_path).map_err(|_| LibraryError::InvalidSourcePath)?;
        if !metadata.is_file() {
            return Err(LibraryError::InvalidSourcePath);
        }
        let Some(entry) = self.entries.iter_mut().find(|entry| entry.id.get() == id) else {
            return Ok(false);
        };
        entry.source_path = source_path;
        entry.availability = Availability::Available;
        Ok(true)
    }

    /// Add a reference to an absolute path; this never copies or opens the source.
    pub fn add(
        &mut self,
        source_path: impl Into<PathBuf>,
        display_name: impl Into<String>,
        media_kind: MediaKind,
    ) -> Result<WallpaperId, LibraryError> {
        let source_path = source_path.into();
        validate_source_path(&source_path)?;
        if source_path.exists() && !source_path.is_file() {
            return Err(LibraryError::InvalidSourcePath);
        }
        let display_name = validate_display_name(display_name.into())?;
        let id = WallpaperId(self.next_id);
        let next_id = self
            .next_id
            .checked_add(1)
            .ok_or(LibraryError::IdExhausted)?;
        let availability = match fs::metadata(&source_path) {
            Ok(metadata) if metadata.is_file() => Availability::Available,
            Ok(_) => return Err(LibraryError::InvalidSourcePath),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Availability::FileMissing,
            Err(_) => Availability::Unavailable,
        };

        self.entries.push(WallpaperEntry {
            id,
            source_path,
            display_name,
            media_kind,
            availability,
        });
        self.next_id = next_id;
        Ok(id)
    }

    /// Refresh availability while retaining every catalog entry.
    pub fn reconcile_availability(&mut self) {
        for entry in &mut self.entries {
            entry.availability = match fs::metadata(&entry.source_path) {
                Ok(metadata) if metadata.is_file() => Availability::Available,
                Ok(_) => Availability::Unavailable,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    Availability::FileMissing
                }
                Err(_) => Availability::Unavailable,
            };
        }
    }

    fn validate(&self) -> Result<(), LibraryError> {
        if self.schema_version != LIBRARY_SCHEMA_VERSION {
            return Err(LibraryError::UnsupportedSchema);
        }
        if self.next_id == 0 {
            return Err(LibraryError::InvalidCatalog);
        }

        let mut ids = HashSet::with_capacity(self.entries.len());
        for entry in &self.entries {
            validate_source_path(&entry.source_path)?;
            validate_display_name(entry.display_name.clone())?;
            if entry.id.0 == 0 || entry.id.0 >= self.next_id || !ids.insert(entry.id) {
                return Err(LibraryError::InvalidCatalog);
            }
        }
        Ok(())
    }
}

/// Persistence boundary for the per-user wallpaper catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogStore {
    path: PathBuf,
}

impl CatalogStore {
    /// Resolve the design's `%APPDATA%\\WallpaperLite\\library.json` location.
    pub fn for_current_user() -> Result<Self, LibraryError> {
        let app_data = env::var_os("APPDATA").ok_or(LibraryError::DataDirectoryUnavailable)?;
        let root = PathBuf::from(app_data);
        if !root.is_absolute() {
            return Err(LibraryError::DataDirectoryUnavailable);
        }
        Ok(Self::at_path(
            root.join("WallpaperLite").join("library.json"),
        ))
    }

    /// Construct a store at an explicit path, primarily for tests and host integration.
    pub fn at_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load and validate the catalog. A missing catalog starts empty; missing media is retained.
    pub fn load(&self) -> Result<WallpaperLibrary, LibraryError> {
        validate_catalog_path(&self.path)?;
        if catalog_is_missing(fs::metadata(&self.path))? {
            return Ok(WallpaperLibrary::default());
        }

        let mut bytes = Vec::new();
        File::open(&self.path)
            .and_then(|mut file| file.read_to_end(&mut bytes))
            .map_err(|_| LibraryError::ReadFailed)?;
        let mut library: WallpaperLibrary =
            serde_json::from_slice(&bytes).map_err(|_| LibraryError::InvalidCatalog)?;
        library.validate()?;
        library.reconcile_availability();
        Ok(library)
    }

    /// Persist a validated snapshot through a same-directory temporary file and replacement.
    pub fn save(&self, library: &WallpaperLibrary) -> Result<(), LibraryError> {
        validate_catalog_path(&self.path)?;
        library.validate()?;
        let mut snapshot = library.clone();
        snapshot.reconcile_availability();
        if snapshot
            .entries
            .iter()
            .any(|entry| paths_refer_to_same_location(&entry.source_path, &self.path))
        {
            return Err(LibraryError::CatalogSourceCollision);
        }

        let parent = self.path.parent().ok_or(LibraryError::InvalidCatalogPath)?;
        fs::create_dir_all(parent).map_err(|_| LibraryError::WriteFailed)?;
        let contents =
            serde_json::to_vec_pretty(&snapshot).map_err(|_| LibraryError::WriteFailed)?;
        let (temporary_path, mut temporary_file) = create_temporary_file(&self.path)?;
        let write_result = temporary_file
            .write_all(&contents)
            .and_then(|()| temporary_file.sync_all());
        drop(temporary_file);

        if write_result.is_err() {
            let _ = fs::remove_file(&temporary_path);
            return Err(LibraryError::WriteFailed);
        }

        if let Err(error) = replace_catalog(&temporary_path, &self.path) {
            let _ = fs::remove_file(&temporary_path);
            return Err(error);
        }
        Ok(())
    }
}

/// Safe, typed failures from catalog validation and persistence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryError {
    DataDirectoryUnavailable,
    InvalidCatalogPath,
    InvalidSourcePath,
    InvalidDisplayName,
    IdExhausted,
    UnsupportedSchema,
    InvalidCatalog,
    CatalogSourceCollision,
    ReadFailed,
    WriteFailed,
    ReplaceFailed,
    RollbackFailed,
}

impl LibraryError {
    pub const fn to_app_error(self) -> AppError {
        AppError::new(
            ErrorCode::LibraryStorageFailed,
            "The wallpaper library could not be loaded or saved.",
        )
    }

    /// Path-free diagnostic stage suitable for application logs.
    pub const fn diagnostic_summary(self) -> &'static str {
        match self {
            Self::DataDirectoryUnavailable => "library_stage=resolve_data_directory",
            Self::InvalidCatalogPath => "library_stage=validate_catalog_path",
            Self::InvalidSourcePath => "library_stage=validate_source_path",
            Self::InvalidDisplayName => "library_stage=validate_display_name",
            Self::IdExhausted => "library_stage=allocate_id",
            Self::UnsupportedSchema => "library_stage=validate_schema",
            Self::InvalidCatalog => "library_stage=validate_catalog",
            Self::CatalogSourceCollision => "library_stage=prevent_source_overwrite",
            Self::ReadFailed => "library_stage=read_catalog",
            Self::WriteFailed => "library_stage=write_temporary_catalog",
            Self::ReplaceFailed => "library_stage=replace_catalog",
            Self::RollbackFailed => "library_stage=restore_catalog_after_replace_failure",
        }
    }
}

impl fmt::Display for LibraryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::DataDirectoryUnavailable => "The per-user data directory is unavailable.",
            Self::InvalidCatalogPath => "The wallpaper catalog path is invalid.",
            Self::InvalidSourcePath => "A wallpaper source path is invalid.",
            Self::InvalidDisplayName => "A wallpaper display name is invalid.",
            Self::IdExhausted => "No more wallpaper identifiers can be allocated.",
            Self::UnsupportedSchema => "The wallpaper catalog schema is unsupported.",
            Self::InvalidCatalog => "The wallpaper catalog data is invalid.",
            Self::CatalogSourceCollision => "The catalog path conflicts with a source media path.",
            Self::ReadFailed => "The wallpaper catalog could not be read.",
            Self::WriteFailed => "The wallpaper catalog could not be written.",
            Self::ReplaceFailed => "The wallpaper catalog could not be replaced.",
            Self::RollbackFailed => "The previous wallpaper catalog could not be restored.",
        })
    }
}

impl std::error::Error for LibraryError {}

fn validate_catalog_path(path: &Path) -> Result<(), LibraryError> {
    if path.is_absolute() && path.file_name().is_some() {
        Ok(())
    } else {
        Err(LibraryError::InvalidCatalogPath)
    }
}

fn catalog_is_missing(metadata: std::io::Result<fs::Metadata>) -> Result<bool, LibraryError> {
    match metadata {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(_) => Err(LibraryError::ReadFailed),
    }
}

fn validate_source_path(path: &Path) -> Result<(), LibraryError> {
    if path.is_absolute() && path.file_name().is_some() {
        Ok(())
    } else {
        Err(LibraryError::InvalidSourcePath)
    }
}

fn validate_display_name(name: String) -> Result<String, LibraryError> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_DISPLAY_NAME_CHARS {
        return Err(LibraryError::InvalidDisplayName);
    }
    Ok(trimmed.to_owned())
}

fn create_temporary_file(target: &Path) -> Result<(PathBuf, File), LibraryError> {
    let parent = target.parent().ok_or(LibraryError::InvalidCatalogPath)?;
    let name = target
        .file_name()
        .ok_or(LibraryError::InvalidCatalogPath)?
        .to_string_lossy();

    for _ in 0..16 {
        let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temporary_path = parent.join(format!(".{name}.{}.{}.tmp", std::process::id(), counter));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
        {
            Ok(file) => return Ok((temporary_path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(LibraryError::WriteFailed),
        }
    }
    Err(LibraryError::WriteFailed)
}

#[cfg(windows)]
fn replace_catalog(temporary: &Path, target: &Path) -> Result<(), LibraryError> {
    use std::os::windows::ffi::OsStrExt;

    use windows::{
        Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW, REPLACE_FILE_FLAGS,
            ReplaceFileW,
        },
        core::PCWSTR,
    };

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    let temporary_wide = wide(temporary);
    let target_wide = wide(target);
    if target.exists() {
        let backup = unique_sibling_path(target, "rollback")?;
        let backup_wide = wide(&backup);
        let replacement = unsafe {
            ReplaceFileW(
                PCWSTR(target_wide.as_ptr()),
                PCWSTR(temporary_wide.as_ptr()),
                PCWSTR(backup_wide.as_ptr()),
                REPLACE_FILE_FLAGS(0),
                None,
                None,
            )
        };
        match replacement {
            Ok(()) => {
                let _ = fs::remove_file(backup);
                Ok(())
            }
            Err(_) => {
                if backup.exists() {
                    let restore = unsafe {
                        MoveFileExW(
                            PCWSTR(backup_wide.as_ptr()),
                            PCWSTR(target_wide.as_ptr()),
                            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                        )
                    };
                    if restore.is_err() {
                        return Err(LibraryError::RollbackFailed);
                    }
                } else if !target.exists() {
                    return Err(LibraryError::RollbackFailed);
                }
                Err(LibraryError::ReplaceFailed)
            }
        }
    } else {
        unsafe {
            MoveFileExW(
                PCWSTR(temporary_wide.as_ptr()),
                PCWSTR(target_wide.as_ptr()),
                MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(|_| LibraryError::ReplaceFailed)
    }
}

#[cfg(not(windows))]
fn replace_catalog(temporary: &Path, target: &Path) -> Result<(), LibraryError> {
    fs::rename(temporary, target).map_err(|_| LibraryError::ReplaceFailed)
}

#[cfg(windows)]
fn unique_sibling_path(target: &Path, suffix: &str) -> Result<PathBuf, LibraryError> {
    let parent = target.parent().ok_or(LibraryError::InvalidCatalogPath)?;
    let name = target
        .file_name()
        .ok_or(LibraryError::InvalidCatalogPath)?
        .to_string_lossy();
    for _ in 0..16 {
        let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(
            ".{name}.{}.{}.{}",
            std::process::id(),
            counter,
            suffix
        ));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(LibraryError::WriteFailed)
}

fn paths_refer_to_same_location(left: &Path, right: &Path) -> bool {
    let left = fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
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

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let counter = TEST_DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "wallpaper-lite-library-test-{}-{counter}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("test directory should be created");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn catalog_round_trip_keeps_stable_entries_and_source_bytes() {
        let directory = TestDirectory::new();
        let source = directory.path().join("source.png");
        let original = b"read-only source bytes";
        fs::write(&source, original).expect("source should be written");
        let store = CatalogStore::at_path(directory.path().join("WallpaperLite/library.json"));
        let mut library = WallpaperLibrary::default();
        let id = library
            .add(&source, "  Sample wallpaper  ", MediaKind::Image)
            .expect("absolute source should be accepted");

        store.save(&library).expect("catalog should save");
        let loaded = store.load().expect("catalog should load");
        let entry = loaded.get(id).expect("stable ID should survive round trip");
        assert_eq!(entry.source_path(), source);
        assert_eq!(entry.display_name(), "Sample wallpaper");
        assert_eq!(entry.media_kind(), MediaKind::Image);
        assert_eq!(entry.availability(), Availability::Available);
        assert_eq!(
            fs::read(&source).expect("source remains readable"),
            original
        );
    }

    #[test]
    fn removing_entry_does_not_touch_source_and_preserves_monotonic_ids() {
        let directory = TestDirectory::new();
        let first_source = directory.path().join("first.png");
        let second_source = directory.path().join("second.png");
        fs::write(&first_source, b"first source").expect("first source should be written");
        fs::write(&second_source, b"second source").expect("second source should be written");
        let mut library = WallpaperLibrary::default();
        let first_id = library
            .add(&first_source, "First", MediaKind::Image)
            .expect("first source should be catalogued");
        let second_id = library
            .add(&second_source, "Second", MediaKind::Image)
            .expect("second source should be catalogued");

        assert!(library.remove(first_id.get()));
        assert!(!library.remove(first_id.get()));
        let third_id = library
            .add(
                directory.path().join("third.png"),
                "Third",
                MediaKind::Image,
            )
            .expect("a missing absolute path can remain catalogued");

        assert!(library.get_by_raw_id(first_id.get()).is_none());
        assert_eq!(
            library.get_by_raw_id(second_id.get()).unwrap().id(),
            second_id
        );
        assert!(third_id.get() > second_id.get());
        assert_eq!(fs::read(first_source).unwrap(), b"first source");
        assert_eq!(fs::read(second_source).unwrap(), b"second source");
    }

    #[test]
    fn relocating_entry_keeps_id_and_does_not_modify_either_source() {
        let directory = TestDirectory::new();
        let missing_source = directory.path().join("missing.png");
        let located_source = directory.path().join("located.png");
        fs::write(&located_source, b"replacement image bytes")
            .expect("replacement source should be written");
        let mut library = WallpaperLibrary::default();
        let id = library
            .add(&missing_source, "Moved image", MediaKind::Image)
            .expect("missing absolute path should be catalogued");

        assert!(library.relocate(id.get(), &located_source).unwrap());
        assert_eq!(library.get(id).unwrap().source_path(), located_source);
        assert_eq!(
            library.get(id).unwrap().availability(),
            Availability::Available
        );
        assert!(!library.relocate(u64::MAX, &located_source).unwrap());
        assert_eq!(
            library.relocate(id.get(), directory.path()),
            Err(LibraryError::InvalidSourcePath)
        );
        assert_eq!(
            fs::read(located_source).unwrap(),
            b"replacement image bytes"
        );
        assert!(!missing_source.exists());
    }

    #[test]
    fn load_marks_missing_sources_without_dropping_other_entries() {
        let directory = TestDirectory::new();
        let existing = directory.path().join("existing.mp4");
        let removed = directory.path().join("removed.png");
        fs::write(&existing, b"video").expect("existing source should be written");
        fs::write(&removed, b"image").expect("removed source should be written");
        let store = CatalogStore::at_path(directory.path().join("library.json"));
        let mut library = WallpaperLibrary::default();
        let existing_id = library
            .add(&existing, "Existing", MediaKind::Video)
            .expect("existing path should be accepted");
        let removed_id = library
            .add(&removed, "Removed", MediaKind::Image)
            .expect("second path should be accepted");
        store.save(&library).expect("catalog should save");
        fs::remove_file(&removed).expect("source should be removed for the test");

        let loaded = store.load().expect("catalog should load");
        assert_eq!(loaded.entries().len(), 2);
        assert_eq!(
            loaded.get(existing_id).unwrap().availability(),
            Availability::Available
        );
        assert_eq!(
            loaded.get(removed_id).unwrap().availability(),
            Availability::FileMissing
        );
    }

    #[test]
    fn load_keeps_non_file_sources_as_unavailable() {
        let directory = TestDirectory::new();
        let source = directory.path().join("source.png");
        fs::write(&source, b"source").expect("source should be written");
        let store = CatalogStore::at_path(directory.path().join("library.json"));
        let mut library = WallpaperLibrary::default();
        let id = library
            .add(&source, "Source", MediaKind::Image)
            .expect("file source should be accepted");
        library.entries[0].source_path = directory.path().to_path_buf();

        store.save(&library).expect("catalog should save");
        let loaded = store.load().expect("catalog should load");
        assert_eq!(
            loaded.get(id).unwrap().availability(),
            Availability::Unavailable
        );
    }

    #[test]
    fn invalid_catalog_data_is_rejected_without_replacing_previous_file() {
        let directory = TestDirectory::new();
        let store = CatalogStore::at_path(directory.path().join("library.json"));
        let library = WallpaperLibrary::default();
        store.save(&library).expect("initial catalog should save");
        let previous = fs::read(store.path()).expect("saved catalog should be readable");

        let mut invalid = library;
        invalid.schema_version = LIBRARY_SCHEMA_VERSION + 1;
        assert_eq!(store.save(&invalid), Err(LibraryError::UnsupportedSchema));
        assert_eq!(fs::read(store.path()).unwrap(), previous);

        fs::write(store.path(), b"not json").expect("corrupt catalog fixture should be written");
        assert_eq!(store.load(), Err(LibraryError::InvalidCatalog));
    }

    #[test]
    fn load_does_not_treat_catalog_metadata_errors_as_a_missing_catalog() {
        assert_eq!(
            catalog_is_missing(Err(std::io::Error::from(std::io::ErrorKind::NotFound))),
            Ok(true)
        );
        assert_eq!(
            catalog_is_missing(Err(std::io::Error::from(
                std::io::ErrorKind::PermissionDenied
            ))),
            Err(LibraryError::ReadFailed)
        );
    }

    #[test]
    fn invalid_paths_names_and_catalog_source_collisions_are_rejected() {
        let directory = TestDirectory::new();
        let store = CatalogStore::at_path(directory.path().join("library.json"));
        let mut library = WallpaperLibrary::default();
        assert_eq!(
            library.add("relative.png", "Relative", MediaKind::Image),
            Err(LibraryError::InvalidSourcePath)
        );
        assert_eq!(
            library.add(directory.path().join("x.png"), "  ", MediaKind::Image),
            Err(LibraryError::InvalidDisplayName)
        );
        assert_eq!(
            library.add(directory.path(), "Directory", MediaKind::Image),
            Err(LibraryError::InvalidSourcePath)
        );

        fs::write(store.path(), b"source bytes").expect("collision source should be written");
        library
            .add(store.path(), "Catalog source", MediaKind::Image)
            .expect("catalog path is a valid absolute source reference");
        assert_eq!(
            store.save(&library),
            Err(LibraryError::CatalogSourceCollision)
        );
        assert_eq!(fs::read(store.path()).unwrap(), b"source bytes");
    }

    #[test]
    fn app_error_and_diagnostic_do_not_expose_user_paths() {
        let error = LibraryError::ReadFailed;
        assert_eq!(error.to_app_error().code, ErrorCode::LibraryStorageFailed);
        assert!(!error.to_string().contains("C:\\Users"));
        assert_eq!(error.diagnostic_summary(), "library_stage=read_catalog");
    }
}
