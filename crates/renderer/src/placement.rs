use crate::RendererError;

/// How a source image is mapped into the physical client area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlacementMode {
    /// Fill the output while preserving aspect ratio, cropping the source as needed.
    #[default]
    Cover,
    /// Show the whole image while preserving aspect ratio, leaving letterbox space as needed.
    Fit,
    /// Fill the output rectangle and allow aspect-ratio distortion.
    Stretch,
    /// Preserve one source pixel per output pixel and center the image.
    Center,
}

/// Nonzero physical pixel dimensions used at the renderer boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlacementDimensions {
    width: u32,
    height: u32,
}

impl PlacementDimensions {
    pub fn new(width: u32, height: u32) -> Result<Self, RendererError> {
        if width == 0 || height == 0 {
            return Err(RendererError::InvalidPlacementDimensions);
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

/// A source crop expressed as `(x, y, width, height)` in normalized top-left-origin UVs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NormalizedCrop {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl NormalizedCrop {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Result<Self, RendererError> {
        if ![x, y, width, height].into_iter().all(f32::is_finite)
            || x < 0.0
            || y < 0.0
            || width <= 0.0
            || height <= 0.0
            || x + width > 1.0
            || y + height > 1.0
        {
            return Err(RendererError::InvalidCropBounds);
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

/// Runtime-only image transform. Position is a physical-pixel offset from the placement center;
/// scale is relative to the placement rectangle; rotation is in degrees; anchor is normalized
/// within that rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageTransform {
    position_x_px: f32,
    position_y_px: f32,
    scale_x: f32,
    scale_y: f32,
    rotation_degrees: f32,
    anchor_x: f32,
    anchor_y: f32,
    crop: NormalizedCrop,
}

impl ImageTransform {
    /// Validate a transform in physical client-pixel and normalized image coordinates.
    ///
    /// `position_px` is a physical-pixel translation, `scale` is relative to the placement
    /// rectangle, `rotation_degrees` is clockwise, `anchor` is normalized within that rectangle,
    /// and `crop` is normalized against the original source image.
    pub fn new(
        position_px: (f32, f32),
        scale: (f32, f32),
        rotation_degrees: f32,
        anchor: (f32, f32),
        crop: NormalizedCrop,
    ) -> Result<Self, RendererError> {
        if ![
            position_px.0,
            position_px.1,
            scale.0,
            scale.1,
            rotation_degrees,
            anchor.0,
            anchor.1,
        ]
        .into_iter()
        .all(f32::is_finite)
            || scale.0 <= 0.0
            || scale.1 <= 0.0
            || !(0.0..=1.0).contains(&anchor.0)
            || !(0.0..=1.0).contains(&anchor.1)
        {
            return Err(RendererError::InvalidTransformParameters);
        }
        Ok(Self {
            position_x_px: position_px.0,
            position_y_px: position_px.1,
            scale_x: scale.0,
            scale_y: scale.1,
            rotation_degrees,
            anchor_x: anchor.0,
            anchor_y: anchor.1,
            crop,
        })
    }

    pub const fn position_px(self) -> (f32, f32) {
        (self.position_x_px, self.position_y_px)
    }

    pub const fn scale(self) -> (f32, f32) {
        (self.scale_x, self.scale_y)
    }

    pub const fn rotation_degrees(self) -> f32 {
        self.rotation_degrees
    }

    pub const fn anchor(self) -> (f32, f32) {
        (self.anchor_x, self.anchor_y)
    }

    pub const fn crop(self) -> NormalizedCrop {
        self.crop
    }
}

impl Default for ImageTransform {
    fn default() -> Self {
        Self {
            position_x_px: 0.0,
            position_y_px: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            rotation_degrees: 0.0,
            anchor_x: 0.5,
            anchor_y: 0.5,
            crop: NormalizedCrop::default(),
        }
    }
}

/// Destination coordinates use output pixels normalized to [0, 1]; source coordinates are
/// normalized texture UVs. For Center, the destination rectangle may extend beyond [0, 1] and is
/// clipped by the output surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacementRect {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

impl PlacementRect {
    pub const fn left(self) -> f32 {
        self.left
    }

    pub const fn top(self) -> f32 {
        self.top
    }

    pub const fn right(self) -> f32 {
        self.right
    }

    pub const fn bottom(self) -> f32 {
        self.bottom
    }
}

/// A point in normalized physical output coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacementPoint {
    x: f32,
    y: f32,
}

impl PlacementPoint {
    pub const fn x(self) -> f32 {
        self.x
    }

    pub const fn y(self) -> f32 {
        self.y
    }
}

/// GPU-ready source UV rectangle and destination quad in normalized physical output coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImagePlacement {
    destination: PlacementRect,
    destination_corners: [PlacementPoint; 4],
    source_uv: PlacementRect,
}

impl ImagePlacement {
    fn new(destination: PlacementRect, source_uv: PlacementRect) -> Self {
        Self {
            destination,
            destination_corners: [
                PlacementPoint {
                    x: destination.left,
                    y: destination.top,
                },
                PlacementPoint {
                    x: destination.right,
                    y: destination.top,
                },
                PlacementPoint {
                    x: destination.right,
                    y: destination.bottom,
                },
                PlacementPoint {
                    x: destination.left,
                    y: destination.bottom,
                },
            ],
            source_uv,
        }
    }

