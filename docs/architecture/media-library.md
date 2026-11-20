# Local media catalog

`wallpaper-media` owns the local wallpaper catalog boundary. It stores references
to user files; it does not copy, rename, re-encode, or delete them.

## Catalog contract

- The per-user catalog path is `%APPDATA%\WallpaperLite\library.json`.
- The versioned schema stores a stable numeric ID, absolute source path, display
  name, media kind, and availability for each entry.
- A missing source remains in the catalog with `file_missing` availability.
  Loading reconciles availability for every entry without dropping the other
  entries. Other filesystem failures use `unavailable` rather than being
  mislabeled as missing.
- Source paths and display names are validated. Catalog errors expose a safe
  `LibraryStorageFailed` application error; diagnostic summaries do not include
  user paths.
- A catalog path may not equal a referenced source path, preventing catalog
  writes from replacing a listed media file.

## Persistence

Saving validates the complete snapshot, writes a uniquely named temporary file
beside the catalog, and flushes it before replacement. On Windows, an existing
catalog is replaced with `ReplaceFileW`; a short-lived same-directory rollback
file protects the prior catalog if replacement fails. A new catalog is moved
into place with `MoveFileExW` and `MOVEFILE_WRITE_THROUGH`. The rollback file is
removed after a successful replacement. This is write protection for the
catalog update, not a user-facing settings backup or recovery system.

The Windows API behavior is documented by Microsoft for
[`ReplaceFileW`](https://learn.microsoft.com/windows/win32/api/winbase/nf-winbase-replacefilew)
and
[`MoveFileExW`](https://learn.microsoft.com/windows/win32/api/winbase/nf-winbase-movefileexw).

## Import

`import_local_media` accepts one or more selected paths. File candidates are
validated by decoded content: WIC accepts PNG, JPEG, and BMP containers, while
Media Foundation accepts MP4 containers that expose an H.264 video stream.
Extensions are not used to classify media. Folder selections are scanned
recursively without following child symlinks, with a depth limit of 32 and a
maximum of 4,096 candidates per request.

Each candidate returns its own status and safe failure category. Duplicate
canonical paths reuse the existing stable ID. Valid additions are saved in one
catalog transaction; a failed save converts provisional successes to catalog
write failures and preserves the prior catalog. Source files are read-only and
are never copied, renamed, or removed.

The Tauri UI provides file and folder selection and displays each import
result. Thumbnail generation, library browsing/search, editor settings, and
settings backup remain later Phase 4 work.
