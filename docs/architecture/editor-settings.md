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
| `placementMode` | `cover`, `fit`, `stretch`, or `center` | `cover` |
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
opens or modifies the source media. This per-wallpaper store does not implement
global-settings backup/restore. Editor UI, edit history, Apply/Cancel, and live
host application remain separate work.
