//! Viewport culling.
//!
//! Skips drawing commands that cannot affect a single pixel of the output. On an ordinary
//! slide this saves nothing worth measuring; on a dense one — a 2000-shape diagram, or any
//! slide viewed zoomed in — it is the difference between holding a frame and not, because
//! the cost of a Canvas2D draw call is paid whether or not the result lands on screen.
//!
//! **This must never change what is drawn.** A command is skipped only when its bounds,
//! transformed into device space and inflated by every effect that can spread it, lie
//! entirely outside the visible area. Every estimate here errs outward: a bounds that is
//! too large costs a wasted draw call, one that is too small drops something the user
//! should have seen.
//!
//! Culling lives here rather than in a backend so that every backend gets it, and so it
//! can be tested on the host against the recording renderer.

use pptx_core::dl::{Command, Path, Rect, Shadow, TextRun, Transform};

/// Tracks transform, clip and shadow state well enough to decide what cannot be seen.
pub struct Culler {
    /// Visible area in device pixels.
    visible: Rect,
    transform: Transform,
    /// Conservative clip bounds in device space; `None` means unclipped.
    clip: Option<Rect>,
    shadow: Option<Shadow>,
    stack: Vec<(Transform, Option<Rect>, Option<Shadow>)>,
    skipped: usize,
    considered: usize,
}

impl Culler {
    pub fn new(visible: Rect, root: Transform) -> Self {
        Culler {
            visible,
            transform: root,
            clip: None,
            shadow: None,
            stack: Vec::new(),
            skipped: 0,
            considered: 0,
        }
    }

    /// Commands skipped, and commands that were candidates. Reported by the dev overlay.
    pub fn stats(&self) -> (usize, usize) {
        (self.skipped, self.considered)
    }

    /// Updates state for `cmd` and reports whether the backend can skip it.
    ///
    /// State commands always return `false`: skipping a `Save` or a `Concat` would
    /// desynchronise the backend from this culler and corrupt everything after it.
    pub fn should_skip(&mut self, cmd: &Command) -> bool {
        match cmd {
            Command::Save => {
                self.stack.push((self.transform, self.clip, self.shadow));
                false
            }
            Command::Restore => {
                if let Some((t, c, s)) = self.stack.pop() {
                    self.transform = t;
                    self.clip = c;
                    self.shadow = s;
                }
                false
            }
            Command::Concat(t) => {
                self.transform = t.then(&self.transform);
                false
            }
            Command::ClipRect(r) => {
                self.intersect_clip(transform_rect(*r, &self.transform));
                false
            }
            Command::ClipPath { path, .. } => {
                self.intersect_clip(transform_rect(path.bounds(), &self.transform));
                false
            }
            Command::SetShadow(s) => {
                self.shadow = *s;
                false
            }
            Command::FillPath { path, .. } => {
                let bounds = self.path_bounds(path, 0.0);
                self.cull_opt(bounds)
            }
            Command::StrokePath { path, stroke } => {
                // Half the stroke width spills either side of the path, and a miter join
                // can reach further still.
                let inflate = stroke.width * stroke.miter_limit.max(1.0) / 2.0;
                let bounds = self.path_bounds(path, inflate);
                self.cull_opt(bounds)
            }
            Command::DrawImage { dst, .. } => {
                let bounds = self.device_bounds(*dst, 0.0);
                self.cull(bounds)
            }
            Command::DrawText(run) => {
                let bounds = self.device_bounds(text_bounds(run), 0.0);
                self.cull(bounds)
            }
        }
    }

    /// Bounds for a path, or `None` when the path has no geometry at all — an empty path
    /// draws nothing, so it is always safe to skip.
    fn path_bounds(&self, path: &Path, inflate: f32) -> Option<Rect> {
        if path.is_empty() {
            return None;
        }
        Some(self.device_bounds(path.bounds(), inflate))
    }

    /// A local-space rect in device space, inflated for the stroke and any live shadow.
    fn device_bounds(&self, local: Rect, inflate: f32) -> Rect {
        let inflated = inflate_rect(local, inflate);
        let mut device = transform_rect(inflated, &self.transform);
        if let Some(s) = self.shadow {
            // A Gaussian blur has no hard edge; three sigma covers essentially all of it.
            let scale = self.transform.approx_scale().max(1e-4);
            let spread = (s.blur * 3.0) * scale;
            device = inflate_rect(device, spread);
            let (dx, dy) = (s.offset_x * scale, s.offset_y * scale);
            device = device.union(&Rect::new(device.x + dx, device.y + dy, device.w, device.h));
        }
        device
    }

    fn intersect_clip(&mut self, r: Rect) {
        self.clip = Some(match self.clip {
            Some(existing) => intersect(existing, r),
            None => r,
        });
    }