    /// Axis-aligned bounds around the destination quad; transformed corners can extend beyond it.
    pub const fn destination(self) -> PlacementRect {
        self.destination
    }

    pub const fn source_uv(self) -> PlacementRect {
        self.source_uv
    }

    /// Destination corners ordered top-left, top-right, bottom-right, bottom-left.
    pub const fn destination_corners(self) -> [PlacementPoint; 4] {
        self.destination_corners
    }
}

/// Calculate placement geometry from validated physical source and output pixel dimensions.
pub fn calculate_image_placement(
    source: PlacementDimensions,
    output: PlacementDimensions,
    mode: PlacementMode,
) -> ImagePlacement {
    calculate_for_dimensions(
        f64::from(source.width),
        f64::from(source.height),
        output,
        mode,
    )
}

/// Calculate placement and apply runtime transform and source crop parameters.
pub fn calculate_transformed_image_placement(
    source: PlacementDimensions,
    output: PlacementDimensions,
    mode: PlacementMode,
    transform: ImageTransform,
) -> Result<ImagePlacement, RendererError> {
    let crop = transform.crop;
    let effective_source_width = f64::from(source.width) * f64::from(crop.width);
    let effective_source_height = f64::from(source.height) * f64::from(crop.height);
    let mut placement = calculate_for_dimensions(
        effective_source_width,
        effective_source_height,
        output,
        mode,
    );
    let placement_crop = placement.source_uv;
    placement.source_uv = PlacementRect {
        left: (crop.x + placement_crop.left * crop.width).clamp(0.0, 1.0),
        top: (crop.y + placement_crop.top * crop.height).clamp(0.0, 1.0),
        right: (crop.x + placement_crop.right * crop.width).clamp(0.0, 1.0),
        bottom: (crop.y + placement_crop.bottom * crop.height).clamp(0.0, 1.0),
    };

    let base = placement.destination;
    let width = f64::from(output.width);
    let height = f64::from(output.height);
    let anchor_x = (f64::from(base.left)
        + f64::from(base.right - base.left) * f64::from(transform.anchor_x))
        * width;
    let anchor_y = (f64::from(base.top)
        + f64::from(base.bottom - base.top) * f64::from(transform.anchor_y))
        * height;
    let radians = f64::from(transform.rotation_degrees).to_radians();
    let cosine = radians.cos();
    let sine = radians.sin();
    let position_x = f64::from(transform.position_x_px);
    let position_y = f64::from(transform.position_y_px);

    let mut transformed = placement.destination_corners;
    for point in &mut transformed {
        let local_x = (f64::from(point.x) * width - anchor_x) * f64::from(transform.scale_x);
        let local_y = (f64::from(point.y) * height - anchor_y) * f64::from(transform.scale_y);
        let physical_x = anchor_x + local_x * cosine - local_y * sine + position_x;
        let physical_y = anchor_y + local_x * sine + local_y * cosine + position_y;
        point.x = finite_f32(physical_x / width)?;
        point.y = finite_f32(physical_y / height)?;
    }
    placement.destination_corners = transformed;
    placement.destination = bounds(&transformed);
    Ok(placement)
}

fn calculate_for_dimensions(
    source_width: f64,
    source_height: f64,
    output: PlacementDimensions,
    mode: PlacementMode,
) -> ImagePlacement {
    let source_aspect = source_width / source_height;
    let output_aspect = f64::from(output.width) / f64::from(output.height);
    let full = rect(0.0, 0.0, 1.0, 1.0);

    let (destination, source_uv) = match mode {
        PlacementMode::Stretch => (full, full),
        PlacementMode::Cover => {
            let source_uv = if source_aspect > output_aspect {
                let visible_width = output_aspect / source_aspect;
                let inset = (1.0 - visible_width) / 2.0;
                rect(inset as f32, 0.0, (1.0 - inset) as f32, 1.0)
            } else if source_aspect < output_aspect {
                let visible_height = source_aspect / output_aspect;
                let inset = (1.0 - visible_height) / 2.0;
                rect(0.0, inset as f32, 1.0, (1.0 - inset) as f32)
            } else {
                full
            };
            (full, source_uv)
        }
        PlacementMode::Fit => {
            let (width, height) = if source_aspect > output_aspect {
                (1.0, output_aspect / source_aspect)
            } else {
                (source_aspect / output_aspect, 1.0)
            };
            (centered_rect(width, height), full)
        }
        PlacementMode::Center => (
            centered_rect(
                source_width / f64::from(output.width),
                source_height / f64::from(output.height),
            ),
            full,
        ),
    };
    ImagePlacement::new(destination, source_uv)
}

fn centered_rect(width: f64, height: f64) -> PlacementRect {
    let left = (1.0 - width) / 2.0;
    let top = (1.0 - height) / 2.0;
    PlacementRect {
        left: left as f32,
        top: top as f32,
        right: (left + width) as f32,
        bottom: (top + height) as f32,
    }
}

fn rect(left: f32, top: f32, right: f32, bottom: f32) -> PlacementRect {
    PlacementRect {
        left,
        top,
        right,
        bottom,
    }
}

fn bounds(points: &[PlacementPoint; 4]) -> PlacementRect {
    let left = points
        .iter()
        .map(|point| point.x)
        .fold(f32::INFINITY, f32::min);
    let top = points
        .iter()
        .map(|point| point.y)
        .fold(f32::INFINITY, f32::min);
    let right = points
        .iter()
        .map(|point| point.x)
        .fold(f32::NEG_INFINITY, f32::max);
    let bottom = points
        .iter()
        .map(|point| point.y)
        .fold(f32::NEG_INFINITY, f32::max);
    rect(left, top, right, bottom)
}

fn finite_f32(value: f64) -> Result<f32, RendererError> {
    if !value.is_finite() || value.abs() > f64::from(f32::MAX) {
        return Err(RendererError::InvalidTransformParameters);
    }
    Ok(value as f32)
}

#[cfg(test)]
mod tests {
    use super::{
        ImageTransform, NormalizedCrop, PlacementDimensions as D, PlacementMode as M,
        calculate_image_placement as calculate,
        calculate_transformed_image_placement as calculate_transformed,
    };
    use crate::RendererError;

