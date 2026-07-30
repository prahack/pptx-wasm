//! A backend that writes commands out as text instead of pixels.
//!
//! Two jobs. It is the host-side test double, so renderer behaviour — clip nesting,
//! transform composition, what gets skipped — can be asserted in `cargo test` without a
//! browser. And it is a debugging tool: dumping a slide's trace is how you tell a layout
//! bug from a rasterisation bug, which is the single most common ambiguity in this
//! project.
//!
//! The output is deliberately stable and diffable. Do not "improve" the formatting
//! without regenerating whatever depends on it.

use std::fmt::Write as _;

use pptx_core::dl::{Command, DisplayList, Paint, Path, PathVerb, Transform};

use crate::Renderer;

#[derive(Debug, Default)]
pub struct RecordingRenderer {
    out: String,
    depth: usize,
    /// Set when a Restore arrives with nothing to pop, which is always a layout bug.
    unbalanced: bool,
    counts: Counts,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub fills: usize,
    pub strokes: usize,
    pub texts: usize,
    pub images: usize,
    pub clips: usize,
    pub shadows: usize,
    pub soft_edges: usize,
}

impl RecordingRenderer {
    pub fn new() -> Self {
        Self::default()
    }

    /// The recorded trace.
    pub fn finish(self) -> String {
        self.out
    }

    pub fn trace(&self) -> &str {
        &self.out
    }

    pub fn counts(&self) -> Counts {
        self.counts
    }

    pub fn is_unbalanced(&self) -> bool {
        self.unbalanced
    }

    fn line(&mut self, s: &str) {
        for _ in 0..self.depth {
            self.out.push_str("  ");
        }
        self.out.push_str(s);
        self.out.push('\n');
    }
}

impl Renderer for RecordingRenderer {
    type Error = std::convert::Infallible;

    fn begin_frame(&mut self, dl: &DisplayList, root: Transform) -> Result<(), Self::Error> {
        self.out.clear();
        self.depth = 0;
        self.unbalanced = false;
        self.counts = Counts::default();
        let header = format!(
            "frame {:.2}x{:.2}pt root={}",
            dl.width_pt,
            dl.height_pt,
            fmt_transform(&root)
        );
        self.line(&header);
        Ok(())
    }

    fn execute(&mut self, cmd: &Command) -> Result<(), Self::Error> {
        match cmd {
            Command::Save => {
                self.line("save");
                self.depth += 1;
            }
            Command::BeginSoftEdge(radius) => {
                self.line(&format!("beginSoftEdge {radius:.2}"));
                self.depth += 1;
                self.counts.soft_edges += 1;
            }
            Command::EndSoftEdge => {
                if self.depth == 0 {
                    self.unbalanced = true;
                    self.line("endSoftEdge (UNBALANCED)");
                } else {
                    self.depth -= 1;
                    self.line("endSoftEdge");
                }
            }
            Command::Restore => {
                if self.depth == 0 {
                    self.unbalanced = true;
                    self.line("restore (UNBALANCED)");
                } else {
                    self.depth -= 1;
                    self.line("restore");
                }
            }
            Command::Concat(t) => {
                let s = format!("concat {}", fmt_transform(t));
                self.line(&s);
            }
            Command::ClipRect(r) => {
                self.counts.clips += 1;
                let s = format!("clipRect {:.2},{:.2} {:.2}x{:.2}", r.x, r.y, r.w, r.h);
                self.line(&s);
            }
            Command::ClipPath { path, rule } => {
                self.counts.clips += 1;
                let s = format!("clipPath {:?} {}", rule, fmt_path(path));
                self.line(&s);
            }
            Command::SetShadow(shadow) => {
                self.counts.shadows += 1;
                let s = match shadow {
                    Some(sh) => format!(
                        "shadow {} blur={:.2} offset={:.2},{:.2}",
                        sh.color.to_css(),
                        sh.blur,
                        sh.offset_x,
                        sh.offset_y
                    ),
                    None => "shadow none".to_string(),
                };
                self.line(&s);
            }
            Command::FillPath { path, paint, rule } => {
                self.counts.fills += 1;
                let s = format!(
                    "fill {} rule={:?} {}",
                    fmt_paint(paint),
                    rule,
                    fmt_path(path)
                );
                self.line(&s);
            }
            Command::StrokePath { path, stroke } => {
                self.counts.strokes += 1;
                let s = format!(
                    "stroke {} w={:.2} cap={:?} join={:?} dash={} {}",
                    fmt_paint(&stroke.paint),
                    stroke.width,
                    stroke.cap,
                    stroke.join,
                    stroke.dash.len(),
                    fmt_path(path)
                );
                self.line(&s);
            }
            Command::DrawImage {
                image,
                src,
                dst,
                opacity,
            } => {
                self.counts.images += 1;
                let s = format!(
                    "image #{} src={:.3},{:.3} {:.3}x{:.3} dst={:.2},{:.2} {:.2}x{:.2} a={:.2}",
                    image.0, src.x, src.y, src.w, src.h, dst.x, dst.y, dst.w, dst.h, opacity
                );
                self.line(&s);
            }
            Command::DrawText(run) => {
                self.counts.texts += 1;
                let s = format!(
                    "text {:?} at {:.2},{:.2} font={:?} w={:.2}{}{}",
                    run.text,
                    run.origin.x,
                    run.origin.y,
                    run.font.to_css(),
                    run.width,
                    if run.decorations.underline { " u" } else { "" },
                    if run.decorations.strikethrough {
                        " s"
                    } else {
                        ""
                    },
                );
                self.line(&s);
            }
        }
        Ok(())
    }

