//! Renderer backends for the display list.
//!
//! **Spike B's outcome.** A backend implements [`Renderer`] — a small command-at-a-time
//! interface — and [`render`] drives it. Nothing above this line knows which backend is
//! in use, and nothing below it knows what a slide is. The two shipped backends prove
//! the seam is real:
//!
//! * [`canvas2d`] draws through the browser's own 2D context, including its text shaper.
//!   It is the default and the accuracy baseline.
//! * [`webgpu`] is the ambitious target: a glyph atlas plus tessellated paths. It is a
//!   stub that reports what it would need, so the abstraction stays honest rather than
//!   being retro-fitted later.
//! * [`record`] serialises commands instead of drawing them. It runs on the host, which
//!   is how `cargo test` can assert on renderer behaviour without a browser.

#![deny(clippy::unwrap_used, clippy::expect_used)]
// Tests are where a failed assumption *should* abort loudly, so the panic lints are
// lifted there. The deny above is about the shipping code paths.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod cull;
pub mod record;
pub mod textlayer;

#[cfg(target_arch = "wasm32")]
pub mod canvas2d;

pub mod webgpu;

use pptx_core::dl::{Command, DisplayList, ImageId, Transform, View};

/// A renderer backend.
///
/// The command-at-a-time shape matters: it lets a backend keep a small explicit state
/// stack, and it lets [`render`] stay the single place that knows how a display list is
/// walked. A backend that wanted the whole list at once would have to re-implement that
/// walk, and the two would drift.
pub trait Renderer {
    type Error;

    /// Prepares for a frame. `root` is the slide-points → device-pixels transform; the
    /// backend must apply it beneath every command's own transform.
    fn begin_frame(&mut self, dl: &DisplayList, root: Transform) -> Result<(), Self::Error>;

    fn execute(&mut self, cmd: &Command) -> Result<(), Self::Error>;

    fn end_frame(&mut self) -> Result<(), Self::Error>;
}

/// Supplies decoded images to a backend.
///
/// Decoding is asynchronous in a browser and synchronous nowhere else, so the display
/// list only ever carries an [`ImageId`]. The host decodes ahead of time and answers
/// through this trait; a miss draws nothing rather than blocking a frame.
pub trait ImageSource {
    /// The backend's native image handle.
    type Handle;
    fn get(&self, id: ImageId) -> Option<Self::Handle>;
}

/// Lets a backend borrow an image source it does not own — which is what a host with a
/// `RefCell<DecodedImages>` needs, since the borrow cannot outlive the frame.
impl<T: ImageSource + ?Sized> ImageSource for &T {
    type Handle = T::Handle;
    fn get(&self, id: ImageId) -> Option<Self::Handle> {
        (**self).get(id)
    }
}

/// An image source that never has anything — used when a deck has no media, and as the
/// default so a backend can be constructed before images finish decoding.
pub struct NoImages;

impl ImageSource for NoImages {
    type Handle = ();
    fn get(&self, _id: ImageId) -> Option<()> {
        None
    }
}

/// Draws a display list through a backend.
///
/// Commands that cannot affect a visible pixel are skipped — see [`cull`]. This never
/// changes the output, and on a dense slide or a zoomed-in view it is the difference
/// between holding a frame and not.
pub fn render<R: Renderer>(backend: &mut R, dl: &DisplayList, view: &View) -> Result<(), R::Error> {
    let root = view.transform_for(dl.width_pt, dl.height_pt);
    backend.begin_frame(dl, root)?;
    let (w, h) = view.canvas_pixels();
    let mut culler =
        cull::Culler::new(pptx_core::dl::Rect::new(0.0, 0.0, w as f32, h as f32), root);
    for cmd in &dl.commands {
        if culler.should_skip(cmd) {
            continue;
        }
        backend.execute(cmd)?;
    }
    backend.end_frame()
}

/// Draws a display list without culling.
///
/// Used by the recording backend when a trace is wanted for diagnosis: culling would make
/// the trace depend on the viewport, and a trace that changes with the window size is
/// useless for telling a layout bug from a rasterisation one.
pub fn render_unculled<R: Renderer>(
    backend: &mut R,
    dl: &DisplayList,
    view: &View,
) -> Result<(), R::Error> {
    let root = view.transform_for(dl.width_pt, dl.height_pt);
    backend.begin_frame(dl, root)?;
    for cmd in &dl.commands {
        backend.execute(cmd)?;
    }
    backend.end_frame()
}

