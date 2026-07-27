//! Preset shape geometry.
//!
//! ECMA-376 defines ~187 preset shapes as guide formulas plus path commands. Rather than
//! transcribe the whole table, this builds the common ones directly — the vocabulary that
//! actually appears in business decks — and falls back to a rectangle for the rest, so an
//! exotic preset still occupies the right space with the right fill.
//!
//! Adjustment values arrive as the raw OOXML numbers (thousandths of a percent, or an
//! absolute value for a few presets) and are normalised here.

use std::collections::HashMap;
use std::f32::consts::PI;

use crate::dl::{Path, Rect};
use crate::model::geometry::default_adjustments;

/// Builds a preset shape's path in a box of `w` x `h` points with its origin at 0,0.
pub fn build(preset: &str, w: f32, h: f32, adjustments: &[(String, f64)]) -> Path {
    let adj = Adjustments::new(preset, adjustments);
    let b = Builder { w, h, adj };
    b.build(preset)
}

/// True when this build has a real path for the preset, rather than falling back to a
/// rectangle. Exposed so tests can assert coverage rather than silently regressing.
pub fn is_supported(preset: &str) -> bool {
    SUPPORTED.contains(&preset)
}

const SUPPORTED: &[&str] = &[
    "rect",
    "roundRect",
    "round1Rect",
    "round2SameRect",
    "round2DiagRect",
    "snip1Rect",
    "snip2SameRect",
    "snip2DiagRect",
    "ellipse",
    "triangle",
    "rtTriangle",
    "diamond",
    "parallelogram",
    "trapezoid",
    "pentagon",
    "hexagon",
    "heptagon",
    "octagon",
    "decagon",
    "plus",
    "star4",
    "star5",
    "star6",
    "star8",
    "star10",
    "star12",
    "rightArrow",
    "leftArrow",
    "upArrow",
    "downArrow",
    "leftRightArrow",
    "upDownArrow",
    "chevron",
    "homePlate",
    "can",
    "cube",
    "donut",
    "line",
    "straightConnector1",
    "bentConnector2",
    "bentConnector3",
    "curvedConnector2",
    "curvedConnector3",
    "flowChartProcess",
    "flowChartDecision",
    "flowChartTerminator",
    "flowChartAlternateProcess",
    "flowChartConnector",
    "flowChartInputOutput",
    "flowChartDocument",
    "flowChartPredefinedProcess",
    "pie",
    "chord",
    "arc",
    "blockArc",
    "teardrop",
    "frame",
    "corner",
    "diagStripe",
    "bevel",
    "halfFrame",
    "leftBrace",
    "rightBrace",
    "leftBracket",
    "rightBracket",
    "wedgeRectCallout",
    "cloud",
    "heart",
    "lightningBolt",
    "sun",
    "moon",
    "smileyFace",
    "noSmoking",
    "plaque",
    "cross",
];

/// Adjustment lookup with the preset's documented defaults filled in.
struct Adjustments {
    values: HashMap<String, f64>,
}

impl Adjustments {
    fn new(preset: &str, supplied: &[(String, f64)]) -> Self {
        let mut values = HashMap::new();
        for (k, v) in default_adjustments(preset) {
            values.insert((*k).to_string(), *v);
        }
        for (k, v) in supplied {
            values.insert(k.clone(), *v);
        }
        Adjustments { values }
    }

    /// Adjustment as a 0..1 fraction, clamped, with `fallback` when absent.
    fn frac(&self, name: &str, fallback: f64) -> f32 {
        let raw = self.values.get(name).copied().unwrap_or(fallback);
        (raw / 100_000.0).clamp(0.0, 1.0) as f32
    }

    /// Adjustment kept as a signed fraction (callout offsets go negative).
    fn signed(&self, name: &str, fallback: f64) -> f32 {
        (self.values.get(name).copied().unwrap_or(fallback) / 100_000.0) as f32
    }

    /// Adjustment as an angle in radians (some presets store 60000ths of a degree).
    fn angle(&self, name: &str, fallback: f64) -> f32 {
        let raw = self.values.get(name).copied().unwrap_or(fallback);
        ((raw / 60_000.0) as f32).to_radians()
    }
}

struct Builder {
    w: f32,
    h: f32,
    adj: Adjustments,
}

impl Builder {
    fn ss(&self) -> f32 {
        self.w.min(self.h)
    }

    fn rect(&self) -> Rect {
        Rect::new(0.0, 0.0, self.w, self.h)
    }

