//! CPU-generated editor previews using the same placement and color reference used by tests.

use wallpaper_core::editor_settings::{
    NormalizedCrop as StoredCrop, PlacementMode as StoredPlacementMode, WallpaperEditorSettings,
};

use crate::color::apply_rgba8_reference;
use crate::{
    ColorAdjustments, ImageTransform, NormalizedCrop, PlacementDimensions, PlacementMode,
    RendererError, calculate_transformed_image_placement,
};

pub const EDITOR_PREVIEW_MAX_EDGE: u32 = 1280;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedEditorPreview {
    width: u32,
    height: u32,
    rgba8: Vec<u8>,
}

impl RenderedEditorPreview {
    pub const fn width(&self) -> u32 {
        self.width
    }

    pub const fn height(&self) -> u32 {
        self.height
    }

    pub fn rgba8(&self) -> &[u8] {
        &self.rgba8
    }
}

/// Render a still or decoded video frame into a bounded preview using stored physical-pixel
/// settings. The output preserves the selected monitor aspect ratio and scales physical offsets
/// and explicit size by the same per-axis preview factor.
pub fn render_editor_preview(
    source_width: u32,
    source_height: u32,
    source_stride: u32,
    source_rgba8: &[u8],
    monitor_width: u32,
    monitor_height: u32,
    settings: &WallpaperEditorSettings,
) -> Result<RenderedEditorPreview, RendererError> {
    settings
        .validate()
        .map_err(|_| RendererError::InvalidEditorSettings)?;
    if source_width == 0
        || source_height == 0
        || monitor_width == 0
        || monitor_height == 0
        || source_width > 16_384
        || source_height > 16_384
        || monitor_width > 32_768
        || monitor_height > 32_768
    {
        return Err(RendererError::InvalidPreviewInput);
    }

    let row_bytes = source_width
        .checked_mul(4)
        .ok_or(RendererError::InvalidPreviewInput)?;
    if source_stride < row_bytes {
        return Err(RendererError::InvalidPreviewInput);
    }
    let required = (source_stride as usize)
        .checked_mul(source_height as usize)
        .ok_or(RendererError::InvalidPreviewInput)?;
    if source_rgba8.len() < required {
        return Err(RendererError::InvalidPreviewInput);
    }

    let factor =
        (EDITOR_PREVIEW_MAX_EDGE as f64 / f64::from(monitor_width.max(monitor_height))).min(1.0);
    let output_width = (f64::from(monitor_width) * factor).round().max(1.0) as u32;
    let output_height = (f64::from(monitor_height) * factor).round().max(1.0) as u32;
    let output = PlacementDimensions::new(output_width, output_height)?;
    let source = PlacementDimensions::new(source_width, source_height)?;

    let render = settings.render();
    let x_factor = output_width as f32 / monitor_width as f32;
    let y_factor = output_height as f32 / monitor_height as f32;
    let stored_crop: StoredCrop = render.crop();
    let crop = NormalizedCrop::new(
        stored_crop.x(),
        stored_crop.y(),
        stored_crop.width(),
        stored_crop.height(),
    )?;
    let size = render.size_px().map(|size| {
        (
            size.width() as f32 * x_factor,
            size.height() as f32 * y_factor,
        )
    });
    let transform = ImageTransform::new(
        (
            render.position_px().x() as f32 * x_factor,
            render.position_px().y() as f32 * y_factor,
        ),
        (render.scale().x(), render.scale().y()),
        render.rotation_degrees(),
        (render.anchor().x(), render.anchor().y()),
        crop,
    )?
    .with_size_px(size)?;
    let mode = match render.placement_mode() {
        StoredPlacementMode::Cover => PlacementMode::Cover,
        StoredPlacementMode::Fit => PlacementMode::Fit,
        StoredPlacementMode::Stretch => PlacementMode::Stretch,
        StoredPlacementMode::Center => PlacementMode::Center,
        StoredPlacementMode::Custom => PlacementMode::Cover,
    };
    let placement = calculate_transformed_image_placement(source, output, mode, transform)?;

    let adjustments = ColorAdjustments::from_editor_settings(settings.color())?;

    let pixel_count = (output_width as usize)
        .checked_mul(output_height as usize)
        .ok_or(RendererError::InvalidPreviewInput)?;
    let mut rgba8 = Vec::new();
    rgba8
        .try_reserve_exact(pixel_count.saturating_mul(4))
        .map_err(|_| RendererError::PreviewAllocationFailed)?;
    rgba8.resize(pixel_count * 4, 0);
    for alpha in rgba8.iter_mut().skip(3).step_by(4) {
        *alpha = 255;
    }

    let corners = placement.destination_corners();
    let top_left = (f64::from(corners[0].x()), f64::from(corners[0].y()));
    let edge_x = (
        f64::from(corners[1].x()) - top_left.0,
        f64::from(corners[1].y()) - top_left.1,
    );
    let edge_y = (
        f64::from(corners[3].x()) - top_left.0,
        f64::from(corners[3].y()) - top_left.1,
    );
    let determinant = edge_x.0 * edge_y.1 - edge_x.1 * edge_y.0;
    if !determinant.is_finite() || determinant.abs() <= f64::EPSILON {
        return Err(RendererError::InvalidPreviewInput);
    }
    let uv = placement.source_uv();
    let src_stride = source_stride as usize;
    for y in 0..output_height {
        for x in 0..output_width {
            let point_x = (f64::from(x) + 0.5) / f64::from(output_width);
            let point_y = (f64::from(y) + 0.5) / f64::from(output_height);
            let dx = point_x - top_left.0;
            let dy = point_y - top_left.1;
            let u = (dx * edge_y.1 - dy * edge_y.0) / determinant;
            let v = (edge_x.0 * dy - edge_x.1 * dx) / determinant;
            if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
                continue;
            }
            let source_u =
                f64::from(uv.left()) + (f64::from(uv.right()) - f64::from(uv.left())) * u;
            let source_v = f64::from(uv.top()) + (f64::from(uv.bottom()) - f64::from(uv.top())) * v;
            let pixel = sample_bilinear(
                source_rgba8,
                src_stride,
                source_width,
                source_height,
                source_u,
                source_v,
            );
            let adjusted = apply_rgba8_reference(pixel, adjustments);
            let offset = ((y as usize * output_width as usize) + x as usize) * 4;
            rgba8[offset..offset + 4].copy_from_slice(&adjusted);
            rgba8[offset + 3] = 255;
        }
    }

    Ok(RenderedEditorPreview {
        width: output_width,
        height: output_height,
        rgba8,
    })
}