    fn d(width: u32, height: u32) -> D {
        D::new(width, height).expect("dimensions should be positive")
    }

    fn close(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 0.0001, "{actual} != {expected}");
    }

    #[test]
    fn zero_dimensions_are_rejected_at_the_renderer_boundary() {
        assert!(D::new(0, 1).is_err());
        assert!(D::new(1, 0).is_err());
        assert!(D::new(0, 0).is_err());
    }

    #[test]
    fn cover_crops_the_wide_source_symmetrically() {
        let p = calculate(d(2000, 1000), d(1000, 1000), M::Cover);
        close(p.destination().left(), 0.0);
        close(p.destination().right(), 1.0);
        close(p.source_uv().left(), 0.25);
        close(p.source_uv().right(), 0.75);
        close(p.source_uv().top(), 0.0);
        close(p.source_uv().bottom(), 1.0);
    }

    #[test]
    fn cover_crops_the_tall_source_symmetrically() {
        let p = calculate(d(1000, 2000), d(1000, 1000), M::Cover);
        close(p.source_uv().left(), 0.0);
        close(p.source_uv().right(), 1.0);
        close(p.source_uv().top(), 0.25);
        close(p.source_uv().bottom(), 0.75);
    }

    #[test]
    fn fit_centers_wide_and_tall_sources_without_cropping() {
        let wide = calculate(d(1920, 1080), d(1000, 1000), M::Fit);
        close(wide.destination().left(), 0.0);
        close(wide.destination().right(), 1.0);
        close(wide.destination().top(), 0.21875);
        close(wide.destination().bottom(), 0.78125);

        let tall = calculate(d(1080, 1920), d(1000, 1000), M::Fit);
        close(tall.destination().left(), 0.21875);
        close(tall.destination().right(), 0.78125);
        close(tall.destination().top(), 0.0);
        close(tall.destination().bottom(), 1.0);
        close(tall.source_uv().left(), 0.0);
        close(tall.source_uv().right(), 1.0);
    }

