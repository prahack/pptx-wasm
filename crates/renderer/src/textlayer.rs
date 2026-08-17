//! A backend that reports where the text is, instead of drawing it.
//!
//! A canvas renders pixels, so its text cannot be selected, copied, found with Ctrl-F or
//! read by a screen reader. Every DOM-based pptx renderer gets all four for nothing, and
//! for a *viewer* — where reading is the whole point — that is the one gap no amount of
//! speed makes up for.
//!
//! The fix does not need a second rasteriser. The display list already carries, for every
//! run, the string, the baseline origin in slide points, the measured advance width and
//! the resolved font. That is enough to place a transparent, correctly-sized `<span>` over
//! the canvas and let the browser handle selection and assistive technology — the same
//! technique pdf.js uses for its text layer.
//!
//! This backend walks the list exactly as the drawing backends do, through the same
//! [`crate::render`], so the geometry it reports cannot drift from the geometry that gets
//! drawn: both see the same transform stack, and a run that is culled is absent from both.

use pptx_core::dl::{Command, DisplayList, Transform};

use crate::Renderer;

/// One run of text, positioned in device pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct PositionedText {
    pub text: String,
    /// Left end of the baseline, in device pixels.
    pub x: f32,
    pub y: f32,
    /// Advance width in device pixels, so the overlay can be stretched to match the
    /// painted run rather than trusting the browser to reproduce its width.
    pub width: f32,
    /// Font size in device pixels.
    pub size: f32,
    /// The CSS `font-family` list, already quoted where it needs to be.
    pub family: String,
    pub weight: u16,
    pub italic: bool,
    /// Clockwise rotation in radians, if the run is not upright.
    pub rotation: f32,
    /// The URL this run links to, if any. Already scheme-checked by the parser.
    pub link: Option<String>,
    /// Per-`char` advance in device pixels, parallel to `text.chars()`.
    ///
    /// Empty when layout had no measurer, in which case a caller wanting a sub-run
    /// rectangle has to fall back to splitting `width` evenly. Carried here so that
    /// highlighting a match *inside* a run does not need a second measuring pass.
    pub advances: Vec<f32>,
}

/// Collects the text of a slide with everything needed to overlay it.
#[derive(Debug, Default)]
pub struct TextLayerRenderer {
    runs: Vec<PositionedText>,
    root: Transform,
    stack: Vec<Transform>,
    current: Transform,
}

impl TextLayerRenderer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn finish(self) -> Vec<PositionedText> {
        self.runs
    }

    pub fn runs(&self) -> &[PositionedText] {
        &self.runs
    }
}

impl Renderer for TextLayerRenderer {
    type Error = core::convert::Infallible;

    fn begin_frame(&mut self, _dl: &DisplayList, root: Transform) -> Result<(), Self::Error> {
        self.runs.clear();
        self.stack.clear();
        self.root = root;
        self.current = root;
        Ok(())
    }

    fn execute(&mut self, cmd: &Command) -> Result<(), Self::Error> {
        match cmd {
            Command::Save => self.stack.push(self.current),
            Command::Restore => {
                if let Some(t) = self.stack.pop() {
                    self.current = t;
                }
            }
            Command::Concat(t) => self.current = t.then(&self.current),
            Command::DrawText(run) => {
                let origin = self.current.apply(run.origin);
                // The scale the transform applies, so a run inside a scaled group is
                // reported at the size it is actually painted rather than its authored
                // size. `approx_scale` is the same figure the drawing backends use to
                // decide device resolution, so the two agree by construction.
                let scale = self.current.approx_scale();
                // Rotation is recovered from where the transform sends the x axis. A run
                // in a rotated group has to be rotated in the overlay too, or selecting
                // it would sweep a horizontal box across unrelated text.
                let ex = self
                    .current
                    .apply(pptx_core::dl::Point::new(run.origin.x + 1.0, run.origin.y));
                let rotation = (ex.y - origin.y).atan2(ex.x - origin.x);
                self.runs.push(PositionedText {
                    text: run.text.clone(),
                    x: origin.x,
                    y: origin.y,
                    width: run.width * scale,
                    size: run.font.size() * scale,
                    family: run.font.families_css(),
                    weight: run.font.weight.css_value(),
                    italic: run.font.style == pptx_core::dl::FontStyle::Italic,
                    rotation,
                    link: run.link.clone(),
                    advances: run.advances.iter().map(|a| a * scale).collect(),
                });
            }
            _ => {}
        }
        Ok(())
    }

