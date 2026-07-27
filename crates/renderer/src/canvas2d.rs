//! Backend A: the browser's own 2D context.
//!
//! This is the accuracy baseline. Text goes through `fillText`, so the browser shapes and
//! rasterises it with the same engine that produced the advances layout wrapped against —
//! which means wrap points and drawn glyphs cannot disagree. Paths go through `Path2D`,
//! which the browser tessellates and antialiases.
//!
//! Everything here is a translation of display-list commands into 2D-context calls. There
//! is no layout logic, no colour resolution, and no font choice: if this file has to make
//! a decision about *what* to draw rather than *how*, that decision belongs upstream.

use wasm_bindgen::{JsCast, JsValue};
use web_sys::{CanvasRenderingContext2d, CanvasWindingRule, HtmlImageElement, Path2d};

use pptx_core::dl::{
    Command, DisplayList, FillRule, Gradient, ImageId, LineCap, LineJoin, Paint, Path, PathVerb,
    Point, Stroke, TextRun, Transform,
};

use crate::{ImageSource, Renderer};

/// Images the backend can draw. Both variants are things `drawImage` accepts.
#[derive(Clone)]
pub enum CanvasImage {
    Element(HtmlImageElement),
    Bitmap(web_sys::ImageBitmap),
}

/// An [`ImageSource`] backed by a decoded-image map the host fills in before rendering.
pub struct DecodedImages {
    images: std::collections::HashMap<u32, CanvasImage>,
}

impl DecodedImages {
    pub fn new() -> Self {
        DecodedImages {
            images: std::collections::HashMap::new(),
        }
    }

    pub fn insert(&mut self, id: ImageId, image: CanvasImage) {
        self.images.insert(id.0, image);
    }

    pub fn len(&self) -> usize {
        self.images.len()
    }

    pub fn is_empty(&self) -> bool {
        self.images.is_empty()
    }
}

impl Default for DecodedImages {
    fn default() -> Self {
        Self::new()
    }
}

impl ImageSource for DecodedImages {
    type Handle = CanvasImage;
    fn get(&self, id: ImageId) -> Option<CanvasImage> {
        self.images.get(&id.0).cloned()
    }
}

/// A resolved canvas paint style.
enum Style {
    Css(String),
    Gradient(web_sys::CanvasGradient),
}

#[derive(Debug)]
pub enum Error {
    Js(String),
}

impl From<JsValue> for Error {
    fn from(v: JsValue) -> Self {
        Error::Js(
            v.as_string()
                .unwrap_or_else(|| format!("{:?}", v.as_f64().unwrap_or(f64::NAN))),
        )
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Js(m) => write!(f, "canvas error: {m}"),
        }
    }
}

pub struct Canvas2dRenderer<I: ImageSource<Handle = CanvasImage>> {
    ctx: CanvasRenderingContext2d,
    images: I,
    root: Transform,
    /// Current transform, tracked so image and text drawing can compute device-space
    /// positions without round-tripping through `getTransform`.
    current: Transform,
    stack: Vec<Transform>,
    /// Backing-store size in device pixels, cleared at the start of each frame.
    pixel_size: (f64, f64),
}

impl<I: ImageSource<Handle = CanvasImage>> Canvas2dRenderer<I> {
    pub fn new(ctx: CanvasRenderingContext2d, images: I) -> Self {
        Canvas2dRenderer {
            ctx,
            images,
            root: Transform::IDENTITY,
            current: Transform::IDENTITY,
            stack: Vec::new(),
            pixel_size: (0.0, 0.0),
        }
    }

    /// Sets the backing-store size to clear each frame. Without this the previous slide
    /// shows through wherever the new one does not paint.
    pub fn set_pixel_size(&mut self, w: f64, h: f64) {
        self.pixel_size = (w, h);
    }

    pub fn images_mut(&mut self) -> &mut I {
        &mut self.images
    }

    fn apply_transform(&self, t: &Transform) -> Result<(), Error> {
        self.ctx
            .set_transform(
                t.a as f64,
                t.b as f64,
                t.c as f64,
                t.d as f64,
                t.e as f64,
                t.f as f64,
            )
            .map_err(Error::from)
    }

