//! Fills and outlines, still unresolved: colours are [`ColorRef`]s and lengths are EMUs.

use crate::emu::Emu;

use super::color::ColorRef;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GradientKind {
    Linear,
    /// Radial, rectangular and shape gradients differ only in how the stop positions map
    /// onto the shape's box; all three are approximated as a radial in the display list.
    Radial,
    Rect,
    Shape,
    Path,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GradientStopSpec {
    /// 0..1 along the gradient.
    pub pos: f32,
    pub color: ColorRef,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GradientFill {
    pub kind: GradientKind,
    pub stops: Vec<GradientStopSpec>,
    /// Direction in degrees clockwise from the +x axis. Linear gradients only.
    pub angle_deg: f32,
    /// Whether the angle is measured against the shape's bounding box after rotation.
    pub scaled: bool,
    /// `<a:fillToRect>` for radial gradients, as 0..1 insets from each edge.
    pub focus: Option<[f32; 4]>,
}

/// How a bitmap fill maps onto the shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BlipMode {
    /// `<a:stretch>` — one copy scaled to the shape's box.
    Stretch,
    /// `<a:tile tx ty sx sy>` — repeated at its natural size, scaled and offset.
    ///
    /// Dropping the repeat and stretching instead is not a small error: a texture tile is
    /// typically a few dozen pixels, so stretching it across a slide averages it into a
    /// flat wash and the texture disappears entirely.
    Tile {
        /// Scale factors as fractions of the image's natural size.
        scale_x: f32,
        scale_y: f32,
        /// Offset of the first tile, in EMUs.
        offset_x: crate::emu::Emu,
        offset_y: crate::emu::Emu,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlipFill {
    /// Relationship id of the image part, resolved against the owning part.
    pub embed_id: Option<String>,
    /// Source crop as 0..1 insets from left/top/right/bottom (`<a:srcRect>`).
    pub src_rect: [f32; 4],
    pub mode: BlipMode,
    /// `<a:alphaModFix amt="..">`, 0..1.
    pub alpha: f32,
    /// Stretch fill destination insets, `<a:fillRect>`.
    pub fill_rect: [f32; 4],
}

impl Default for BlipFill {
    fn default() -> Self {
        BlipFill {
            embed_id: None,
            src_rect: [0.0; 4],
            mode: BlipMode::Stretch,
            alpha: 1.0,
            fill_rect: [0.0; 4],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PatternFill {
    pub foreground: ColorRef,
    pub background: ColorRef,
    /// Preset pattern name, e.g. `pct25`, `ltHorz`.
    pub preset: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Fill {
    /// `<a:noFill/>` — explicitly nothing, which is different from "not specified".
    NoFill,
    Solid(ColorRef),
    Gradient(GradientFill),
    Blip(BlipFill),
    Pattern(PatternFill),
    /// `<a:grpFill/>` — take the parent group's fill.
    Group,
    /// Nothing was specified at this level; keep looking up the inheritance chain.
    #[default]
    Inherit,
}

impl Fill {
    /// True when the property was actually specified here, and so should stop the
    /// inheritance walk. `NoFill` counts — "no fill" is a decision, not an absence.
    pub fn is_specified(&self) -> bool {
        !matches!(self, Fill::Inherit)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DashStyle {
    Solid,
    Dot,
    Dash,
    LargeDash,
    DashDot,
    LargeDashDot,
    LargeDashDotDot,
    SystemDash,
    SystemDot,
    SystemDashDot,
    SystemDashDotDot,
}

impl DashStyle {
    pub fn parse(s: &str) -> Option<DashStyle> {
        Some(match s {
            "solid" => DashStyle::Solid,
            "dot" => DashStyle::Dot,
            "dash" => DashStyle::Dash,
            "lgDash" => DashStyle::LargeDash,
            "dashDot" => DashStyle::DashDot,
            "lgDashDot" => DashStyle::LargeDashDot,
            "lgDashDotDot" => DashStyle::LargeDashDotDot,
            "sysDash" => DashStyle::SystemDash,
            "sysDot" => DashStyle::SystemDot,
            "sysDashDot" => DashStyle::SystemDashDot,
            "sysDashDotDot" => DashStyle::SystemDashDotDot,
            _ => return None,
        })
    }

    /// Dash pattern in multiples of the line width, matching the ECMA-376 preset table.
    pub fn pattern(self) -> &'static [f32] {
        match self {
            DashStyle::Solid => &[],
            DashStyle::Dot => &[1.0, 3.0],
            DashStyle::Dash => &[4.0, 3.0],
            DashStyle::LargeDash => &[8.0, 3.0],
            DashStyle::DashDot => &[4.0, 3.0, 1.0, 3.0],
            DashStyle::LargeDashDot => &[8.0, 3.0, 1.0, 3.0],
            DashStyle::LargeDashDotDot => &[8.0, 3.0, 1.0, 3.0, 1.0, 3.0],
            DashStyle::SystemDash => &[3.0, 1.0],
            DashStyle::SystemDot => &[1.0, 1.0],
            DashStyle::SystemDashDot => &[3.0, 1.0, 1.0, 1.0],
            DashStyle::SystemDashDotDot => &[3.0, 1.0, 1.0, 1.0, 1.0, 1.0],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineCapStyle {
    Flat,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineJoinStyle {
    Round,
    Bevel,
    Miter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrowType {
    None,
    Triangle,
    Stealth,
    Diamond,
    Oval,
    Arrow,
}

impl ArrowType {
    pub fn parse(s: &str) -> ArrowType {
        match s {
            "triangle" => ArrowType::Triangle,
            "stealth" => ArrowType::Stealth,
            "diamond" => ArrowType::Diamond,
            "oval" => ArrowType::Oval,
            "arrow" => ArrowType::Arrow,
            _ => ArrowType::None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArrowEnd {
    pub kind: ArrowType,
    /// Width and length as small/medium/large multipliers of the line width.
    pub width: f32,
    pub length: f32,
}

impl Default for ArrowEnd {
    fn default() -> Self {
        ArrowEnd {
            kind: ArrowType::None,
            width: 3.0,
            length: 3.0,
        }
    }
}

/// A shape outline. `None` fields mean "inherit"; `Fill::NoFill` means "explicitly none".
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Line {
    pub width: Option<Emu>,
    pub fill: Fill,
    pub dash: Option<DashStyle>,
    pub cap: Option<LineCapStyle>,
    pub join: Option<LineJoinStyle>,
    pub head: Option<ArrowEnd>,
    pub tail: Option<ArrowEnd>,
}

impl Line {
    /// True when nothing at all was specified, so the whole outline should be inherited.
    pub fn is_empty(&self) -> bool {
        self.width.is_none()
            && !self.fill.is_specified()
            && self.dash.is_none()
            && self.cap.is_none()
            && self.join.is_none()
            && self.head.is_none()
            && self.tail.is_none()
    }

    /// Fills in anything unspecified from `parent`.
    pub fn inherit_from(&mut self, parent: &Line) {
        if self.width.is_none() {
            self.width = parent.width;
        }
        if !self.fill.is_specified() {
            self.fill = parent.fill.clone();
        }
        if self.dash.is_none() {
            self.dash = parent.dash;
        }
        if self.cap.is_none() {
            self.cap = parent.cap;
        }
        if self.join.is_none() {
            self.join = parent.join;
        }
        if self.head.is_none() {
            self.head = parent.head;
        }
        if self.tail.is_none() {
            self.tail = parent.tail;
        }
    }
}

/// Outer shadow, the only effect common enough to model precisely.
#[derive(Debug, Clone, PartialEq)]
pub struct OuterShadow {
    /// Gaussian blur radius in EMUs.
    pub blur: Emu,
    /// Offset distance in EMUs and direction in degrees.
    pub distance: Emu,
    pub direction_deg: f32,
    pub color: ColorRef,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Glow {
    pub radius: Emu,
    pub color: ColorRef,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Effects {
    pub outer_shadow: Option<OuterShadow>,
    pub glow: Option<Glow>,
    /// `<a:softEdge rad="..">` radius in EMUs.
    pub soft_edge: Option<Emu>,
}

impl Effects {
    pub fn is_empty(&self) -> bool {
        self.outer_shadow.is_none() && self.glow.is_none() && self.soft_edge.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dl::Color;
    use crate::model::color::ColorRef as CR;

    #[test]
    fn no_fill_is_specified_but_inherit_is_not() {
        assert!(Fill::NoFill.is_specified());
        assert!(Fill::Solid(CR::srgb(Color::BLACK)).is_specified());
        assert!(!Fill::Inherit.is_specified());
        assert!(!Fill::default().is_specified());
    }

    #[test]
    fn line_inheritance_fills_only_the_gaps() {
        let mut child = Line {
            width: Some(12_700),
            ..Default::default()
        };
        let parent = Line {
            width: Some(25_400),
            fill: Fill::Solid(CR::srgb(Color::BLACK)),
            dash: Some(DashStyle::Dash),
            ..Default::default()
        };
        child.inherit_from(&parent);
        assert_eq!(child.width, Some(12_700), "child width must win");
        assert_eq!(child.dash, Some(DashStyle::Dash), "dash comes from parent");
        assert!(child.fill.is_specified());
    }

    #[test]
    fn an_explicit_no_fill_outline_is_not_overwritten_by_the_parent() {
        let mut child = Line {
            fill: Fill::NoFill,
            ..Default::default()
        };
        child.inherit_from(&Line {
            fill: Fill::Solid(CR::srgb(Color::WHITE)),
            ..Default::default()
        });
        assert_eq!(child.fill, Fill::NoFill);
    }

    #[test]
    fn dash_presets_map_to_patterns_and_solid_is_empty() {
        assert_eq!(DashStyle::parse("lgDashDot"), Some(DashStyle::LargeDashDot));
        assert!(DashStyle::Solid.pattern().is_empty());
        assert_eq!(DashStyle::Dot.pattern(), &[1.0, 3.0]);
        assert_eq!(DashStyle::parse("nonsense"), None);
    }
}
