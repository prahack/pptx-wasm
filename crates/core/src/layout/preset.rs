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
use std::f32::consts::{FRAC_1_SQRT_2, PI};

use crate::dl::{Path, Rect};
use crate::model::geometry::{default_adjustments, PathFillMode};

/// Builds a preset shape's path in a box of `w` x `h` points with its origin at 0,0.
/// One drawable face of a preset, with the shading DrawingML asks for.
#[derive(Debug, Clone)]
pub struct Face {
    pub path: Path,
    pub fill: PathFillMode,
    pub stroke: bool,
}

/// A preset's faces, in paint order.
///
/// Almost every preset is one face painted with the shape's fill. The 3-D-looking ones are
/// not: ECMA-376 gives `can` a `fill="lighten"` lid and `cube` a lightened top and a
/// darkened side, so that one solid fill reads as a lit solid. Collapsing those to a
/// single path is what made the lid and the top face come out unpainted — a white hole
/// where the light face should be, which is far more visible than a shade being slightly
/// off.
pub fn faces(preset: &str, w: f32, h: f32, adjustments: &[(String, f64)]) -> Vec<Face> {
    let adj = Adjustments::new(preset, adjustments);
    let b = Builder { w, h, adj };
    match preset {
        "can" => {
            let ry = b.can_ry();
            let mut body = Path::new();
            body.move_to(0.0, ry);
            body.arc_to(w / 2.0, ry, w / 2.0, ry, PI, PI);
            body.line_to(w, h - ry);
            body.arc_to(w / 2.0, h - ry, w / 2.0, ry, 0.0, PI);
            body.close();
            let lid = Path::ellipse(Rect::new(0.0, 0.0, w, ry * 2.0));
            vec![
                Face {
                    path: body,
                    fill: PathFillMode::Normal,
                    stroke: true,
                },
                Face {
                    path: lid,
                    fill: PathFillMode::Lighten,
                    stroke: true,
                },
            ]
        }
        // Cylinders and the document stack are several closed outlines that overlap.
        // Concatenated into one path they cancel under the non-zero rule and come out as
        // white holes, so each is its own face and each is filled in its own right.
        "flowChartMagneticDisk" => {
            let ry = h / 6.0;
            let mut body = Path::new();
            body.move_to(0.0, ry);
            body.arc_to(w / 2.0, ry, w / 2.0, ry, PI, PI);
            body.line_to(w, h - ry);
            body.arc_to(w / 2.0, h - ry, w / 2.0, ry, 0.0, PI);
            body.close();
            let lid = Path::ellipse(Rect::new(0.0, 0.0, w, ry * 2.0));
            vec![
                Face {
                    path: body,
                    fill: PathFillMode::Normal,
                    stroke: true,
                },
                Face {
                    path: lid,
                    fill: PathFillMode::Normal,
                    stroke: true,
                },
            ]
        }
        "flowChartMagneticDrum" => {
            let rx = w / 6.0;
            let mut body = Path::new();
            body.move_to(rx, 0.0);
            body.arc_to(rx, h / 2.0, rx, h / 2.0, -PI / 2.0, -PI);
            body.line_to(w - rx, h);
            body.arc_to(w - rx, h / 2.0, rx, h / 2.0, PI / 2.0, -PI);
            body.close();
            let cap = Path::ellipse(Rect::new(w - rx * 2.0, 0.0, rx * 2.0, h));
            vec![
                Face {
                    path: body,
                    fill: PathFillMode::Normal,
                    stroke: true,
                },
                Face {
                    path: cap,
                    fill: PathFillMode::Normal,
                    stroke: true,
                },
            ]
        }
        "flowChartMultidocument" => {
            let (ox, oy) = (w * 0.08, h * 0.12);
            let (bw, bh) = (w - ox * 2.0, h - oy * 2.0);
            let doc = |x: f32, y: f32| -> Path {
                let wave = bh * 0.14;
                let mut d = Path::new();
                d.move_to(x, y)
                    .line_to(x + bw, y)
                    .line_to(x + bw, y + bh - wave)
                    .cubic_to(
                        x + bw * 0.75,
                        y + bh,
                        x + bw * 0.25,
                        y + bh - wave * 2.0,
                        x,
                        y + bh - wave,
                    );
                d.close();
                d
            };
            // Back to front, so the nearest sheet overlaps those behind it.
            [(ox * 2.0, 0.0), (ox, oy), (0.0, oy * 2.0)]
                .into_iter()
                .map(|(x, y)| Face {
                    path: doc(x, y),
                    fill: PathFillMode::Normal,
                    stroke: true,
                })
                .collect()
        }
        // Action buttons: a bevelled plate with a darkened glyph on it. The bevel and the
        // glyph are separate faces because both are shaded relative to the shape's own
        // fill — the same mechanism `cube` uses — so a button in any theme colour still
        // reads as a raised button rather than a flat rectangle with a hole in it.
        p if p.starts_with("actionButton") => {
            let ss = w.min(h);
            let d = ss * 0.06;
            let mut faces = vec![Face {
                path: Path::rect(Rect::new(0.0, 0.0, w, h)),
                fill: PathFillMode::Normal,
                stroke: true,
            }];
            let quad = |a: (f32, f32), b: (f32, f32), c: (f32, f32), e: (f32, f32)| {
                let mut q = Path::new();
                q.move_to(a.0, a.1)
                    .line_to(b.0, b.1)
                    .line_to(c.0, c.1)
                    .line_to(e.0, e.1)
                    .close();
                q
            };
            // Lit from the top-left, as every desktop bevel has been since 1984, and
            // with the *Less* shades the spec specifies — a highlight, not a slab.
            // LibreOffice draws these buttons flat, so this suite cannot be scored
            // against it; see suites.json.
            for (path, mode) in [
                (
                    quad((0.0, 0.0), (w, 0.0), (w - d, d), (d, d)),
                    PathFillMode::Lighten,
                ),
                (
                    quad((0.0, 0.0), (d, d), (d, h - d), (0.0, h)),
                    PathFillMode::Lighten,
                ),
                (
                    quad((0.0, h), (d, h - d), (w - d, h - d), (w, h)),
                    PathFillMode::Darken,
                ),
                (
                    quad((w, 0.0), (w, h), (w - d, h - d), (w - d, d)),
                    PathFillMode::Darken,
                ),
            ] {
                faces.push(Face {
                    path,
                    fill: mode,
                    stroke: false,
                });
            }
            if let Some(glyph) = action_button_glyph(p, w, h, ss) {
                faces.push(Face {
                    path: glyph,
                    fill: PathFillMode::Darken,
                    stroke: false,
                });
            }
            faces
        }
        "cube" => {
            let d = b.cube_d();
            let (x4, y4) = (w - d, h - d);
            let mut front = Path::new();
            front
                .move_to(0.0, d)
                .line_to(0.0, h)
                .line_to(x4, h)
                .line_to(x4, d)
                .close();
            let mut top = Path::new();
            top.move_to(0.0, d)
                .line_to(d, 0.0)
                .line_to(w, 0.0)
                .line_to(x4, d)
                .close();
            let mut side = Path::new();
            side.move_to(x4, d)
                .line_to(w, 0.0)
                .line_to(w, y4)
                .line_to(x4, h)
                .close();
            vec![
                Face {
                    path: front,
                    fill: PathFillMode::Normal,
                    stroke: true,
                },
                Face {
                    path: top,
                    fill: PathFillMode::Lighten,
                    stroke: true,
                },
                Face {
                    path: side,
                    fill: PathFillMode::Darken,
                    stroke: true,
                },
            ]
        }
        _ => vec![Face {
            path: build(preset, w, h, adjustments),
            fill: PathFillMode::Normal,
            stroke: true,
        }],
    }
}

