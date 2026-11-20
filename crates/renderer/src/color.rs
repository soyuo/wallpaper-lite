use crate::RendererError;
use wallpaper_core::editor_settings::EditorColorSettings;

/// Runtime color controls for one still-image presenter.
///
/// RGB adjustments operate in linear sRGB. `temperature` is normalized warmth,
/// with positive values warmer and negative values cooler. `opacity` scales
/// adjusted RGB toward black because the current HWND swap chain ignores alpha;
/// it does not make the desktop child transparent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorAdjustments {
    brightness: f32,
    contrast: f32,
    saturation: f32,
    gamma: f32,
    opacity: f32,
    temperature: f32,
    rgb_balance: (f32, f32, f32),
}

impl ColorAdjustments {
    /// Create bounded adjustments. Brightness, contrast, saturation and each
    /// RGB balance are in `0.0..=4.0`, gamma is `0.1..=4.0`, opacity is
    /// `0.0..=1.0`, and normalized temperature is `-1.0..=1.0`.
    pub fn new(
        brightness: f32,
        contrast: f32,
        saturation: f32,
        gamma: f32,
        opacity: f32,
        temperature: f32,
        rgb_balance: (f32, f32, f32),
    ) -> Result<Self, RendererError> {
        let values = [
            brightness,
            contrast,
            saturation,
            gamma,
            opacity,
            temperature,
            rgb_balance.0,
            rgb_balance.1,
            rgb_balance.2,
        ];
        if values.iter().any(|value| !value.is_finite())
            || !(0.0..=4.0).contains(&brightness)
            || !(0.0..=4.0).contains(&contrast)
            || !(0.0..=4.0).contains(&saturation)
            || !(0.1..=4.0).contains(&gamma)
            || !(0.0..=1.0).contains(&opacity)
            || !(-1.0..=1.0).contains(&temperature)
            || rgb_balance.0 < 0.0
            || rgb_balance.0 > 4.0
            || rgb_balance.1 < 0.0
            || rgb_balance.1 > 4.0
            || rgb_balance.2 < 0.0
            || rgb_balance.2 > 4.0
        {
            return Err(RendererError::InvalidColorAdjustments);
        }
        Ok(Self {
            brightness,
            contrast,
            saturation,
            gamma,
            opacity,
            temperature,
            rgb_balance,
        })
    }

    /// Convert the validated shared editor color model into GPU parameters.
    ///
    /// Keeping this conversion here makes the CPU preview and D3D presenter use
    /// the same values and validation rules.
    pub fn from_editor_settings(settings: EditorColorSettings) -> Result<Self, RendererError> {
        Self::new(
            settings.brightness(),
            settings.contrast(),
            settings.saturation(),
            settings.gamma(),
            settings.opacity(),
            settings.temperature(),
            settings.rgb_balance(),
        )
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
        self.rgb_balance
    }
}

impl Default for ColorAdjustments {
    fn default() -> Self {
        Self {
            brightness: 1.0,
            contrast: 1.0,
            saturation: 1.0,
            gamma: 1.0,
            opacity: 1.0,
            temperature: 0.0,
            rgb_balance: (1.0, 1.0, 1.0),
        }
    }
}

/// CPU reference for the D3D11 pixel shader, useful for deterministic tests.
pub(crate) fn apply_rgba8_reference(pixel: [u8; 4], adjustments: ColorAdjustments) -> [u8; 4] {
    if adjustments == ColorAdjustments::default() {
        return pixel;
    }

    let mut rgb = [
        srgb_to_linear(f32::from(pixel[0]) / 255.0),
        srgb_to_linear(f32::from(pixel[1]) / 255.0),
        srgb_to_linear(f32::from(pixel[2]) / 255.0),
    ];
    let warm = adjustments.temperature * 0.1;
    rgb[0] *= adjustments.rgb_balance.0 * (1.0 + warm);
    rgb[1] *= adjustments.rgb_balance.1;
    rgb[2] *= adjustments.rgb_balance.2 * (1.0 - warm);

    for channel in &mut rgb {
        *channel = ((*channel * adjustments.brightness - 0.18) * adjustments.contrast + 0.18)
            .clamp(0.0, 1.0);
    }
    let luminance = rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
    for channel in &mut rgb {
        *channel = (luminance + (*channel - luminance) * adjustments.saturation).clamp(0.0, 1.0);
        *channel = channel.powf(1.0 / adjustments.gamma);
        *channel *= adjustments.opacity;
    }

    [
        linear_to_srgb8(rgb[0]),
        linear_to_srgb8(rgb[1]),
        linear_to_srgb8(rgb[2]),
        255,
    ]
}

fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb8(value: f32) -> u8 {
    let value = value.clamp(0.0, 1.0);
    let encoded = if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::{ColorAdjustments, apply_rgba8_reference};
    use crate::RendererError;

    #[test]
    fn shared_editor_color_settings_map_to_the_gpu_parameters() {
        let stored = wallpaper_core::editor_settings::EditorColorSettings::new(
            1.25, 1.1, 0.8, 1.2, 0.9, 0.25, 1.0, 0.95, 1.05,
        )
        .unwrap();
        let expected =
            ColorAdjustments::new(1.25, 1.1, 0.8, 1.2, 0.9, 0.25, (1.0, 0.95, 1.05)).unwrap();

        assert_eq!(
            ColorAdjustments::from_editor_settings(stored).unwrap(),
            expected
        );
    }

    fn adjustments(
        brightness: f32,
        contrast: f32,
        saturation: f32,
        gamma: f32,
        opacity: f32,
        temperature: f32,
        rgb_balance: (f32, f32, f32),
    ) -> ColorAdjustments {
        ColorAdjustments::new(
            brightness,
            contrast,
            saturation,
            gamma,
            opacity,
            temperature,
            rgb_balance,
        )
        .expect("values are valid")
    }

    #[test]
    fn neutral_controls_preserve_rgba8_exactly() {
        let pixel = [64, 128, 192, 77];
        assert_eq!(
            apply_rgba8_reference(pixel, ColorAdjustments::default()),
            pixel
        );
    }

    #[test]
    fn opacity_fades_to_black_without_using_ignored_source_alpha() {
        let low_alpha = apply_rgba8_reference(
            [128, 96, 64, 32],
            adjustments(1.0, 1.0, 1.0, 1.0, 0.5, 0.0, (1.0, 1.0, 1.0)),
        );
        let high_alpha = apply_rgba8_reference(
            [128, 96, 64, 240],
            adjustments(1.0, 1.0, 1.0, 1.0, 0.5, 0.0, (1.0, 1.0, 1.0)),
        );

        assert_eq!(low_alpha, high_alpha);
        assert_eq!(low_alpha[3], 255);
        assert!(low_alpha[0] < 128 && low_alpha[1] < 96 && low_alpha[2] < 64);
    }

    #[test]
    fn each_control_changes_the_reference_pixel_as_documented() {
        let pixel = [64, 128, 192, 255];
        let neutral = ColorAdjustments::default();
        let adjusted = |value| apply_rgba8_reference(pixel, value);

        assert!(
            adjusted(adjustments(2.0, 1.0, 1.0, 1.0, 1.0, 0.0, (1.0, 1.0, 1.0)))
                .iter()
                .zip(pixel)
                .any(|(after, before)| *after > before)
        );
        assert_ne!(
            adjusted(adjustments(1.0, 2.0, 1.0, 1.0, 1.0, 0.0, (1.0, 1.0, 1.0))),
            pixel
        );
        let grayscale = adjusted(adjustments(1.0, 1.0, 0.0, 1.0, 1.0, 0.0, (1.0, 1.0, 1.0)));
        assert_eq!(grayscale[0], grayscale[1]);
        assert_eq!(grayscale[1], grayscale[2]);
        assert_ne!(
            adjusted(adjustments(1.0, 1.0, 1.0, 2.0, 1.0, 0.0, (1.0, 1.0, 1.0))),
            pixel
        );
        assert_eq!(
            adjusted(adjustments(1.0, 1.0, 1.0, 1.0, 0.0, 0.0, (1.0, 1.0, 1.0))),
            [0, 0, 0, 255]
        );
        let warm = adjusted(adjustments(1.0, 1.0, 1.0, 1.0, 1.0, 1.0, (1.0, 1.0, 1.0)));
        assert!(warm[0] > pixel[0] && warm[2] < pixel[2]);
        let balanced = adjusted(adjustments(1.0, 1.0, 1.0, 1.0, 1.0, 0.0, (2.0, 1.0, 1.0)));
        assert!(balanced[0] > pixel[0]);
        assert_eq!(apply_rgba8_reference(pixel, neutral), pixel);
    }

    #[test]
    fn invalid_ranges_and_non_finite_values_return_typed_errors() {
        for values in [
            (-0.1, 1.0, 1.0, 1.0, 1.0, 0.0, (1.0, 1.0, 1.0)),
            (f32::NAN, 1.0, 1.0, 1.0, 1.0, 0.0, (1.0, 1.0, 1.0)),
            (1.0, f32::INFINITY, 1.0, 1.0, 1.0, 0.0, (1.0, 1.0, 1.0)),
            (1.0, 1.0, -0.1, 1.0, 1.0, 0.0, (1.0, 1.0, 1.0)),
            (1.0, 1.0, 1.0, 0.0, 1.0, 0.0, (1.0, 1.0, 1.0)),
            (1.0, 1.0, 1.0, 1.0, 1.1, 0.0, (1.0, 1.0, 1.0)),
            (1.0, 1.0, 1.0, 1.0, 1.0, -1.1, (1.0, 1.0, 1.0)),
            (1.0, 1.0, 1.0, 1.0, 1.0, 0.0, (1.0, 1.0, 4.1)),
        ] {
            assert!(matches!(
                ColorAdjustments::new(
                    values.0, values.1, values.2, values.3, values.4, values.5, values.6
                ),
                Err(RendererError::InvalidColorAdjustments)
            ));
        }
    }
}