    fn end_frame(&mut self) -> Result<(), Self::Error> {
        if self.depth != 0 {
            self.unbalanced = true;
            self.depth = 0;
            self.line("end (UNBALANCED: saves left open)");
        }
        Ok(())
    }
}

fn fmt_transform(t: &Transform) -> String {
    if *t == Transform::IDENTITY {
        return "identity".to_string();
    }
    format!(
        "[{:.4} {:.4} {:.4} {:.4} {:.3} {:.3}]",
        t.a, t.b, t.c, t.d, t.e, t.f
    )
}

fn fmt_paint(p: &Paint) -> String {
    match p {
        Paint::Solid(c) => c.to_css(),
        Paint::Gradient(g) => match g {
            pptx_core::dl::Gradient::Linear { stops, .. } => {
                format!("linear-gradient({} stops)", stops.len())
            }
            pptx_core::dl::Gradient::Radial { stops, .. } => {
                format!("radial-gradient({} stops)", stops.len())
            }
        },
        Paint::Image {
            image,
            opacity,
            tile,
            ..
        } => match tile {
            Some(t) => format!(
                "image#{}@{:.2} tiled({:.2}x{:.2}+{:.1},{:.1})",
                image.0, opacity, t.scale_x, t.scale_y, t.offset_x, t.offset_y
            ),
            None => format!("image#{}@{:.2}", image.0, opacity),
        },
        Paint::Hatch {
            pattern,
            foreground,
            background,
        } => format!(
            "hatch {pattern:?} {} on {}",
            foreground.to_css(),
            background.to_css()
        ),
    }
}

/// A path as its bounds plus a verb summary. The full point list would make traces
/// unreadable and would churn on sub-pixel changes that do not matter.
fn fmt_path(p: &Path) -> String {
    if p.is_empty() {
        return "path{}".to_string();
    }
    let b = p.bounds();
    let mut verbs = String::new();
    let (mut moves, mut lines, mut curves, mut closes) = (0, 0, 0, 0);
    for v in &p.verbs {
        match v {
            PathVerb::MoveTo => moves += 1,
            PathVerb::LineTo => lines += 1,
            PathVerb::QuadTo | PathVerb::CubicTo => curves += 1,
            PathVerb::Close => closes += 1,
        }
    }
    let _ = write!(verbs, "M{moves} L{lines} C{curves} Z{closes}");
    format!(
        "path{{{:.2},{:.2} {:.2}x{:.2} {verbs}}}",
        b.x, b.y, b.w, b.h
    )
}

