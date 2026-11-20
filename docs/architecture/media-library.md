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

The catalog model and persistence boundary do not perform media import,
thumbnail generation, search, editor settings, or settings backup. Those are
implemented by later Phase 4 issues.
