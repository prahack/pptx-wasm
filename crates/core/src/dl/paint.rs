//! Resolved paint. Everything here is concrete: by the time layout emits a `Paint`,
//! theme lookups, `phClr` substitution, and tint/shade/alpha modulation have all been
//! applied. A renderer never resolves a colour.

use super::geom::{Point, Rect};

/// Straight (non-premultiplied) sRGB with an alpha channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0);
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const WHITE: Color = Color::rgb(255, 255, 255);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Parses the 6-hex-digit form OOXML uses in `<a:srgbClr val="RRGGBB"/>`.
    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.trim().trim_start_matches('#');
        if s.len() != 6 || !s.is_ascii() {
            return None;
        }
        let v = u32::from_str_radix(s, 16).ok()?;
        Some(Color::rgb(
            ((v >> 16) & 0xff) as u8,
            ((v >> 8) & 0xff) as u8,
            (v & 0xff) as u8,
        ))
    }

    pub fn is_transparent(&self) -> bool {
        self.a == 0
    }

    /// Multiplies the existing alpha by `factor` (0..=1). OOXML alpha modulation is
    /// multiplicative, so stacked `<a:alpha>` elements compose correctly.
    pub fn with_alpha_factor(self, factor: f32) -> Self {
        let a = (self.a as f32 * factor.clamp(0.0, 1.0)).round().clamp(0.0, 255.0);
        Color { a: a as u8, ..self }
    }

    /// CSS `rgba()` string — the form the Canvas2D backend needs.
    pub fn to_css(self) -> String {
        if self.a == 255 {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!(
                "rgba({},{},{},{:.4})",
                self.r,
                self.g,
                self.b,
                self.a as f32 / 255.0
            )
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientStop {
    /// 0.0 at the start of the gradient, 1.0 at the end.
    pub offset: f32,
    pub color: Color,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Gradient {
    Linear {
        start: Point,
        end: Point,
        stops: Vec<GradientStop>,
    },
    Radial {
        center: Point,
        radius: f32,
        /// Non-uniform radial gradients (OOXML `path="circle"` inside a non-square box)
        /// are expressed as a scale applied around `center`.
        scale_y: f32,
        stops: Vec<GradientStop>,
    },
}

/// An image the renderer resolves through the presentation's media registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ImageId(pub u32);

#[derive(Debug, Clone, PartialEq)]
pub enum Paint {
    Solid(Color),
    Gradient(Gradient),
    /// A bitmap fill. `tile` selects stretch vs. repeat semantics.
    Image {
        image: ImageId,
        /// Region of the source image to use, in normalised 0..1 coordinates.
        src: Rect,
        opacity: f32,
    },
}

impl Paint {
    pub fn solid(c: Color) -> Self {
        Paint::Solid(c)
    }

    /// True when drawing this paint cannot change any pixel.
    pub fn is_invisible(&self) -> bool {
        match self {
            Paint::Solid(c) => c.is_transparent(),
            Paint::Gradient(g) => match g {
                Gradient::Linear { stops, .. } | Gradient::Radial { stops, .. } => {
                    stops.iter().all(|s| s.color.is_transparent())
                }
            },
            Paint::Image { opacity, .. } => *opacity <= 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineCap {
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineJoin {
    Miter,
    Round,
    Bevel,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stroke {
    pub paint: Paint,
    /// Width in points. Zero means a hairline, which renderers draw at the thinnest
    /// visible width for the current scale rather than skipping.
    pub width: f32,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter_limit: f32,
    /// Dash pattern in multiples of the stroke width, empty for a solid line.
    pub dash: Vec<f32>,
}

impl Default for Stroke {
    fn default() -> Self {
        Stroke {
            paint: Paint::Solid(Color::BLACK),
            width: 1.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter_limit: 8.0,
            dash: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parsing_accepts_the_ooxml_form_and_rejects_junk() {
        assert_eq!(Color::from_hex("FF8000"), Some(Color::rgb(255, 128, 0)));
        assert_eq!(Color::from_hex("#ff8000"), Some(Color::rgb(255, 128, 0)));
        assert_eq!(Color::from_hex("FFF"), None);
        assert_eq!(Color::from_hex("GGGGGG"), None);
        assert_eq!(Color::from_hex(""), None);
    }

    #[test]
    fn alpha_factors_compose_multiplicatively() {
        let c = Color::rgb(10, 20, 30).with_alpha_factor(0.5).with_alpha_factor(0.5);
        assert_eq!(c.a, 64); // 255 * 0.5 = 128 (rounded), * 0.5 = 64
    }

    #[test]
    fn css_output_drops_the_alpha_channel_when_opaque() {
        assert_eq!(Color::rgb(255, 128, 0).to_css(), "#ff8000");
        assert!(Color::rgba(255, 128, 0, 128).to_css().starts_with("rgba(255,128,0,"));
    }
}