    fn build_path(&self, path: &Path) -> Result<Path2d, Error> {
        let p = Path2d::new().map_err(Error::from)?;
        let mut i = 0usize;
        let pt = |idx: usize| -> Point { path.points.get(idx).copied().unwrap_or_default() };
        for verb in &path.verbs {
            match verb {
                PathVerb::MoveTo => {
                    let a = pt(i);
                    p.move_to(a.x as f64, a.y as f64);
                    i += 1;
                }
                PathVerb::LineTo => {
                    let a = pt(i);
                    p.line_to(a.x as f64, a.y as f64);
                    i += 1;
                }
                PathVerb::QuadTo => {
                    let (c, a) = (pt(i), pt(i + 1));
                    p.quadratic_curve_to(c.x as f64, c.y as f64, a.x as f64, a.y as f64);
                    i += 2;
                }
                PathVerb::CubicTo => {
                    let (c1, c2, a) = (pt(i), pt(i + 1), pt(i + 2));
                    p.bezier_curve_to(
                        c1.x as f64,
                        c1.y as f64,
                        c2.x as f64,
                        c2.y as f64,
                        a.x as f64,
                        a.y as f64,
                    );
                    i += 3;
                }
                PathVerb::Close => p.close_path(),
            }
        }
        Ok(p)
    }

    /// A concrete `fillStyle`/`strokeStyle` value.
    ///
    /// Split by type rather than passed as a `JsValue` because the untyped setters are
    /// deprecated in `web-sys`: the typed ones skip a runtime type check per call, and a
    /// slide can set a style thousands of times.
    fn set_fill(&self, style: &Style) -> Result<(), Error> {
        match style {
            Style::Css(s) => self.ctx.set_fill_style_str(s),
            Style::Gradient(g) => self.ctx.set_fill_style_canvas_gradient(g),
        }
        Ok(())
    }

    fn set_stroke(&self, style: &Style) -> Result<(), Error> {
        match style {
            Style::Css(s) => self.ctx.set_stroke_style_str(s),
            Style::Gradient(g) => self.ctx.set_stroke_style_canvas_gradient(g),
        }
        Ok(())
    }

    /// Turns a paint into something assignable to `fillStyle`/`strokeStyle`.
    fn paint_style(&self, paint: &Paint) -> Result<Style, Error> {
        Ok(match paint {
            Paint::Solid(c) => Style::Css(c.to_css()),
            Paint::Gradient(g) => match g {
                Gradient::Linear { start, end, stops } => {
                    let grad = self.ctx.create_linear_gradient(
                        start.x as f64,
                        start.y as f64,
                        end.x as f64,
                        end.y as f64,
                    );
                    for s in stops {
                        grad.add_color_stop(s.offset.clamp(0.0, 1.0), &s.color.to_css())
                            .map_err(Error::from)?;
                    }
                    Style::Gradient(grad)
                }
                Gradient::Radial {
                    center,
                    radius,
                    stops,
                    ..
                } => {
                    // A zero inner radius keeps the centre stop sharp; the scale_y of a
                    // non-uniform radial is folded into the transform by the caller.
                    let grad = self
                        .ctx
                        .create_radial_gradient(
                            center.x as f64,
                            center.y as f64,
                            0.0,
                            center.x as f64,
                            center.y as f64,
                            (*radius).max(0.01) as f64,
                        )
                        .map_err(Error::from)?;
                    for s in stops {
                        grad.add_color_stop(s.offset.clamp(0.0, 1.0), &s.color.to_css())
                            .map_err(Error::from)?;
                    }
                    Style::Gradient(grad)
                }
            },
            // An image paint on a path is drawn as a clipped image rather than a canvas
            // pattern, so `srcRect` cropping works the same way it does for a picture.
            Paint::Image { .. } => Style::Css("transparent".to_string()),
        })
    }

    fn winding(rule: FillRule) -> CanvasWindingRule {
        match rule {
            FillRule::NonZero => CanvasWindingRule::Nonzero,
            FillRule::EvenOdd => CanvasWindingRule::Evenodd,
        }
    }