    fn build(&self, preset: &str) -> Path {
        let (w, h) = (self.w, self.h);
        match preset {
            "rect" | "flowChartProcess" | "plaque" | "actionButtonBlank" => Path::rect(self.rect()),
            "ellipse" | "flowChartConnector" => Path::ellipse(self.rect()),
            "roundRect" | "flowChartAlternateProcess" => {
                let r = self.adj.frac("adj", 16667.0) * self.ss();
                self.round_rect([r, r, r, r])
            }
            "round1Rect" => {
                let r = self.adj.frac("adj", 16667.0) * self.ss();
                self.round_rect([0.0, r, 0.0, 0.0])
            }
            "round2SameRect" => {
                let r1 = self.adj.frac("adj1", 16667.0) * self.ss();
                let r2 = self.adj.frac("adj2", 0.0) * self.ss();
                self.round_rect([r1, r1, r2, r2])
            }
            "round2DiagRect" => {
                let r1 = self.adj.frac("adj1", 16667.0) * self.ss();
                let r2 = self.adj.frac("adj2", 0.0) * self.ss();
                self.round_rect([r1, r2, r1, r2])
            }
            "flowChartTerminator" => {
                let r = self.ss() * 0.5;
                self.round_rect([r, r, r, r])
            }
            "snip1Rect" => {
                let c = self.adj.frac("adj", 16667.0) * self.ss();
                self.snip_rect([0.0, c, 0.0, 0.0])
            }
            "snip2SameRect" => {
                let c1 = self.adj.frac("adj1", 16667.0) * self.ss();
                let c2 = self.adj.frac("adj2", 0.0) * self.ss();
                self.snip_rect([c1, c1, c2, c2])
            }
            "snip2DiagRect" => {
                let c1 = self.adj.frac("adj1", 16667.0) * self.ss();
                let c2 = self.adj.frac("adj2", 0.0) * self.ss();
                self.snip_rect([c1, c2, c1, c2])
            }
            "triangle" => {
                let a = self.adj.frac("adj", 50000.0);
                let mut p = Path::new();
                p.move_to(w * a, 0.0).line_to(w, h).line_to(0.0, h).close();
                p
            }
            "rtTriangle" => {
                let mut p = Path::new();
                p.move_to(0.0, 0.0).line_to(w, h).line_to(0.0, h).close();
                p
            }
            "diamond" => {
                let mut p = Path::new();
                p.move_to(w / 2.0, 0.0)
                    .line_to(w, h / 2.0)
                    .line_to(w / 2.0, h)
                    .line_to(0.0, h / 2.0)
                    .close();
                p
            }
            "flowChartDecision" => {
                let mut p = Path::new();
                p.move_to(w / 2.0, 0.0)
                    .line_to(w, h / 2.0)
                    .line_to(w / 2.0, h)
                    .line_to(0.0, h / 2.0)
                    .close();
                p
            }
            "parallelogram" | "flowChartInputOutput" => {
                let a = if preset == "parallelogram" {
                    self.adj.frac("adj", 25000.0)
                } else {
                    0.2
                };
                let dx = (a * self.ss()).min(w);
                let mut p = Path::new();
                p.move_to(dx, 0.0)
                    .line_to(w, 0.0)
                    .line_to(w - dx, h)
                    .line_to(0.0, h)
                    .close();
                p
            }
            "trapezoid" => {
                let dx = (self.adj.frac("adj", 25000.0) * self.ss()).min(w / 2.0);
                let mut p = Path::new();
                p.move_to(dx, 0.0)
                    .line_to(w - dx, 0.0)
                    .line_to(w, h)
                    .line_to(0.0, h)
                    .close();
                p
            }
            "pentagon" => self.regular_polygon(5, -PI / 2.0),
            "hexagon" => self.regular_polygon(6, 0.0),
            "heptagon" => self.regular_polygon(7, -PI / 2.0),
            "octagon" => self.regular_polygon(8, PI / 8.0),
            "decagon" => self.regular_polygon(10, 0.0),
            "star4" => self.star(4, self.adj.frac("adj", 12500.0)),
            "star5" => self.star(5, self.adj.frac("adj", 19098.0)),
            "star6" => self.star(6, self.adj.frac("adj", 28868.0)),
            "star8" => self.star(8, self.adj.frac("adj", 37500.0)),
            "star10" => self.star(10, self.adj.frac("adj", 42533.0)),
            "star12" => self.star(12, self.adj.frac("adj", 37500.0)),
            "plus" | "cross" => {
                let a = self.adj.frac("adj", 25000.0);
                let dx = w * a;
                let dy = h * a;
                let mut p = Path::new();
                p.move_to(dx, 0.0)
                    .line_to(w - dx, 0.0)
                    .line_to(w - dx, dy)
                    .line_to(w, dy)
                    .line_to(w, h - dy)
                    .line_to(w - dx, h - dy)
                    .line_to(w - dx, h)
                    .line_to(dx, h)
                    .line_to(dx, h - dy)
                    .line_to(0.0, h - dy)
                    .line_to(0.0, dy)
                    .line_to(dx, dy)
                    .close();
                p
            }
            "rightArrow" => self.arrow_h(false),
            "leftArrow" => self.arrow_h(true),
            "upArrow" => self.arrow_v(true),
            "downArrow" => self.arrow_v(false),
            "leftRightArrow" => {
                let shaft = self.adj.frac("adj1", 50000.0) * h;
                let head = (self.adj.frac("adj2", 50000.0) * w).min(w / 2.0);
                let y0 = (h - shaft) / 2.0;
                let y1 = y0 + shaft;
                let mut p = Path::new();
                p.move_to(0.0, h / 2.0)
                    .line_to(head, 0.0)
                    .line_to(head, y0)
                    .line_to(w - head, y0)
                    .line_to(w - head, 0.0)
                    .line_to(w, h / 2.0)
                    .line_to(w - head, h)
                    .line_to(w - head, y1)
                    .line_to(head, y1)
                    .line_to(head, h)
                    .close();
                p
            }
            "upDownArrow" => {
                let shaft = self.adj.frac("adj1", 50000.0) * w;
                let head = (self.adj.frac("adj2", 50000.0) * h).min(h / 2.0);
                let x0 = (w - shaft) / 2.0;
                let x1 = x0 + shaft;
                let mut p = Path::new();
                p.move_to(w / 2.0, 0.0)
                    .line_to(w, head)
                    .line_to(x1, head)
                    .line_to(x1, h - head)
                    .line_to(w, h - head)
                    .line_to(w / 2.0, h)
                    .line_to(0.0, h - head)
                    .line_to(x0, h - head)
                    .line_to(x0, head)
                    .line_to(0.0, head)
                    .close();
                p
            }
            "chevron" | "homePlate" => {
                let notch = (self.adj.frac("adj", 50000.0) * self.ss()).min(w);
                let mut p = Path::new();
                if preset == "homePlate" {
                    p.move_to(0.0, 0.0)
                        .line_to(w - notch, 0.0)
                        .line_to(w, h / 2.0)
                        .line_to(w - notch, h)
                        .line_to(0.0, h)
                        .close();
                } else {
                    p.move_to(0.0, 0.0)
                        .line_to(w - notch, 0.0)
                        .line_to(w, h / 2.0)
                        .line_to(w - notch, h)
                        .line_to(0.0, h)
                        .line_to(notch, h / 2.0)
                        .close();
                }
                p
            }
            "can" => {
                let ry = (self.adj.frac("adj", 25000.0) * h / 2.0).min(h / 2.0);
                let mut p = Path::new();
                // Body, then the visible top ellipse as a second subpath.
                p.move_to(0.0, ry);
                p.arc_to(w / 2.0, ry, w / 2.0, ry, PI, PI);
                p.line_to(w, h - ry);
                p.arc_to(w / 2.0, h - ry, w / 2.0, ry, 0.0, PI);
                p.close();
                let top = Path::ellipse(Rect::new(0.0, 0.0, w, ry * 2.0));
                append(p, &top)
            }
            "cube" => {
                let d = self.adj.frac("adj", 25000.0) * self.ss();
                let mut p = Path::new();
                p.move_to(0.0, d)
                    .line_to(d, 0.0)
                    .line_to(w, 0.0)
                    .line_to(w, h - d)
                    .line_to(w - d, h)
                    .line_to(0.0, h)
                    .close();
                let mut edges = Path::new();
                edges
                    .move_to(0.0, d)
                    .line_to(w - d, d)
                    .line_to(w, 0.0)
                    .move_to(w - d, d)
                    .line_to(w - d, h);
                append(p, &edges)
            }
            "donut" | "noSmoking" => {
                let t = self.adj.frac("adj", 25000.0) * self.ss();
                let outer = Path::ellipse(self.rect());
                let inner = Path::ellipse(Rect::new(
                    t,
                    t,
                    (w - 2.0 * t).max(0.0),
                    (h - 2.0 * t).max(0.0),
                ));
                append(outer, &inner)
            }
            "frame" => {
                let t = self.adj.frac("adj1", 12500.0) * self.ss();
                let outer = Path::rect(self.rect());
                let inner = Path::rect(Rect::new(
                    t,
                    t,
                    (w - 2.0 * t).max(0.0),
                    (h - 2.0 * t).max(0.0),
                ));
                append(outer, &inner)
            }
            "halfFrame" => {
                let t = self.adj.frac("adj1", 33333.0) * self.ss();
                let mut p = Path::new();
                p.move_to(0.0, 0.0)
                    .line_to(w, 0.0)
                    .line_to(w - t, t)
                    .line_to(t, t)
                    .line_to(t, h - t)
                    .line_to(0.0, h)
                    .close();
                p
            }
            "corner" => {
                let tx = self.adj.frac("adj1", 50000.0) * w;
                let ty = self.adj.frac("adj2", 50000.0) * h;
                let mut p = Path::new();
                p.move_to(0.0, 0.0)
                    .line_to(tx, 0.0)
                    .line_to(tx, h - ty)
                    .line_to(w, h - ty)
                    .line_to(w, h)
                    .line_to(0.0, h)
                    .close();
                p
            }
            "diagStripe" => {
                let a = self.adj.frac("adj", 50000.0);
                let mut p = Path::new();
                p.move_to(0.0, h * a)
                    .line_to(w * a, 0.0)
                    .line_to(w, 0.0)
                    .line_to(0.0, h)
                    .close();
                p
            }
            "bevel" => {
                let t = self.adj.frac("adj", 12500.0) * self.ss();
                let outer = Path::rect(self.rect());
                let inner = Path::rect(Rect::new(
                    t,
                    t,
                    (w - 2.0 * t).max(0.0),
                    (h - 2.0 * t).max(0.0),
                ));
                append(outer, &inner)
            }
            "line" | "straightConnector1" => {
                let mut p = Path::new();
                p.move_to(0.0, 0.0).line_to(w, h);
                p
            }
            "bentConnector2" => {
                let mut p = Path::new();
                p.move_to(0.0, 0.0).line_to(w, 0.0).line_to(w, h);
                p
            }
            "bentConnector3" => {
                let mid = w * self.adj.frac("adj1", 50000.0);
                let mut p = Path::new();
                p.move_to(0.0, 0.0)
                    .line_to(mid, 0.0)
                    .line_to(mid, h)
                    .line_to(w, h);
                p
            }
            "curvedConnector2" => {
                let mut p = Path::new();
                p.move_to(0.0, 0.0).cubic_to(w / 2.0, 0.0, w, h / 2.0, w, h);
                p
            }
            "curvedConnector3" => {
                let mid = w * self.adj.frac("adj1", 50000.0);
                let mut p = Path::new();
                p.move_to(0.0, 0.0).cubic_to(mid, 0.0, mid, h, w, h);
                p
            }
            "flowChartDocument" => {
                let wave = h * 0.15;
                let mut p = Path::new();
                p.move_to(0.0, 0.0)
                    .line_to(w, 0.0)
                    .line_to(w, h - wave)
                    .cubic_to(w * 0.75, h, w * 0.25, h - wave * 2.0, 0.0, h - wave)
                    .close();
                p
            }
            "flowChartPredefinedProcess" => {
                let inset = w * 0.125;
                let outer = Path::rect(self.rect());
                let mut bars = Path::new();
                bars.move_to(inset, 0.0)
                    .line_to(inset, h)
                    .move_to(w - inset, 0.0)
                    .line_to(w - inset, h);
                append(outer, &bars)
            }
            "pie" | "arc" | "chord" | "blockArc" => self.pie_family(preset),
            "teardrop" => {
                // A circle whose top-right quadrant is pulled out into a corner.
                let a = self.adj.frac("adj", 100000.0);
                let cx = w / 2.0;
                let cy = h / 2.0;
                let mut p = Path::new();
                p.move_to(0.0, cy);
                p.arc_to(cx, cy, cx, cy, PI, PI / 2.0);
                p.line_to(w * (0.5 + 0.5 * a), h * (0.5 - 0.5 * a));
                p.line_to(w, cy);
                p.arc_to(cx, cy, cx, cy, 0.0, PI);
                p.close();
                p
            }
            "leftBrace" | "rightBrace" | "leftBracket" | "rightBracket" => self.brace(preset),
            "wedgeRectCallout" => {
                let dx = w * (0.5 + self.adj.signed("adj1", -20833.0));
                let dy = h * (0.5 + self.adj.signed("adj2", 62500.0));
                let mut p = Path::rect(self.rect());
                let mut tail = Path::new();
                tail.move_to(w * 0.25, h)
                    .line_to(dx, dy)
                    .line_to(w * 0.45, h)
                    .close();
                p = append(p, &tail);
                p
            }
            "heart" => {
                // Control points are kept inside the unit box on purpose: a heart drawn
                // with overshooting handles reads the same but reports bounds larger than
                // the shape, which then breaks culling and gradient sizing.
                let (x, y) = (|f: f32| f * w, |f: f32| f * h);
                let mut p = Path::new();
                p.move_to(x(0.5), y(0.28));
                p.cubic_to(x(0.5), y(0.10), x(0.30), y(0.0), x(0.16), y(0.0));
                p.cubic_to(x(0.0), y(0.0), x(0.0), y(0.22), x(0.0), y(0.30));
                p.cubic_to(x(0.0), y(0.55), x(0.30), y(0.72), x(0.5), y(1.0));
                p.cubic_to(x(0.70), y(0.72), x(1.0), y(0.55), x(1.0), y(0.30));
                p.cubic_to(x(1.0), y(0.22), x(1.0), y(0.0), x(0.84), y(0.0));
                p.cubic_to(x(0.70), y(0.0), x(0.5), y(0.10), x(0.5), y(0.28));
                p.close();
                p
            }
            "moon" => {
                let a = self.adj.frac("adj", 50000.0).max(0.05);
                let mut p = Path::new();
                p.move_to(w, 0.0);
                p.cubic_to(
                    w * (1.0 - a * 2.0),
                    h * 0.15,
                    w * (1.0 - a * 2.0),
                    h * 0.85,
                    w,
                    h,
                );
                p.cubic_to(w * 0.1, h * 0.85, w * 0.1, h * 0.15, w, 0.0);
                p.close();
                p
            }
            "sun" => self.sun(),
            "cloud" => self.cloud(),
            "smileyFace" => {
                let face = Path::ellipse(self.rect());
                let eye_r = self.ss() * 0.06;
                let left = Path::ellipse(Rect::new(
                    w * 0.3 - eye_r,
                    h * 0.32 - eye_r,
                    eye_r * 2.0,
                    eye_r * 2.0,
                ));
                let right = Path::ellipse(Rect::new(
                    w * 0.7 - eye_r,
                    h * 0.32 - eye_r,
                    eye_r * 2.0,
                    eye_r * 2.0,
                ));
                let mut mouth = Path::new();
                mouth.move_to(w * 0.28, h * 0.6);
                mouth.cubic_to(w * 0.4, h * 0.82, w * 0.6, h * 0.82, w * 0.72, h * 0.6);
                append(append(append(face, &left), &right), &mouth)
            }
            "lightningBolt" => {
                let mut p = Path::new();
                p.move_to(w * 0.44, 0.0)
                    .line_to(w * 0.15, h * 0.44)
                    .line_to(w * 0.41, h * 0.44)
                    .line_to(w * 0.24, h)
                    .line_to(w * 0.85, h * 0.42)
                    .line_to(w * 0.56, h * 0.42)
                    .line_to(w * 0.82, 0.0)
                    .close();
                p
            }
            _ => {
                log::debug!(
                    "preset {preset:?} has no path; falling back to its bounding rectangle"
                );
                Path::rect(self.rect())
            }
        }
    }

