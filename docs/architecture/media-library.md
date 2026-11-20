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
result. The library view loads catalog entries independently, searches names and
paths in the UI, and isolates source availability and preview failures to the
affected entry. Available image and video entries expose dimensions, file size,
and video duration when previewed. Video thumbnails use the first decoded frame.

Thumbnails are generated only as entries approach the visible list, resized to
at most 320 pixels on either axis, and stored as BMP cache files under
`%LOCALAPPDATA%\WallpaperLite\thumbnails\v1`. The cache is capped at 64 MiB;
its key includes the source path, size, and modification time. Repeated views
reuse the cached thumbnail and metadata. Cache files are disposable and never
replace or modify source media.

Missing or unavailable entries remain searchable. Locate validates the selected
file against the existing image/video kind and updates the original catalog ID;
Remove deletes only the catalog entry; Keep leaves it listed for later. These
actions do not delete or modify source files. Catalog persistence, browsing,
thumbnail generation, and source decoding failures are reported independently
so one bad entry does not stop other entries from appearing.