    /// `None` bounds means "nothing to draw", which is always skippable.
    fn cull_opt(&mut self, bounds: Option<Rect>) -> bool {
        match bounds {
            Some(b) => self.cull(b),
            None => {
                self.considered += 1;
                self.skipped += 1;
                true
            }
        }
    }

    fn cull(&mut self, bounds: Rect) -> bool {
        self.considered += 1;
        // A zero-area bounds is not evidence of invisibility: a horizontal rule is a real
        // path with no height, and a degenerate path may still stroke.
        let limit = match self.clip {
            Some(c) => intersect(self.visible, c),
            None => self.visible,
        };
        let hidden = !overlaps(bounds, limit);
        if hidden {
            self.skipped += 1;
        }
        hidden
    }
}

/// Vertical extent of a text run, generously.
///
/// The display list carries the advance width but no vertical metrics, so this uses the
/// font size: 1.2em above the baseline covers any ascender or diacritic, 0.5em below
/// covers descenders. Over-estimating costs a draw call; under-estimating clips a glyph.
fn text_bounds(run: &TextRun) -> Rect {
    let size = run.font.size().max(1.0);
    let width = run.width.max(size * run.text.chars().count() as f32);
    Rect::new(
        run.origin.x - size * 0.25,
        run.origin.y - size * 1.2,
        width + size * 0.5,
        size * 1.7,
    )
}

/// The axis-aligned bounds of `r` after `t`.
///
/// All four corners are transformed, not just two: under a rotation the min/max corners
/// of the source rect are not the min/max corners of the result.
fn transform_rect(r: Rect, t: &Transform) -> Rect {
    let corners = [
        t.apply(pptx_core::dl::Point::new(r.x, r.y)),
        t.apply(pptx_core::dl::Point::new(r.right(), r.y)),
        t.apply(pptx_core::dl::Point::new(r.right(), r.bottom())),
        t.apply(pptx_core::dl::Point::new(r.x, r.bottom())),
    ];
    // Every corner is checked directly, because `f32::min`/`max` *ignore* NaN — a NaN
    // corner would silently leave the running extremes untouched and produce a plausible
    // but wrong rect, which is how culling starts hiding real content.
    if corners.iter().any(|c| !c.x.is_finite() || !c.y.is_finite()) {
        return EVERYWHERE;
    }
    let (mut minx, mut miny) = (f32::MAX, f32::MAX);
    let (mut maxx, mut maxy) = (f32::MIN, f32::MIN);
    for c in corners {
        minx = minx.min(c.x);
        miny = miny.min(c.y);
        maxx = maxx.max(c.x);
        maxy = maxy.max(c.y);
    }
    Rect::new(minx, miny, maxx - minx, maxy - miny)
}

/// A rect that overlaps anything. Returned when the transform is degenerate, so that a
/// situation we cannot reason about results in drawing too much rather than too little.
const EVERYWHERE: Rect = Rect::new(-1e30, -1e30, 2e30, 2e30);

fn inflate_rect(r: Rect, by: f32) -> Rect {
    if by <= 0.0 {
        return r;
    }
    Rect::new(r.x - by, r.y - by, r.w + by * 2.0, r.h + by * 2.0)
}

fn intersect(a: Rect, b: Rect) -> Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = a.right().min(b.right());
    let bottom = a.bottom().min(b.bottom());
    Rect::new(x, y, (right - x).max(0.0), (bottom - y).max(0.0))
}

/// Overlap test that treats a zero-width or zero-height rect as still able to draw.
fn overlaps(a: Rect, b: Rect) -> bool {
    a.x <= b.right() && a.right() >= b.x && a.y <= b.bottom() && a.bottom() >= b.y
}

#[cfg(test)]
mod tests {
    use super::*;
    use pptx_core::dl::{Color, Paint, Point, Stroke, TextRun};

    const VIEW: Rect = Rect::new(0.0, 0.0, 960.0, 540.0);

    fn culler() -> Culler {
        Culler::new(VIEW, Transform::IDENTITY)
    }

    fn fill(x: f32, y: f32, w: f32, h: f32) -> Command {
        Command::FillPath {
            path: Path::rect(Rect::new(x, y, w, h)),
            paint: Paint::Solid(Color::BLACK),
            rule: Default::default(),
        }
    }

    fn text(x: f32, y: f32) -> Command {
        Command::DrawText(TextRun {
            text: "hello".into(),
            font: pptx_core::dl::FontSpec::new("Arial", 12.0),
            origin: Point::new(x, y),
            paint: Paint::Solid(Color::BLACK),
            advances: vec![6.0; 5],
            width: 30.0,
            decorations: Default::default(),
            letter_spacing: 0.0,
        })
    }

    #[test]
    fn something_on_screen_is_kept() {
        let mut c = culler();
        assert!(!c.should_skip(&fill(10.0, 10.0, 100.0, 100.0)));
    }