    /// Corner radii in [top-left, top-right, bottom-right, bottom-left] order.
    fn round_rect(&self, r: [f32; 4]) -> Path {
        let (w, h) = (self.w, self.h);
        let cap = |v: f32| v.max(0.0).min(w / 2.0).min(h / 2.0);
        let (tl, tr, br, bl) = (cap(r[0]), cap(r[1]), cap(r[2]), cap(r[3]));
        let mut p = Path::new();
        p.move_to(tl, 0.0);
        p.line_to(w - tr, 0.0);
        if tr > 0.0 {
            p.arc_to(w - tr, tr, tr, tr, -PI / 2.0, PI / 2.0);
        }
        p.line_to(w, h - br);
        if br > 0.0 {
            p.arc_to(w - br, h - br, br, br, 0.0, PI / 2.0);
        }
        p.line_to(bl, h);
        if bl > 0.0 {
            p.arc_to(bl, h - bl, bl, bl, PI / 2.0, PI / 2.0);
        }
        p.line_to(0.0, tl);
        if tl > 0.0 {
            p.arc_to(tl, tl, tl, tl, PI, PI / 2.0);
        }
        p.close();
        p
    }

    fn snip_rect(&self, c: [f32; 4]) -> Path {
        let (w, h) = (self.w, self.h);
        let cap = |v: f32| v.max(0.0).min(w / 2.0).min(h / 2.0);
        let (tl, tr, br, bl) = (cap(c[0]), cap(c[1]), cap(c[2]), cap(c[3]));
        let mut p = Path::new();
        p.move_to(tl, 0.0)
            .line_to(w - tr, 0.0)
            .line_to(w, tr)
            .line_to(w, h - br)
            .line_to(w - br, h)
            .line_to(bl, h)
            .line_to(0.0, h - bl)
            .line_to(0.0, tl)
            .close();
        p
    }

