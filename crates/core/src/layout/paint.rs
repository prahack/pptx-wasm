//! Fill/Line → resolved [`Paint`] and [`Stroke`].
//!
//! Everything colour-related is resolved here: theme lookup, colour map, `phClr`
//! substitution, tint/shade. Downstream of this module a colour is just four bytes.

use crate::dl::{Gradient, GradientStop, LineCap, LineJoin, Paint, Point, Rect, Stroke};
use crate::emu;
use crate::model::color::{apply_mods, ColorMod};
use crate::model::fill::{
    BlipMode, DashStyle, Fill, GradientKind, Line, LineCapStyle, LineJoinStyle,
};
use crate::model::geometry::PathFillMode;
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
            let tile = match b.mode {
                BlipMode::Tile {
                    scale_x,
                    scale_y,
                    offset_x,
                    offset_y,
                } => Some(crate::dl::Tile {
                    scale_x,
                    scale_y,
                    offset_x: emu::to_pt(offset_x),
                    offset_y: emu::to_pt(offset_y),
                }),
                BlipMode::Stretch => None,
            };
            Some(Paint::Image {
                image: id,
                src,
                opacity: b.alpha.clamp(0.0, 1.0),
                tile,
            })
        }
        Fill::Pattern(p) => {
            let foreground = resolver.color(&p.foreground);
            let background = resolver.color(&p.background);
            Some(Paint::Hatch {
                pattern: crate::dl::HatchPattern::from_preset(&p.preset),
                foreground,
                background,
            })
        }
    }
}