    fn fill(&mut self, path: &Path, paint: &Paint, rule: FillRule) -> Result<(), Error> {
        let p2d = self.build_path(path)?;
        if let Paint::Image {
            image,
            src,
            opacity,
        } = paint
        {
            let Some(handle) = self.images.get(*image) else {
                return Ok(());
            };
            let b = path.bounds();
            self.ctx.save();
            self.ctx.clip_with_path_2d_and_winding(&p2d, Self::winding(rule));
            let prev = self.ctx.global_alpha();
            self.ctx.set_global_alpha((*opacity).clamp(0.0, 1.0) as f64);
            self.draw_image(&handle, *src, b)?;
            self.ctx.set_global_alpha(prev);
            self.ctx.restore();
            return Ok(());
        }
        let style = self.paint_style(paint)?;
        self.set_fill(&style)?;
        self.ctx.fill_with_path_2d_and_winding(&p2d, Self::winding(rule));
        Ok(())
    }

    fn stroke(&mut self, path: &Path, stroke: &Stroke) -> Result<(), Error> {
        let p2d = self.build_path(path)?;
        let style = self.paint_style(&stroke.paint)?;
        self.set_stroke(&style)?;
        // A zero width means a hairline: the thinnest line still visible at this scale.
        let scale = self.current.approx_scale().max(1e-4) as f64;
        let width = if stroke.width <= 0.0 {
            1.0 / scale
        } else {
            stroke.width as f64
        };
        self.ctx.set_line_width(width);
        self.ctx.set_line_cap(match stroke.cap {
            LineCap::Butt => "butt",
            LineCap::Round => "round",
            LineCap::Square => "square",
        });
        self.ctx.set_line_join(match stroke.join {
            LineJoin::Miter => "miter",
            LineJoin::Round => "round",
            LineJoin::Bevel => "bevel",
        });
        self.ctx.set_miter_limit(stroke.miter_limit as f64);
        let dash = js_sys::Array::new();
        for d in &stroke.dash {
            dash.push(&JsValue::from_f64((*d).max(0.0) as f64));
        }
        self.ctx.set_line_dash(&dash).map_err(Error::from)?;
        self.ctx.stroke_with_path(&p2d);
        Ok(())
    }

    fn draw_image(
        &self,
        handle: &CanvasImage,
        src: pptx_core::dl::Rect,
        dst: pptx_core::dl::Rect,
    ) -> Result<(), Error> {
        // `src` is normalised; scale it by the decoded image's real pixel size.
        let (iw, ih) = match handle {
            CanvasImage::Element(e) => (e.natural_width() as f64, e.natural_height() as f64),
            CanvasImage::Bitmap(b) => (b.width() as f64, b.height() as f64),
        };
        if iw <= 0.0 || ih <= 0.0 {
            return Ok(());
        }
        let (sx, sy) = (src.x as f64 * iw, src.y as f64 * ih);
        let (sw, sh) = ((src.w as f64 * iw).max(1.0), (src.h as f64 * ih).max(1.0));
        let result = match handle {
            CanvasImage::Element(e) => self
                .ctx
                .draw_image_with_html_image_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
                    e, sx, sy, sw, sh, dst.x as f64, dst.y as f64, dst.w as f64, dst.h as f64,
                ),
            CanvasImage::Bitmap(b) => self
                .ctx
                .draw_image_with_image_bitmap_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
                    b, sx, sy, sw, sh, dst.x as f64, dst.y as f64, dst.w as f64, dst.h as f64,
                ),
        };
        result.map_err(Error::from)
    }

    fn draw_text(&mut self, run: &TextRun) -> Result<(), Error> {
        if run.text.is_empty() {
            return Ok(());
        }
        let style = self.paint_style(&run.paint)?;
        self.set_fill(&style)?;
        self.ctx.set_font(&run.font.to_css());
        self.ctx.set_text_baseline("alphabetic");
        self.ctx.set_text_align("left");

        if run.letter_spacing.abs() > f32::EPSILON && !run.advances.is_empty() {
            // Per-character placement, because `letterSpacing` on the 2D context is not
            // available everywhere and its rounding differs from ours.
            let mut x = run.origin.x;
            for (i, ch) in run.text.chars().enumerate() {
                let mut buf = [0u8; 4];
                let s = ch.encode_utf8(&mut buf);
                self.ctx
                    .fill_text(s, x as f64, run.origin.y as f64)
                    .map_err(Error::from)?;
                x += run.advances.get(i).copied().unwrap_or(0.0) + run.letter_spacing;
            }
        } else {
            self.ctx
                .fill_text(&run.text, run.origin.x as f64, run.origin.y as f64)
                .map_err(Error::from)?;
        }

        if run.decorations.any() {
            self.draw_decorations(run)?;
        }
        Ok(())
    }

    fn draw_decorations(&mut self, run: &TextRun) -> Result<(), Error> {
        let size = run.font.size();
        // Ratios chosen to sit where a typical face's own underline metrics fall; the
        // 2D context does not expose the face's real underline position.
        let thickness = (size * 0.06).max(0.5);
        let style = self.paint_style(&run.paint)?;
        self.set_fill(&style)?;
        if run.decorations.underline {
            self.ctx.fill_rect(
                run.origin.x as f64,
                (run.origin.y + size * 0.12) as f64,
                run.width as f64,
                thickness as f64,
            );
        }
        if run.decorations.strikethrough {
            self.ctx.fill_rect(
                run.origin.x as f64,
                (run.origin.y - size * 0.28) as f64,
                run.width as f64,
                thickness as f64,
            );
        }
        Ok(())
    }
}

