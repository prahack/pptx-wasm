//! Geometry evaluation: [`Geometry`] → concrete [`Path`]s in shape-local points.

use crate::dl::{FillRule, Path, Rect};
use crate::model::geometry::{
    CustomGeometry, Expr, GeomCommand, GeomPath, Geometry, GuideContext, PathFillMode,
};

use super::preset;

/// One evaluated subpath together with how it should be painted.
#[derive(Debug, Clone)]
pub struct EvaluatedPath {
    pub path: Path,
    pub fill: bool,
    pub stroke: bool,
    /// How this face shades the shape's fill. `Normal` for everything except the faces
    /// of 3-D-looking presets and custom paths that ask for it.
    pub shade: PathFillMode,
}

/// Evaluates a shape's geometry inside a `w` x `h` point box at the origin.
///
/// Custom geometry can declare several `<a:path>` elements with independent fill/stroke
/// flags, so the result is a list. Presets always come back as a single entry.
pub fn evaluate(geometry: &Geometry, w: f32, h: f32) -> Vec<EvaluatedPath> {
    match geometry {
        Geometry::Preset {
            preset: name,
            adjustments,
        } => {
            let line_like = geometry.is_line_like();
            preset::faces(name, w, h, adjustments)
                .into_iter()
                .map(|f| EvaluatedPath {
                    path: f.path,
                    fill: !line_like,
                    stroke: f.stroke,
                    shade: f.fill,
                })
                .collect()
        }
        Geometry::Custom(c) => evaluate_custom(c, w, h),
        Geometry::None => vec![EvaluatedPath {
            path: Path::rect(Rect::new(0.0, 0.0, w, h)),
            fill: true,
            stroke: true,
            shade: PathFillMode::Normal,
        }],
    }
}

fn evaluate_custom(geom: &CustomGeometry, w: f32, h: f32) -> Vec<EvaluatedPath> {
    // Guides are evaluated in the shape's own coordinate space, which for a custom
    // geometry is whatever `<a:path w=".." h="..">` declares — but the adjust and guide
    // lists are written against the shape box, so seed with that.
    let mut ctx = GuideContext::new(w as f64, h as f64);
    ctx.eval_guides(&geom.adjust);
    ctx.eval_guides(&geom.guides);

    let mut out = Vec::with_capacity(geom.paths.len());
    for gp in &geom.paths {
        // Path-space to shape-space scale.
        let sx = match gp.width {
            Some(pw) if pw > 0 => w / pw as f32,
            _ => 1.0,
        };
        let sy = match gp.height {
            Some(ph) if ph > 0 => h / ph as f32,
            _ => 1.0,
        };
        let path = build_path(gp, &ctx, sx, sy);
        if path.is_empty() {
            continue;
        }
        out.push(EvaluatedPath {
            path,
            fill: gp.fill_mode != PathFillMode::None,
            shade: gp.fill_mode,
            stroke: gp.stroke,
        });
    }
    if out.is_empty() {
        // A custom geometry with no usable paths still needs to occupy its box so any
        // fill or text on the shape lands somewhere.
        out.push(EvaluatedPath {
            path: Path::rect(Rect::new(0.0, 0.0, w, h)),
            fill: true,
            stroke: false,
            shade: PathFillMode::Normal,
        });
    }
    out
}

fn build_path(gp: &GeomPath, ctx: &GuideContext, sx: f32, sy: f32) -> Path {
    let mut p = Path::new();
    let mut cursor = (0.0f32, 0.0f32);
    let x = |e: &Expr| (ctx.eval_expr(e) as f32) * sx;
    let y = |e: &Expr| (ctx.eval_expr(e) as f32) * sy;

    for cmd in &gp.commands {
        match cmd {
            GeomCommand::MoveTo(ex, ey) => {
                cursor = (x(ex), y(ey));
                p.move_to(cursor.0, cursor.1);
            }
            GeomCommand::LineTo(ex, ey) => {
                cursor = (x(ex), y(ey));
                if p.is_empty() {
                    p.move_to(cursor.0, cursor.1);
                } else {
                    p.line_to(cursor.0, cursor.1);
                }
            }
            GeomCommand::CubicTo(a, b, c, d, e, f) => {
                if p.is_empty() {
                    p.move_to(x(a), y(b));
                }
                cursor = (x(e), y(f));
                p.cubic_to(x(a), y(b), x(c), y(d), cursor.0, cursor.1);
            }
            GeomCommand::QuadTo(a, b, c, d) => {
                if p.is_empty() {
                    p.move_to(x(a), y(b));
                }
                cursor = (x(c), y(d));
                p.quad_to(x(a), y(b), cursor.0, cursor.1);
            }
            GeomCommand::ArcTo {
                wr,
                hr,
                start_angle,
                swing_angle,
            } => {
                // `arcTo` is relative: the arc starts at the current point, so the centre
                // is derived by walking back along the start angle.
                let rx = (ctx.eval_expr(wr) as f32) * sx;
                let ry = (ctx.eval_expr(hr) as f32) * sy;
                let start = (ctx.eval_expr(start_angle) as f32 / 60_000.0).to_radians();
                let sweep = (ctx.eval_expr(swing_angle) as f32 / 60_000.0).to_radians();
                if rx.abs() < f32::EPSILON || ry.abs() < f32::EPSILON {
                    continue;
                }
                let cx = cursor.0 - rx * start.cos();
                let cy = cursor.1 - ry * start.sin();
                if p.is_empty() {
                    p.move_to(cursor.0, cursor.1);
                }
                p.arc_to(cx, cy, rx, ry, start, sweep);
                cursor = (
                    cx + rx * (start + sweep).cos(),
                    cy + ry * (start + sweep).sin(),
                );
            }
            GeomCommand::Close => {
                p.close();
            }
        }
    }
    p
}

