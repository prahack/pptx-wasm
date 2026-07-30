//! The display list: the contract between layout and every renderer backend.
//!
//! It is a flat command stream rather than a tree. Flat means a backend can iterate it
//! once with a small explicit state stack, cache it, replay it at a different zoom, or
//! ship it across a worker boundary — none of which a tree of shapes with back-pointers
//! into the model would allow.

pub mod geom;
pub mod paint;
pub mod text;

pub use geom::{Path, PathVerb, Point, Rect, Transform};
pub use paint::{
    Color, FillRule, Gradient, GradientStop, HatchPattern, ImageId, LineCap, LineJoin, Paint,
    Shadow, Stroke, Tile,
};
pub use text::{Decorations, FontSpec, FontStyle, FontWeight, TextRun};

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Pushes the clip/transform state.
    Save,
    /// Pops it. A well-formed list has balanced Save/Restore pairs; renderers must
    /// tolerate an unbalanced one rather than underflow.
    Restore,
    /// Multiplies the current transform.
    Concat(Transform),
    /// Intersects the clip with a rectangle in current-transform space.
    ClipRect(Rect),
    ClipPath {
        path: Path,
        rule: FillRule,
    },
    /// Sets (or clears) the drop shadow applied to subsequent drawing. Scoped by the
    /// enclosing `Save`/`Restore`, like a clip.
    SetShadow(Option<Shadow>),
    /// Begins a soft-edged group. Everything up to the matching [`Command::EndSoftEdge`]
    /// is drawn into its own surface, whose alpha is then faded inward from the edge over
    /// `radius` points before being composited back.
    ///
    /// This is a *group* rather than a state flag, unlike `SetShadow`, because there is no
    /// way to feather one drawing operation at a time: the fade is over the silhouette of
    /// everything inside, and a shape's fill and its outline together are one silhouette.
    /// A backend that cannot render to a separate surface must draw the contents plainly
    /// rather than skip them — a hard edge is wrong, an invisible shape is worse.
    BeginSoftEdge(f32),
    /// Ends the group opened by the nearest unmatched [`Command::BeginSoftEdge`].
    EndSoftEdge,
    FillPath {
        path: Path,
        paint: Paint,
        rule: FillRule,
    },
    StrokePath {
        path: Path,
        stroke: Stroke,
    },
    DrawImage {
        image: ImageId,
        /// Source region in normalised 0..1 image coordinates, so the command survives
        /// the image being decoded at any resolution.
        src: Rect,
        dst: Rect,
        opacity: f32,
    },
    DrawText(TextRun),
}

/// One slide, resolved to drawing commands.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DisplayList {
    /// Slide size in points. The renderer maps this onto the canvas via its view transform.
    pub width_pt: f32,
    pub height_pt: f32,
    pub commands: Vec<Command>,
}

impl DisplayList {
    pub fn new(width_pt: f32, height_pt: f32) -> Self {
        DisplayList {
            width_pt,
            height_pt,
            commands: Vec::new(),
        }
    }

    pub fn push(&mut self, cmd: Command) {
        self.commands.push(cmd);
    }

    pub fn len(&self) -> usize {
        self.commands.len()
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Every text run in draw order. Used by tests and by the accessibility text layer.
    pub fn text_runs(&self) -> impl Iterator<Item = &TextRun> {
        self.commands.iter().filter_map(|c| match c {
            Command::DrawText(r) => Some(r),
            _ => None,
        })
    }

    /// Concatenated plain text, one line per run. The React layer exposes this for
    /// selection and search without giving callers the whole display list.
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        for run in self.text_runs() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&run.text);
        }
        out
    }

    /// Verifies Save/Restore balance. Layout bugs that leak a clip are invisible on the
    /// slide that caused them and catastrophic on the next one, so this is asserted in
    /// tests for every fixture rather than left to visual inspection.
    pub fn is_balanced(&self) -> bool {
        let mut depth: i32 = 0;
        for c in &self.commands {
            match c {
                Command::Save => depth += 1,
                Command::Restore => {
                    depth -= 1;
                    if depth < 0 {
                        return false;
                    }
                }
                _ => {}
            }
        }
        depth == 0
    }
}

/// How a slide is fitted into the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fit {
    /// Scale to fit entirely, preserving aspect ratio, centred. The default.
    #[default]
    Contain,
    /// Fill the canvas, preserving aspect ratio, cropping the overflow.
    Cover,
    /// Ignore aspect ratio and stretch.
    Fill,
    /// 1 point = 1 CSS pixel, no scaling.
    Actual,
}

