//! The TS-facing wasm surface.
//!
//! Kept deliberately narrow. Everything crossing this boundary is either a primitive, a
//! small JSON string, or a handle the browser already owns (a canvas context, a decoded
//! image). Display lists, shapes and models stay in Rust: serialising them per frame
//! would cost more than the drawing does.

#![deny(clippy::unwrap_used, clippy::expect_used)]
// Everything here needs a browser: a canvas context to measure text with, and a canvas to
// draw on. Compiling to nothing on the host is what lets `cargo test --workspace` run the
// core and renderer suites without a wasm target, rather than failing on an import that
// only exists under `cfg(target_arch = "wasm32")`.
#![cfg(target_arch = "wasm32")]

mod measure;

use std::cell::RefCell;
use std::collections::HashMap;

use pptx_core::dl::{DisplayList, Fit, ImageId, View};
use pptx_core::text::CachingMeasure;
use pptx_renderer::canvas2d::{Canvas2dRenderer, CanvasImage, DecodedImages};
use pptx_renderer::ImageSource;
use wasm_bindgen::prelude::*;
use web_sys::CanvasRenderingContext2d;

pub use measure::CanvasTextMeasure;

/// Installs the panic hook. Idempotent; the host may call it more than once.
#[wasm_bindgen(start)]
pub fn start() {
    #[cfg(feature = "panic-hook")]
    console_error_panic_hook::set_once();
}

fn parse_fit(s: &str) -> Fit {
    match s {
        "cover" => Fit::Cover,
        "fill" => Fit::Fill,
        "actual" => Fit::Actual,
        _ => Fit::Contain,
    }
}

/// An open presentation.
#[wasm_bindgen]
pub struct Presentation {
    inner: pptx_core::Presentation,
    measure: CachingMeasure<CanvasTextMeasure>,
    /// Display lists, cached per slide. Laying out costs far more than drawing, and a
    /// resize or a zoom must not trigger it — that is exactly why the display list is
    /// resolution-independent.
    layouts: RefCell<HashMap<usize, DisplayList>>,
    images: RefCell<DecodedImages>,
    /// Image ids the host has already been asked about, so it is not asked twice —
    /// including for images that turned out to be undecodable.
    requested: RefCell<HashMap<u32, bool>>,
}

