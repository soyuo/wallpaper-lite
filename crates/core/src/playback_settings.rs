//! Validated, per-wallpaper video playback preferences.

use serde::{Deserialize, Deserializer, Serialize};
use std::{error::Error, fmt};

pub const PLAYBACK_SETTINGS_SCHEMA_VERSION: u32 = 1;
pub const MIN_SEGMENT_DURATION_MS: u64 = 500;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopMode {
    #[default]
    Disabled,
    Full,
    Segment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentRange {
    start_ms: u64,
    end_ms: u64,
}

impl<'de> Deserialize<'de> for SegmentRange {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct WireSegmentRange {
            start_ms: u64,
            end_ms: u64,
        }

        let value = WireSegmentRange::deserialize(deserializer)?;
        Self::new(value.start_ms, value.end_ms).map_err(serde::de::Error::custom)
    }
}

impl SegmentRange {
    pub fn new(start_ms: u64, end_ms: u64) -> Result<Self, PlaybackSettingsValidationError> {
        if end_ms <= start_ms || end_ms - start_ms < MIN_SEGMENT_DURATION_MS {
            return Err(PlaybackSettingsValidationError::InvalidSegmentRange);
        }
        Ok(Self { start_ms, end_ms })
    }

    pub const fn start_ms(self) -> u64 {
        self.start_ms
    }

    pub const fn end_ms(self) -> u64 {
        self.end_ms
    }

    pub fn validate_for_duration(
        self,
        duration_ms: u64,
    ) -> Result<(), PlaybackSettingsValidationError> {
        Self::new(self.start_ms, self.end_ms)?;
        if duration_ms == 0 || self.end_ms > duration_ms {
            return Err(PlaybackSettingsValidationError::SegmentExceedsDuration);
        }
        Ok(())
    }
}

/// All persisted values for one wallpaper. Runtime application is performed by the host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackSettings {
    schema_version: u32,
    wallpaper_id: u64,
    loop_mode: LoopMode,
    segment: Option<SegmentRange>,
    speed_percent: u16,
    fps_limit: u16,
    volume_percent: u8,
    muted: bool,
}

impl<'de> Deserialize<'de> for PlaybackSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct WirePlaybackSettings {
            schema_version: u32,
            wallpaper_id: u64,
            loop_mode: LoopMode,
            segment: Option<SegmentRange>,
            speed_percent: u16,
            fps_limit: u16,
            volume_percent: u8,
            muted: bool,
        }

        let value = WirePlaybackSettings::deserialize(deserializer)?;
        let settings = Self {
            schema_version: value.schema_version,
            wallpaper_id: value.wallpaper_id,
            loop_mode: value.loop_mode,
            segment: value.segment,
            speed_percent: value.speed_percent,
            fps_limit: value.fps_limit,
            volume_percent: value.volume_percent,
            muted: value.muted,
        };
        settings.validate().map_err(serde::de::Error::custom)?;
        Ok(settings)
    }
}

impl PlaybackSettings {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        wallpaper_id: u64,
        loop_mode: LoopMode,
        segment: Option<SegmentRange>,
        speed_percent: u16,
        fps_limit: u16,
        volume_percent: u8,
        muted: bool,
    ) -> Result<Self, PlaybackSettingsValidationError> {
        let settings = Self {
            schema_version: PLAYBACK_SETTINGS_SCHEMA_VERSION,
            wallpaper_id,
            loop_mode,
            segment,
            speed_percent,
            fps_limit,
            volume_percent,
            muted,
        };
        settings.validate()?;
        Ok(settings)
    }

    pub fn default_for(wallpaper_id: u64) -> Result<Self, PlaybackSettingsValidationError> {
        Self::new(wallpaper_id, LoopMode::Full, None, 100, 60, 100, true)
    }

    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub const fn wallpaper_id(&self) -> u64 {
        self.wallpaper_id
    }

    pub const fn loop_mode(&self) -> LoopMode {
        self.loop_mode
    }

    pub const fn segment(&self) -> Option<SegmentRange> {
        self.segment
    }

    pub const fn speed_percent(&self) -> u16 {
        self.speed_percent
    }

    pub const fn fps_limit(&self) -> u16 {
        self.fps_limit
    }

    pub const fn volume_percent(&self) -> u8 {
        self.volume_percent
    }

    pub const fn muted(&self) -> bool {
        self.muted
    }

    pub fn with_loop(
        mut self,
        loop_mode: LoopMode,
        segment: Option<SegmentRange>,
    ) -> Result<Self, PlaybackSettingsValidationError> {
        self.loop_mode = loop_mode;
        self.segment = segment;
        self.validate()?;
        Ok(self)
    }

    pub fn with_speed_percent(
        mut self,
        speed_percent: u16,
    ) -> Result<Self, PlaybackSettingsValidationError> {
        self.speed_percent = speed_percent;
        self.validate()?;
        Ok(self)
    }

    pub fn with_fps_limit(
        mut self,
        fps_limit: u16,
    ) -> Result<Self, PlaybackSettingsValidationError> {
        self.fps_limit = fps_limit;
        self.validate()?;
        Ok(self)
    }

    pub fn with_audio(
        mut self,
        volume_percent: u8,
        muted: bool,
    ) -> Result<Self, PlaybackSettingsValidationError> {
        self.volume_percent = volume_percent;
        self.muted = muted;
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<(), PlaybackSettingsValidationError> {
        if self.wallpaper_id == 0 {
            return Err(PlaybackSettingsValidationError::InvalidWallpaperId);
        }
        if self.schema_version != PLAYBACK_SETTINGS_SCHEMA_VERSION {
            return Err(PlaybackSettingsValidationError::UnsupportedSchema);
        }
        if !(1..=120).contains(&self.fps_limit) {
            return Err(PlaybackSettingsValidationError::InvalidFpsLimit);
        }
        if self.volume_percent > 100 {
            return Err(PlaybackSettingsValidationError::InvalidVolume);
        }
        if !matches!(self.speed_percent, 25 | 50 | 75 | 100 | 125 | 150 | 200) {
            return Err(PlaybackSettingsValidationError::UnsupportedSpeed);
        }
        match (self.loop_mode, self.segment) {
            (LoopMode::Segment, Some(segment)) => {
                SegmentRange::new(segment.start_ms, segment.end_ms)?;
            }
            (LoopMode::Segment, None) => {
                return Err(PlaybackSettingsValidationError::SegmentRequired);
            }
            (_, Some(_)) => {
                return Err(PlaybackSettingsValidationError::SegmentNotAllowed);
            }
            (_, None) => {}
        }
        Ok(())
    }

    pub fn validate_for_duration(
        &self,
        duration_ms: u64,
    ) -> Result<(), PlaybackSettingsValidationError> {
        self.validate()?;
        if let Some(segment) = self.segment {
            segment.validate_for_duration(duration_ms)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackSettingsValidationError {
    InvalidWallpaperId,
    UnsupportedSchema,
    InvalidFpsLimit,
    InvalidVolume,
    UnsupportedSpeed,
    SegmentRequired,
    SegmentNotAllowed,
    InvalidSegmentRange,
    SegmentExceedsDuration,
}

impl fmt::Display for PlaybackSettingsValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidWallpaperId => "wallpaper ID must be positive",
            Self::UnsupportedSchema => "playback settings schema is unsupported",
            Self::InvalidFpsLimit => "FPS limit must be between 1 and 120",
            Self::InvalidVolume => "volume must be between 0 and 100 percent",
            Self::UnsupportedSpeed => "playback speed is unsupported",
            Self::SegmentRequired => "segment loop requires a segment range",
            Self::SegmentNotAllowed => "segment range is only valid for segment loop mode",
            Self::InvalidSegmentRange => "segment range must be at least 500 ms",
            Self::SegmentExceedsDuration => "segment range exceeds video duration",
        })
    }
}