    /// A regular n-gon inscribed in the box, starting at `start` radians.
    fn regular_polygon(&self, n: usize, start: f32) -> Path {
        let (cx, cy) = (self.w / 2.0, self.h / 2.0);
        let (rx, ry) = (self.w / 2.0, self.h / 2.0);
        let mut p = Path::new();
        for i in 0..n {
            let t = start + (i as f32) * 2.0 * PI / n as f32;
            let (x, y) = (cx + rx * t.cos(), cy + ry * t.sin());
            if i == 0 {
                p.move_to(x, y);
            } else {
                p.line_to(x, y);
            }
        }
        p.close();
        p
    }

    /// An n-pointed star whose inner radius is `inner` of the outer.
    fn star(&self, points: usize, inner: f32) -> Path {
        let (cx, cy) = (self.w / 2.0, self.h / 2.0);
        let (rx, ry) = (self.w / 2.0, self.h / 2.0);
        let inner = inner.clamp(0.05, 0.95);
        let mut p = Path::new();
        for i in 0..points * 2 {
            let t = -PI / 2.0 + (i as f32) * PI / points as f32;
            let scale = if i % 2 == 0 { 1.0 } else { inner };
            let (x, y) = (cx + rx * scale * t.cos(), cy + ry * scale * t.sin());
            if i == 0 {
                p.move_to(x, y);
            } else {
                p.line_to(x, y);
            }
        }
        p.close();
        p
    }