/// Maps slide points onto device pixels. This is the only place a resolution enters the
/// pipeline; the display list above it never changes when zoom or DPR does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    /// Canvas size in CSS pixels.
    pub viewport_w: f32,
    pub viewport_h: f32,
    /// `window.devicePixelRatio`.
    pub dpr: f32,
    pub fit: Fit,
    /// User zoom applied on top of the fit, 1.0 = no additional zoom.
    pub zoom: f32,
    /// Pan offset in CSS pixels, applied after fit and zoom.
    pub pan_x: f32,
    pub pan_y: f32,
}

impl Default for View {
    fn default() -> Self {
        View {
            viewport_w: 0.0,
            viewport_h: 0.0,
            dpr: 1.0,
            fit: Fit::Contain,
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
        }
    }
}

impl View {
    /// The slide-points → device-pixels transform for a display list of the given size.
    pub fn transform_for(&self, dl_w: f32, dl_h: f32) -> Transform {
        if dl_w <= 0.0 || dl_h <= 0.0 {
            return Transform::scale(self.dpr, self.dpr);
        }
        let sx = self.viewport_w / dl_w;
        let sy = self.viewport_h / dl_h;
        let (scale_x, scale_y) = match self.fit {
            Fit::Contain => {
                let s = sx.min(sy);
                (s, s)
            }
            Fit::Cover => {
                let s = sx.max(sy);
                (s, s)
            }
            Fit::Fill => (sx, sy),
            Fit::Actual => (1.0, 1.0),
        };
        let (scale_x, scale_y) = (scale_x * self.zoom, scale_y * self.zoom);
        // Centre whatever is left over, then apply pan, then device pixels.
        let tx = (self.viewport_w - dl_w * scale_x) / 2.0 + self.pan_x;
        let ty = (self.viewport_h - dl_h * scale_y) / 2.0 + self.pan_y;
        Transform::scale(scale_x, scale_y)
            .then(&Transform::translate(tx, ty))
            .then(&Transform::scale(self.dpr, self.dpr))
    }

    /// Backing-store size in device pixels for the current viewport and DPR.
    pub fn canvas_pixels(&self) -> (u32, u32) {
        (
            (self.viewport_w * self.dpr).round().max(1.0) as u32,
            (self.viewport_h * self.dpr).round().max(1.0) as u32,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(w: f32, h: f32, dpr: f32) -> View {
        View {
            viewport_w: w,
            viewport_h: h,
            dpr,
            ..Default::default()
        }
    }

    #[test]
    fn contain_letterboxes_and_centres() {
        // 960x540 slide into a 960x1080 viewport: scale 1, 270pt of letterbox each side.
        let t = view(960.0, 1080.0, 1.0).transform_for(960.0, 540.0);
        assert_eq!(t.apply(Point::new(0.0, 0.0)), Point::new(0.0, 270.0));
        assert_eq!(t.apply(Point::new(960.0, 540.0)), Point::new(960.0, 810.0));
    }

    #[test]
    fn dpr_scales_the_backing_store_but_not_the_layout() {
        let v = view(480.0, 270.0, 2.0);
        assert_eq!(v.canvas_pixels(), (960, 540));
        let t = v.transform_for(960.0, 540.0);
        // Slide fills the viewport at 0.5 CSS px/pt, doubled to 1.0 device px/pt.
        assert_eq!(t.apply(Point::new(960.0, 540.0)), Point::new(960.0, 540.0));
    }

    #[test]
    fn actual_fit_is_one_point_per_css_pixel() {
        let mut v = view(1200.0, 800.0, 1.0);
        v.fit = Fit::Actual;
        let t = v.transform_for(960.0, 540.0);
        let origin = t.apply(Point::new(0.0, 0.0));
        assert_eq!(origin, Point::new(120.0, 130.0)); // centred
        let corner = t.apply(Point::new(960.0, 540.0));
        assert_eq!(corner, Point::new(1080.0, 670.0));
    }

    #[test]
    fn zero_sized_display_list_does_not_divide_by_zero() {
        let t = view(100.0, 100.0, 2.0).transform_for(0.0, 0.0);
        assert!(t.a.is_finite() && t.d.is_finite());
    }

    #[test]
    fn balance_check_catches_a_leaked_save() {
        let mut dl = DisplayList::new(10.0, 10.0);
        dl.push(Command::Save);
        assert!(!dl.is_balanced());
        dl.push(Command::Restore);
        assert!(dl.is_balanced());
        dl.push(Command::Restore);
        assert!(!dl.is_balanced());
    }
}
