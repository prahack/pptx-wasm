//! Shape geometry: preset references and custom paths, plus the DrawingML guide
//! (formula) evaluator they both depend on.
//!
//! Geometry is stored unevaluated. A preset is a name and its adjustment values; a
//! custom shape is its guide list and path commands. Both are turned into concrete
//! [`crate::dl::Path`]s at layout time, when the shape's final extent is known — which is
//! the only point at which `w` and `h` mean anything.

use std::collections::HashMap;

use crate::emu::Emu;

/// A DrawingML guide: a named value computed by a small stack-free formula language.
#[derive(Debug, Clone, PartialEq)]
pub struct Guide {
    pub name: String,
    pub formula: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathFillMode {
    None,
    Normal,
    Lighten,
    LightenLess,
    Darken,
    DarkenLess,
}

/// One `<a:path>` inside a custom geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct GeomPath {
    /// The coordinate space this path's numbers are in. `None` means the shape's own
    /// extent, which is the common case.
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub fill_mode: PathFillMode,
    pub stroke: bool,
    pub commands: Vec<GeomCommand>,
}

impl Default for GeomPath {
    fn default() -> Self {
        GeomPath {
            width: None,
            height: None,
            fill_mode: PathFillMode::Normal,
            stroke: true,
            commands: Vec::new(),
        }
    }
}

/// Path commands, with operands still as unevaluated formula expressions — an operand
/// can be a literal, a guide name, or an adjustment name.
#[derive(Debug, Clone, PartialEq)]
pub enum GeomCommand {
    MoveTo(Expr, Expr),
    LineTo(Expr, Expr),
    /// Two control points and an endpoint.
    CubicTo(Expr, Expr, Expr, Expr, Expr, Expr),
    QuadTo(Expr, Expr, Expr, Expr),
    /// `<a:arcTo wR hR stAng swAng/>` — relative to the current point.
    ArcTo {
        wr: Expr,
        hr: Expr,
        start_angle: Expr,
        swing_angle: Expr,
    },
    Close,
}

/// An operand: a number, or the name of a guide/adjustment to look up.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(f64),
    Name(String),
}