    #[test]
    fn something_far_off_screen_is_skipped() {
        let mut c = culler();
        assert!(c.should_skip(&fill(-5000.0, -5000.0, 10.0, 10.0)));
        assert!(c.should_skip(&fill(5000.0, 100.0, 10.0, 10.0)));
        assert_eq!(c.stats(), (2, 2));
    }

    #[test]
    fn something_straddling_the_edge_is_kept() {
        let mut c = culler();
        assert!(!c.should_skip(&fill(-50.0, -50.0, 100.0, 100.0)));
        assert!(!c.should_skip(&fill(940.0, 520.0, 100.0, 100.0)));
    }

    #[test]
    fn state_commands_are_never_skipped() {
        let mut c = culler();
        for cmd in [
            Command::Save,
            Command::Concat(Transform::translate(1.0, 1.0)),
            Command::ClipRect(Rect::new(0.0, 0.0, 10.0, 10.0)),
            Command::SetShadow(None),
            Command::Restore,
        ] {
            assert!(
                !c.should_skip(&cmd),
                "state command {cmd:?} must not be skipped"
            );
        }
    }

    #[test]
    fn a_transform_moves_what_is_visible() {
        let mut c = culler();
        // Pushed 2000pt to the right, a shape at the origin is off screen.
        c.should_skip(&Command::Concat(Transform::translate(2000.0, 0.0)));
        assert!(c.should_skip(&fill(0.0, 0.0, 100.0, 100.0)));
        // And pulling it back brings it into view.
        c.should_skip(&Command::Concat(Transform::translate(-2000.0, 0.0)));
        assert!(!c.should_skip(&fill(0.0, 0.0, 100.0, 100.0)));
    }

    #[test]
    fn restore_undoes_a_transform_for_culling_too() {
        let mut c = culler();
        c.should_skip(&Command::Save);
        c.should_skip(&Command::Concat(Transform::translate(5000.0, 0.0)));
        assert!(c.should_skip(&fill(0.0, 0.0, 10.0, 10.0)));
        c.should_skip(&Command::Restore);
        assert!(!c.should_skip(&fill(0.0, 0.0, 10.0, 10.0)));
    }

    #[test]
    fn a_rotation_uses_all_four_corners() {
        // A rect rotated 45 degrees about a point just off screen can still reach onto it,
        // which a two-corner bounds calculation would miss.
        let mut c = culler();
        c.should_skip(&Command::Concat(Transform::rotate_about(
            std::f32::consts::PI / 4.0,
            0.0,
            0.0,
        )));
        assert!(!c.should_skip(&fill(0.0, -50.0, 100.0, 100.0)));
    }

    #[test]
    fn a_clip_can_hide_something_that_is_otherwise_on_screen() {
        let mut c = culler();
        c.should_skip(&Command::ClipRect(Rect::new(0.0, 0.0, 50.0, 50.0)));
        assert!(c.should_skip(&fill(400.0, 400.0, 20.0, 20.0)));
        assert!(!c.should_skip(&fill(10.0, 10.0, 20.0, 20.0)));
    }

    #[test]
    fn a_shadow_keeps_a_shape_that_is_just_off_screen() {
        let mut c = culler();
        c.should_skip(&Command::SetShadow(Some(Shadow {
            blur: 20.0,
            offset_x: 30.0,
            offset_y: 0.0,
            color: Color::BLACK,
        })));
        // The shape is off the left edge, but its shadow reaches back onto the canvas.
        assert!(!c.should_skip(&fill(-40.0, 100.0, 20.0, 20.0)));
    }

    #[test]
    fn a_wide_stroke_keeps_a_hairline_that_straddles_the_edge() {
        let mut c = culler();
        let mut path = Path::new();
        path.move_to(-10.0, 100.0).line_to(-2.0, 100.0);
        assert!(!c.should_skip(&Command::StrokePath {
            path,
            stroke: Stroke {
                width: 20.0,
                ..Default::default()
            },
        }));
    }

    #[test]
    fn text_is_culled_on_its_own_extent_not_just_its_origin() {
        let mut c = culler();
        // Baseline above the top edge by more than the ascent.
        assert!(c.should_skip(&text(100.0, -200.0)));
        // Baseline just above the top edge: the glyphs still show.
        assert!(!c.should_skip(&text(100.0, 2.0)));
    }

    #[test]
    fn an_empty_path_is_skipped_without_panicking() {
        let mut c = culler();
        assert!(c.should_skip(&Command::FillPath {
            path: Path::new(),
            paint: Paint::Solid(Color::BLACK),
            rule: Default::default(),
        }));
    }

    #[test]
    fn a_degenerate_transform_disables_culling_rather_than_hiding_everything() {
        let mut c = Culler::new(
            VIEW,
            Transform {
                a: f32::NAN,
                ..Transform::IDENTITY
            },
        );
        assert!(!c.should_skip(&fill(10.0, 10.0, 10.0, 10.0)));
    }
}
