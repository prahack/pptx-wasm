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
        let a = (self.a as f32 * factor.clamp(0.0, 1.0))
            .round()
            .clamp(0.0, 255.0);
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

/// How a bitmap fill repeats.
///
/// The tile's size is the image's own pixel size scaled by these factors — which is why
/// it is expressed as a scale rather than an absolute size. Layout does not know how
/// large the decoded image is; only the renderer does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tile {
    pub scale_x: f32,
    pub scale_y: f32,
    /// Offset of the first tile from the fill's origin, in points.
    pub offset_x: f32,
    pub offset_y: f32,
}

impl Default for Tile {
    fn default() -> Self {
        Tile {
            scale_x: 1.0,
            scale_y: 1.0,
            offset_x: 0.0,
            offset_y: 0.0,
        }
    }
}

/// A preset hatch, from `<a:pattFill prst="...">`.
///
/// The ~48 OOXML presets collapse to a handful of drawable families. Keeping them as a
/// small enum rather than the raw preset string means the backend has a closed set to
/// implement, and an unrecognised preset degrades to a dot screen of roughly the right
/// darkness instead of disappearing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HatchPattern {
    /// A dot screen at the given coverage percentage.
    Percent(u8),
    Horizontal {
        heavy: bool,
    },
    Vertical {
        heavy: bool,
    },
    DiagonalUp {
        heavy: bool,
    },
    DiagonalDown {
        heavy: bool,
    },
    Grid {
        heavy: bool,
    },
    DiagonalGrid {
        heavy: bool,
    },
}

impl HatchPattern {
    /// Maps an OOXML preset name onto a drawable family.
    pub fn from_preset(name: &str) -> HatchPattern {
        match name {
            "pct5" => HatchPattern::Percent(5),
            "pct10" => HatchPattern::Percent(10),
            "pct20" => HatchPattern::Percent(20),
            "pct25" => HatchPattern::Percent(25),
            "pct30" => HatchPattern::Percent(30),
            "pct40" => HatchPattern::Percent(40),
            "pct50" => HatchPattern::Percent(50),
            "pct60" => HatchPattern::Percent(60),
            "pct70" => HatchPattern::Percent(70),
            "pct75" => HatchPattern::Percent(75),
            "pct80" => HatchPattern::Percent(80),
            "pct90" => HatchPattern::Percent(90),
            "ltHorz" | "narHorz" => HatchPattern::Horizontal { heavy: false },
            "horz" | "dkHorz" => HatchPattern::Horizontal { heavy: true },
            "ltVert" | "narVert" => HatchPattern::Vertical { heavy: false },
            "vert" | "dkVert" => HatchPattern::Vertical { heavy: true },
            "ltUpDiag" | "wdUpDiag" => HatchPattern::DiagonalUp { heavy: false },
            "upDiag" | "dkUpDiag" => HatchPattern::DiagonalUp { heavy: true },
            "ltDnDiag" | "wdDnDiag" => HatchPattern::DiagonalDown { heavy: false },
            "dnDiag" | "dkDnDiag" => HatchPattern::DiagonalDown { heavy: true },
            "smGrid" | "lgGrid" | "dotGrid" => HatchPattern::Grid { heavy: false },
            "smCheck" | "lgCheck" | "openDmnd" | "solidDmnd" => HatchPattern::Grid { heavy: true },
            "smConfetti" | "lgConfetti" | "diagBrick" | "horzBrick" => {
                HatchPattern::DiagonalGrid { heavy: false }
            }
            "trellis" | "weave" | "plaid" | "shingle" | "zigZag" | "wave" => {
                HatchPattern::DiagonalGrid { heavy: true }
            }
            // Unknown presets read as a mid screen rather than vanishing.
            _ => HatchPattern::Percent(50),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Paint {
    Solid(Color),
    Gradient(Gradient),
    /// A bitmap fill.
    Image {
        image: ImageId,
        /// Region of the source image to use, in normalised 0..1 coordinates.
        src: Rect,
        opacity: f32,
        /// `Some` when the image repeats rather than stretching to the shape.
        tile: Option<Tile>,
    },
    /// A two-colour preset hatch. The backend builds the tile; the display list only
    /// says which pattern and in what colours, so layout never rasterises anything.
    Hatch {
        pattern: HatchPattern,
        foreground: Color,
        background: Color,
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
            Paint::Hatch {
                foreground,
                background,
                ..
            } => foreground.is_transparent() && background.is_transparent(),
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

/// A drop shadow, in display-list points.
///
/// Modelled as renderer *state* rather than as a property of a path because that is what
/// it is in every backend: Canvas2D has `shadowBlur`/`shadowOffset*` on the context, and a
/// GPU backend renders the shape to an offscreen target and blurs it. Making it state also
/// means `Save`/`Restore` scopes it for free.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
    /// Gaussian blur radius.
    pub blur: f32,
    pub offset_x: f32,
    pub offset_y: f32,
    pub color: Color,
}

impl Shadow {
    /// True when the shadow cannot change any pixel.
    pub fn is_invisible(&self) -> bool {
        self.color.is_transparent()
            || (self.blur <= 0.0 && self.offset_x == 0.0 && self.offset_y == 0.0)
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
        let c = Color::rgb(10, 20, 30)
            .with_alpha_factor(0.5)
            .with_alpha_factor(0.5);
        assert_eq!(c.a, 64); // 255 * 0.5 = 128 (rounded), * 0.5 = 64
    }

    #[test]
    fn hatch_presets_map_onto_drawable_families() {
        assert_eq!(
            HatchPattern::from_preset("pct25"),
            HatchPattern::Percent(25)
        );
        assert_eq!(
            HatchPattern::from_preset("ltUpDiag"),
            HatchPattern::DiagonalUp { heavy: false }
        );
        assert_eq!(
            HatchPattern::from_preset("dkUpDiag"),
            HatchPattern::DiagonalUp { heavy: true }
        );
        assert_eq!(
            HatchPattern::from_preset("dkHorz"),
            HatchPattern::Horizontal { heavy: true }
        );
        // An unrecognised preset still draws something of about the right darkness.
        assert_eq!(
            HatchPattern::from_preset("somethingNew"),
            HatchPattern::Percent(50)
        );
    }

    #[test]
    fn a_hatch_is_invisible_only_when_both_colours_are() {
        let clear = Color::TRANSPARENT;
        assert!(Paint::Hatch {
            pattern: HatchPattern::Percent(50),
            foreground: clear,
            background: clear,
        }
        .is_invisible());
        assert!(!Paint::Hatch {
            pattern: HatchPattern::Percent(50),
            foreground: Color::BLACK,
            background: clear,
        }
        .is_invisible());
    }

    #[test]
    fn css_output_drops_the_alpha_channel_when_opaque() {
        assert_eq!(Color::rgb(255, 128, 0).to_css(), "#ff8000");
        assert!(Color::rgba(255, 128, 0, 128)
            .to_css()
            .starts_with("rgba(255,128,0,"));
    }
}