#[wasm_bindgen]
impl Presentation {
    /// Opens a `.pptx` from its bytes.
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: Vec<u8>) -> Result<Presentation, JsValue> {
        let inner = pptx_core::open(bytes).map_err(|e| JsValue::from_str(&e.to_string()))?;
        let measure = CanvasTextMeasure::new().ok_or_else(|| {
            JsValue::from_str("could not create a 2D context for text measurement")
        })?;
        Ok(Presentation {
            inner,
            measure: CachingMeasure::new(measure),
            layouts: RefCell::new(HashMap::new()),
            images: RefCell::new(DecodedImages::new()),
            requested: RefCell::new(HashMap::new()),
        })
    }

    #[wasm_bindgen(js_name = slideCount)]
    pub fn slide_count(&self) -> usize {
        self.inner.slide_count()
    }

    /// Slide width in points.
    #[wasm_bindgen(js_name = slideWidth)]
    pub fn slide_width(&self) -> f32 {
        self.inner.slide_size_pt().0
    }

    /// Slide height in points.
    #[wasm_bindgen(js_name = slideHeight)]
    pub fn slide_height(&self) -> f32 {
        self.inner.slide_size_pt().1
    }

    /// Lays a slide out (or returns the cached layout) and reports its command count.
    /// Hosts call this on neighbouring slides to keep navigation off the critical path.
    pub fn prepare(&self, index: usize) -> Result<usize, JsValue> {
        self.ensure_layout(index)
            .ok_or_else(|| JsValue::from_str("slide index out of range"))
    }

    fn ensure_layout(&self, index: usize) -> Option<usize> {
        if let Some(dl) = self.layouts.borrow().get(&index) {
            return Some(dl.len());
        }
        let dl = pptx_core::layout::layout_slide(&self.inner, index, &self.measure)?;
        let len = dl.len();
        self.layouts.borrow_mut().insert(index, dl);
        Some(len)
    }

    /// Image ids a slide still needs decoded, comma-separated.
    ///
    /// Decoding is the host's job: it is asynchronous, and a synchronous wasm call cannot
    /// wait on it. The host decodes and calls [`Presentation::set_image`].
    #[wasm_bindgen(js_name = pendingImages)]
    pub fn pending_images(&self, index: usize) -> String {
        if self.ensure_layout(index).is_none() {
            return String::new();
        }
        let layouts = self.layouts.borrow();
        let Some(dl) = layouts.get(&index) else {
            return String::new();
        };
        let images = self.images.borrow();
        let requested = self.requested.borrow();
        let out: Vec<String> = pptx_renderer::images_used(dl)
            .into_iter()
            .filter(|id| images.get(*id).is_none() && !requested.contains_key(&id.0))
            .map(|id| id.0.to_string())
            .collect();
        out.join(",")
    }

    /// The bytes of an image, for the host to wrap in a Blob and decode.
    #[wasm_bindgen(js_name = imageBytes)]
    pub fn image_bytes(&self, id: u32) -> Option<Vec<u8>> {
        self.inner.image_bytes(ImageId(id)).map(|b| b.to_vec())
    }

    /// The MIME type of an image, so the host builds the right Blob.
    #[wasm_bindgen(js_name = imageMime)]
    pub fn image_mime(&self, id: u32) -> String {
        self.inner
            .image_entry(ImageId(id))
            .map(|e| e.mime.to_string())
            .unwrap_or_else(|| "application/octet-stream".to_string())
    }

    /// Hands a decoded image back so later frames can draw it.
    #[wasm_bindgen(js_name = setImage)]
    pub fn set_image(&self, id: u32, image: &JsValue) {
        self.requested.borrow_mut().insert(id, true);
        let handle = if let Some(bmp) = image.dyn_ref::<web_sys::ImageBitmap>() {
            CanvasImage::Bitmap(bmp.clone())
        } else if let Some(el) = image.dyn_ref::<web_sys::HtmlImageElement>() {
            CanvasImage::Element(el.clone())
        } else {
            log::warn!("setImage({id}) received something that is not an image");
            return;
        };
        self.images.borrow_mut().insert(ImageId(id), handle);
    }

    /// Marks an image as undecodable — EMF and WMF always are — so the viewer stops
    /// asking for it every frame.
    #[wasm_bindgen(js_name = markImageFailed)]
    pub fn mark_image_failed(&self, id: u32) {
        self.requested.borrow_mut().insert(id, true);
    }

    /// Fonts a slide needs, as a JSON array of CSS `font` shorthand strings.
    ///
    /// The host passes these to `document.fonts.load()` before the first frame. Drawing
    /// against a face the browser has not loaded silently substitutes it, and the result
    /// looks exactly like a layout bug.
    #[wasm_bindgen(js_name = fontsNeeded)]
    pub fn fonts_needed(&self, index: usize) -> String {
        if self.ensure_layout(index).is_none() {
            return "[]".to_string();
        }
        let layouts = self.layouts.borrow();
        let Some(dl) = layouts.get(&index) else {
            return "[]".to_string();
        };
        let escaped: Vec<String> = pptx_renderer::fonts_used(dl)
            .iter()
            .map(|f| json_string(f))
            .collect();
        format!("[{}]", escaped.join(","))
    }

    /// Draws a slide into a 2D context.
    #[wasm_bindgen(js_name = renderSlide)]
    #[allow(clippy::too_many_arguments)]
    pub fn render_slide(
        &self,
        index: usize,
        ctx: &CanvasRenderingContext2d,
        viewport_w: f32,
        viewport_h: f32,
        dpr: f32,
        fit: &str,
        zoom: f32,
        pan_x: f32,
        pan_y: f32,
    ) -> Result<(), JsValue> {
        if self.ensure_layout(index).is_none() {
            return Err(JsValue::from_str("slide index out of range"));
        }
        let layouts = self.layouts.borrow();
        let Some(dl) = layouts.get(&index) else {
            return Err(JsValue::from_str("slide layout missing"));
        };
        let view = View {
            viewport_w,
            viewport_h,
            dpr: if dpr > 0.0 { dpr } else { 1.0 },
            fit: parse_fit(fit),
            zoom: if zoom > 0.0 { zoom } else { 1.0 },
            pan_x,
            pan_y,
        };
        let images = self.images.borrow();
        let mut backend = Canvas2dRenderer::new(ctx.clone(), &*images);
        let (pw, ph) = view.canvas_pixels();
        backend.set_pixel_size(pw as f64, ph as f64);
        pptx_renderer::render(&mut backend, dl, &view)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// A slide's text, for selection, search and accessibility.
    #[wasm_bindgen(js_name = slideText)]
    pub fn slide_text(&self, index: usize) -> String {
        if self.ensure_layout(index).is_none() {
            return String::new();
        }
        let layouts = self.layouts.borrow();
        layouts
            .get(&index)
            .map(|dl| dl.plain_text())
            .unwrap_or_default()
    }

    /// Speaker notes for a slide.
    #[wasm_bindgen(js_name = slideNotes)]
    pub fn slide_notes(&self, index: usize) -> String {
        self.inner
            .slide(index)
            .and_then(|s| s.notes.clone())
            .unwrap_or_default()
    }

    /// A textual trace of a slide's draw commands.
    ///
    /// Not part of the public TS API. It is what the golden harness diffs when a pixel
    /// difference needs to be pinned on layout rather than on rasterisation — the single
    /// most common ambiguity in this project.
    #[wasm_bindgen(js_name = debugTrace)]
    pub fn debug_trace(&self, index: usize, viewport_w: f32, viewport_h: f32) -> String {
        if self.ensure_layout(index).is_none() {
            return String::new();
        }
        let layouts = self.layouts.borrow();
        let Some(dl) = layouts.get(&index) else {
            return String::new();
        };
        let view = View {
            viewport_w,
            viewport_h,
            dpr: 1.0,
            ..Default::default()
        };
        pptx_renderer::record::trace(dl, &view)
    }

    /// What a WebGPU backend would need for this slide. Exposed so Backend B's cost stays
    /// visible rather than theoretical.
    #[wasm_bindgen(js_name = gpuRequirements)]
    pub fn gpu_requirements(&self, index: usize) -> String {
        if self.ensure_layout(index).is_none() {
            return String::new();
        }
        let layouts = self.layouts.borrow();
        layouts
            .get(&index)
            .map(|dl| pptx_renderer::webgpu::Requirements::analyse(dl).summary())
            .unwrap_or_default()
    }

    /// `measureText` calls made so far.
    ///
    /// A figure that keeps climbing across re-renders of the same slide means the metrics
    /// cache is being defeated, which is the first thing to check when navigation feels
    /// slow.
    #[wasm_bindgen(js_name = measureCalls)]
    pub fn measure_calls(&self) -> usize {
        self.measure.inner_ref().js_calls()
    }

    /// Drops cached layouts and parsed slides. Layouts rebuild on demand.
    #[wasm_bindgen(js_name = evictLayouts)]
    pub fn evict_layouts(&self) {
        self.layouts.borrow_mut().clear();
        self.inner.evict_slide_cache();
    }

    /// Slide metadata as JSON: `[{"index":0,"id":256,"part":"ppt/slides/slide1.xml"}, …]`.
    #[wasm_bindgen(js_name = slideIndex)]
    pub fn slide_index(&self) -> String {
        let entries: Vec<String> = self
            .inner
            .slides()
            .iter()
            .map(|s| {
                format!(
                    r#"{{"index":{},"id":{},"part":{}}}"#,
                    s.index,
                    s.id,
                    json_string(&s.part_name)
                )
            })
            .collect();
        format!("[{}]", entries.join(","))
    }

    /// Fonts the deck embeds, as JSON. The host installs these as `FontFace`s so text
    /// renders in the authored face even on a machine that lacks it.
    #[wasm_bindgen(js_name = embeddedFonts)]
    pub fn embedded_fonts(&self) -> String {
        let entries: Vec<String> = self
            .inner
            .embedded_fonts
            .iter()
            .map(|f| {
                let variants: Vec<String> = f
                    .variants()
                    .iter()
                    .map(|(rid, bold, italic)| {
                        format!(
                            r#"{{"rel":{},"bold":{},"italic":{}}}"#,
                            json_string(rid),
                            bold,
                            italic
                        )
                    })
                    .collect();
                format!(
                    r#"{{"typeface":{},"variants":[{}]}}"#,
                    json_string(&f.typeface),
                    variants.join(",")
                )
            })
            .collect();
        format!("[{}]", entries.join(","))
    }

    /// Bytes of an embedded font part, by relationship id.
    ///
    /// PowerPoint obfuscates embedded fonts by XOR-ing the first 32 bytes with the GUID
    /// in the file name; `fntdata` parts in `.pptx` are not obfuscated, so the bytes are
    /// handed over as-is and the host wraps them in a `FontFace`.
    #[wasm_bindgen(js_name = embeddedFontBytes)]
    pub fn embedded_font_bytes(&self, rel_id: &str) -> Option<Vec<u8>> {
        let pkg = self.inner.package();
        // Embedded font relationships hang off the presentation part.
        let part = self
            .inner
            .slides()
            .first()
            .map(|_| "ppt/presentation.xml".to_string())?;
        pkg.resolve_part(&part, rel_id).map(|b| b.to_vec())
    }
}

/// Minimal JSON string escaping. A dependency for this would be all cost and no benefit:
/// the only strings crossing here are part names and font names.
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Version of the wasm module, so a host can check what it loaded.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