    #[test]
    fn stretch_uses_the_full_source_and_destination_rectangles() {
        let p = calculate(d(3840, 2160), d(1000, 1000), M::Stretch);
        assert_eq!(p.destination(), p.source_uv());
        close(p.destination().left(), 0.0);
        close(p.destination().bottom(), 1.0);
    }

    #[test]
    fn matching_aspect_and_physical_dimensions_fill_for_every_mode() {
        let dimensions = d(1920, 1080);
        for mode in [M::Cover, M::Fit, M::Stretch, M::Center] {
            let placement = calculate(dimensions, dimensions, mode);
            close(placement.destination().left(), 0.0);
            close(placement.destination().top(), 0.0);
            close(placement.destination().right(), 1.0);
            close(placement.destination().bottom(), 1.0);
            close(placement.source_uv().left(), 0.0);
            close(placement.source_uv().top(), 0.0);
            close(placement.source_uv().right(), 1.0);
            close(placement.source_uv().bottom(), 1.0);
        }
    }

    #[test]
    fn center_preserves_source_pixel_size_even_when_output_clips_it() {
        let p = calculate(d(3840, 2160), d(1920, 1080), M::Center);
        close(p.destination().left(), -0.5);
        close(p.destination().top(), -0.5);
        close(p.destination().right(), 1.5);
        close(p.destination().bottom(), 1.5);
        close(p.source_uv().left(), 0.0);
        close(p.source_uv().right(), 1.0);
    }

    #[test]
    fn four_k_wide_source_maps_correctly_to_portrait_output() {
        let source = d(3840, 2160);
        let output = d(1080, 1920);
        let cover = calculate(source, output, M::Cover);
        close(cover.destination().left(), 0.0);
        close(cover.destination().right(), 1.0);
        close(cover.source_uv().left(), 0.341796875);
        close(cover.source_uv().right(), 0.658203125);

        let fit = calculate(source, output, M::Fit);
        close(fit.destination().left(), 0.0);
        close(fit.destination().right(), 1.0);
        close(fit.destination().top(), 0.341796875);
        close(fit.destination().bottom(), 0.658203125);

        let center = calculate(source, output, M::Center);
        close(center.destination().left(), -1.2777778);
        close(center.destination().right(), 2.2777778);
        close(center.destination().top(), -0.0625);
        close(center.destination().bottom(), 1.0625);

        let tall_source = d(2160, 3840);
        let landscape_output = d(1920, 1080);
        let tall_cover = calculate(tall_source, landscape_output, M::Cover);
        close(tall_cover.source_uv().left(), 0.0);
        close(tall_cover.source_uv().right(), 1.0);
        close(tall_cover.source_uv().top(), 0.341796875);
        close(tall_cover.source_uv().bottom(), 0.658203125);

        let tall_fit = calculate(tall_source, landscape_output, M::Fit);
        close(tall_fit.destination().left(), 0.341796875);
        close(tall_fit.destination().right(), 0.658203125);
        close(tall_fit.destination().top(), 0.0);
        close(tall_fit.destination().bottom(), 1.0);
    }

    #[test]
    fn identity_transform_preserves_each_placement_mode() {
        let source = d(3840, 2160);
        let output = d(1920, 1080);
        for mode in [M::Cover, M::Fit, M::Stretch, M::Center] {
            let base = calculate(source, output, mode);
            let transformed =
                calculate_transformed(source, output, mode, ImageTransform::default())
                    .expect("identity transform should be valid");
            assert_eq!(base.source_uv(), transformed.source_uv());
            for (left, right) in base
                .destination_corners()
                .into_iter()
                .zip(transformed.destination_corners())
            {
                close(left.x(), right.x());
                close(left.y(), right.y());
            }
        }
    }