impl Error for PlaybackSettingsValidationError {}

#[cfg(test)]
mod tests {
    use super::{
        LoopMode, MIN_SEGMENT_DURATION_MS, PlaybackSettings, PlaybackSettingsValidationError,
        SegmentRange,
    };

    #[test]
    fn defaults_are_safe_and_validated() {
        let settings = PlaybackSettings::default_for(7).unwrap();
        assert_eq!(settings.wallpaper_id(), 7);
        assert_eq!(settings.loop_mode(), LoopMode::Full);
        assert_eq!(settings.speed_percent(), 100);
        assert_eq!(settings.fps_limit(), 60);
        assert_eq!(settings.volume_percent(), 100);
        assert!(settings.muted());
        settings.validate().unwrap();
    }

    #[test]
    fn validates_supported_ranges_and_segment_duration() {
        let segment = SegmentRange::new(1_000, 1_000 + MIN_SEGMENT_DURATION_MS).unwrap();
        let settings = PlaybackSettings::default_for(7)
            .unwrap()
            .with_loop(LoopMode::Segment, Some(segment))
            .unwrap()
            .with_speed_percent(200)
            .unwrap()
            .with_fps_limit(120)
            .unwrap()
            .with_audio(0, true)
            .unwrap();
        settings.validate_for_duration(2_000).unwrap();
        assert_eq!(
            settings.validate_for_duration(1_499),
            Err(PlaybackSettingsValidationError::SegmentExceedsDuration)
        );
        assert_eq!(
            SegmentRange::new(1_000, 1_499),
            Err(PlaybackSettingsValidationError::InvalidSegmentRange)
        );
    }

    #[test]
    fn rejects_invalid_loop_speed_and_fps_values() {
        let defaults = PlaybackSettings::default_for(1).unwrap();
        assert_eq!(
            defaults.clone().with_loop(LoopMode::Segment, None),
            Err(PlaybackSettingsValidationError::SegmentRequired)
        );
        assert_eq!(
            defaults
                .clone()
                .with_loop(LoopMode::Disabled, Some(SegmentRange::new(0, 500).unwrap())),
            Err(PlaybackSettingsValidationError::SegmentNotAllowed)
        );
        assert_eq!(
            defaults.clone().with_speed_percent(110),
            Err(PlaybackSettingsValidationError::UnsupportedSpeed)
        );
        assert_eq!(
            defaults.with_fps_limit(0),
            Err(PlaybackSettingsValidationError::InvalidFpsLimit)
        );
        assert_eq!(
            PlaybackSettings::default_for(0),
            Err(PlaybackSettingsValidationError::InvalidWallpaperId)
        );
    }

    #[test]
    fn serde_values_are_revalidated_after_deserialization() {
        let defaults = PlaybackSettings::default_for(1).unwrap();
        let mut value = serde_json::to_value(defaults).unwrap();
        value["speedPercent"] = serde_json::json!(333);
        assert!(serde_json::from_value::<PlaybackSettings>(value).is_err());

        let mut value = serde_json::to_value(PlaybackSettings::default_for(1).unwrap()).unwrap();
        value["volumePercent"] = serde_json::json!(101);
        assert!(serde_json::from_value::<PlaybackSettings>(value).is_err());
    }
}
