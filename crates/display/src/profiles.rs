//! Validated, versioned display assignments independent of current topology.

use super::DisplayId;
use serde::{Deserialize, Deserializer, Serialize};
use std::{error::Error, fmt};

pub const DISPLAY_PROFILE_SCHEMA_VERSION: u32 = 1;
const MAX_DISPLAY_PROFILE_COUNT: usize = 256;

/// How an assigned wallpaper is presented by a display group.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayPresentationMode {
    #[default]
    Independent,
    Duplicate,
    Span,
}

/// Persistent assignment and optional frame-rate override for one stable display.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayProfile {
    display_id: DisplayId,
    wallpaper_id: u64,
    presentation_mode: DisplayPresentationMode,
    fps_override: Option<u16>,
}

impl<'de> Deserialize<'de> for DisplayProfile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct WireDisplayProfile {
            display_id: DisplayId,
            wallpaper_id: u64,
            presentation_mode: DisplayPresentationMode,
            #[serde(default)]
            fps_override: Option<u16>,
        }

        let value = WireDisplayProfile::deserialize(deserializer)?;
        Self::new(
            value.display_id,
            value.wallpaper_id,
            value.presentation_mode,
            value.fps_override,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl DisplayProfile {
    pub fn new(
        display_id: DisplayId,
        wallpaper_id: u64,
        presentation_mode: DisplayPresentationMode,
        fps_override: Option<u16>,
    ) -> Result<Self, DisplayProfileValidationError> {
        if wallpaper_id == 0 {
            return Err(DisplayProfileValidationError::InvalidWallpaperId);
        }
        if fps_override.is_some_and(|fps| !(1..=120).contains(&fps)) {
            return Err(DisplayProfileValidationError::InvalidFpsOverride);
        }
        Ok(Self {
            display_id,
            wallpaper_id,
            presentation_mode,
            fps_override,
        })
    }

    pub fn display_id(&self) -> &DisplayId {
        &self.display_id
    }

    pub const fn wallpaper_id(&self) -> u64 {
        self.wallpaper_id
    }

    pub const fn presentation_mode(&self) -> DisplayPresentationMode {
        self.presentation_mode
    }

    pub const fn fps_override(&self) -> Option<u16> {
        self.fps_override
    }
}

/// Complete profile snapshot; disconnected displays remain ordinary entries.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayProfileSet {
    schema_version: u32,
    profiles: Vec<DisplayProfile>,
}

impl<'de> Deserialize<'de> for DisplayProfileSet {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct WireDisplayProfileSet {
            schema_version: u32,
            profiles: Vec<DisplayProfile>,
        }

        let value = WireDisplayProfileSet::deserialize(deserializer)?;
        if value.schema_version != DISPLAY_PROFILE_SCHEMA_VERSION {
            return Err(serde::de::Error::custom(
                DisplayProfileValidationError::UnsupportedSchema,
            ));
        }
        Self::new(value.profiles).map_err(serde::de::Error::custom)
    }
}

impl DisplayProfileSet {
    pub fn new(mut profiles: Vec<DisplayProfile>) -> Result<Self, DisplayProfileValidationError> {
        if profiles.len() > MAX_DISPLAY_PROFILE_COUNT {
            return Err(DisplayProfileValidationError::TooManyProfiles);
        }
        profiles.sort_by(|left, right| left.display_id.as_str().cmp(right.display_id.as_str()));
        if profiles
            .windows(2)
            .any(|pair| pair[0].display_id == pair[1].display_id)
        {
            return Err(DisplayProfileValidationError::DuplicateDisplayId);
        }
        Ok(Self {
            schema_version: DISPLAY_PROFILE_SCHEMA_VERSION,
            profiles,
        })
    }

    pub fn profiles(&self) -> &[DisplayProfile] {
        &self.profiles
    }

    pub fn get(&self, display_id: &DisplayId) -> Option<&DisplayProfile> {
        self.profiles
            .binary_search_by(|profile| profile.display_id.as_str().cmp(display_id.as_str()))
            .ok()
            .map(|index| &self.profiles[index])
    }

    pub fn upsert(&mut self, profile: DisplayProfile) -> Result<(), DisplayProfileValidationError> {
        match self.profiles.binary_search_by(|existing| {
            existing
                .display_id
                .as_str()
                .cmp(profile.display_id.as_str())
        }) {
            Ok(index) => self.profiles[index] = profile,
            Err(index) => {
                if self.profiles.len() >= MAX_DISPLAY_PROFILE_COUNT {
                    return Err(DisplayProfileValidationError::TooManyProfiles);
                }
                self.profiles.insert(index, profile);
            }
        }
        Ok(())
    }
}

impl Default for DisplayProfileSet {
    fn default() -> Self {
        Self {
            schema_version: DISPLAY_PROFILE_SCHEMA_VERSION,
            profiles: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisplayProfileValidationError {
    DuplicateDisplayId,
    InvalidWallpaperId,
    InvalidFpsOverride,
    TooManyProfiles,
    UnsupportedSchema,
}

impl fmt::Display for DisplayProfileValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::DuplicateDisplayId => "display profiles contain a duplicate display ID",
            Self::InvalidWallpaperId => "display profile contains an invalid wallpaper ID",
            Self::InvalidFpsOverride => "display profile FPS override must be between 1 and 120",
            Self::TooManyProfiles => "display profile count exceeds the supported limit",
            Self::UnsupportedSchema => "display profile schema version is unsupported",
        };
        formatter.write_str(message)
    }
}

impl Error for DisplayProfileValidationError {}
