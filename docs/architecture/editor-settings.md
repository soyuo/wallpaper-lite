# Per-wallpaper editor settings

`wallpaper_core::editor_settings::WallpaperEditorSettings` is the shared,
versioned model for non-destructive editor values. It is stored separately from
both the catalog and the original media. The numeric `wallpaperId` is also
checked against the settings file name, so a file copied to another ID cannot
silently apply to the wrong catalog item.

## Stored shape

```json
{
  "schemaVersion": 1,
  "wallpaperId": 7,
  "render": {
    "placementMode": "cover",
    "positionPx": { "x": 0, "y": 0 },
    "sizePx": null,
    "scale": { "x": 1.0, "y": 1.0 },
    "rotationDegrees": 0.0,
    "anchor": { "x": 0.5, "y": 0.5 },
    "crop": { "x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0 }
  },
  "color": {
    "brightness": 1.0,
    "contrast": 1.0,
    "saturation": 1.0,
    "gamma": 1.0,
    "opacity": 1.0,
    "temperature": 0.0,
    "red": 1.0,
    "green": 1.0,
    "blue": 1.0
  }
}
```

## Units, defaults, and validation

| Property | Unit and valid range | Default |
| --- | --- | --- |
| `placementMode` | `cover`, `fit`, `stretch`, `center`, or `custom` | `cover` |
| `positionPx.x/y` | Physical output pixels, each `-32768..=32768`; positive X is right, positive Y is down | `0, 0` |
| `sizePx` | Optional physical output size; width and height each `1..=32768`; `null` uses the placement mode's computed size | `null` |
| `scale.x/y` | Relative to the selected output size, each `0.01..=8.0` | `1.0, 1.0` |
| `rotationDegrees` | Clockwise degrees, `-3600..=3600` | `0` |
| `anchor.x/y` | Normalized point in `0..=1` | `0.5, 0.5` |
| `crop.x/y/width/height` | Top-left-origin source coordinates normalized to `0..=1`; width/height must be positive and the rectangle must remain inside the source | `0, 0, 1, 1` |
| brightness, contrast, saturation | Linear multipliers in `0..=4` | `1.0` |
| gamma | `0.1..=4.0` | `1.0` |
| opacity | `0..=1` | `1.0` |
| temperature | Normalized warmth in `-1..=1` | `0.0` |
| red, green, blue | Linear channel multipliers in `0..=4` | `1.0` |

All floating values must be finite. Crop and anchor coordinates are normalized
media/shape coordinates, not UI dimensions. The UI's CSS pixels must be
converted to physical output pixels before storing position or size.

## Persistence boundary

`EditorSettingsStore::for_current_user()` stores each record at
`%APPDATA%\WallpaperLite\wallpaper-settings\<wallpaperId>.json`.
`load(id)` returns validated defaults if the file does not exist and rejects
corrupt, oversized, mismatched-ID, or unsupported-schema data with a safe typed
error. `save(settings)` validates before writing, writes and flushes a unique
same-directory temporary file, then atomically replaces the prior file using
the catalog's rollback-aware replacement path. Invalid settings therefore do
not write or replace the last valid settings; replacement failures return a
typed storage error.

Saving editor settings never rewrites `library.json`, changes catalog IDs, or
opens or modifies the source media. The `custom` placement mode starts from
Cover geometry and applies stored physical size, position, scale, rotation,
anchor, and crop. Canvas pointer deltas are converted from CSS dimensions to
selected-monitor physical pixels before position and size are stored. This
per-wallpaper store does not implement global-settings backup/restore. Edit
history is handled separately from persistence. Editor changes remain in the UI
draft until Apply. Apply validates the complete snapshot, sends it to the host,
and waits for the active presenter to render it. After host acknowledgement, the
UI atomically saves the snapshot. If saving fails, the UI asks the host to
reapply the previously loaded snapshot and returns a visible error. If the host
request times out or its response is uncertain, the UI also asks the host to
reapply the previous snapshot before returning. A D3D render/present failure
receives the same compensation because the back buffer may have been drawn
before presentation failed. Validation and inactive-source rejections return
directly without changing persistence. Concurrent Apply requests are serialized.
Cancel, Escape, and closing the editor discard the draft without sending IPC or
writing settings. Reopening the editor loads the last successfully applied
snapshot.

Before Apply, the UI resolves the selected library ID to its available source
path. The host compares the canonical path with the active image or video
presenter and rejects mismatches before changing the presenter or the settings
snapshot retained for Explorer recovery. The recovery worker binds that active
snapshot to its canonical source path. It reapplies the snapshot only when
restoring the same source; switching to a different image or video clears the
old snapshot and starts the new source with default render settings, so edits do
not leak between library items.

If a failed or uncertain Apply cannot be compensated, the command returns
`editor_apply_rollback_failed` and logs a path-free rollback failure. The UI
reports that active desktop output may differ from saved settings and asks the
user to retry after restoring host or storage access.

## Presets and recently applied items

Named presets are stored as versioned per-wallpaper snapshots in
`%APPDATA%\\WallpaperLite\\editor-presets\\<wallpaperId>.json`. Names are
trimmed, case-insensitively unique, limited to 64 characters, and each wallpaper
can have up to 32 presets. Preset snapshots pass the same settings validation
as ordinary editor settings and never include or modify source media. Loading a
preset replaces only the current in-memory draft and undo history; it does not
write active settings or contact the desktop host. **Apply** is still required
to update the wallpaper.

Recently applied wallpaper IDs are stored in
`%APPDATA%\\WallpaperLite\\recent-wallpapers.json`, newest first, capped at
100 entries. The Apply command stages the recent ordering under the same UI
transaction lock and restores the previous ordering if host application or
settings persistence fails. The UI displays only IDs still present in the
catalog. Opening or previewing an item does not change recency.
