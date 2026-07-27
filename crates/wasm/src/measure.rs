//! Spike A's answer: text measurement through the browser's own 2D context.
//!
//! Layout needs advance widths before it can wrap. There were two ways to get them:
//!
//! * **Canvas2D `measureText`** (this file). The browser measures with the same engine
//!   that will later draw the text, so a wrap point and the glyphs on that line can never
//!   disagree. Costs nothing in payload. The metrics differ slightly between browsers.
//! * **`cosmic-text` in Rust.** Identical metrics everywhere, at the cost of shipping a
//!   shaper and — since the browser will not hand us its font files — the fonts too.
//!
//! The decision, and the measurements behind it, are recorded in `CLAUDE.md`. This is the
//! default; the alternative lives behind the `text-cosmic` feature so the choice stays
//! reversible rather than baked into every call site.
//!
//! ## Why the caching matters
//!
//! Each `measureText` is a call across the wasm/JS boundary. Wrapping a paragraph
//! measures the same words repeatedly — once per candidate break, then again when the
//! line is committed — so an uncached measurer makes hundreds of boundary crossings per
//! text box. Two caches fix that: a per-font ASCII advance table that answers most
//! strings without any call at all, and a string-level memo for everything else.

use std::cell::RefCell;
use std::collections::HashMap;

use pptx_core::dl::FontSpec;
use pptx_core::text::{FontMetrics, MeasuredRun, TextMeasure};
use wasm_bindgen::JsCast;
use web_sys::CanvasRenderingContext2d;

/// Advance widths for the printable ASCII range, in points, for one font.
struct AsciiTable {
    /// Indexed by `c as usize - 0x20`, covering 0x20..=0x7E.
    advances: [f32; 95],
    metrics: FontMetrics,
}

pub struct CanvasTextMeasure {
    ctx: CanvasRenderingContext2d,
    ascii: RefCell<HashMap<FontSpec, AsciiTable>>,
    strings: RefCell<HashMap<(String, FontSpec), MeasuredRun>>,
    /// Families the browser reports as unavailable, so the fallback chain is walked once.
    missing: RefCell<HashMap<String, bool>>,
    /// Counted so tests and the dev overlay can prove the caches are working.
    js_calls: std::cell::Cell<usize>,
}

impl CanvasTextMeasure {
    /// Builds a measurer over an offscreen 2D context.
    ///
    /// An `OffscreenCanvas` is used rather than a DOM canvas so measurement works
    /// identically on the main thread and in the Web Worker M6 moves parsing into.
    pub fn new() -> Option<Self> {
        let canvas = web_sys::OffscreenCanvas::new(1, 1).ok()?;
        let ctx = canvas
            .get_context("2d")
            .ok()??
            .dyn_into::<web_sys::OffscreenCanvasRenderingContext2d>()
            .ok()?;
        // The two context types share the measurement API but not a Rust type, so the
        // offscreen one is re-cast through JsValue.
        let ctx: CanvasRenderingContext2d = ctx.unchecked_into();
        Some(CanvasTextMeasure::from_context(ctx))
    }

    /// Builds a measurer over an existing context — used when the host already has one,
    /// and by the golden harness so measurement and drawing share a context.
    pub fn from_context(ctx: CanvasRenderingContext2d) -> Self {
        CanvasTextMeasure {
            ctx,
            ascii: RefCell::new(HashMap::new()),
            strings: RefCell::new(HashMap::new()),
            missing: RefCell::new(HashMap::new()),
            js_calls: std::cell::Cell::new(0),
        }
    }

    /// Number of `measureText` calls made. A wrapping pass that does not plateau here is
    /// missing the cache.
    pub fn js_calls(&self) -> usize {
        self.js_calls.get()
    }

    pub fn clear(&self) {
        self.ascii.borrow_mut().clear();
        self.strings.borrow_mut().clear();
        self.missing.borrow_mut().clear();
    }

    fn measure_raw(&self, text: &str, font: &FontSpec) -> f32 {
        self.ctx.set_font(&font.to_css());
        self.js_calls.set(self.js_calls.get() + 1);
        match self.ctx.measure_text(text) {
            Ok(m) => m.width() as f32,
            Err(_) => {
                // A context that refuses to measure is unusable; fall back to a synthetic
                // estimate rather than laying every line out at zero width.
                text.chars().count() as f32 * font.size() * 0.5
            }
        }
    }