    fn arrow_h(&self, left: bool) -> Path {
        let (w, h) = (self.w, self.h);
        let shaft = self.adj.frac("adj1", 50000.0) * h;
        let head = (self.adj.frac("adj2", 50000.0) * w).min(w);
        let y0 = (h - shaft) / 2.0;
        let y1 = y0 + shaft;
        let mut p = Path::new();
        if left {
            p.move_to(0.0, h / 2.0)
                .line_to(head, 0.0)
                .line_to(head, y0)
                .line_to(w, y0)
                .line_to(w, y1)
                .line_to(head, y1)
                .line_to(head, h)
                .close();
        } else {
            p.move_to(0.0, y0)
                .line_to(w - head, y0)
                .line_to(w - head, 0.0)
                .line_to(w, h / 2.0)
                .line_to(w - head, h)
                .line_to(w - head, y1)
                .line_to(0.0, y1)
                .close();
        }
        p
    }

    fn arrow_v(&self, up: bool) -> Path {
        let (w, h) = (self.w, self.h);
        let shaft = self.adj.frac("adj1", 50000.0) * w;
        let head = (self.adj.frac("adj2", 50000.0) * h).min(h);
        let x0 = (w - shaft) / 2.0;
        let x1 = x0 + shaft;
        let mut p = Path::new();
        if up {
            p.move_to(w / 2.0, 0.0)
                .line_to(w, head)
                .line_to(x1, head)
                .line_to(x1, h)
                .line_to(x0, h)
                .line_to(x0, head)
                .line_to(0.0, head)
                .close();
        } else {
            p.move_to(x0, 0.0)
                .line_to(x1, 0.0)
                .line_to(x1, h - head)
                .line_to(w, h - head)
                .line_to(w / 2.0, h)
                .line_to(0.0, h - head)
                .line_to(x0, h - head)
                .close();
        }
        p
    }