impl Expr {
    pub fn parse(s: &str) -> Expr {
        let t = s.trim();
        match t.parse::<f64>() {
            Ok(v) => Expr::Literal(v),
            Err(_) => Expr::Name(t.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CustomGeometry {
    pub adjust: Vec<Guide>,
    pub guides: Vec<Guide>,
    pub paths: Vec<GeomPath>,
    /// `<a:rect>` text-body inset rectangle, as four guide expressions (l, t, r, b).
    pub text_rect: Option<[Expr; 4]>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Geometry {
    /// `<a:prstGeom prst="roundRect">` with its `<a:avLst>` adjustments.
    Preset {
        preset: String,
        adjustments: Vec<(String, f64)>,
    },
    Custom(Box<CustomGeometry>),
    /// No geometry element at all — the shape has no outline of its own (a picture
    /// frame, a graphic frame). Renders as its bounding rectangle when it needs a fill.
    #[default]
    None,
}

impl Geometry {
    pub fn preset(name: &str) -> Geometry {
        Geometry::Preset {
            preset: name.to_string(),
            adjustments: Vec::new(),
        }
    }

    pub fn preset_name(&self) -> Option<&str> {
        match self {
            Geometry::Preset { preset, .. } => Some(preset),
            _ => None,
        }
    }

    /// Whether this geometry is a straight or bent line rather than a closed area —
    /// connectors and `line` presets must never be filled.
    pub fn is_line_like(&self) -> bool {
        match self.preset_name() {
            Some(p) => {
                p == "line"
                    || p == "straightConnector1"
                    || p.starts_with("bentConnector")
                    || p.starts_with("curvedConnector")
                    || p.starts_with("arc")
            }
            None => false,
        }
    }
}

/// Evaluates DrawingML guide formulas.
///
/// The language is prefix, fixed-arity, and operates on a flat namespace of previously
/// defined guides. Values are in the shape's coordinate space (usually EMUs scaled to
/// the shape box) except angles, which are in 60000ths of a degree.
pub struct GuideContext {
    values: HashMap<String, f64>,
}

impl GuideContext {
    /// Seeds the built-in guides for a shape of the given size.
    pub fn new(width: f64, height: f64) -> Self {
        let ss = width.min(height);
        let mut values = HashMap::with_capacity(32);
        values.insert("w".into(), width);
        values.insert("h".into(), height);
        values.insert("ss".into(), ss);
        values.insert("l".into(), 0.0);
        values.insert("t".into(), 0.0);
        values.insert("r".into(), width);
        values.insert("b".into(), height);
        values.insert("hc".into(), width / 2.0);
        values.insert("vc".into(), height / 2.0);
        values.insert("wd2".into(), width / 2.0);
        values.insert("hd2".into(), height / 2.0);
        values.insert("ssd2".into(), ss / 2.0);
        // The divisor family: PowerPoint's preset definitions use wd4, hd8, ssd6, …
        for d in [2u32, 3, 4, 5, 6, 8, 10, 12, 16, 32] {
            values.insert(format!("wd{d}"), width / d as f64);
            values.insert(format!("hd{d}"), height / d as f64);
            values.insert(format!("ssd{d}"), ss / d as f64);
        }
        // Angle constants, in 60000ths of a degree.
        values.insert("cd8".into(), 2_700_000.0);
        values.insert("cd4".into(), 5_400_000.0);
        values.insert("cd2".into(), 10_800_000.0);
        values.insert("3cd8".into(), 8_100_000.0);
        values.insert("3cd4".into(), 16_200_000.0);
        values.insert("5cd8".into(), 13_500_000.0);
        values.insert("7cd8".into(), 18_900_000.0);
        values.insert("cd0".into(), 21_600_000.0);
        GuideContext { values }
    }

    pub fn set(&mut self, name: &str, value: f64) {
        self.values.insert(name.to_string(), value);
    }

    pub fn get(&self, name: &str) -> Option<f64> {
        self.values.get(name).copied()
    }

    pub fn eval_expr(&self, e: &Expr) -> f64 {
        match e {
            Expr::Literal(v) => *v,
            Expr::Name(n) => self.values.get(n).copied().unwrap_or(0.0),
        }
    }

    fn operand(&self, tok: &str) -> f64 {
        match tok.parse::<f64>() {
            Ok(v) => v,
            Err(_) => self.values.get(tok).copied().unwrap_or(0.0),
        }
    }

    /// Evaluates one formula. Unknown operations yield 0 rather than failing, so a
    /// preset we do not fully understand still produces *a* shape.
    pub fn eval_formula(&self, formula: &str) -> f64 {
        let mut it = formula.split_whitespace();
        let Some(op) = it.next() else {
            return 0.0;
        };
        let a = it.next().map(|t| self.operand(t)).unwrap_or(0.0);
        let b = it.next().map(|t| self.operand(t)).unwrap_or(0.0);
        let c = it.next().map(|t| self.operand(t)).unwrap_or(0.0);

        // Angles arrive in 60000ths of a degree and trig works in radians.
        const ANG: f64 = 60_000.0;
        let to_rad = |v: f64| (v / ANG).to_radians();

        match op {
            "val" => a,
            "+-" => a + b - c,
            "+/" => {
                if c == 0.0 {
                    0.0
                } else {
                    (a + b) / c
                }
            }
            "*/" => {
                if c == 0.0 {
                    0.0
                } else {
                    a * b / c
                }
            }
            "abs" => a.abs(),
            "min" => a.min(b),
            "max" => a.max(b),
            "mod" => (a * a + b * b + c * c).sqrt(),
            "sqrt" => {
                if a < 0.0 {
                    0.0
                } else {
                    a.sqrt()
                }
            }
            "pin" => {
                // pin x y z: clamp y into [x, z]
                if b < a {
                    a
                } else if b > c {
                    c
                } else {
                    b
                }
            }
            "?:" => {
                if a > 0.0 {
                    b
                } else {
                    c
                }
            }
            "sin" => a * to_rad(b).sin(),
            "cos" => a * to_rad(b).cos(),
            "tan" => a * to_rad(b).tan(),
            "at2" => at2_degrees(a, b),
            "cat2" => {
                // cat2 x y z: x * cos(atan2(z, y))
                a * at2_radians(b, c).cos()
            }
            "sat2" => a * at2_radians(b, c).sin(),
            _ => {
                log::debug!("unknown guide operation {op:?} in {formula:?}");
                0.0
            }
        }
    }

    /// Evaluates a guide list in order, defining each name as it goes.
    pub fn eval_guides(&mut self, guides: &[Guide]) {
        for g in guides {
            let v = self.eval_formula(&g.formula);
            self.values.insert(g.name.clone(), v);
        }
    }
}

/// `at2 x y` → the angle of (x, y) in 60000ths of a degree, normalised to [0, 360).
fn at2_degrees(x: f64, y: f64) -> f64 {
    let deg = y.atan2(x).to_degrees().rem_euclid(360.0);
    deg * 60_000.0
}

fn at2_radians(x: f64, y: f64) -> f64 {
    y.atan2(x)
}

/// The default adjustment values for a preset, as PowerPoint would supply them.
/// Only presets whose defaults are not zero need an entry.
pub fn default_adjustments(preset: &str) -> &'static [(&'static str, f64)] {
    match preset {
        "roundRect" | "round1Rect" | "round2SameRect" | "round2DiagRect" | "snipRoundRect" => {
            &[("adj", 16667.0), ("adj1", 16667.0), ("adj2", 0.0)]
        }
        "snip1Rect" | "snip2SameRect" | "snip2DiagRect" => {
            &[("adj", 16667.0), ("adj1", 16667.0), ("adj2", 0.0)]
        }
        "parallelogram" => &[("adj", 25000.0)],
        "trapezoid" => &[("adj", 25000.0)],
        "star4" => &[("adj", 12500.0)],
        "star5" => &[("adj", 19098.0), ("hf", 105146.0), ("vf", 110557.0)],
        "star6" => &[("adj", 28868.0), ("hf", 115470.0)],
        "star8" => &[("adj", 37500.0)],
        "rightArrow" | "leftArrow" | "upArrow" | "downArrow" => {
            &[("adj1", 50000.0), ("adj2", 50000.0)]
        }
        "leftRightArrow" | "upDownArrow" => &[("adj1", 50000.0), ("adj2", 50000.0)],
        "bentArrow" => &[
            ("adj1", 25000.0),
            ("adj2", 25000.0),
            ("adj3", 25000.0),
            ("adj4", 43750.0),
        ],
        "chevron" | "homePlate" => &[("adj", 50000.0)],
        "plus" => &[("adj", 25000.0)],
        "can" => &[("adj", 25000.0)],
        "cube" => &[("adj", 25000.0)],
        "donut" | "noSmoking" => &[("adj", 25000.0)],
        "pie" | "arc" | "chord" | "blockArc" => &[("adj1", 0.0), ("adj2", 16200000.0)],
        "wedgeRectCallout" | "wedgeRoundRectCallout" | "wedgeEllipseCallout" => {
            &[("adj1", -20833.0), ("adj2", 62500.0), ("adj3", 16667.0)]
        }
        "teardrop" => &[("adj", 100000.0)],
        "bevel" => &[("adj", 12500.0)],
        "frame" => &[("adj1", 12500.0)],
        "corner" => &[("adj1", 50000.0), ("adj2", 50000.0)],
        "diagStripe" => &[("adj", 50000.0)],
        "pentagon" => &[("hf", 105146.0), ("vf", 110557.0)],
        _ => &[],
    }
}

/// `<a:xfrm>` on a shape, in EMUs.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Transform2D {
    pub offset_x: Emu,
    pub offset_y: Emu,
    pub extent_x: Emu,
    pub extent_y: Emu,
    /// Rotation in 60000ths of a degree.
    pub rotation: i32,
    pub flip_h: bool,
    pub flip_v: bool,
    /// True when the element actually carried an `<a:off>`/`<a:ext>`; a shape without
    /// one inherits its position from its placeholder.
    pub specified: bool,
}

/// A group shape's `<a:xfrm>` also carries the child coordinate space, which is what
/// makes nested group transforms work: children are authored in `chOff`/`chExt` space and
/// mapped onto the group's own `off`/`ext` box.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GroupTransform {
    pub xfrm: Transform2D,
    pub child_offset_x: Emu,
    pub child_offset_y: Emu,
    pub child_extent_x: Emu,
    pub child_extent_y: Emu,
}

impl GroupTransform {
    /// Scale factors mapping child space onto the group's box.
    pub fn child_scale(&self) -> (f32, f32) {
        let sx = if self.child_extent_x != 0 {
            self.xfrm.extent_x as f32 / self.child_extent_x as f32
        } else {
            1.0
        };
        let sy = if self.child_extent_y != 0 {
            self.xfrm.extent_y as f32 / self.child_extent_y as f32
        } else {
            1.0
        };
        (sx, sy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> GuideContext {
        GuideContext::new(200.0, 100.0)
    }

    #[test]
    fn builtin_guides_derive_from_the_shape_box() {
        let c = ctx();
        assert_eq!(c.get("w"), Some(200.0));
        assert_eq!(c.get("h"), Some(100.0));
        assert_eq!(c.get("hc"), Some(100.0));
        assert_eq!(c.get("vc"), Some(50.0));
        assert_eq!(c.get("ss"), Some(100.0), "ss is the smaller side");
        assert_eq!(c.get("wd4"), Some(50.0));
        assert_eq!(c.get("ssd6"), Some(100.0 / 6.0));
    }

    #[test]
    fn arithmetic_operations_match_the_spec() {
        let c = ctx();
        assert_eq!(c.eval_formula("val 42"), 42.0);
        assert_eq!(c.eval_formula("+- 10 5 3"), 12.0);
        assert_eq!(c.eval_formula("*/ w 1 2"), 100.0);
        assert_eq!(c.eval_formula("+/ 10 6 4"), 4.0);
        assert_eq!(c.eval_formula("abs -7"), 7.0);
        assert_eq!(c.eval_formula("min 3 9"), 3.0);
        assert_eq!(c.eval_formula("max 3 9"), 9.0);
        assert_eq!(c.eval_formula("sqrt 16"), 4.0);
        assert_eq!(c.eval_formula("mod 3 4 0"), 5.0);
    }

    #[test]
    fn division_by_zero_yields_zero_rather_than_infinity() {
        let c = ctx();
        assert_eq!(c.eval_formula("*/ 10 10 0"), 0.0);
        assert_eq!(c.eval_formula("+/ 10 10 0"), 0.0);
        assert!(c.eval_formula("*/ 10 10 0").is_finite());
    }

    #[test]
    fn pin_clamps_and_ternary_tests_greater_than_zero() {
        let c = ctx();
        assert_eq!(c.eval_formula("pin 0 -5 100"), 0.0);
        assert_eq!(c.eval_formula("pin 0 50 100"), 50.0);
        assert_eq!(c.eval_formula("pin 0 500 100"), 100.0);
        assert_eq!(c.eval_formula("?: 1 10 20"), 10.0);
        assert_eq!(c.eval_formula("?: 0 10 20"), 20.0);
        assert_eq!(c.eval_formula("?: -1 10 20"), 20.0);
    }

    #[test]
    fn trig_uses_sixty_thousandths_of_a_degree() {
        let c = ctx();
        // sin(90°) = 1, so "sin 100 5400000" is 100.
        assert!((c.eval_formula("sin 100 5400000") - 100.0).abs() < 1e-6);
        assert!(c.eval_formula("cos 100 5400000").abs() < 1e-6);
        // at2 of (1,1) is 45°.
        assert!((c.eval_formula("at2 1 1") - 45.0 * 60_000.0).abs() < 1.0);
    }

    #[test]
    fn at2_normalises_into_the_positive_range() {
        let c = ctx();
        // (−1, −1) is 225°, not −135°.
        assert!((c.eval_formula("at2 -1 -1") - 225.0 * 60_000.0).abs() < 1.0);
    }

    #[test]
    fn guides_can_reference_earlier_guides() {
        let mut c = ctx();
        c.eval_guides(&[
            Guide {
                name: "a".into(),
                formula: "*/ w 1 4".into(),
            },
            Guide {
                name: "bb".into(),
                formula: "+- a 0 10".into(),
            },
        ]);
        assert_eq!(c.get("a"), Some(50.0));
        assert_eq!(c.get("bb"), Some(40.0));
    }

    #[test]
    fn unknown_names_and_operations_degrade_to_zero() {
        let c = ctx();
        assert_eq!(c.eval_formula("val nosuchguide"), 0.0);
        assert_eq!(c.eval_formula("frobnicate 1 2 3"), 0.0);
        assert_eq!(c.eval_formula(""), 0.0);
    }

    #[test]
    fn group_child_scale_handles_a_zero_extent() {
        let g = GroupTransform::default();
        assert_eq!(g.child_scale(), (1.0, 1.0));
    }

    #[test]
    fn group_child_scale_maps_child_space_onto_the_group_box() {
        let g = GroupTransform {
            xfrm: Transform2D {
                extent_x: 200,
                extent_y: 100,
                ..Default::default()
            },
            child_extent_x: 400,
            child_extent_y: 400,
            ..Default::default()
        };
        assert_eq!(g.child_scale(), (0.5, 0.25));
    }

    #[test]
    fn connectors_and_lines_are_recognised_as_unfillable() {
        assert!(Geometry::preset("line").is_line_like());
        assert!(Geometry::preset("bentConnector3").is_line_like());
        assert!(!Geometry::preset("rect").is_line_like());
        assert!(!Geometry::None.is_line_like());
    }
}