    /// Builds (and caches) the ASCII advance table plus vertical metrics for a font.
    fn table_for(&self, font: &FontSpec) -> std::cell::Ref<'_, HashMap<FontSpec, AsciiTable>> {
        if !self.ascii.borrow().contains_key(font) {
            let mut advances = [0.0f32; 95];
            self.ctx.set_font(&font.to_css());
            for (i, slot) in advances.iter_mut().enumerate() {
                let c = char::from(0x20u8 + i as u8);
                let mut buf = [0u8; 4];
                let s = c.encode_utf8(&mut buf);
                self.js_calls.set(self.js_calls.get() + 1);
                *slot = self
                    .ctx
                    .measure_text(s)
                    .map(|m| m.width() as f32)
                    .unwrap_or(font.size() * 0.5);
            }
            let metrics = self.vertical_metrics(font);
            self.ascii
                .borrow_mut()
                .insert(font.clone(), AsciiTable { advances, metrics });
        }
        self.ascii.borrow()
    }

    /// Ascent and descent from the face, falling back through progressively worse
    /// approximations. Firefox only grew `fontBoundingBox*` recently, and some contexts
    /// report zeros, so each step is validated before being trusted.
    fn vertical_metrics(&self, font: &FontSpec) -> FontMetrics {
        self.ctx.set_font(&font.to_css());
        self.js_calls.set(self.js_calls.get() + 1);
        // A string with both a tall ascender and a descender, so the actual-bounding-box
        // fallback has something to measure.
        let probe = self.ctx.measure_text("Hxpdfg");
        let size = font.size();
        if let Ok(m) = probe {
            let a = m.font_bounding_box_ascent() as f32;
            let d = m.font_bounding_box_descent() as f32;
            if a > 0.0 && d >= 0.0 && a + d > size * 0.5 && a + d < size * 4.0 {
                // The face's own line gap is not exposed; 0.2em matches what most
                // OpenType faces declare and what PowerPoint assumes.
                return FontMetrics {
                    ascent: a,
                    descent: d,
                    line_gap: (size * 0.2).max(0.0),
                };
            }
            let aa = m.actual_bounding_box_ascent() as f32;
            let ad = m.actual_bounding_box_descent() as f32;
            if aa > 0.0 && aa + ad > size * 0.4 {
                return FontMetrics {
                    ascent: aa,
                    descent: ad.max(size * 0.2),
                    line_gap: size * 0.2,
                };
            }
        }
        FontMetrics::approximate_for(size)
    }

    /// Advances for a string, from the ASCII table when it can be, else from JS.
    fn advances_for(&self, text: &str, font: &FontSpec) -> Vec<f32> {
        if text.is_ascii() {
            let tables = self.table_for(font);
            if let Some(t) = tables.get(font) {
                return text
                    .bytes()
                    .map(|b| {
                        // Control characters have no advance of their own; a tab is
                        // resolved by layout against the paragraph's tab stops.
                        if (0x20..=0x7E).contains(&b) {
                            t.advances
                                .get((b - 0x20) as usize)
                                .copied()
                                .unwrap_or(0.0)
                        } else {
                            0.0
                        }
                    })
                    .collect();
            }
        }
        // Non-ASCII: measure each character, then correct the total for any kerning or
        // shaping the per-character sum missed. The correction lands on the last
        // character so contiguous-range sums stay right.
        let mut advances: Vec<f32> = Vec::with_capacity(text.chars().count());
        self.ctx.set_font(&font.to_css());
        for c in text.chars() {
            let mut buf = [0u8; 4];
            let s = c.encode_utf8(&mut buf);
            self.js_calls.set(self.js_calls.get() + 1);
            advances.push(
                self.ctx
                    .measure_text(s)
                    .map(|m| m.width() as f32)
                    .unwrap_or(font.size() * 0.5),
            );
        }
        let whole = self.measure_raw(text, font);
        let sum: f32 = advances.iter().sum();
        if (whole - sum).abs() > 0.01 {
            if let Some(last) = advances.last_mut() {
                *last += whole - sum;
            }
        }
        advances
    }
}

impl TextMeasure for CanvasTextMeasure {
    fn measure(&self, text: &str, font: &FontSpec) -> MeasuredRun {
        if text.is_empty() {
            return MeasuredRun::default();
        }
        let key = (text.to_owned(), font.clone());
        if let Some(hit) = self.strings.borrow().get(&key) {
            return hit.clone();
        }
        let advances = self.advances_for(text, font);
        // Trust the whole-string measurement for the total: it is what the browser will
        // actually advance by when it draws, kerning included.
        let width = if text.is_ascii() {
            advances.iter().sum()
        } else {
            self.measure_raw(text, font)
        };
        let run = MeasuredRun { width, advances };
        self.strings.borrow_mut().insert(key, run.clone());
        run
    }

    fn font_metrics(&self, font: &FontSpec) -> FontMetrics {
        let tables = self.table_for(font);
        tables
            .get(font)
            .map(|t| t.metrics)
            .unwrap_or_else(|| FontMetrics::approximate_for(font.size()))
    }

    fn has_family(&self, family: &str) -> bool {
        if let Some(hit) = self.missing.borrow().get(family) {
            return *hit;
        }
        // The 2D context has no font-availability API. Compare a probe string measured in
        // the requested family against the same string in a known-different generic: if
        // they match exactly, the family fell back and is therefore absent.
        const PROBE: &str = "MMMWWWiiilll@#$%";
        let mut spec = FontSpec::new(family, 72.0);
        spec.fallbacks = vec!["monospace".into()];
        let with = self.measure_raw(PROBE, &spec);
        let mut bare = FontSpec::new("monospace", 72.0);
        bare.fallbacks.clear();
        let without = self.measure_raw(PROBE, &bare);
        let present = (with - without).abs() > 0.5;
        self.missing
            .borrow_mut()
            .insert(family.to_owned(), present);
        present
    }
}

/// The `cosmic-text` measurer Spike A weighed against the Canvas2D one.
///
/// Deliberately not implemented: enabling the feature is a build-level experiment (what
/// does the shaper cost in payload?) and turning it on without the fonts to go with it
/// would silently produce *worse* metrics than the default, not deterministic ones. The
/// build fails loudly instead. See CLAUDE.md for the numbers and the reasoning.
#[cfg(feature = "text-cosmic")]
compile_error!(
    "the text-cosmic feature is a placeholder for the cosmic-text measurer. Shipping it \
     needs (a) the cosmic-text dependency and (b) an embedded font set, since the browser \
     will not expose its own font files. See the Spike A decision in CLAUDE.md before \
     enabling."
);