/// Every image the display list references, in first-use order.
///
/// Hosts call this to know what to decode before the first frame.
pub fn images_used(dl: &DisplayList) -> Vec<ImageId> {
    let mut seen = Vec::new();
    for cmd in &dl.commands {
        let id = match cmd {
            Command::DrawImage { image, .. } => Some(*image),
            Command::FillPath {
                paint: pptx_core::dl::Paint::Image { image, .. },
                ..
            } => Some(*image),
            Command::StrokePath { stroke, .. } => match &stroke.paint {
                pptx_core::dl::Paint::Image { image, .. } => Some(*image),
                _ => None,
            },
            _ => None,
        };
        if let Some(id) = id {
            if !seen.contains(&id) {
                seen.push(id);
            }
        }
    }
    seen
}

/// Every font the display list asks for, deduplicated.
///
/// The Canvas2D backend uses this to wait on `document.fonts.load()` before the first
/// frame; drawing text against a face the browser has not loaded silently substitutes it
/// and produces exactly the kind of "close but wrong" render the golden suite would then
/// blame on layout.
pub fn fonts_used(dl: &DisplayList) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for run in dl.text_runs() {
        let css = run.font.to_css();
        if !seen.contains(&css) {
            seen.push(css);
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;
    use pptx_core::dl::{Color, FontSpec, Paint, Path, Point, Rect, Stroke, TextRun};

    fn text_run(family: &str, size: f32) -> TextRun {
        TextRun {
            text: "x".into(),
            font: FontSpec::new(family, size),
            origin: Point::new(0.0, 0.0),
            paint: Paint::Solid(Color::BLACK),
            advances: Vec::new(),
            width: 1.0,
            decorations: Default::default(),
            letter_spacing: 0.0,
        }
    }

    #[test]
    fn images_used_deduplicates_and_preserves_first_use_order() {
        let mut dl = DisplayList::new(10.0, 10.0);
        dl.push(Command::DrawImage {
            image: ImageId(2),
            src: Rect::new(0.0, 0.0, 1.0, 1.0),
            dst: Rect::new(0.0, 0.0, 1.0, 1.0),
            opacity: 1.0,
        });
        dl.push(Command::FillPath {
            path: Path::rect(Rect::new(0.0, 0.0, 1.0, 1.0)),
            paint: Paint::Image {
                image: ImageId(5),
                src: Rect::new(0.0, 0.0, 1.0, 1.0),
                opacity: 1.0,
                tile: None,
            },
            rule: Default::default(),
        });
        dl.push(Command::DrawImage {
            image: ImageId(2),
            src: Rect::new(0.0, 0.0, 1.0, 1.0),
            dst: Rect::new(0.0, 0.0, 1.0, 1.0),
            opacity: 1.0,
        });
        assert_eq!(images_used(&dl), vec![ImageId(2), ImageId(5)]);
    }

    #[test]
    fn images_used_finds_an_image_paint_on_a_stroke() {
        let mut dl = DisplayList::new(10.0, 10.0);
        dl.push(Command::StrokePath {
            path: Path::rect(Rect::new(0.0, 0.0, 1.0, 1.0)),
            stroke: Stroke {
                paint: Paint::Image {
                    image: ImageId(7),
                    src: Rect::new(0.0, 0.0, 1.0, 1.0),
                    opacity: 1.0,
                    tile: None,
                },
                ..Default::default()
            },
        });
        assert_eq!(images_used(&dl), vec![ImageId(7)]);
    }

    #[test]
    fn fonts_used_lists_each_distinct_css_font_once() {
        let mut dl = DisplayList::new(10.0, 10.0);
        dl.push(Command::DrawText(text_run("Arial", 12.0)));
        dl.push(Command::DrawText(text_run("Arial", 12.0)));
        dl.push(Command::DrawText(text_run("Arial", 24.0)));
        dl.push(Command::DrawText(text_run("Georgia", 12.0)));
        let fonts = fonts_used(&dl);
        assert_eq!(fonts.len(), 3);
        assert!(fonts.iter().any(|f| f.contains("24px")));
    }

    #[test]
    fn an_empty_display_list_needs_no_images_or_fonts() {
        let dl = DisplayList::new(10.0, 10.0);
        assert!(images_used(&dl).is_empty());
        assert!(fonts_used(&dl).is_empty());
    }
}