/// The symbol on an action button, in the centred square the spec reserves for it.
///
/// Coordinates are written in a 0..1 box and mapped once, which keeps each glyph readable
/// as the picture it is meant to be. `actionButtonBlank` has no glyph and returns `None`.
fn action_button_glyph(preset: &str, w: f32, h: f32, ss: f32) -> Option<Path> {
    let side = ss * 0.5;
    let (ox, oy) = ((w - side) / 2.0, (h - side) / 2.0);
    let x = |u: f32| ox + u * side;
    let y = |v: f32| oy + v * side;

    let poly = |pts: &[(f32, f32)]| -> Path {
        let mut p = Path::new();
        for (i, &(u, v)) in pts.iter().enumerate() {
            if i == 0 {
                p.move_to(x(u), y(v));
            } else {
                p.line_to(x(u), y(v));
            }
        }
        p.close();
        p
    };

    Some(match preset {
        "actionButtonBlank" => return None,
        "actionButtonForwardNext" => poly(&[(0.2, 0.05), (0.85, 0.5), (0.2, 0.95)]),
        "actionButtonBackPrevious" => poly(&[(0.8, 0.05), (0.15, 0.5), (0.8, 0.95)]),
        "actionButtonBeginning" => {
            let bar = poly(&[(0.08, 0.05), (0.24, 0.05), (0.24, 0.95), (0.08, 0.95)]);
            append(bar, &poly(&[(0.92, 0.05), (0.3, 0.5), (0.92, 0.95)]))
        }
        "actionButtonEnd" => {
            let bar = poly(&[(0.76, 0.05), (0.92, 0.05), (0.92, 0.95), (0.76, 0.95)]);
            append(bar, &poly(&[(0.08, 0.05), (0.7, 0.5), (0.08, 0.95)]))
        }
        "actionButtonDocument" => {
            // A sheet with its top-right corner turned down.
            poly(&[
                (0.22, 0.03),
                (0.62, 0.03),
                (0.78, 0.19),
                (0.78, 0.97),
                (0.22, 0.97),
            ])
        }
        "actionButtonHome" => {
            // Wide body, or the roof and a narrow box together just read as an arrow.
            let roof = poly(&[(0.5, 0.04), (0.98, 0.47), (0.02, 0.47)]);
            let body = poly(&[(0.16, 0.47), (0.84, 0.47), (0.84, 0.97), (0.16, 0.97)]);
            // Wound the opposite way, so the non-zero rule cuts it out as the doorway.
            let door = poly(&[(0.42, 0.97), (0.42, 0.70), (0.58, 0.70), (0.58, 0.97)]);
            append(append(roof, &body), &door)
        }
        "actionButtonSound" => {
            // A cone speaker and three radiating ticks.
            let cone = poly(&[
                (0.05, 0.35),
                (0.28, 0.35),
                (0.55, 0.05),
                (0.55, 0.95),
                (0.28, 0.65),
                (0.05, 0.65),
            ]);
            // Filled slivers rather than lines: this face is drawn with stroking off,
            // so a zero-width path would contribute nothing at all.
            let t = 0.035;
            let mut ticks = Path::new();
            for (v0, v1) in [(0.20_f32, 0.06_f32), (0.5, 0.5), (0.80, 0.94)] {
                let sliver = poly(&[
                    (0.66, v0 - t),
                    (0.97, v1 - t),
                    (0.97, v1 + t),
                    (0.66, v0 + t),
                ]);
                ticks = append(ticks, &sliver);
            }
            append(cone, &ticks)
        }
        "actionButtonReturn" => {
            // A U-turn: down the left leg, round the bottom bend, up the right leg and
            // out into an arrowhead. Traced as one outline — inner edge down, bend,
            // inner edge up, head, then the outer edge all the way back — so the ribbon
            // has constant width and no subpath can cancel another.
            let (cx, cy) = (x(0.45), y(0.62));
            let (ri, ro) = (0.07 * side, 0.23 * side);
            let mut p = Path::new();
            p.move_to(x(0.22), y(0.28));
            p.line_to(x(0.38), y(0.28));
            p.line_to(x(0.38), y(0.62));
            p.arc_to(cx, cy, ri, ri, PI, -PI);
            p.line_to(x(0.52), y(0.30));
            p.line_to(x(0.44), y(0.30));
            p.line_to(x(0.60), y(0.03));
            p.line_to(x(0.76), y(0.30));
            p.line_to(x(0.68), y(0.30));
            p.line_to(x(0.68), y(0.62));
            p.arc_to(cx, cy, ro, ro, 0.0, PI);
            p.close();
            p
        }
        "actionButtonInformation" => {
            // A disc with a lowercase i punched out of it: the counter is a second
            // subpath, and the non-zero rule cuts it because it winds the other way.
            let disc = Path::ellipse(Rect::new(x(0.02), y(0.02), side * 0.96, side * 0.96));
            let stem = poly(&[(0.42, 0.40), (0.58, 0.40), (0.58, 0.82), (0.42, 0.82)]);
            let dot = poly(&[(0.42, 0.18), (0.58, 0.18), (0.58, 0.32), (0.42, 0.32)]);
            append(append(disc, &stem), &dot)
        }
        "actionButtonHelp" => {
            // A question mark: hook, stem and point.
            let mut p = Path::new();
            p.move_to(x(0.28), y(0.30));
            p.cubic_to(x(0.28), y(-0.02), x(0.86), y(0.02), x(0.72), y(0.36));
            p.cubic_to(x(0.66), y(0.50), x(0.55), y(0.52), x(0.55), y(0.68));
            p.line_to(x(0.42), y(0.68));
            p.cubic_to(x(0.42), y(0.46), x(0.56), y(0.44), x(0.60), y(0.32));
            p.cubic_to(x(0.65), y(0.16), x(0.42), y(0.16), x(0.42), y(0.30));
            p.close();
            let dot = Path::ellipse(Rect::new(x(0.41), y(0.78), side * 0.16, side * 0.16));
            append(p, &dot)
        }
        "actionButtonMovie" => {
            // A camera body with a lens barrel and a reel bump.
            let body = poly(&[
                (0.05, 0.32),
                (0.62, 0.32),
                (0.62, 0.46),
                (0.80, 0.34),
                (0.95, 0.34),
                (0.95, 0.74),
                (0.80, 0.74),
                (0.62, 0.62),
                (0.62, 0.76),
                (0.05, 0.76),
            ]);
            let reel = Path::ellipse(Rect::new(x(0.12), y(0.16), side * 0.22, side * 0.22));
            append(body, &reel)
        }
        _ => return None,
    })
}

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
    "flowChartInternalStorage",
    "flowChartMultidocument",
    "flowChartPreparation",
    "flowChartManualInput",
    "flowChartManualOperation",
    "flowChartOffpageConnector",
    "flowChartPunchedCard",
    "flowChartPunchedTape",
    "flowChartSummingJunction",
    "flowChartOr",
    "flowChartCollate",
    "flowChartSort",
    "flowChartExtract",
    "flowChartMerge",
    "flowChartOnlineStorage",
    "flowChartDelay",
    "flowChartDisplay",
    "flowChartMagneticDisk",
    "flowChartMagneticDrum",
    "flowChartMagneticTape",
    "flowChartOfflineStorage",
    "actionButtonHome",
    "actionButtonHelp",
    "actionButtonInformation",
    "actionButtonForwardNext",
    "actionButtonBackPrevious",
    "actionButtonBeginning",
    "actionButtonEnd",
    "actionButtonReturn",
    "actionButtonDocument",
    "actionButtonSound",
    "actionButtonMovie",
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

    /// Adjustment over 50000 rather than 100000.
    ///
    /// The star presets scale their inner radius by `*/ swd2 a 50000`, so `adj="19098"`
    /// on `star5` means 0.38197 — the golden-ratio waist of a five-pointed star — not
    /// 0.19098. Reading it as a hundred-thousandth halves the waist and renders the star
    /// as five thin spikes.
    fn ratio_50k(&self, name: &str, fallback: f64) -> f32 {
        let raw = self.values.get(name).copied().unwrap_or(fallback);
        (raw / 50_000.0).clamp(0.0, 1.0) as f32
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
            "hexagon" => {
                // Spec: the flat top and bottom edges sit *on* the box, and the slanted
                // ends are inset by the smaller side — not by the width. A regular
                // hexagon in the box's ellipse leaves a visible band at top and bottom.
                let x1 = (self.adj.frac("adj", 25000.0) * self.ss()).min(w / 2.0);
                let mut p = Path::new();
                p.move_to(0.0, h / 2.0)
                    .line_to(x1, 0.0)
                    .line_to(w - x1, 0.0)
                    .line_to(w, h / 2.0)
                    .line_to(w - x1, h)
                    .line_to(x1, h)
                    .close();
                p
            }
            "heptagon" => self.regular_polygon(7, -PI / 2.0),
            "octagon" => {
                // Also box-filling: the corner cut is the smaller side times the adjust.
                let x1 = (self.adj.frac("adj", 29289.0) * self.ss()).min(w.min(h) / 2.0);
                let mut p = Path::new();
                p.move_to(0.0, x1)
                    .line_to(x1, 0.0)
                    .line_to(w - x1, 0.0)
                    .line_to(w, x1)
                    .line_to(w, h - x1)
                    .line_to(w - x1, h)
                    .line_to(x1, h)
                    .line_to(0.0, h - x1)
                    .close();
                p
            }
            "decagon" => self.regular_polygon(10, PI / 10.0),
            "star4" => self.star(4, self.adj.ratio_50k("adj", 12500.0)),
            "star5" => self.star(5, self.adj.ratio_50k("adj", 19098.0)),
            "star6" => self.star(6, self.adj.ratio_50k("adj", 28868.0)),
            "star8" => self.star(8, self.adj.ratio_50k("adj", 37500.0)),
            "star10" => self.star(10, self.adj.ratio_50k("adj", 42533.0)),
            "star12" => self.star(12, self.adj.ratio_50k("adj", 37500.0)),
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
                let head = (self.adj.frac("adj2", 50000.0) * self.ss()).min(w / 2.0);
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
                let head = (self.adj.frac("adj2", 50000.0) * self.ss()).min(h / 2.0);
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
            // `can` and `cube` are built by `faces()`, which is what carries their
            // per-face shading. Reaching them here means someone asked for a single
            // path, so give the silhouette — the shape without its 3-D shading.
            "can" => {
                let ry = self.can_ry();
                let mut p = Path::new();
                p.move_to(0.0, ry);
                p.arc_to(w / 2.0, ry, w / 2.0, ry, PI, PI);
                p.line_to(w, h - ry);
                p.arc_to(w / 2.0, h - ry, w / 2.0, ry, 0.0, PI);
                p.close();
                p
            }
            "cube" => {
                let d = self.cube_d();
                let mut p = Path::new();
                p.move_to(0.0, d)
                    .line_to(d, 0.0)
                    .line_to(w, 0.0)
                    .line_to(w, h - d)
                    .line_to(w - d, h)
                    .line_to(0.0, h)
                    .close();
                p
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
            // ---- the rest of the flowchart family ------------------------------
            // ECMA-376 gives these fixed fractions of the box rather than adjustments,
            // so there is nothing to tune: the numbers below are the spec's.
            "flowChartInternalStorage" => {
                let (x1, y1) = (w * 0.125, h * 0.125);
                let outer = Path::rect(self.rect());
                let mut rules = Path::new();
                rules
                    .move_to(x1, 0.0)
                    .line_to(x1, h)
                    .move_to(0.0, y1)
                    .line_to(w, y1);
                append(outer, &rules)
            }
            "flowChartPreparation" => {
                // A hexagon at fixed fifths, unlike `hexagon` which is adjustable.
                let x1 = w / 5.0;
                let mut p = Path::new();
                p.move_to(0.0, h / 2.0)
                    .line_to(x1, 0.0)
                    .line_to(w - x1, 0.0)
                    .line_to(w, h / 2.0)
                    .line_to(w - x1, h)
                    .line_to(x1, h)
                    .close();
                p
            }
            "flowChartManualInput" => {
                let y1 = h / 5.0;
                let mut p = Path::new();
                p.move_to(0.0, y1)
                    .line_to(w, 0.0)
                    .line_to(w, h)
                    .line_to(0.0, h)
                    .close();
                p
            }
            "flowChartManualOperation" => {
                let x1 = w / 5.0;
                let mut p = Path::new();
                p.move_to(0.0, 0.0)
                    .line_to(w, 0.0)
                    .line_to(w - x1, h)
                    .line_to(x1, h)
                    .close();
                p
            }
            "flowChartOffpageConnector" => {
                let y1 = h * 0.8;
                let mut p = Path::new();
                p.move_to(0.0, 0.0)
                    .line_to(w, 0.0)
                    .line_to(w, y1)
                    .line_to(w / 2.0, h)
                    .line_to(0.0, y1)
                    .close();
                p
            }
            "flowChartPunchedCard" => {
                // The corner cut is one length used on both axes, as the spec writes it.
                let d = h / 5.0;
                let mut p = Path::new();
                p.move_to(0.0, d)
                    .line_to(d, 0.0)
                    .line_to(w, 0.0)
                    .line_to(w, h)
                    .line_to(0.0, h)
                    .close();
                p
            }
            "flowChartPunchedTape" => {
                // A ribbon whose top and bottom edges each make one S-shaped step, low on
                // the left and high on the right, so the thickness stays constant. Not a
                // full sine period — two humps looks like a plausible "wavy tape" and is
                // a completely different shape, worth 44% of the cell against the oracle.
                let y1 = h / 5.0;
                let mut p = Path::new();
                p.move_to(0.0, y1);
                p.cubic_to(w * 0.5, y1, w * 0.5, 0.0, w, 0.0);
                p.line_to(w, h - y1);
                p.cubic_to(w * 0.5, h - y1, w * 0.5, h, 0.0, h);
                p.close();
                p
            }
            "flowChartSummingJunction" => {
                // A circle crossed by its two 45-degree diameters.
                let outer = Path::ellipse(self.rect());
                let (cx, cy) = (w / 2.0, h / 2.0);
                let (dx, dy) = (w / 2.0 * FRAC_1_SQRT_2, h / 2.0 * FRAC_1_SQRT_2);
                let mut cross = Path::new();
                cross
                    .move_to(cx - dx, cy - dy)
                    .line_to(cx + dx, cy + dy)
                    .move_to(cx + dx, cy - dy)
                    .line_to(cx - dx, cy + dy);
                append(outer, &cross)
            }
            "flowChartOr" => {
                let outer = Path::ellipse(self.rect());
                let mut cross = Path::new();
                cross
                    .move_to(w / 2.0, 0.0)
                    .line_to(w / 2.0, h)
                    .move_to(0.0, h / 2.0)
                    .line_to(w, h / 2.0);
                append(outer, &cross)
            }
            "flowChartCollate" => {
                // Two triangles meeting at the centre — the winding makes the hourglass.
                let mut p = Path::new();
                p.move_to(0.0, 0.0)
                    .line_to(w, 0.0)
                    .line_to(0.0, h)
                    .line_to(w, h)
                    .close();
                p
            }
            "flowChartSort" => {
                let mut outer = Path::new();
                outer
                    .move_to(w / 2.0, 0.0)
                    .line_to(w, h / 2.0)
                    .line_to(w / 2.0, h)
                    .line_to(0.0, h / 2.0)
                    .close();
                let mut rule = Path::new();
                rule.move_to(0.0, h / 2.0).line_to(w, h / 2.0);
                append(outer, &rule)
            }
            "flowChartExtract" => {
                let mut p = Path::new();
                p.move_to(w / 2.0, 0.0)
                    .line_to(w, h)
                    .line_to(0.0, h)
                    .close();
                p
            }
            "flowChartMerge" => {
                let mut p = Path::new();
                p.move_to(0.0, 0.0)
                    .line_to(w, 0.0)
                    .line_to(w / 2.0, h)
                    .close();
                p
            }
            "flowChartOfflineStorage" => {
                // A downward triangle with a rule near its apex end, its ends meeting
                // the two slanted edges rather than floating past them.
                let mut outer = Path::new();
                outer
                    .move_to(0.0, 0.0)
                    .line_to(w, 0.0)
                    .line_to(w / 2.0, h)
                    .close();
                let t = 0.8_f32;
                let y = h * t;
                let (x0, x1) = (w / 2.0 * t, w - w / 2.0 * t);
                let mut rule = Path::new();
                rule.move_to(x0, y).line_to(x1, y);
                append(outer, &rule)
            }
            "flowChartOnlineStorage" => {
                // A tape spool seen edge-on: the left cap bulges *out* past the box's
                // left edge line, and the right edge is bitten *in* by the same radius.
                // Getting these the wrong way round mirrors the shape, which reads as
                // plausible on its own and is obvious beside the reference.
                let rx = w / 6.0;
                let mut p = Path::new();
                p.move_to(rx, 0.0);
                p.line_to(w, 0.0);
                // Concave right edge, bulging left to `w - rx`.
                p.arc_to(w, h / 2.0, rx, h / 2.0, -PI / 2.0, -PI);
                p.line_to(rx, h);
                // Convex left cap, reaching x = 0.
                p.arc_to(rx, h / 2.0, rx, h / 2.0, PI / 2.0, PI);
                p.close();
                p
            }
            "flowChartDelay" => {
                // Square on the left, semicircular on the right.
                let mut p = Path::new();
                p.move_to(0.0, 0.0);
                p.line_to(w / 2.0, 0.0);
                p.arc_to(w / 2.0, h / 2.0, w / 2.0, h / 2.0, -PI / 2.0, PI);
                p.line_to(0.0, h);
                p.close();
                p
            }
            "flowChartDisplay" => {
                // Pointed on the left, rounded on the right.
                let x1 = w / 6.0;
                let mut p = Path::new();
                p.move_to(0.0, h / 2.0);
                p.line_to(x1, 0.0);
                p.line_to(w - x1, 0.0);
                p.arc_to(w - x1, h / 2.0, x1, h / 2.0, -PI / 2.0, PI);
                p.line_to(x1, h);
                p.close();
                p
            }
            "flowChartMagneticDisk" => {
                // Built by `faces()`; this is the silhouette.
                // A cylinder: the same construction as `can`, at a fixed sixth.
                let ry = h / 6.0;
                let mut body = Path::new();
                body.move_to(0.0, ry);
                body.arc_to(w / 2.0, ry, w / 2.0, ry, PI, PI);
                body.line_to(w, h - ry);
                body.arc_to(w / 2.0, h - ry, w / 2.0, ry, 0.0, PI);
                body.close();
                let lid = Path::ellipse(Rect::new(0.0, 0.0, w, ry * 2.0));
                append(body, &lid)
            }
            "flowChartMagneticDrum" => {
                // Built by `faces()`; this is the silhouette.
                // The same cylinder lying on its side.
                let rx = w / 6.0;
                let mut body = Path::new();
                body.move_to(rx, 0.0);
                body.arc_to(rx, h / 2.0, rx, h / 2.0, -PI / 2.0, -PI);
                body.line_to(w - rx, h);
                body.arc_to(w - rx, h / 2.0, rx, h / 2.0, PI / 2.0, -PI);
                body.close();
                let cap = Path::ellipse(Rect::new(w - rx * 2.0, 0.0, rx * 2.0, h));
                append(body, &cap)
            }
            "flowChartMagneticTape" => {
                // A circle whose bottom-right quadrant is squared off into the tape's
                // foot. The arc runs bottom -> left -> top -> right, so every endpoint
                // lands on a quadrant boundary; an arc that stops mid-quadrant puts its
                // Bezier control points outside the circle and the path then reports a
                // bounding box larger than its own shape.
                let (cx, cy) = (w / 2.0, h / 2.0);
                let mut p = Path::new();
                p.move_to(cx, h);
                p.arc_to(cx, cy, w / 2.0, h / 2.0, PI / 2.0, 3.0 * PI / 2.0);
                p.line_to(w, h);
                p.line_to(cx, h);
                p.close();
                p
            }
            "flowChartMultidocument" => {
                // Built by `faces()`; this is the silhouette.
                // Three stacked documents, offset up and to the right.
                let (ox, oy) = (w * 0.08, h * 0.12);
                let doc = |x: f32, y: f32, dw: f32, dh: f32| -> Path {
                    let wave = dh * 0.14;
                    let mut d = Path::new();
                    d.move_to(x, y)
                        .line_to(x + dw, y)
                        .line_to(x + dw, y + dh - wave)
                        // The trough control point sits exactly on the bottom edge, so
                        // the wave reaches it and never passes it — a cubic stays inside
                        // the hull of its control points.
                        .cubic_to(
                            x + dw * 0.75,
                            y + dh,
                            x + dw * 0.25,
                            y + dh - wave * 2.0,
                            x,
                            y + dh - wave,
                        );
                    d.close();
                    d
                };
                let bw = w - ox * 2.0;
                let bh = h - oy * 2.0;
                let mut p = doc(ox * 2.0, 0.0, bw, bh);
                p = append(p, &doc(ox, oy, bw, bh));
                append(p, &doc(0.0, oy * 2.0, bw, bh))
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
    /// Half-height of `can`'s elliptical lid: `*/ ss a 200000`.
    fn can_ry(&self) -> f32 {
        (self.adj.frac("adj", 25000.0) * self.ss() / 2.0).min(self.h / 2.0)
    }

    /// Depth of `cube`'s projection: `*/ ss a 100000`.
    fn cube_d(&self) -> f32 {
        (self.adj.frac("adj", 25000.0) * self.ss()).min(self.w.min(self.h))
    }

    /// Closes a path through unit-circle points, scaled so it exactly fills the box.
    ///
    /// ECMA-376 does this with a precomputed `hf`/`vf` pair per preset — `pentagon`'s
    /// `hf="105146"` is 1/cos(18°), and `star5`'s `vf="110557"` is 2/(1+sin(54°)) — plus a
    /// shifted vertical centre for the shapes that are not symmetric about it. Measuring
    /// the extent gives the same numbers without a table of constants to mistype, and it
    /// stays right for a preset whose factors nobody looked up.
    ///
    /// A polygon inscribed in the box's *ellipse* is the tempting shortcut and is wrong:
    /// a pentagon's widest points are at ±18°, so it would sit 5% narrow, and a five-
    /// pointed star would float 10% short of the bottom edge.
    fn fill_box(&self, pts: &[(f32, f32)]) -> Path {
        let mut p = Path::new();
        let (mut x0, mut y0) = (f32::INFINITY, f32::INFINITY);
        let (mut x1, mut y1) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
        for &(x, y) in pts {
            x0 = x0.min(x);
            x1 = x1.max(x);
            y0 = y0.min(y);
            y1 = y1.max(y);
        }
        // A degenerate set would divide by zero; fall back to leaving it at the origin.
        let sx = if x1 > x0 { self.w / (x1 - x0) } else { 0.0 };
        let sy = if y1 > y0 { self.h / (y1 - y0) } else { 0.0 };
        for (i, &(x, y)) in pts.iter().enumerate() {
            let (px, py) = ((x - x0) * sx, (y - y0) * sy);
            if i == 0 {
                p.move_to(px, py);
            } else {
                p.line_to(px, py);
            }
        }
        p.close();
        p
    }

    fn regular_polygon(&self, n: usize, start: f32) -> Path {
        let pts: Vec<(f32, f32)> = (0..n)
            .map(|i| {
                let t = start + (i as f32) * 2.0 * PI / n as f32;
                (t.cos(), t.sin())
            })
            .collect();
        self.fill_box(&pts)
    }

    /// An n-pointed star whose inner radius is `inner` of the outer.
    fn star(&self, points: usize, inner: f32) -> Path {
        let inner = inner.clamp(0.05, 0.95);
        let pts: Vec<(f32, f32)> = (0..points * 2)
            .map(|i| {
                // Vertex 0 at the top, then alternating outer and inner every half step.
                let t = -PI / 2.0 + (i as f32) * PI / points as f32;
                let r = if i % 2 == 0 { 1.0 } else { inner };
                (r * t.cos(), r * t.sin())
            })
            .collect();
        self.fill_box(&pts)
    }

    fn arrow_h(&self, left: bool) -> Path {
        let (w, h) = (self.w, self.h);
        let shaft = self.adj.frac("adj1", 50000.0) * h;
        // `dx2 = */ ss a2 100000` — the head is measured against the *shorter side*, not
        // the length. Using the width makes a wide arrow's head grow with the shaft, so a
        // long thin arrow comes out as a giant chevron with a stub behind it.
        let head = (self.adj.frac("adj2", 50000.0) * self.ss()).min(w);
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
        // Measured against the shorter side; see the note in `arrow_h`.
        let head = (self.adj.frac("adj2", 50000.0) * self.ss()).min(h);
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

    /// The extent of a built path, as (x0, y0, x1, y1).
    fn extent(preset: &str, w: f32, h: f32) -> (f32, f32, f32, f32) {
        let b = build(preset, w, h, &[]).bounds();
        (b.x, b.y, b.right(), b.bottom())
    }

    #[test]
    fn an_arrow_head_is_measured_against_the_shorter_side() {
        // `dx2 = */ ss a2 100000`: on a 400x100 arrow the default head is 50 long, not
        // 200. Scaling it by the width instead grows the head with the shaft, so a long
        // thin arrow renders as a giant chevron — the shaft matches the oracle and the
        // head does not, which is exactly how it showed up in the m3 diff.
        let pts: Vec<(f32, f32)> = build("rightArrow", 400.0, 100.0, &[])
            .points
            .iter()
            .map(|q| (q.x, q.y))
            .collect();
        let back = pts
            .iter()
            .map(|p| p.0)
            .filter(|x| *x < 399.0)
            .fold(0.0f32, f32::max);
        assert!(
            (back - 350.0).abs() < 0.01,
            "head starts at {back}, expected 350 (400 - ss*0.5)"
        );
        // The tip still reaches the right edge and the shaft still spans the full width.
        let (x0, _, x1, _) = extent("rightArrow", 400.0, 100.0);
        assert!(x0.abs() < 0.01 && (x1 - 400.0).abs() < 0.01);
    }

    #[test]
    fn box_filling_polygons_and_stars_touch_every_edge() {
        // ECMA-376 gives these presets hf/vf factors precisely so they fill the box.
        // Inscribing them in the box's ellipse instead leaves a gap that is small enough
        // to look like antialiasing in a pixel diff and obvious when placed next to the
        // real thing — which is how it was found.
        for preset in ["pentagon", "heptagon", "decagon", "star4", "star5", "star6"] {
            let (x0, y0, x1, y1) = extent(preset, 200.0, 100.0);
            assert!(
                x0.abs() < 0.01 && y0.abs() < 0.01,
                "{preset} origin: {x0},{y0}"
            );
            assert!(
                (x1 - 200.0).abs() < 0.01 && (y1 - 100.0).abs() < 0.01,
                "{preset} extent: {x1},{y1}"
            );
        }
    }

    #[test]
    fn a_five_pointed_star_has_a_golden_ratio_waist() {
        // star5 scales its inner radius by `*/ swd2 a 50000`, so adj="19098" means 0.382
        // — 1/phi^2, the waist of a five-pointed star — not 0.191. Reading it over
        // 100000 halves the waist and draws five thin spikes instead of a star.
        //
        // Checked at a vertex rather than as a radius ratio, because hf and vf differ, so
        // the star is scaled anisotropically and its radii are not in a single ratio.
        // Working the spec through for a 100x100 box:
        //   swd2 = 50*1.05146   iwd2 = swd2*0.38197 = 20.081
        //   shd2 = svc = 50*1.10557   ihd2 = shd2*0.38197 = 21.115
        //   pt1 (the inner vertex at -54 degrees)
        //     x = 50 + 20.081*cos(-54) = 61.803
        //     y = 55.279 + 21.115*sin(-54) = 38.197
        // Halving the waist would put it at roughly (55.9, 46.7).
        let path = build("star5", 100.0, 100.0, &[]);
        let p1 = path.points.get(1).copied().expect("star5 has ten vertices");
        assert!(
            (p1.x - 61.803).abs() < 0.05 && (p1.y - 38.197).abs() < 0.05,
            "inner vertex at ({}, {}), expected (61.803, 38.197)",
            p1.x,
            p1.y
        );
    }

    #[test]
    fn a_hexagon_spans_the_full_height_and_insets_by_the_shorter_side() {
        // The slanted ends are inset by ss*adj, not w*adj. On a wide shape those are very
        // different, and using the width makes the hexagon lose its flat top entirely.
        let (x0, y0, x1, y1) = extent("hexagon", 400.0, 100.0);
        assert!(
            y0.abs() < 0.01 && (y1 - 100.0).abs() < 0.01,
            "height {y0}..{y1}"
        );
        assert!(
            x0.abs() < 0.01 && (x1 - 400.0).abs() < 0.01,
            "width {x0}..{x1}"
        );
        let path = build("hexagon", 400.0, 100.0, &[]);
        let pts: Vec<(f32, f32)> = path.points.iter().map(|q| (q.x, q.y)).collect();
        // adj 25000 of the shorter side (100) = 25, not 25000 of the width (100).
        let top: Vec<f32> = pts.iter().filter(|p| p.1 < 0.01).map(|p| p.0).collect();
        assert_eq!(top.len(), 2, "expected a flat top edge, got {top:?}");
        assert!(
            (top[0] - 25.0).abs() < 0.01,
            "inset {} should be 25",
            top[0]
        );
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
