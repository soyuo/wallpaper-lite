# Settings backup and restore

The Settings page can save the app's settings to a folder and restore them later.
The implementation is `crates/media/src/backup.rs`; the Tauri commands are in
`apps/ui/src-tauri/src/settings_backup.rs`.

## What a backup contains

A backup is a new folder named `wallpaper-lite-backup-YYYYMMDD-HHMMSS` (UTC) inside
the folder the user picks. It holds copies of these files from
`%APPDATA%\WallpaperLite`, plus `backup.json`:

- `library.json`, `display-profiles.json`, `performance-policy.json`,
  `recent-wallpapers.json`
- the `*.json` files directly inside `editor-presets`, `wallpaper-settings` and
  `playback-settings`

`backup.json` records the schema version (`1`), the app version, the creation time
and each file's relative path and size. It is written last, so a folder without it
is an unfinished backup.

Original images and videos are not copied. The library stores only their paths, so
after restoring on another PC the entries point at paths that may not exist there;
the library already shows such entries as missing and lets the user relocate them.
Nothing in backup or restore modifies, moves or deletes a source media file.

## What restore checks

Before anything is replaced, restore checks that:

1. `backup.json` parses and its schema version is supported.
2. Every listed path is on the allow-list above. Paths with `..`, drive letters,
   backslashes, absolute paths and unknown names are rejected.
3. Each file exists as a regular file, is at most 8 MiB, matches its recorded size
   and is valid JSON.
4. The staged `library.json`, `display-profiles.json` and `performance-policy.json`
   load through their real stores. The other files are checked only as JSON.

## What restore does, and does not guarantee

1. Copies the backup to `restore-staging` inside the settings folder.
2. Replaces `restore-previous` with the current settings files, moved there.
3. Moves the staged files into place.
4. Removes `restore-staging`.

Settings that are not in the backup are removed from the active folder but remain in
`restore-previous` until the next restore replaces it.

If step 2 or 3 fails, restore moves the previous files back and reports
`restore_failed`; if that also fails, the error says the previous settings are kept
in `restore-previous`.

Restore is **not atomic** and keeps no history: a crash or power loss between steps 2
and 3 can leave the active folder incomplete, with the previous settings in
`restore-previous`. Only one previous generation is kept. Writers other than the
library (for example editor or playback settings saved at the same moment) are not
blocked during a restore.

After a successful restore the UI restarts the wallpaper host so it reads the
restored display profiles and performance policy, then reloads its views.