fn sample_bilinear(
    pixels: &[u8],
    stride: usize,
    width: u32,
    height: u32,
    u: f64,
    v: f64,
) -> [u8; 4] {
    let x = u * f64::from(width) - 0.5;
    let y = v * f64::from(height) - 0.5;
    let x0 = x.floor() as i64;
    let y0 = y.floor() as i64;
    let fx = (x - x.floor()) as f32;
    let fy = (y - y.floor()) as f32;
    let sample = |sx: i64, sy: i64, channel: usize| -> f32 {
        let sx = sx.clamp(0, i64::from(width) - 1) as usize;
        let sy = sy.clamp(0, i64::from(height) - 1) as usize;
        f32::from(pixels[sy * stride + sx * 4 + channel])
    };
    let mut result = [0; 4];
    for (channel, value) in result.iter_mut().enumerate() {
        let top = sample(x0, y0, channel) * (1.0 - fx) + sample(x0 + 1, y0, channel) * fx;
        let bottom =
            sample(x0, y0 + 1, channel) * (1.0 - fx) + sample(x0 + 1, y0 + 1, channel) * fx;
        *value = (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::render_editor_preview;
    use wallpaper_core::editor_settings::WallpaperEditorSettings;

    #[test]
    fn preview_preserves_monitor_aspect_and_caps_output_edge() {
        let source = [255, 0, 0, 255].repeat(4);
        let preview = render_editor_preview(
            2,
            2,
            8,
            &source,
            2560,
            1440,
            &WallpaperEditorSettings::default_for(1).unwrap(),
        )
        .unwrap();
        assert_eq!((preview.width(), preview.height()), (1280, 720));
        assert_eq!(preview.rgba8().len(), 1280 * 720 * 4);
    }

    #[test]
    fn invalid_source_buffer_is_rejected() {
        assert!(
            render_editor_preview(
                2,
                2,
                8,
                &[0; 15],
                16,
                9,
                &WallpaperEditorSettings::default_for(1).unwrap()
            )
            .is_err()
        );
    }
}