    fn end_frame(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pptx_core::dl::{Color, DisplayList, Fit, FontSpec, Paint, Point, TextRun, View};

    fn run(text: &str, x: f32, y: f32) -> TextRun {
        let mut t = TextRun {
            link: None,
            text: text.into(),
            font: FontSpec::new("Calibri", 18.0),
            origin: Point::new(x, y),
            paint: Paint::Solid(Color::BLACK),
            advances: Vec::new(),
            width: 40.0,
            decorations: Default::default(),
            letter_spacing: 0.0,
        };
        t.font.fallbacks = vec!["sans-serif".into()];
        t
    }

    fn view(w: f32, h: f32) -> View {
        View {
            viewport_w: w,
            viewport_h: h,
            dpr: 1.0,
            fit: Fit::Contain,
            ..Default::default()
        }
    }

    fn collect(dl: &DisplayList, view: &View) -> Vec<PositionedText> {
        let mut r = TextLayerRenderer::new();
        crate::render(&mut r, dl, view).expect("infallible");
        r.finish()
    }

    #[test]
    fn a_run_is_reported_at_its_device_position() {
        let mut dl = DisplayList::new(960.0, 540.0);
        dl.push(Command::DrawText(run("Highlights", 100.0, 50.0)));
        let out = collect(&dl, &view(960.0, 540.0));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "Highlights");
        assert!((out[0].x - 100.0).abs() < 0.01 && (out[0].y - 50.0).abs() < 0.01);
        assert!(
            out[0].rotation.abs() < 1e-4,
            "an upright run is not rotated"
        );
    }

    #[test]
    fn position_and_size_scale_with_the_view() {
        let mut dl = DisplayList::new(960.0, 540.0);
        dl.push(Command::DrawText(run("Highlights", 100.0, 50.0)));
        // Twice the device pixels per point: everything the overlay places must double,
        // or the spans drift away from the glyphs as soon as the user zooms.
        let out = collect(&dl, &view(1920.0, 1080.0));
        assert!((out[0].x - 200.0).abs() < 0.5, "x was {}", out[0].x);
        assert!((out[0].size - 36.0).abs() < 0.5, "size was {}", out[0].size);
        assert!(
            (out[0].width - 80.0).abs() < 0.5,
            "width was {}",
            out[0].width
        );
    }

    #[test]
    fn a_rotated_run_reports_its_angle() {
        let mut dl = DisplayList::new(960.0, 540.0);
        dl.push(Command::Save);
        dl.push(Command::Concat(Transform::rotate(
            core::f32::consts::FRAC_PI_2,
        )));
        dl.push(Command::DrawText(run("sideways", 0.0, 0.0)));
        dl.push(Command::Restore);
        let out = collect(&dl, &view(960.0, 540.0));
        assert_eq!(out.len(), 1);
        assert!(
            (out[0].rotation - core::f32::consts::FRAC_PI_2).abs() < 1e-3,
            "rotation was {}",
            out[0].rotation
        );
    }

    #[test]
    fn the_layer_sees_exactly_what_the_drawing_backends_see() {
        // Both walk through `render`, so a run outside the viewport is culled from both.
        // If this ever diverges, the overlay would offer selectable text where nothing is
        // painted, or leave painted text unselectable.
        let mut dl = DisplayList::new(960.0, 540.0);
        dl.push(Command::DrawText(run("on-screen", 10.0, 20.0)));
        dl.push(Command::DrawText(run("far away", 50_000.0, 20.0)));
        let out = collect(&dl, &view(960.0, 540.0));
        assert_eq!(
            out.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(),
            ["on-screen"],
        );
    }

    #[test]
    fn restore_without_save_does_not_corrupt_later_positions() {
        let mut dl = DisplayList::new(960.0, 540.0);
        dl.push(Command::Restore);
        dl.push(Command::DrawText(run("after", 30.0, 40.0)));
        let out = collect(&dl, &view(960.0, 540.0));
        assert_eq!(out.len(), 1);
        assert!((out[0].x - 30.0).abs() < 0.01);
    }
}
