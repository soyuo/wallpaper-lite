# Phase 4 editor transaction acceptance

## Environment

- Windows 11 Education 26H2, build 26300.9974, x64
- Intel Arc 140T, driver 32.0.101.6104
- Isolated `%APPDATA%` and `%LOCALAPPDATA%` under a temporary directory
- Image fixture: `crates/media/tests/fixtures/pixels.png`
- Fixture SHA-256 before and after the session:
  `D1834273D59897DA619B76E1C0ADE794398AE09EFBF6E57896545FD38A4F60A8`

## Apply, Cancel, and reload

The Tauri UI opened the fixture preview while `wallpaper-host --image` rendered
it in the desktop child window. Applying brightness `1.5` produced the host
`editor_settings_applied` event and atomically wrote wallpaper ID `1` settings.
After reopening the editor, the brightness control loaded the persisted value
`1.5`.

With `1.5` already applied, changing the draft to `2.0` and selecting **Cancel**
closed the editor without changing the settings file hash or producing another
host apply event. Repeating with brightness `2.2` and pressing Escape had the
same result.

## Save failure rollback

The existing settings file was marked read-only after loading it. Apply then
changed draft brightness to `2.5`. The UI reported that the save failed and the
previous settings remained active. The settings file hash and stored brightness
`1.5` were unchanged. Host logs recorded two apply events: the candidate render
and the compensating render of the prior snapshot.

Desktop captures taken after applying `2.5` and after rollback to `1.5` differed
at 43,225 sampled grid pixels. At the center sample, the red-channel pixel
changed from `(202, 31, 48)` to `(161, 23, 36)`, confirming the restored host
render matched the persisted prior value. The original fixture hash remained
unchanged.

## Active-source identity

The isolated library contained two available image entries. The host displayed
entry `1` (`pixels.png`). Applying settings to entry `2` (`solid.jpg`) was
rejected with “This library item is not the active desktop wallpaper. Its
settings were not changed.” No settings file was created for entry `2`; the
hash of entry `1` settings remained `06E848DDA313102458A1A4C9456E8610D03DB056AAC5372045980C013A28952D`,
and the host's successful-apply event count stayed at one. The host logged the
inactive-source rejection. The active fixture hash remained
`D1834273D59897DA619B76E1C0ADE794398AE09EFBF6E57896545FD38A4F60A8`.

## Automated checks and limits

- Windows-target tests passed for core, IPC, media, renderer, host, and UI,
  including IPC v4 settings-command round-trip coverage.
- Workspace Clippy with `-D warnings`, workspace check, formatting, diff check,
  UI editor-history tests, and the UI production build passed.
- The Tauri UI, named-pipe IPC, host recovery worker, renderer, and settings file
  were exercised together in the Windows 11 smoke above.
- Windows 10 runtime verification was unavailable; no Windows 10 result is
  claimed.
