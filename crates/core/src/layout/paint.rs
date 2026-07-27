//! Fill/Line → resolved [`Paint`] and [`Stroke`].
//!
//! Everything colour-related is resolved here: theme lookup, colour map, `phClr`
//! substitution, tint/shade. Downstream of this module a colour is just four bytes.

use crate::dl::{Color, Gradient, GradientStop, LineCap, LineJoin, Paint, Point, Rect, Stroke};
use crate::emu;
use crate::model::fill::{
    BlipMode, DashStyle, Fill, GradientKind, Line, LineCapStyle, LineJoinStyle,
};
use crate::model::Presentation;

use super::inherit::Resolver;

/// Converts a resolved [`Fill`] into a [`Paint`] for a shape occupying `bounds`.
///
/// Returns `None` when the fill draws nothing, so callers can skip emitting a command
/// rather than push a transparent fill the renderer has to discover is a no-op.
pub fn fill_to_paint(
    fill: &Fill,
    bounds: Rect,
    resolver: &Resolver<'_>,
    pres: &Presentation,
    source_part: &str,
) -> Option<Paint> {
    match fill {
        Fill::NoFill | Fill::Inherit | Fill::Group => None,
        Fill::Solid(c) => {
            let color = resolver.color(c);
            (!color.is_transparent()).then_some(Paint::Solid(color))
        }
        Fill::Gradient(g) => {
            let stops: Vec<GradientStop> = g
                .stops
                .iter()
                .map(|s| GradientStop {
                    offset: s.pos.clamp(0.0, 1.0),
                    color: resolver.color(&s.color),
                })
                .collect();
            if stops.is_empty() {
                return None;
            }
            // A single stop is a solid fill; emitting a degenerate gradient makes
            // Canvas2D throw and WebGPU divide by zero.
            if stops.len() == 1 {
                return stops.first().map(|s| Paint::Solid(s.color));
            }
            Some(Paint::Gradient(match g.kind {
                GradientKind::Linear => {
                    let (start, end) = linear_endpoints(g.angle_deg, bounds);
                    Gradient::Linear { start, end, stops }
                }
                _ => {
                    // Radial-family gradients start from the focus rectangle's centre.
                    let focus = g.focus.unwrap_or([0.5, 0.5, 0.5, 0.5]);
                    let cx = bounds.x + bounds.w * focus[0].clamp(0.0, 1.0);
                    let cy = bounds.y + bounds.h * focus[1].clamp(0.0, 1.0);
                    let radius = (bounds.w.max(bounds.h) / 2.0).max(0.01);
                    // OOXML radial gradients run outside-in; the display list's run
                    // inside-out, so the stops are reversed.
                    let mut stops = stops;
                    stops.reverse();
                    for s in &mut stops {
                        s.offset = 1.0 - s.offset;
                    }
                    stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
                    Gradient::Radial {
                        center: Point::new(cx, cy),
                        radius,
                        scale_y: if bounds.w > 0.0 {
                            (bounds.h / bounds.w).max(0.01)
                        } else {
                            1.0
                        },
                        stops,
                    }
                }
            }))
        }
        Fill::Blip(b) => {
            let id = b
                .embed_id
                .as_deref()
                .and_then(|rid| pres.intern_image(source_part, rid))?;
            let [l, t, r, bo] = b.src_rect;
            let src = Rect::new(l, t, (1.0 - l - r).max(0.0), (1.0 - t - bo).max(0.0));
            if src.is_empty() {
                return None;
            }
            let _ = b.mode == BlipMode::Tile;
            Some(Paint::Image {
                image: id,
                src,
                opacity: b.alpha.clamp(0.0, 1.0),
            })
        }
        Fill::Pattern(p) => {
            // Patterns are approximated by blending the two colours by the preset's
            // documented coverage. A real hatch needs a tile the display list cannot
            // express yet, and a flat blend reads far better than dropping the fill.
            let fg = resolver.color(&p.foreground);
            let bg = resolver.color(&p.background);
            let coverage = pattern_coverage(&p.preset);
            Some(Paint::Solid(blend(fg, bg, coverage)))
        }
    }
}