    fn pie_family(&self, preset: &str) -> Path {
        let (w, h) = (self.w, self.h);
        let (cx, cy) = (w / 2.0, h / 2.0);
        let (rx, ry) = (w / 2.0, h / 2.0);
        let start = self.adj.angle("adj1", 0.0);
        let end = self.adj.angle("adj2", 16_200_000.0);
        let sweep = {
            let s = end - start;
            if s.abs() < 1e-6 {
                2.0 * PI
            } else {
                s
            }
        };
        let mut p = Path::new();
        match preset {
            "arc" => {
                p.arc_to(cx, cy, rx, ry, start, sweep);
            }
            "chord" => {
                p.arc_to(cx, cy, rx, ry, start, sweep);
                p.close();
            }
            "blockArc" => {
                let t = self.adj.frac("adj3", 25000.0).max(0.01);
                let (irx, iry) = (rx * (1.0 - t), ry * (1.0 - t));
                p.arc_to(cx, cy, rx, ry, start, sweep);
                p.line_to(
                    cx + irx * (start + sweep).cos(),
                    cy + iry * (start + sweep).sin(),
                );
                p.arc_to(cx, cy, irx, iry, start + sweep, -sweep);
                p.close();
            }
            // "pie" and anything else: a wedge back through the centre.
            _ => {
                p.move_to(cx, cy);
                p.line_to(cx + rx * start.cos(), cy + ry * start.sin());
                p.arc_to(cx, cy, rx, ry, start, sweep);
                p.close();
            }
        }
        p
    }