/// Moves a gradient onto `bounds`, keeping its direction and stops.
///
/// Text needs this. A run's paint has to be resolved before layout knows where the run
/// will sit or how wide it is, so a gradient is first computed over a placeholder box and
/// re-anchored here once the glyphs are placed. Skipping this step is not a subtle error:
/// the gradient ends up in the slide's top-left corner and every glyph beyond it samples
/// the clamped final stop, so a gradient title renders as one flat colour.
///
/// The direction is recovered from the existing endpoints, which is exact — they were
/// derived from the angle in the first place.
pub fn reanchor_gradient(paint: &Paint, bounds: Rect) -> Paint {
    if bounds.w <= 0.0 || bounds.h <= 0.0 {
        return paint.clone();
    }
    match paint {
        Paint::Gradient(Gradient::Linear { start, end, stops }) => {
            let angle = (end.y - start.y).atan2(end.x - start.x);
            let (new_start, new_end) = linear_endpoints(angle.to_degrees(), bounds);
            Paint::Gradient(Gradient::Linear {
                start: new_start,
                end: new_end,
                stops: stops.clone(),
            })
        }
        Paint::Gradient(Gradient::Radial { stops, .. }) => Paint::Gradient(Gradient::Radial {
            center: bounds.center(),
            radius: (bounds.w.max(bounds.h) / 2.0).max(0.01),
            scale_y: if bounds.w > 0.0 {
                (bounds.h / bounds.w).max(0.01)
            } else {
                1.0
            },
            stops: stops.clone(),
        }),
        // Solid paints have no geometry, and an image paint's `src` is normalised.
        other => other.clone(),
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

/// Applies a `<a:path fill="lighten|darken|..">` modifier to a resolved paint.
///
/// DrawingML states the modes but not their strengths; the convention every renderer
/// converges on is the luminance pair PowerPoint writes elsewhere — 60000/80000 — so
/// `lighten` is 40% of the way to white and `darkenLess` is 20% of the way to black.
/// The blend goes through the same linear-light path as tint and shade, for the reason
/// recorded on [`crate::model::color::ColorMod::Tint`]: mixing in sRGB darkens midtones
/// visibly, and these faces sit right next to the unshaded one where it would show.
pub fn shade_paint(paint: &Paint, mode: PathFillMode) -> Paint {
    let mods: &[ColorMod] = match mode {
        PathFillMode::Normal | PathFillMode::None => return paint.clone(),
        PathFillMode::Lighten => &[ColorMod::Tint(0.6)],
        PathFillMode::LightenLess => &[ColorMod::Tint(0.8)],
        PathFillMode::Darken => &[ColorMod::Shade(0.6)],
        PathFillMode::DarkenLess => &[ColorMod::Shade(0.8)],
    };
    match paint {
        Paint::Solid(c) => Paint::Solid(apply_mods(*c, mods)),
        Paint::Gradient(g) => {
            let mut g = g.clone();
            let stops = match &mut g {
                Gradient::Linear { stops, .. } | Gradient::Radial { stops, .. } => stops,
            };
            for stop in stops.iter_mut() {
                stop.color = apply_mods(stop.color, mods);
            }
            Paint::Gradient(g)
        }
        Paint::Hatch {
            pattern,
            foreground,
            background,
        } => Paint::Hatch {
            pattern: *pattern,
            foreground: apply_mods(*foreground, mods),
            background: apply_mods(*background, mods),
        },
        // A bitmap cannot be shaded without touching its pixels, which layout does not do.
        Paint::Image { .. } => paint.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dl::Color;
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
            w.write_all(
                br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#,
            )
            .expect("w");
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
        assert_eq!(
            fill_to_paint(&Fill::Solid(clear), BOX, &r, &e.pres, "p"),
            None
        );
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
    fn a_lit_face_is_lighter_and_a_shaded_one_darker_than_the_fill() {
        use crate::model::geometry::PathFillMode as M;
        let base = Paint::Solid(Color::rgb(0x4F, 0x81, 0xBD));
        let lum = |p: &Paint| match p {
            Paint::Solid(c) => c.r as u32 + c.g as u32 + c.b as u32,
            _ => unreachable!("solid in, solid out"),
        };
        let normal = lum(&base);
        // The failure this guards is not "the shade is a few percent off" but the lit
        // face coming out unpainted — a white hole where cube's top should be.
        assert!(lum(&shade_paint(&base, M::Lighten)) > lum(&shade_paint(&base, M::LightenLess)));
        assert!(lum(&shade_paint(&base, M::LightenLess)) > normal);
        assert!(lum(&shade_paint(&base, M::DarkenLess)) < normal);
        assert!(lum(&shade_paint(&base, M::Darken)) < lum(&shade_paint(&base, M::DarkenLess)));
        assert_eq!(
            shade_paint(&base, M::Normal),
            base,
            "norm must not touch it"
        );
    }

    #[test]
    fn cube_and_can_come_back_as_shaded_faces() {
        use crate::layout::preset::faces;
        use crate::model::geometry::PathFillMode as M;
        let modes = |p: &str| {
            faces(p, 100.0, 100.0, &[])
                .into_iter()
                .map(|f| f.fill)
                .collect::<Vec<_>>()
        };
        assert_eq!(modes("cube"), vec![M::Normal, M::Lighten, M::Darken]);
        assert_eq!(modes("can"), vec![M::Normal, M::Lighten]);
        // Everything else stays a single unshaded face.
        assert_eq!(modes("rect"), vec![M::Normal]);
        assert_eq!(modes("star5"), vec![M::Normal]);
    }

    #[test]
    fn a_pattern_fill_reaches_the_renderer_as_a_pattern() {
        let e = env();
        let r = Resolver::new(&e.pres, &e.chain);
        let p = Fill::Pattern(crate::model::fill::PatternFill {
            foreground: ColorRef::srgb(Color::BLACK),
            background: ColorRef::srgb(Color::WHITE),
            preset: "ltUpDiag".into(),
        });
        match fill_to_paint(&p, BOX, &r, &e.pres, "p") {
            Some(Paint::Hatch {
                pattern,
                foreground,
                background,
            }) => {
                // The pattern reaches the renderer as a pattern, not pre-flattened into
                // an average colour — flattening is what made textured backgrounds
                // render as a flat wash.
                assert_eq!(
                    pattern,
                    crate::dl::HatchPattern::DiagonalUp { heavy: false }
                );
                assert_eq!(foreground, Color::BLACK);
                assert_eq!(background, Color::WHITE);
            }
            other => panic!("expected a hatch, got {other:?}"),
        }
    }

    /// A package with one slide part related to one image, so blip fills resolve.
    fn env_with_image() -> Env {
        use std::io::{Cursor, Write};
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            w.start_file("[Content_Types].xml", opts).expect("s");
            w.write_all(
                br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="png" ContentType="image/png"/></Types>"#,
            )
            .expect("w");
            w.start_file("ppt/slides/_rels/slide1.xml.rels", opts)
                .expect("s");
            w.write_all(
                br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/>
</Relationships>"#,
            )
            .expect("w");
            w.start_file("ppt/media/image1.png", opts).expect("s");
            w.write_all(b"\x89PNG\r\n\x1a\n").expect("w");
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

    const SLIDE_PART: &str = "ppt/slides/slide1.xml";

    #[test]
    fn a_tiled_bitmap_fill_keeps_its_repeat_rather_than_stretching() {
        let e = env_with_image();
        let r = Resolver::new(&e.pres, &e.chain);
        let tiled = crate::model::fill::BlipFill {
            embed_id: Some("rId1".into()),
            mode: BlipMode::Tile {
                scale_x: 0.5,
                scale_y: 0.25,
                // 1pt and 2pt, in EMUs.
                offset_x: 12_700,
                offset_y: 25_400,
            },
            ..Default::default()
        };
        // The tile must survive into the display list. Stretching a texture tile instead
        // of repeating it averages it into a flat wash, which is how this looked when it
        // was broken: plausible, and completely wrong.
        match fill_to_paint(&Fill::Blip(tiled), BOX, &r, &e.pres, SLIDE_PART) {
            Some(Paint::Image {
                tile: Some(t),
                opacity,
                ..
            }) => {
                assert_eq!((t.scale_x, t.scale_y), (0.5, 0.25));
                assert_eq!((t.offset_x, t.offset_y), (1.0, 2.0), "offsets are points");
                assert_eq!(opacity, 1.0);
            }
            other => panic!("expected a tiled image, got {other:?}"),
        }
    }

    #[test]
    fn a_stretched_bitmap_fill_carries_no_tile() {
        let e = env_with_image();
        let r = Resolver::new(&e.pres, &e.chain);
        let stretched = crate::model::fill::BlipFill {
            embed_id: Some("rId1".into()),
            mode: BlipMode::Stretch,
            ..Default::default()
        };
        assert!(
            matches!(
                fill_to_paint(&Fill::Blip(stretched), BOX, &r, &e.pres, SLIDE_PART),
                Some(Paint::Image { tile: None, .. })
            ),
            "stretch must not be reported as a tile"
        );
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
        assert_eq!(
            fill_to_paint(&f, BOX, &r, &e.pres, "ppt/slides/slide1.xml"),
            None
        );
    }
}
