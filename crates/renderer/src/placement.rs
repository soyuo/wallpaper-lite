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

/// GPU-ready normalized source and destination rectangles for one presentation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImagePlacement {
    destination: PlacementRect,
    source_uv: PlacementRect,
}

impl ImagePlacement {
    pub const fn destination(self) -> PlacementRect {
        self.destination
    }

    pub const fn source_uv(self) -> PlacementRect {
        self.source_uv
    }
}

/// Calculate placement geometry from validated physical source and output pixel dimensions.
pub fn calculate_image_placement(
    source: PlacementDimensions,
    output: PlacementDimensions,
    mode: PlacementMode,
) -> ImagePlacement {
    let source_aspect = source.width as f64 / source.height as f64;
    let output_aspect = output.width as f64 / output.height as f64;
    let full = PlacementRect {
        left: 0.0,
        top: 0.0,
        right: 1.0,
        bottom: 1.0,
    };

    match mode {
        PlacementMode::Stretch => ImagePlacement {
            destination: full,
            source_uv: full,
        },
        PlacementMode::Cover => {
            let source_uv = if source_aspect > output_aspect {
                let visible_width = output_aspect / source_aspect;
                let inset = (1.0 - visible_width) / 2.0;
                PlacementRect {
                    left: inset as f32,
                    top: 0.0,
                    right: (1.0 - inset) as f32,
                    bottom: 1.0,
                }
            } else if source_aspect < output_aspect {
                let visible_height = source_aspect / output_aspect;
                let inset = (1.0 - visible_height) / 2.0;
                PlacementRect {
                    left: 0.0,
                    top: inset as f32,
                    right: 1.0,
                    bottom: (1.0 - inset) as f32,
                }
            } else {
                full
            };
            ImagePlacement {
                destination: full,
                source_uv,
            }
        }
        PlacementMode::Fit => {
            let (width, height) = if source_aspect > output_aspect {
                (1.0, output_aspect / source_aspect)
            } else {
                (source_aspect / output_aspect, 1.0)
            };
            ImagePlacement {
                destination: centered_rect(width, height),
                source_uv: full,
            }
        }
        PlacementMode::Center => {
            let width = source.width as f64 / output.width as f64;
            let height = source.height as f64 / output.height as f64;
            ImagePlacement {
                destination: centered_rect(width, height),
                source_uv: full,
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::{
        PlacementDimensions as D, PlacementMode as M, calculate_image_placement as calculate,
    };

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
}
