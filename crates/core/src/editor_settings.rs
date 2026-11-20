//! Versioned, non-destructive settings stored separately from media and catalog entries.

use serde::{Deserialize, Serialize};
use std::{error::Error, fmt};

pub const EDITOR_SETTINGS_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlacementMode {
    #[default]
    Cover,
    Fit,
    Stretch,
    Center,
    /// User-defined physical bounds and transforms, initialized from Cover.
    Custom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PhysicalPosition {
    x: i32,
    y: i32,
}

impl PhysicalPosition {
    pub fn new(x: i32, y: i32) -> Result<Self, EditorSettingsValidationError> {
        if !(-32_768..=32_768).contains(&x) || !(-32_768..=32_768).contains(&y) {
            return Err(EditorSettingsValidationError::OutOfRange);
        }
        Ok(Self { x, y })
    }

    pub const fn x(self) -> i32 {
        self.x
    }

    pub const fn y(self) -> i32 {
        self.y
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PhysicalSize {
    width: u32,
    height: u32,
}

impl PhysicalSize {
    pub fn new(width: u32, height: u32) -> Result<Self, EditorSettingsValidationError> {
        if !(1..=32_768).contains(&width) || !(1..=32_768).contains(&height) {
            return Err(EditorSettingsValidationError::OutOfRange);
        }
        Ok(Self { width, height })
    }

    pub const fn width(self) -> u32 {
        self.width
    }

    pub const fn height(self) -> u32 {
        self.height
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scale2d {
    x: f32,
    y: f32,
}

impl Scale2d {
    pub fn new(x: f32, y: f32) -> Result<Self, EditorSettingsValidationError> {
        validate_finite(&[x, y])?;
        if !(0.01..=8.0).contains(&x) || !(0.01..=8.0).contains(&y) {
            return Err(EditorSettingsValidationError::OutOfRange);
        }
        Ok(Self { x, y })
    }

    pub const fn x(self) -> f32 {
        self.x
    }

    pub const fn y(self) -> f32 {
        self.y
    }
}

impl Default for Scale2d {
    fn default() -> Self {
        Self { x: 1.0, y: 1.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NormalizedPoint {
    x: f32,
    y: f32,
}

impl NormalizedPoint {
    pub fn new(x: f32, y: f32) -> Result<Self, EditorSettingsValidationError> {
        validate_finite(&[x, y])?;
        if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
            return Err(EditorSettingsValidationError::OutOfRange);
        }
        Ok(Self { x, y })
    }

    pub const fn x(self) -> f32 {
        self.x
    }

    pub const fn y(self) -> f32 {
        self.y
    }
}

impl Default for NormalizedPoint {
    fn default() -> Self {
        Self { x: 0.5, y: 0.5 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NormalizedCrop {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl NormalizedCrop {
    pub fn new(
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Result<Self, EditorSettingsValidationError> {
        validate_finite(&[x, y, width, height])?;
        if x < 0.0
            || y < 0.0
            || width <= 0.0
            || height <= 0.0
            || x + width > 1.0
            || y + height > 1.0
        {
            return Err(EditorSettingsValidationError::OutOfRange);
        }
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    pub const fn x(self) -> f32 {
        self.x
    }

    pub const fn y(self) -> f32 {
        self.y
    }

    pub const fn width(self) -> f32 {
        self.width
    }

    pub const fn height(self) -> f32 {
        self.height
    }
}

impl Default for NormalizedCrop {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenderSettings {
    placement_mode: PlacementMode,
    position_px: PhysicalPosition,
    size_px: Option<PhysicalSize>,
    scale: Scale2d,
    rotation_degrees: f32,
    anchor: NormalizedPoint,
    crop: NormalizedCrop,
}

impl RenderSettings {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        placement_mode: PlacementMode,
        position_px: PhysicalPosition,
        size_px: Option<PhysicalSize>,
        scale: Scale2d,
        rotation_degrees: f32,
        anchor: NormalizedPoint,
        crop: NormalizedCrop,
    ) -> Result<Self, EditorSettingsValidationError> {
        validate_finite(&[rotation_degrees])?;
        if !(-3_600.0..=3_600.0).contains(&rotation_degrees) {
            return Err(EditorSettingsValidationError::OutOfRange);
        }
        Ok(Self {
            placement_mode,
            position_px,
            size_px,
            scale,
            rotation_degrees,
            anchor,
            crop,
        })
    }

    pub const fn placement_mode(self) -> PlacementMode {
        self.placement_mode
    }
    pub const fn position_px(self) -> PhysicalPosition {
        self.position_px
    }
    pub const fn size_px(self) -> Option<PhysicalSize> {
        self.size_px
    }
    pub const fn scale(self) -> Scale2d {
        self.scale
    }
    pub const fn rotation_degrees(self) -> f32 {
        self.rotation_degrees
    }
    pub const fn anchor(self) -> NormalizedPoint {
        self.anchor
    }
    pub const fn crop(self) -> NormalizedCrop {
        self.crop
    }

    fn validate(self) -> Result<(), EditorSettingsValidationError> {
        PhysicalPosition::new(self.position_px.x, self.position_px.y)?;
        if let Some(size) = self.size_px {
            PhysicalSize::new(size.width, size.height)?;
        }
        Scale2d::new(self.scale.x, self.scale.y)?;
        NormalizedPoint::new(self.anchor.x, self.anchor.y)?;
        NormalizedCrop::new(self.crop.x, self.crop.y, self.crop.width, self.crop.height)?;
        validate_finite(&[self.rotation_degrees])?;
        if !(-3_600.0..=3_600.0).contains(&self.rotation_degrees) {
            return Err(EditorSettingsValidationError::OutOfRange);
        }
        Ok(())
    }
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            placement_mode: PlacementMode::Cover,
            position_px: PhysicalPosition::default(),
            size_px: None,
            scale: Scale2d::default(),
            rotation_degrees: 0.0,
            anchor: NormalizedPoint::default(),
            crop: NormalizedCrop::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditorColorSettings {
    brightness: f32,
    contrast: f32,
    saturation: f32,
    gamma: f32,
    opacity: f32,
    temperature: f32,
    red: f32,
    green: f32,
    blue: f32,
}

impl EditorColorSettings {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        brightness: f32,
        contrast: f32,
        saturation: f32,
        gamma: f32,
        opacity: f32,
        temperature: f32,
        red: f32,
        green: f32,
        blue: f32,
    ) -> Result<Self, EditorSettingsValidationError> {
        let value = Self {
            brightness,
            contrast,
            saturation,
            gamma,
            opacity,
            temperature,
            red,
            green,
            blue,
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(self) -> Result<(), EditorSettingsValidationError> {
        validate_finite(&[
            self.brightness,
            self.contrast,
            self.saturation,
            self.gamma,
            self.opacity,
            self.temperature,
            self.red,
            self.green,
            self.blue,
        ])?;
        if !(0.0..=4.0).contains(&self.brightness)
            || !(0.0..=4.0).contains(&self.contrast)
            || !(0.0..=4.0).contains(&self.saturation)
            || !(0.1..=4.0).contains(&self.gamma)
            || !(0.0..=1.0).contains(&self.opacity)
            || !(-1.0..=1.0).contains(&self.temperature)
            || !(0.0..=4.0).contains(&self.red)
            || !(0.0..=4.0).contains(&self.green)
            || !(0.0..=4.0).contains(&self.blue)
        {
            return Err(EditorSettingsValidationError::OutOfRange);
        }
        Ok(())
    }

    pub const fn brightness(self) -> f32 {
        self.brightness
    }
    pub const fn contrast(self) -> f32 {
        self.contrast
    }
    pub const fn saturation(self) -> f32 {
        self.saturation
    }
    pub const fn gamma(self) -> f32 {
        self.gamma
    }
    pub const fn opacity(self) -> f32 {
        self.opacity
    }
    pub const fn temperature(self) -> f32 {
        self.temperature
    }
    pub const fn rgb_balance(self) -> (f32, f32, f32) {
        (self.red, self.green, self.blue)
    }
}

impl Default for EditorColorSettings {
    fn default() -> Self {
        Self {
            brightness: 1.0,
            contrast: 1.0,
            saturation: 1.0,
            gamma: 1.0,
            opacity: 1.0,
            temperature: 0.0,
            red: 1.0,
            green: 1.0,
            blue: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WallpaperEditorSettings {
    schema_version: u32,
    wallpaper_id: u64,
    render: RenderSettings,
    color: EditorColorSettings,
}

impl WallpaperEditorSettings {
    pub fn new(
        wallpaper_id: u64,
        render: RenderSettings,
        color: EditorColorSettings,
    ) -> Result<Self, EditorSettingsValidationError> {
        let settings = Self {
            schema_version: EDITOR_SETTINGS_SCHEMA_VERSION,
            wallpaper_id,
            render,
            color,
        };
        settings.validate()?;
        Ok(settings)
    }

    pub fn default_for(wallpaper_id: u64) -> Result<Self, EditorSettingsValidationError> {
        Self::new(
            wallpaper_id,
            RenderSettings::default(),
            EditorColorSettings::default(),
        )
    }

    pub const fn wallpaper_id(&self) -> u64 {
        self.wallpaper_id
    }
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }
    pub const fn render(&self) -> RenderSettings {
        self.render
    }
    pub const fn color(&self) -> EditorColorSettings {
        self.color
    }

    pub fn with_render(
        mut self,
        render: RenderSettings,
    ) -> Result<Self, EditorSettingsValidationError> {
        self.render = render;
        self.validate()?;
        Ok(self)
    }

    pub fn with_color(
        mut self,
        color: EditorColorSettings,
    ) -> Result<Self, EditorSettingsValidationError> {
        self.color = color;
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<(), EditorSettingsValidationError> {
        if self.wallpaper_id == 0 {
            return Err(EditorSettingsValidationError::InvalidWallpaperId);
        }
        if self.schema_version != EDITOR_SETTINGS_SCHEMA_VERSION {
            return Err(EditorSettingsValidationError::UnsupportedSchema);
        }
        self.render.validate()?;
        self.color.validate()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorSettingsValidationError {
    InvalidWallpaperId,
    UnsupportedSchema,
    NonFiniteValue,
    OutOfRange,
}

impl fmt::Display for EditorSettingsValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidWallpaperId => "wallpaper identifier is invalid",
            Self::UnsupportedSchema => "editor settings schema is unsupported",
            Self::NonFiniteValue => "editor settings contain a non-finite value",
            Self::OutOfRange => "editor setting is outside its supported range",
        })
    }
}

impl Error for EditorSettingsValidationError {}

fn validate_finite(values: &[f32]) -> Result<(), EditorSettingsValidationError> {
    if values.iter().all(|value| value.is_finite()) {
        Ok(())
    } else {
        Err(EditorSettingsValidationError::NonFiniteValue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_renderer_contract_and_serialize_units_explicitly() {
        let settings = WallpaperEditorSettings::default_for(7).unwrap();
        assert_eq!(settings.schema_version(), 1);
        assert_eq!(settings.render().placement_mode(), PlacementMode::Cover);
        assert_eq!(settings.render().position_px(), PhysicalPosition::default());
        assert_eq!(settings.render().size_px(), None);
        assert_eq!(settings.render().scale(), Scale2d::default());
        assert_eq!(settings.render().rotation_degrees(), 0.0);
        assert_eq!(settings.render().anchor(), NormalizedPoint::default());
        assert_eq!(settings.render().crop(), NormalizedCrop::default());

        let json = serde_json::to_value(settings).unwrap();
        assert_eq!(json["render"]["positionPx"]["x"], 0);
        assert_eq!(json["render"]["crop"]["width"], 1.0);
        assert_eq!(json["color"]["temperature"], 0.0);
    }

    #[test]
    fn validates_finite_values_bounds_and_nonzero_crop() {
        assert_eq!(
            Scale2d::new(f32::NAN, 1.0),
            Err(EditorSettingsValidationError::NonFiniteValue)
        );
        assert_eq!(
            Scale2d::new(0.0, 1.0),
            Err(EditorSettingsValidationError::OutOfRange)
        );
        assert_eq!(
            NormalizedPoint::new(1.01, 0.5),
            Err(EditorSettingsValidationError::OutOfRange)
        );
        assert_eq!(
            NormalizedCrop::new(0.8, 0.0, 0.3, 1.0),
            Err(EditorSettingsValidationError::OutOfRange)
        );
        assert_eq!(
            NormalizedCrop::new(0.0, 0.0, 0.0, 1.0),
            Err(EditorSettingsValidationError::OutOfRange)
        );
        assert_eq!(
            PhysicalPosition::new(-32_769, 0),
            Err(EditorSettingsValidationError::OutOfRange)
        );
        assert_eq!(
            PhysicalSize::new(0, 720),
            Err(EditorSettingsValidationError::OutOfRange)
        );
        assert_eq!(
            EditorColorSettings::new(1.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0),
            Err(EditorSettingsValidationError::OutOfRange)
        );
        assert_eq!(
            WallpaperEditorSettings::default_for(0),
            Err(EditorSettingsValidationError::InvalidWallpaperId)
        );
    }

    #[test]
    fn serde_input_is_revalidated_and_unknown_placement_is_rejected() {
        let invalid = r#"{"schemaVersion":1,"wallpaperId":1,"render":{"placementMode":"unknown","positionPx":{"x":0,"y":0},"sizePx":null,"scale":{"x":1.0,"y":1.0},"rotationDegrees":0.0,"anchor":{"x":0.5,"y":0.5},"crop":{"x":0.0,"y":0.0,"width":1.0,"height":1.0}},"color":{"brightness":1.0,"contrast":1.0,"saturation":1.0,"gamma":1.0,"opacity":1.0,"temperature":0.0,"red":1.0,"green":1.0,"blue":1.0}}"#;
        assert!(serde_json::from_str::<WallpaperEditorSettings>(invalid).is_err());
    }
}