/// Convenience: render a display list to a trace string.
///
/// Deliberately unculled. A trace is a diagnostic, and one that omitted off-screen
/// commands would change with the window size — exactly when you are trying to work out
/// why something is not being drawn.
pub fn trace(dl: &DisplayList, view: &pptx_core::dl::View) -> String {
    let mut r = RecordingRenderer::new();
    // The recording backend is infallible, so this cannot fail.
    let _ = crate::render_unculled(&mut r, dl, view);
    r.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pptx_core::dl::{Color, Fit, Rect, View};

    fn view() -> View {
        View {
            viewport_w: 960.0,
            viewport_h: 540.0,
            dpr: 1.0,
            fit: Fit::Contain,
            ..Default::default()
        }
    }

    #[test]
    fn a_trace_records_the_frame_header_and_each_command() {
        let mut dl = DisplayList::new(960.0, 540.0);
        dl.push(Command::FillPath {
            path: Path::rect(Rect::new(10.0, 20.0, 30.0, 40.0)),
            paint: Paint::Solid(Color::rgb(255, 0, 0)),
            rule: Default::default(),
        });
        let out = trace(&dl, &view());
        assert!(out.starts_with("frame 960.00x540.00pt"), "{out}");
        assert!(out.contains("fill #ff0000"), "{out}");
        assert!(out.contains("10.00,20.00 30.00x40.00"), "{out}");
    }

    #[test]
    fn save_and_restore_indent_the_trace() {
        let mut dl = DisplayList::new(10.0, 10.0);
        dl.push(Command::Save);
        dl.push(Command::ClipRect(Rect::new(0.0, 0.0, 5.0, 5.0)));
        dl.push(Command::Restore);
        let out = trace(&dl, &view());
        assert!(
            out.contains("\n  clipRect"),
            "clip should be indented: {out}"
        );
        assert!(!out.contains("UNBALANCED"));
    }

    #[test]
    fn an_extra_restore_is_reported_rather_than_underflowing() {
        let mut dl = DisplayList::new(10.0, 10.0);
        dl.push(Command::Restore);
        let mut r = RecordingRenderer::new();
        let _ = crate::render(&mut r, &dl, &view());
        assert!(r.is_unbalanced());
        assert!(r.trace().contains("UNBALANCED"));
    }

    #[test]
    fn a_save_left_open_is_reported_at_end_of_frame() {
        let mut dl = DisplayList::new(10.0, 10.0);
        dl.push(Command::Save);
        let mut r = RecordingRenderer::new();
        let _ = crate::render(&mut r, &dl, &view());
        assert!(r.is_unbalanced(), "trace: {}", r.trace());
    }

    #[test]
    fn counts_track_each_command_kind() {
        let mut dl = DisplayList::new(10.0, 10.0);
        dl.push(Command::FillPath {
            path: Path::rect(Rect::new(0.0, 0.0, 1.0, 1.0)),
            paint: Paint::Solid(Color::BLACK),
            rule: Default::default(),
        });
        dl.push(Command::StrokePath {
            path: Path::rect(Rect::new(0.0, 0.0, 1.0, 1.0)),
            stroke: Default::default(),
        });
        dl.push(Command::DrawImage {
            image: pptx_core::dl::ImageId(0),
            src: Rect::new(0.0, 0.0, 1.0, 1.0),
            dst: Rect::new(0.0, 0.0, 1.0, 1.0),
            opacity: 1.0,
        });
        let mut r = RecordingRenderer::new();
        let _ = crate::render(&mut r, &dl, &view());
        assert_eq!(
            r.counts(),
            Counts {
                fills: 1,
                strokes: 1,
                texts: 0,
                images: 1,
                clips: 0,
                shadows: 0,
                soft_edges: 0,
            }
        );
    }

    #[test]
    fn the_root_transform_appears_in_the_header_and_reflects_the_view() {
        let dl = DisplayList::new(960.0, 540.0);
        let half = View {
            viewport_w: 480.0,
            viewport_h: 270.0,
            dpr: 1.0,
            ..Default::default()
        };
        let out = trace(&dl, &half);
        assert!(out.contains("0.5000"), "expected a 0.5 scale in {out}");
    }

    #[test]
    fn traces_are_stable_across_runs() {
        let mut dl = DisplayList::new(100.0, 100.0);
        dl.push(Command::FillPath {
            path: Path::ellipse(Rect::new(0.0, 0.0, 50.0, 50.0)),
            paint: Paint::Solid(Color::rgb(1, 2, 3)),
            rule: Default::default(),
        });
        assert_eq!(trace(&dl, &view()), trace(&dl, &view()));
    }
}