    #[test]
    fn translation_uses_physical_output_pixels() {
        let transform = ImageTransform::new(
            (100.0, -50.0),
            (1.0, 1.0),
            0.0,
            (0.5, 0.5),
            NormalizedCrop::default(),
        )
        .expect("finite transform should be valid");
        let placement = calculate_transformed(d(1000, 500), d(1000, 500), M::Stretch, transform)
            .expect("translated placement should be representable");
        let corners = placement.destination_corners();
        close(corners[0].x(), 0.1);
        close(corners[0].y(), -0.1);
        close(corners[2].x(), 1.1);
        close(corners[2].y(), 0.9);
    }

    #[test]
    fn scale_and_rotation_are_applied_around_the_normalized_anchor() {
        let scale = ImageTransform::new(
            (0.0, 0.0),
            (2.0, 0.5),
            0.0,
            (0.5, 0.5),
            NormalizedCrop::default(),
        )
        .expect("scale should be valid");
        let scaled = calculate_transformed(d(1000, 1000), d(1000, 1000), M::Stretch, scale)
            .expect("scale placement should be valid");
        close(scaled.destination().left(), -0.5);
        close(scaled.destination().right(), 1.5);
        close(scaled.destination().top(), 0.25);
        close(scaled.destination().bottom(), 0.75);

        let rotate = ImageTransform::new(
            (0.0, 0.0),
            (1.0, 1.0),
            90.0,
            (0.0, 0.0),
            NormalizedCrop::default(),
        )
        .expect("rotation should be valid");
        let rotated = calculate_transformed(d(1000, 1000), d(1000, 1000), M::Stretch, rotate)
            .expect("rotated placement should be valid");
        let corners = rotated.destination_corners();
        close(corners[0].x(), 0.0);
        close(corners[0].y(), 0.0);
        close(corners[1].x(), 0.0);
        close(corners[1].y(), 1.0);
    }

    #[test]
    fn normalized_crop_is_composed_before_placement_mode() {
        let crop = NormalizedCrop::new(0.25, 0.0, 0.5, 1.0).expect("crop should be valid");
        let transform = ImageTransform::new((0.0, 0.0), (1.0, 1.0), 0.0, (0.5, 0.5), crop)
            .expect("transform should be valid");
        let placement = calculate_transformed(d(2000, 1000), d(1000, 1000), M::Cover, transform)
            .expect("cropped placement should be valid");
        close(placement.source_uv().left(), 0.25);
        close(placement.source_uv().right(), 0.75);
        close(placement.source_uv().top(), 0.0);
        close(placement.source_uv().bottom(), 1.0);
    }

    #[test]
    fn crop_and_transform_parameters_return_typed_validation_errors() {
        assert!(NormalizedCrop::new(0.2, 0.2, 0.8, 0.8).is_ok());
        assert!(matches!(
            NormalizedCrop::new(0.8, 0.0, 0.3, 1.0),
            Err(RendererError::InvalidCropBounds)
        ));
        assert!(matches!(
            NormalizedCrop::new(0.0, 0.0, 0.0, 1.0),
            Err(RendererError::InvalidCropBounds)
        ));
        assert!(matches!(
            NormalizedCrop::new(f32::NAN, 0.0, 1.0, 1.0),
            Err(RendererError::InvalidCropBounds)
        ));
        assert!(matches!(
            ImageTransform::new(
                (f32::INFINITY, 0.0),
                (1.0, 1.0),
                0.0,
                (0.5, 0.5),
                NormalizedCrop::default()
            ),
            Err(RendererError::InvalidTransformParameters)
        ));
        assert!(matches!(
            ImageTransform::new(
                (0.0, 0.0),
                (0.0, 1.0),
                0.0,
                (0.5, 0.5),
                NormalizedCrop::default()
            ),
            Err(RendererError::InvalidTransformParameters)
        ));
        assert!(matches!(
            ImageTransform::new(
                (0.0, 0.0),
                (1.0, 1.0),
                0.0,
                (1.1, 0.5),
                NormalizedCrop::default()
            ),
            Err(RendererError::InvalidTransformParameters)
        ));
    }

    #[test]
    fn unrepresentable_transformed_geometry_is_rejected() {
        let transform = ImageTransform::new(
            (f32::MAX, 0.0),
            (f32::MAX, f32::MAX),
            45.0,
            (0.5, 0.5),
            NormalizedCrop::default(),
        )
        .expect("finite parameters should construct");
        assert!(matches!(
            calculate_transformed(d(1, 1), d(1, 1), M::Stretch, transform),
            Err(RendererError::InvalidTransformParameters)
        ));
    }
}
