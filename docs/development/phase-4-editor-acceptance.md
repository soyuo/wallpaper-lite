# Phase 4 editor transaction acceptance

## Environment

- Windows 11 Education 26H2, build 26300.9974, x64
- Intel Arc 140T, driver 32.0.101.6104
- Verification dates: 2026-11-20 through 2026-11-21
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

## Library and editor flow results

The Phase 4 flows below were verified in the per-issue Windows 11 checks; they
were not presented as one uninterrupted session. Each linked issue records its
implementation-specific steps and result.

| Flow | Result | Evidence |
| --- | --- | --- |
| Catalog persistence and content-validated import | Pass | [#55](https://github.com/soyuo/wallpaper-lite/issues/55) verifies stable catalog IDs, atomic writes, corrupt-file preservation, and unchanged source bytes. [#56](https://github.com/soyuo/wallpaper-lite/issues/56) verifies content-based PNG/JPEG/BMP and H.264 MP4 imports, folder bounds, per-item failures, and original paths. |
| Browse, search, thumbnails, missing-file recovery, favorites, rename, and remove | Pass | [#57](https://github.com/soyuo/wallpaper-lite/issues/57) records search, thumbnail, Locate, and Remove smoke results with stable IDs and unchanged source hash. [#58](https://github.com/soyuo/wallpaper-lite/issues/58) records favorite persistence/filtering, rename, and catalog-only removal. |
| Validated settings and preview editing | Pass | [#59](https://github.com/soyuo/wallpaper-lite/issues/59) covers per-wallpaper settings validation and corrupt/mismatched data rejection. [#60](https://github.com/soyuo/wallpaper-lite/issues/60), [#61](https://github.com/soyuo/wallpaper-lite/issues/61), and [#62](https://github.com/soyuo/wallpaper-lite/issues/62) record still/video preview, placement/crop gestures, physical-pixel geometry, color controls, and unchanged source hash. [#63](https://github.com/soyuo/wallpaper-lite/issues/63) records undo/redo, reset, redo invalidation, and shortcut behavior. |
| Apply, Cancel, presets, and recent ordering | Pass | [#64](https://github.com/soyuo/wallpaper-lite/issues/64) records host+disk Apply, reopen, Cancel/Escape, save-failure rollback, and inactive-source rejection. [#65](https://github.com/soyuo/wallpaper-lite/issues/65) records draft-only preset load, preset rename/remove, successful-Apply recents, restart visibility, offline Apply failure, and unchanged source hash. |

## Preview and desktop parity

The still-image GPU test compares the D3D11 desktop presenter with the editor's
CPU preview for the PNG fixture using combined crop, physical position and size,
anisotropic scale, rotation, off-center anchor, and color settings. The accepted
interior RGB difference is at most 5/255; the two-physical-pixel transformed
edge band is excluded for raster coverage differences, and alpha is ignored.

The video GPU test opens `short-h264-80x64.mp4` through both decode paths used by
the editor and desktop, presents the first desktop-decoded frame through D3D11,
and renders the first editor-decoded frame through the CPU preview with the same
editor settings. On this system, software and D3D11-preferred decoded source
frames differed by at most 7/255 per channel. Comparing the same desktop-decoded
frame across the CPU preview and D3D11 presenter differed by at most 2/255;
the end-to-end editor-preview-to-desktop difference was 14/255. The video test
excludes the outer two physical pixels at the output edges. The documented
tolerance for this video sample is 16/255 to include the measured decode-path
variance. This measures the first frame of this fixture; it does not claim
identical decoding for every codec, frame, or machine.

Reproduce both GPU comparisons on an interactive Windows 11 desktop with:

```powershell
cargo +stable test -p wallpaper-renderer --target x86_64-pc-windows-msvc -- --exact image::tests::editor_preview_matches_d3d_output_within_documented_tolerance --test-threads=1
cargo +stable test -p wallpaper-renderer --target x86_64-pc-windows-msvc -- --exact image::tests::video_editor_preview_matches_d3d_output_within_documented_tolerance --test-threads=1 --nocapture
```

## Automated checks and limits

- `cargo +stable test --workspace --target x86_64-pc-windows-msvc -- --test-threads=1 --quiet`
  passed for core, desktop, IPC, media, renderer, host, and UI, including IPC v4
  settings-command round-trip and both still/video GPU parity tests.
- Workspace Clippy with `-D warnings`, workspace check, formatting, diff check,
  UI editor-history tests, and the UI production build passed.
- The Tauri UI, named-pipe IPC, host recovery worker, renderer, and settings file
  were exercised together in the Windows 11 smoke above.
- Windows 10 runtime verification was unavailable; no Windows 10 result is
  claimed.
