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
   backslashes, absolute paths and unknown names are rejected, and so are file
   names that are not plain ASCII letters, digits, `_` or `-`, or that are Windows
   device names such as `nul` and `con`. A folder inside the backup that is a link
   is rejected.
3. Each file exists as a regular file, is at most 8 MiB, matches its recorded size
   and is valid JSON. Backup applies the same size limit, so it refuses to create a
   backup that restore would reject.
4. The staged `library.json`, `display-profiles.json` and `performance-policy.json`
   load through their real stores. The other files are checked only as JSON.

## What restore does, and does not guarantee

The UI backend first asks the wallpaper host to shut down, so the host cannot write
its old in-memory settings over the restored files. If the host cannot be stopped,
nothing is changed. Then:

1. Copies the backup to `restore-staging` inside the settings folder.
2. Moves the current settings files into `restore-previous-new`.
3. Moves the staged files into place.
4. On success, replaces `restore-previous` with `restore-previous-new` and removes
   `restore-staging`.

The host is started again afterwards, whether or not the restore worked, so it reads
the restored display profiles and performance policy from disk.

Settings that are not in the backup are removed from the active folder but remain in
`restore-previous` until the next restore replaces it. The older `restore-previous`
is kept until a restore has succeeded.

If step 2 or 3 fails, restore moves the files back and reports `restore_failed`. If
that also fails, the displaced settings stay in `restore-previous-unrestored` and the
error says so.

Restore is **not atomic** and keeps no history: a crash or power loss between steps 2
and 3 can leave the active folder incomplete, with the displaced settings in
`restore-previous-new`. Only one previous generation is kept. Settings written by the
UI itself at the same moment (for example editor or playback settings) are not
blocked during a restore; only the library catalog is locked.

After a successful restore the UI reloads its views.