    fn brace(&self, preset: &str) -> Path {
        let (w, h) = (self.w, self.h);
        let mut p = Path::new();
        match preset {
            "leftBracket" => {
                p.move_to(w, 0.0)
                    .line_to(0.0, 0.0)
                    .line_to(0.0, h)
                    .line_to(w, h);
            }
            "rightBracket" => {
                p.move_to(0.0, 0.0)
                    .line_to(w, 0.0)
                    .line_to(w, h)
                    .line_to(0.0, h);
            }
            "leftBrace" => {
                p.move_to(w, 0.0);
                p.cubic_to(w * 0.5, 0.0, w * 0.5, h * 0.2, w * 0.5, h * 0.5);
                p.cubic_to(w * 0.5, h * 0.5, 0.0, h * 0.5, 0.0, h * 0.5);
                p.cubic_to(w * 0.5, h * 0.5, w * 0.5, h * 0.5, w * 0.5, h * 0.5);
                p.cubic_to(w * 0.5, h * 0.8, w * 0.5, h, w, h);
            }
            // "rightBrace"
            _ => {
                p.move_to(0.0, 0.0);
                p.cubic_to(w * 0.5, 0.0, w * 0.5, h * 0.2, w * 0.5, h * 0.5);
                p.cubic_to(w * 0.5, h * 0.5, w, h * 0.5, w, h * 0.5);
                p.cubic_to(w * 0.5, h * 0.5, w * 0.5, h * 0.5, w * 0.5, h * 0.5);
                p.cubic_to(w * 0.5, h * 0.8, w * 0.5, h, 0.0, h);
            }
        }
        p
    }

    fn sun(&self) -> Path {
        let (w, h) = (self.w, self.h);
        let (cx, cy) = (w / 2.0, h / 2.0);
        let core = 0.28;
        let mut p = Path::ellipse(Rect::new(
            cx - w * core,
            cy - h * core,
            w * core * 2.0,
            h * core * 2.0,
        ));
        let mut rays = Path::new();
        for i in 0..8 {
            let t = i as f32 * PI / 4.0;
            let (c, s) = (t.cos(), t.sin());
            let inner = (cx + w * core * c, cy + h * core * s);
            let outer = (cx + w * 0.5 * c, cy + h * 0.5 * s);
            let n = (-s * w * 0.06, c * h * 0.06);
            rays.move_to(inner.0 + n.0, inner.1 + n.1)
                .line_to(outer.0, outer.1)
                .line_to(inner.0 - n.0, inner.1 - n.1)
                .close();
        }
        p = append(p, &rays);
        p
    }

    fn cloud(&self) -> Path {
        let (w, h) = (self.w, self.h);
        // Five overlapping ellipses, which is what the real preset is under its guides.
        let bumps = [
            (0.22, 0.62, 0.24, 0.38),
            (0.35, 0.36, 0.26, 0.34),
            (0.58, 0.30, 0.28, 0.36),
            (0.78, 0.52, 0.24, 0.34),
            (0.55, 0.70, 0.34, 0.34),
        ];
        let mut p = Path::new();
        for (cx, cy, rx, ry) in bumps {
            let e = Path::ellipse(Rect::new(
                (cx - rx / 2.0) * w,
                (cy - ry / 2.0) * h,
                rx * w,
                ry * h,
            ));
            p = append(p, &e);
        }
        p
    }
}