impl<I: ImageSource<Handle = CanvasImage>> Renderer for Canvas2dRenderer<I> {
    type Error = Error;

    fn begin_frame(&mut self, _dl: &DisplayList, root: Transform) -> Result<(), Error> {
        self.root = root;
        self.current = root;
        self.stack.clear();
        // Clear in device space, before the root transform goes on.
        self.ctx.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0).map_err(Error::from)?;
        self.ctx.set_global_alpha(1.0);
        if self.pixel_size.0 > 0.0 && self.pixel_size.1 > 0.0 {
            self.ctx
                .clear_rect(0.0, 0.0, self.pixel_size.0, self.pixel_size.1);
        }
        self.apply_transform(&root)
    }

    fn execute(&mut self, cmd: &Command) -> Result<(), Error> {
        match cmd {
            Command::Save => {
                self.ctx.save();
                self.stack.push(self.current);
            }
            Command::Restore => {
                // An unbalanced Restore is a layout bug, but underflowing the canvas
                // state stack would corrupt every later slide, so it is ignored here.
                if let Some(prev) = self.stack.pop() {
                    self.ctx.restore();
                    self.current = prev;
                    self.apply_transform(&self.current)?;
                } else {
                    log::warn!("display list restored more than it saved");
                }
            }
            Command::Concat(t) => {
                self.current = t.then(&self.current);
                self.apply_transform(&self.current)?;
            }
            Command::ClipRect(r) => {
                let p = Path2d::new().map_err(Error::from)?;
                p.rect(r.x as f64, r.y as f64, r.w as f64, r.h as f64);
                self.ctx
                    .clip_with_path_2d_and_winding(&p, CanvasWindingRule::Nonzero);
            }
            Command::ClipPath { path, rule } => {
                let p = self.build_path(path)?;
                self.ctx.clip_with_path_2d_and_winding(&p, Self::winding(*rule));
            }
            Command::FillPath { path, paint, rule } => self.fill(path, paint, *rule)?,
            Command::StrokePath { path, stroke } => self.stroke(path, stroke)?,
            Command::DrawImage {
                image,
                src,
                dst,
                opacity,
            } => {
                let Some(handle) = self.images.get(*image) else {
                    log::debug!("image {} has not been decoded; skipping", image.0);
                    return Ok(());
                };
                let prev = self.ctx.global_alpha();
                self.ctx.set_global_alpha((*opacity).clamp(0.0, 1.0) as f64);
                self.draw_image(&handle, *src, *dst)?;
                self.ctx.set_global_alpha(prev);
            }
            Command::DrawText(run) => self.draw_text(run)?,
        }
        Ok(())
    }

    fn end_frame(&mut self) -> Result<(), Error> {
        // Unwind anything the display list left open so the next frame starts clean.
        while self.stack.pop().is_some() {
            self.ctx.restore();
        }
        self.ctx.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0).map_err(Error::from)?;
        Ok(())
    }
}

/// Casts an untyped 2D context, for hosts that obtained it from JS.
pub fn context_from_js(value: JsValue) -> Option<CanvasRenderingContext2d> {
    value.dyn_into::<CanvasRenderingContext2d>().ok()
}