/// A linear gradient's endpoints for an angle measured clockwise from +x.
///
/// The line runs through the centre of `bounds` and is extended so the gradient covers
/// the whole rectangle at any angle — otherwise a 45° gradient leaves the corners flat.
fn linear_endpoints(angle_deg: f32, bounds: Rect) -> (Point, Point) {
    let a = angle_deg.to_radians();
    let (cx, cy) = (bounds.center().x, bounds.center().y);
    let (dx, dy) = (a.cos(), a.sin());
    // Projection of the half-diagonal onto the gradient direction.
    let half = (bounds.w * dx).abs() / 2.0 + (bounds.h * dy).abs() / 2.0;
    (
        Point::new(cx - dx * half, cy - dy * half),
        Point::new(cx + dx * half, cy + dy * half),
    )
}

fn blend(fg: Color, bg: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| ((a as f32 * t) + (b as f32 * (1.0 - t))).round().clamp(0.0, 255.0) as u8;
    Color {
        r: mix(fg.r, bg.r),
        g: mix(fg.g, bg.g),
        b: mix(fg.b, bg.b),
        a: mix(fg.a, bg.a),
    }
}

/// Fraction of a pattern tile covered by the foreground colour.
fn pattern_coverage(preset: &str) -> f32 {
    match preset {
        "pct5" => 0.05,
        "pct10" => 0.10,
        "pct20" => 0.20,
        "pct25" => 0.25,
        "pct30" => 0.30,
        "pct40" => 0.40,
        "pct50" => 0.50,
        "pct60" => 0.60,
        "pct70" => 0.70,
        "pct75" => 0.75,
        "pct80" => 0.80,
        "pct90" => 0.90,
        "ltHorz" | "ltVert" | "ltUpDiag" | "ltDnDiag" => 0.25,
        "dkHorz" | "dkVert" | "dkUpDiag" | "dkDnDiag" => 0.5,
        "smGrid" | "smCheck" => 0.35,
        "lgGrid" | "lgCheck" => 0.45,
        "solidDmnd" => 0.6,
        _ => 0.5,
    }
}

/// Default outline width when a line specifies a fill but no width: 0.75pt, PowerPoint's
/// own default for a new shape.
const DEFAULT_LINE_WIDTH_EMU: i64 = 9_525;