/// The rectangle a shape's text body occupies, honouring a custom geometry's `<a:rect>`.
pub fn text_rect(geometry: &Geometry, w: f32, h: f32) -> Rect {
    let full = Rect::new(0.0, 0.0, w, h);
    let Geometry::Custom(c) = geometry else {
        return full;
    };
    let Some(r) = &c.text_rect else { return full };
    let mut ctx = GuideContext::new(w as f64, h as f64);
    ctx.eval_guides(&c.adjust);
    ctx.eval_guides(&c.guides);
    let l = ctx.eval_expr(&r[0]) as f32;
    let t = ctx.eval_expr(&r[1]) as f32;
    let right = ctx.eval_expr(&r[2]) as f32;
    let b = ctx.eval_expr(&r[3]) as f32;
    if right <= l || b <= t {
        return full;
    }
    Rect::new(l, t, right - l, b - t)
}

/// OOXML shapes with multiple subpaths use the even-odd rule, which is what makes a donut
/// have a hole.
pub const FILL_RULE: FillRule = FillRule::EvenOdd;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::geometry::{Guide, PathFillMode};

    fn custom(paths: Vec<GeomPath>, guides: Vec<Guide>) -> Geometry {
        Geometry::Custom(Box::new(CustomGeometry {
            adjust: Vec::new(),
            guides,
            paths,
            text_rect: None,
        }))
    }

    #[test]
    fn a_preset_evaluates_to_one_fillable_path() {
        let out = evaluate(&Geometry::preset("rect"), 100.0, 50.0);
        assert_eq!(out.len(), 1);
        assert!(out[0].fill && out[0].stroke);
        assert_eq!(out[0].path.bounds(), Rect::new(0.0, 0.0, 100.0, 50.0));
    }

    #[test]
    fn a_line_preset_is_stroked_but_not_filled() {
        let out = evaluate(&Geometry::preset("straightConnector1"), 100.0, 50.0);
        assert!(!out[0].fill, "connectors must never be filled");
        assert!(out[0].stroke);
    }

    #[test]
    fn no_geometry_falls_back_to_the_shape_box() {
        let out = evaluate(&Geometry::None, 40.0, 20.0);
        assert_eq!(out[0].path.bounds(), Rect::new(0.0, 0.0, 40.0, 20.0));
    }

    #[test]
    fn custom_path_coordinates_scale_from_path_space_to_the_shape_box() {
        let g = custom(
            vec![GeomPath {
                width: Some(200),
                height: Some(100),
                commands: vec![
                    GeomCommand::MoveTo(Expr::Literal(0.0), Expr::Literal(0.0)),
                    GeomCommand::LineTo(Expr::Literal(200.0), Expr::Literal(100.0)),
                    GeomCommand::Close,
                ],
                ..Default::default()
            }],
            Vec::new(),
        );
        // Same path space, but a shape box twice as large: coordinates must double.
        let out = evaluate(&g, 400.0, 200.0);
        assert_eq!(out.len(), 1);
        let b = out[0].path.bounds();
        assert_eq!(b, Rect::new(0.0, 0.0, 400.0, 200.0));
    }

    #[test]
    fn a_path_without_a_declared_space_is_already_in_shape_coordinates() {
        let g = custom(
            vec![GeomPath {
                width: None,
                height: None,
                commands: vec![
                    GeomCommand::MoveTo(Expr::Literal(10.0), Expr::Literal(10.0)),
                    GeomCommand::LineTo(Expr::Literal(30.0), Expr::Literal(20.0)),
                ],
                ..Default::default()
            }],
            Vec::new(),
        );
        let out = evaluate(&g, 100.0, 100.0);
        assert_eq!(out[0].path.bounds(), Rect::new(10.0, 10.0, 20.0, 10.0));
    }

    #[test]
    fn guide_names_in_path_commands_are_resolved() {
        let g = custom(
            vec![GeomPath {
                width: Some(100),
                height: Some(100),
                commands: vec![
                    GeomCommand::MoveTo(Expr::Literal(0.0), Expr::Literal(0.0)),
                    GeomCommand::LineTo(Expr::Name("half".into()), Expr::Literal(0.0)),
                ],
                ..Default::default()
            }],
            vec![Guide {
                name: "half".into(),
                // Guides are evaluated against the shape box (100 wide here).
                formula: "*/ w 1 2".into(),
            }],
        );
        let out = evaluate(&g, 100.0, 100.0);
        assert_eq!(out[0].path.bounds().w, 50.0);
    }

    #[test]
    fn a_path_marked_fill_none_is_stroke_only() {
        let g = custom(
            vec![GeomPath {
                fill_mode: PathFillMode::None,
                stroke: true,
                commands: vec![
                    GeomCommand::MoveTo(Expr::Literal(0.0), Expr::Literal(0.0)),
                    GeomCommand::LineTo(Expr::Literal(10.0), Expr::Literal(10.0)),
                ],
                ..Default::default()
            }],
            Vec::new(),
        );
        let out = evaluate(&g, 100.0, 100.0);
        assert!(!out[0].fill);
        assert!(out[0].stroke);
    }

    #[test]
    fn several_paths_each_keep_their_own_paint_flags() {
        let g = custom(
            vec![
                GeomPath {
                    fill_mode: PathFillMode::Normal,
                    stroke: false,
                    commands: vec![
                        GeomCommand::MoveTo(Expr::Literal(0.0), Expr::Literal(0.0)),
                        GeomCommand::LineTo(Expr::Literal(10.0), Expr::Literal(0.0)),
                        GeomCommand::Close,
                    ],
                    ..Default::default()
                },
                GeomPath {
                    fill_mode: PathFillMode::None,
                    stroke: true,
                    commands: vec![
                        GeomCommand::MoveTo(Expr::Literal(0.0), Expr::Literal(5.0)),
                        GeomCommand::LineTo(Expr::Literal(10.0), Expr::Literal(5.0)),
                    ],
                    ..Default::default()
                },
            ],
            Vec::new(),
        );
        let out = evaluate(&g, 100.0, 100.0);
        assert_eq!(out.len(), 2);
        assert!(out[0].fill && !out[0].stroke);
        assert!(!out[1].fill && out[1].stroke);
    }

    #[test]
    fn an_empty_custom_geometry_still_occupies_its_box() {
        let g = custom(Vec::new(), Vec::new());
        let out = evaluate(&g, 60.0, 30.0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].path.bounds(), Rect::new(0.0, 0.0, 60.0, 30.0));
    }

    #[test]
    fn an_arc_command_starts_at_the_current_point() {
        let g = custom(
            vec![GeomPath {
                width: Some(100),
                height: Some(100),
                commands: vec![
                    GeomCommand::MoveTo(Expr::Literal(50.0), Expr::Literal(0.0)),
                    GeomCommand::ArcTo {
                        wr: Expr::Literal(50.0),
                        hr: Expr::Literal(50.0),
                        // Start at -90° (top), sweep 90° clockwise.
                        start_angle: Expr::Literal(-5_400_000.0),
                        swing_angle: Expr::Literal(5_400_000.0),
                    },
                ],
                ..Default::default()
            }],
            Vec::new(),
        );
        let out = evaluate(&g, 100.0, 100.0);
        let last = out[0].path.points.last().copied().unwrap_or_default();
        // Ends 90° further round: at the right of the circle centred on (50, 50).
        assert!((last.x - 100.0).abs() < 0.5, "x={}", last.x);
        assert!((last.y - 50.0).abs() < 0.5, "y={}", last.y);
    }

    #[test]
    fn a_zero_radius_arc_is_skipped_rather_than_producing_nan() {
        let g = custom(
            vec![GeomPath {
                commands: vec![
                    GeomCommand::MoveTo(Expr::Literal(0.0), Expr::Literal(0.0)),
                    GeomCommand::ArcTo {
                        wr: Expr::Literal(0.0),
                        hr: Expr::Literal(0.0),
                        start_angle: Expr::Literal(0.0),
                        swing_angle: Expr::Literal(5_400_000.0),
                    },
                ],
                ..Default::default()
            }],
            Vec::new(),
        );
        let out = evaluate(&g, 100.0, 100.0);
        assert!(out[0]
            .path
            .points
            .iter()
            .all(|p| p.x.is_finite() && p.y.is_finite()));
    }

    #[test]
    fn text_rect_defaults_to_the_whole_box_and_honours_a_custom_one() {
        assert_eq!(
            text_rect(&Geometry::preset("rect"), 100.0, 50.0),
            Rect::new(0.0, 0.0, 100.0, 50.0)
        );
        let g = Geometry::Custom(Box::new(CustomGeometry {
            text_rect: Some([
                Expr::Literal(10.0),
                Expr::Literal(5.0),
                Expr::Literal(90.0),
                Expr::Literal(45.0),
            ]),
            ..Default::default()
        }));
        assert_eq!(text_rect(&g, 100.0, 50.0), Rect::new(10.0, 5.0, 80.0, 40.0));
    }

    #[test]
    fn an_inverted_text_rect_is_rejected_in_favour_of_the_whole_box() {
        let g = Geometry::Custom(Box::new(CustomGeometry {
            text_rect: Some([
                Expr::Literal(90.0),
                Expr::Literal(45.0),
                Expr::Literal(10.0),
                Expr::Literal(5.0),
            ]),
            ..Default::default()
        }));
        assert_eq!(text_rect(&g, 100.0, 50.0), Rect::new(0.0, 0.0, 100.0, 50.0));
    }
}
