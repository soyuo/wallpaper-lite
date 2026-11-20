# Per-wallpaper playback settings

`wallpaper_core::playback_settings::PlaybackSettings` is the versioned,
validated record for playback preferences that belong to one catalog item. It
is stored separately from both the catalog and the source media. Runtime loop,
audio, and timeline behavior are applied by later playback features.

## Defaults and bounds

| Property | Contract | Default |
| --- | --- | --- |
| `loopMode` | `disabled`, `full`, or `segment` | `disabled` |
| `segment` | Optional millisecond range; required for segment mode; minimum 500 ms | absent |
| `speedPercent` | One of 25, 50, 75, 100, 125, 150, or 200 | 100 |
| `fpsLimit` | Integer from 1 through 120 | 60 |
| `volumePercent` | Integer from 0 through 100 | 100 |
| `muted` | Boolean | `false` |

When video duration is known, `validate_for_duration` also requires
`0 <= startMs < endMs <= durationMs`. The model validates serialized data
again after deserialization, so untrusted IPC or file values cannot bypass the
same bounds.

## Persistence

`PlaybackSettingsStore::for_current_user()` stores records at
`%APPDATA%\WallpaperLite\playback-settings\<wallpaperId>.json`. Missing files
return the defaults for that ID. Loads reject files larger than 64 KiB,
corrupt JSON, unsupported schemas, mismatched IDs, and invalid values. Saves
validate before using the catalog's same-directory temporary-file and atomic
replacement path. Failures return safe typed errors with path-free diagnostic
stages.

Playback settings contain no media bytes or paths. Loading and saving them
never modifies the catalog record or original image/video. This issue provides
the settings contract only; host application, looping, timeline UI, audio
output, and loop-boundary optimization are tracked separately.