/// Converts a resolved [`Line`] into a [`Stroke`], or `None` when it draws nothing.
pub fn line_to_stroke(
    line: &Line,
    bounds: Rect,
    resolver: &Resolver<'_>,
    pres: &Presentation,
    source_part: &str,
) -> Option<Stroke> {
    if matches!(line.fill, Fill::NoFill) {
        return None;
    }
    let paint = fill_to_paint(&line.fill, bounds, resolver, pres, source_part)?;
    let width = emu::to_pt(line.width.unwrap_or(DEFAULT_LINE_WIDTH_EMU)).max(0.0);
    let dash = line
        .dash
        .unwrap_or(DashStyle::Solid)
        .pattern()
        .iter()
        .map(|m| m * width.max(0.25))
        .collect();
    Some(Stroke {
        paint,
        width,
        cap: match line.cap {
            Some(LineCapStyle::Round) => LineCap::Round,
            Some(LineCapStyle::Square) => LineCap::Square,
            _ => LineCap::Butt,
        },
        join: match line.join {
            Some(LineJoinStyle::Round) => LineJoin::Round,
            Some(LineJoinStyle::Bevel) => LineJoin::Bevel,
            _ => LineJoin::Miter,
        },
        miter_limit: 8.0,
        dash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::color::{ColorRef, SchemeColor};
    use crate::model::fill::{GradientFill, GradientStopSpec};
    use crate::model::shape::{SlideLayout, SlideMaster};
    use crate::model::theme::Theme;
    use crate::model::{Slide, SlideChain};
    use std::rc::Rc;

    struct Env {
        pres: Presentation,
        chain: SlideChain,
    }

    fn env() -> Env {
        use std::io::{Cursor, Write};
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            w.start_file("[Content_Types].xml", opts).expect("s");
            w.write_all(br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#).expect("w");
            w.finish().expect("f");
        }
        let pkg = crate::opc::Package::open(buf).expect("open");
        Env {
            pres: Presentation::new(pkg, 1, 1),
            chain: SlideChain {
                slide: Rc::new(Slide::default()),
                layout: Some(Rc::new(SlideLayout::default())),
                master: Some(Rc::new(SlideMaster::default())),
                theme: Rc::new(Theme::default()),
            },
        }
    }

    const BOX: Rect = Rect::new(0.0, 0.0, 100.0, 50.0);

    #[test]
    fn a_solid_theme_fill_resolves_to_the_theme_colour() {
        let e = env();
        let r = Resolver::new(&e.pres, &e.chain);
        let paint = fill_to_paint(
            &Fill::Solid(ColorRef::scheme(SchemeColor::Accent1)),
            BOX,
            &r,
            &e.pres,
            "ppt/slides/slide1.xml",
        );
        assert_eq!(paint, Some(Paint::Solid(Color::rgb(0x44, 0x72, 0xC4))));
    }

    #[test]
    fn nofill_and_inherit_produce_no_paint() {
        let e = env();
        let r = Resolver::new(&e.pres, &e.chain);
        for f in [Fill::NoFill, Fill::Inherit, Fill::Group] {
            assert_eq!(fill_to_paint(&f, BOX, &r, &e.pres, "p"), None);
        }
    }

    #[test]
    fn a_fully_transparent_colour_produces_no_paint() {
        let e = env();
        let r = Resolver::new(&e.pres, &e.chain);
        let clear = ColorRef {
            spec: crate::model::color::ColorSpec::Srgb(Color::rgb(255, 0, 0)),
            mods: vec![crate::model::color::ColorMod::Alpha(0.0)],
        };
        assert_eq!(fill_to_paint(&Fill::Solid(clear), BOX, &r, &e.pres, "p"), None);
    }

    #[test]
    fn a_single_stop_gradient_degrades_to_a_solid_fill() {
        let e = env();
        let r = Resolver::new(&e.pres, &e.chain);
        let g = Fill::Gradient(GradientFill {
            kind: GradientKind::Linear,
            stops: vec![GradientStopSpec {
                pos: 0.0,
                color: ColorRef::srgb(Color::rgb(1, 2, 3)),
            }],
            angle_deg: 0.0,
            scaled: false,
            focus: None,
        });
        assert_eq!(
            fill_to_paint(&g, BOX, &r, &e.pres, "p"),
            Some(Paint::Solid(Color::rgb(1, 2, 3)))
        );
    }

    #[test]
    fn a_zero_degree_linear_gradient_runs_left_to_right_across_the_box() {
        let e = env();
        let r = Resolver::new(&e.pres, &e.chain);
        let g = Fill::Gradient(GradientFill {
            kind: GradientKind::Linear,
            stops: vec![
                GradientStopSpec {
                    pos: 0.0,
                    color: ColorRef::srgb(Color::WHITE),
                },
                GradientStopSpec {
                    pos: 1.0,
                    color: ColorRef::srgb(Color::BLACK),
                },
            ],
            angle_deg: 0.0,
            scaled: false,
            focus: None,
        });
        match fill_to_paint(&g, BOX, &r, &e.pres, "p") {
            Some(Paint::Gradient(Gradient::Linear { start, end, stops })) => {
                assert_eq!(stops.len(), 2);
                assert!((start.x - 0.0).abs() < 0.01, "start.x={}", start.x);
                assert!((end.x - 100.0).abs() < 0.01, "end.x={}", end.x);
                assert!((start.y - 25.0).abs() < 0.01 && (end.y - 25.0).abs() < 0.01);
            }
            other => panic!("expected a linear gradient, got {other:?}"),
        }
    }

    #[test]
    fn a_ninety_degree_gradient_runs_top_to_bottom() {
        let e = env();
        let r = Resolver::new(&e.pres, &e.chain);
        let g = Fill::Gradient(GradientFill {
            kind: GradientKind::Linear,
            stops: vec![
                GradientStopSpec {
                    pos: 0.0,
                    color: ColorRef::srgb(Color::WHITE),
                },
                GradientStopSpec {
                    pos: 1.0,
                    color: ColorRef::srgb(Color::BLACK),
                },
            ],
            angle_deg: 90.0,
            scaled: false,
            focus: None,
        });
        match fill_to_paint(&g, BOX, &r, &e.pres, "p") {
            Some(Paint::Gradient(Gradient::Linear { start, end, .. })) => {
                assert!((start.y - 0.0).abs() < 0.01, "start.y={}", start.y);
                assert!((end.y - 50.0).abs() < 0.01, "end.y={}", end.y);
            }
            other => panic!("expected a linear gradient, got {other:?}"),
        }
    }

    #[test]
    fn a_pattern_fill_blends_toward_its_coverage() {
        let e = env();
        let r = Resolver::new(&e.pres, &e.chain);
        let p = Fill::Pattern(crate::model::fill::PatternFill {
            foreground: ColorRef::srgb(Color::BLACK),
            background: ColorRef::srgb(Color::WHITE),
            preset: "pct25".into(),
        });
        match fill_to_paint(&p, BOX, &r, &e.pres, "p") {
            // 25% black over white.
            Some(Paint::Solid(c)) => assert_eq!(c.r, 191),
            other => panic!("expected a solid blend, got {other:?}"),
        }
    }

    #[test]
    fn an_explicit_nofill_outline_produces_no_stroke() {
        let e = env();
        let r = Resolver::new(&e.pres, &e.chain);
        let line = Line {
            fill: Fill::NoFill,
            width: Some(12_700),
            ..Default::default()
        };
        assert!(line_to_stroke(&line, BOX, &r, &e.pres, "p").is_none());
    }

    #[test]
    fn stroke_width_converts_emus_to_points_and_defaults_sensibly() {
        let e = env();
        let r = Resolver::new(&e.pres, &e.chain);
        let with_width = Line {
            fill: Fill::Solid(ColorRef::srgb(Color::BLACK)),
            width: Some(25_400),
            ..Default::default()
        };
        assert_eq!(
            line_to_stroke(&with_width, BOX, &r, &e.pres, "p").map(|s| s.width),
            Some(2.0)
        );
        let no_width = Line {
            fill: Fill::Solid(ColorRef::srgb(Color::BLACK)),
            ..Default::default()
        };
        assert_eq!(
            line_to_stroke(&no_width, BOX, &r, &e.pres, "p").map(|s| s.width),
            Some(0.75)
        );
    }

    #[test]
    fn dash_patterns_scale_with_the_stroke_width() {
        let e = env();
        let r = Resolver::new(&e.pres, &e.chain);
        let line = Line {
            fill: Fill::Solid(ColorRef::srgb(Color::BLACK)),
            width: Some(25_400), // 2pt
            dash: Some(DashStyle::Dash),
            ..Default::default()
        };
        let s = line_to_stroke(&line, BOX, &r, &e.pres, "p").expect("stroke");
        assert_eq!(s.dash, vec![8.0, 6.0], "4x and 3x a 2pt width");
    }

    #[test]
    fn a_solid_dash_style_produces_an_empty_pattern() {
        let e = env();
        let r = Resolver::new(&e.pres, &e.chain);
        let line = Line {
            fill: Fill::Solid(ColorRef::srgb(Color::BLACK)),
            dash: Some(DashStyle::Solid),
            ..Default::default()
        };
        assert!(line_to_stroke(&line, BOX, &r, &e.pres, "p")
            .map(|s| s.dash.is_empty())
            .unwrap_or(false));
    }

    #[test]
    fn a_blip_fill_without_a_resolvable_image_produces_no_paint() {
        let e = env();
        let r = Resolver::new(&e.pres, &e.chain);
        let f = Fill::Blip(crate::model::fill::BlipFill {
            embed_id: Some("rId99".into()),
            ..Default::default()
        });
        assert_eq!(fill_to_paint(&f, BOX, &r, &e.pres, "ppt/slides/slide1.xml"), None);
    }
}