/// Concatenates two paths into one multi-subpath path.
fn append(mut base: Path, other: &Path) -> Path {
    base.verbs.extend_from_slice(&other.verbs);
    base.points.extend_from_slice(&other.points);
    base
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds_within(p: &Path, w: f32, h: f32, slack: f32) -> bool {
        let b = p.bounds();
        b.x >= -slack && b.y >= -slack && b.right() <= w + slack && b.bottom() <= h + slack
    }

    /// Callouts are the one family whose geometry legitimately leaves the shape box:
    /// the tail points at whatever the callout is labelling, which is elsewhere.
    fn escapes_by_design(preset: &str) -> bool {
        preset.ends_with("Callout")
    }

    #[test]
    fn every_supported_preset_produces_a_path_inside_its_box() {
        for preset in SUPPORTED {
            let p = build(preset, 200.0, 100.0, &[]);
            assert!(!p.is_empty(), "{preset} produced an empty path");
            if escapes_by_design(preset) {
                continue;
            }
            assert!(
                bounds_within(&p, 200.0, 100.0, 1.0),
                "{preset} escaped its box: {:?}",
                p.bounds()
            );
        }
    }

    #[test]
    fn a_callout_tail_reaches_outside_the_shape_box() {
        let p = build("wedgeRectCallout", 200.0, 100.0, &[]);
        assert!(
            p.bounds().bottom() > 100.0,
            "the tail should point below the box: {:?}",
            p.bounds()
        );
    }

    #[test]
    fn an_unknown_preset_falls_back_to_its_bounding_rectangle() {
        let p = build("someShapeFromTheFuture", 50.0, 30.0, &[]);
        assert_eq!(p, Path::rect(Rect::new(0.0, 0.0, 50.0, 30.0)));
        assert!(!is_supported("someShapeFromTheFuture"));
        assert!(is_supported("roundRect"));
    }

    #[test]
    fn round_rect_radius_follows_the_adjustment() {
        let square = build("roundRect", 100.0, 100.0, &[("adj".into(), 0.0)]);
        // A zero radius degenerates to a rectangle's four corners plus the close.
        assert!(
            square.verbs.len() <= 6,
            "zero-radius roundRect should be a rectangle"
        );
        let rounded = build("roundRect", 100.0, 100.0, &[("adj".into(), 50000.0)]);
        assert!(rounded.verbs.len() > square.verbs.len());
    }

    #[test]
    fn round_rect_radius_is_capped_at_half_the_shorter_side() {
        // An adjustment of 100% on a wide box must not produce a self-crossing path.
        let p = build("roundRect", 400.0, 40.0, &[("adj".into(), 100_000.0)]);
        assert!(bounds_within(&p, 400.0, 40.0, 0.5));
    }

    #[test]
    fn triangle_apex_moves_with_its_adjustment() {
        let centred = build("triangle", 100.0, 50.0, &[("adj".into(), 50000.0)]);
        let left = build("triangle", 100.0, 50.0, &[("adj".into(), 0.0)]);
        assert_eq!(centred.points.first().map(|p| p.x), Some(50.0));
        assert_eq!(left.points.first().map(|p| p.x), Some(0.0));
    }

    #[test]
    fn arrow_heads_respect_the_shaft_and_head_adjustments() {
        let thin = build(
            "rightArrow",
            100.0,
            100.0,
            &[("adj1".into(), 20000.0), ("adj2".into(), 50000.0)],
        );
        let thick = build(
            "rightArrow",
            100.0,
            100.0,
            &[("adj1".into(), 80000.0), ("adj2".into(), 50000.0)],
        );
        // The shaft's top edge sits lower on the thin arrow.
        let thin_y = thin.points.first().map(|p| p.y).unwrap_or_default();
        let thick_y = thick.points.first().map(|p| p.y).unwrap_or_default();
        assert!(thin_y > thick_y, "thin={thin_y} thick={thick_y}");
    }

    #[test]
    fn donut_and_frame_emit_two_subpaths_so_the_hole_can_be_cut() {
        for preset in ["donut", "frame"] {
            let p = build(preset, 100.0, 100.0, &[]);
            let moves = p
                .verbs
                .iter()
                .filter(|v| **v == crate::dl::PathVerb::MoveTo)
                .count();
            assert!(
                moves >= 2,
                "{preset} needs an inner subpath, got {moves} MoveTo verbs"
            );
        }
    }

    #[test]
    fn a_star_alternates_between_two_radii() {
        let p = build("star5", 100.0, 100.0, &[]);
        // 5 points = 10 vertices.
        assert_eq!(p.points.len(), 10);
        let cx = 50.0;
        let cy = 50.0;
        let radius = |i: usize| {
            let pt = p.points.get(i).copied().unwrap_or_default();
            ((pt.x - cx).powi(2) + (pt.y - cy).powi(2)).sqrt()
        };
        assert!(
            radius(0) > radius(1),
            "outer point should be further than inner"
        );
    }

    #[test]
    fn degenerate_sizes_do_not_produce_nan_coordinates() {
        for preset in SUPPORTED {
            for (w, h) in [(0.0, 0.0), (0.0, 100.0), (100.0, 0.0), (-5.0, 10.0)] {
                let p = build(preset, w, h, &[]);
                assert!(
                    p.points
                        .iter()
                        .all(|pt| pt.x.is_finite() && pt.y.is_finite()),
                    "{preset} at {w}x{h} produced a non-finite point"
                );
            }
        }
    }

    #[test]
    fn out_of_range_adjustments_are_clamped() {
        for adj in [-500_000.0, 0.0, 999_999.0] {
            let p = build("roundRect", 100.0, 60.0, &[("adj".into(), adj)]);
            assert!(
                bounds_within(&p, 100.0, 60.0, 0.5),
                "adj={adj} escaped the box"
            );
        }
    }

    #[test]
    fn line_presets_are_open_paths() {
        let p = build("line", 100.0, 100.0, &[]);
        assert!(!p.verbs.contains(&crate::dl::PathVerb::Close));
        assert_eq!(p.points.first().map(|p| (p.x, p.y)), Some((0.0, 0.0)));
        assert_eq!(p.points.last().map(|p| (p.x, p.y)), Some((100.0, 100.0)));
    }

    #[test]
    fn pie_defaults_to_a_full_circle_when_its_angles_are_equal() {
        let p = build(
            "pie",
            100.0,
            100.0,
            &[("adj1".into(), 0.0), ("adj2".into(), 0.0)],
        );
        assert!(!p.is_empty());
        assert!(bounds_within(&p, 100.0, 100.0, 0.5));
    }
}
