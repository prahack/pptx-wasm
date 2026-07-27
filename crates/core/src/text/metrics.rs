use std::cell::RefCell;
use std::collections::HashMap;

use crate::dl::FontSpec;

use super::measure::{MeasuredRun, TextMeasure};

/// Vertical metrics for a face at a given size, in points, all positive-down from the
/// baseline except `ascent` which is positive-up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
}

impl FontMetrics {
    /// Default line height: the face's own ascent + descent + gap.
    pub fn line_height(&self) -> f32 {
        self.ascent + self.descent + self.line_gap
    }

    /// Metrics synthesised from a size alone, for when nothing better is available.
    /// The ratios are close to Calibri's, which is the most common default in decks.
    pub fn approximate_for(size_pt: f32) -> Self {
        FontMetrics {
            ascent: size_pt * 0.75,
            descent: size_pt * 0.25,
            line_gap: size_pt * 0.2,
        }
    }
}

/// Wraps any [`TextMeasure`] in a memo table.
///
/// Wrapping re-measures the same words constantly — once per candidate break, then again
/// when the line is committed — and each miss on the Canvas2D backend is a JS call
/// across the wasm boundary. Caching turns an O(words²) pile of boundary crossings into
/// one call per distinct (string, font).
pub struct CachingMeasure<M: TextMeasure> {
    inner: M,
    runs: RefCell<HashMap<(String, FontSpec), MeasuredRun>>,
    fonts: RefCell<HashMap<FontSpec, FontMetrics>>,
    families: RefCell<HashMap<String, bool>>,
}

impl<M: TextMeasure> CachingMeasure<M> {
    pub fn new(inner: M) -> Self {
        CachingMeasure {
            inner,
            runs: RefCell::new(HashMap::new()),
            fonts: RefCell::new(HashMap::new()),
            families: RefCell::new(HashMap::new()),
        }
    }

    pub fn into_inner(self) -> M {
        self.inner
    }

    /// The wrapped measurer, for backend-specific diagnostics (call counts, loaded
    /// families) that have no place on the [`TextMeasure`] trait itself.
    pub fn inner_ref(&self) -> &M {
        &self.inner
    }

    /// Number of cached string measurements — asserted in tests to prove the cache is
    /// actually being hit rather than silently missing on every call.
    pub fn cached_runs(&self) -> usize {
        self.runs.borrow().len()
    }

    pub fn clear(&self) {
        self.runs.borrow_mut().clear();
        self.fonts.borrow_mut().clear();
        self.families.borrow_mut().clear();
    }
}

impl<M: TextMeasure> TextMeasure for CachingMeasure<M> {
    fn measure(&self, text: &str, font: &FontSpec) -> MeasuredRun {
        let key = (text.to_owned(), font.clone());
        if let Some(hit) = self.runs.borrow().get(&key) {
            return hit.clone();
        }
        let measured = self.inner.measure(text, font);
        self.runs.borrow_mut().insert(key, measured.clone());
        measured
    }

    fn font_metrics(&self, font: &FontSpec) -> FontMetrics {
        if let Some(hit) = self.fonts.borrow().get(font) {
            return *hit;
        }
        let m = self.inner.font_metrics(font);
        self.fonts.borrow_mut().insert(font.clone(), m);
        m
    }

    fn has_family(&self, family: &str) -> bool {
        if let Some(hit) = self.families.borrow().get(family) {
            return *hit;
        }
        let present = self.inner.has_family(family);
        self.families
            .borrow_mut()
            .insert(family.to_owned(), present);
        present
    }
}

/// A synthetic measurer with no font files behind it.
///
/// Core's layout tests use this so they assert on layout logic — where the breaks fall,
/// how alignment distributes slack — without depending on which fonts a machine happens
/// to have installed. Pixel fidelity is the golden suite's job, not `cargo test`'s.
#[derive(Debug, Clone, Default)]
pub struct StubMeasure;

impl StubMeasure {
    /// Advance as a fraction of the em, roughly tracking a humanist sans. Enough
    /// variation that a wrapping bug that ignores per-char widths shows up.
    fn ratio(c: char) -> f32 {
        match c {
            ' ' => 0.26,
            'i' | 'l' | 'j' | 'I' | '.' | ',' | '\'' | '!' | ':' | ';' | '|' => 0.24,
            'f' | 't' | 'r' | '(' | ')' | '[' | ']' | '-' => 0.33,
            'm' | 'w' | 'M' | 'W' => 0.86,
            'A'..='Z' => 0.65,
            '\t' => 1.0,
            c if c.is_ascii_digit() => 0.55,
            c if c.is_ascii_graphic() => 0.55,
            // Assume CJK and other non-Latin scripts are full-width.
            c if (c as u32) > 0x2E00 => 1.0,
            _ => 0.55,
        }
    }
}

impl TextMeasure for StubMeasure {
    fn measure(&self, text: &str, font: &FontSpec) -> MeasuredRun {
        let size = font.size();
        let bold_factor = match font.weight {
            crate::dl::FontWeight::Bold => 1.04,
            crate::dl::FontWeight::Regular => 1.0,
        };
        let advances: Vec<f32> = text
            .chars()
            .map(|c| Self::ratio(c) * size * bold_factor)
            .collect();
        MeasuredRun {
            width: advances.iter().sum(),
            advances,
        }
    }

    fn font_metrics(&self, font: &FontSpec) -> FontMetrics {
        FontMetrics::approximate_for(font.size())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CountingMeasure(RefCell<usize>);
    impl TextMeasure for CountingMeasure {
        fn measure(&self, text: &str, font: &FontSpec) -> MeasuredRun {
            *self.0.borrow_mut() += 1;
            StubMeasure.measure(text, font)
        }
        fn font_metrics(&self, font: &FontSpec) -> FontMetrics {
            StubMeasure.font_metrics(font)
        }
    }

    #[test]
    fn cache_collapses_repeat_measurements_to_one_inner_call() {
        let counter = CountingMeasure(RefCell::new(0));
        let cached = CachingMeasure::new(counter);
        let font = FontSpec::new("Arial", 12.0);
        for _ in 0..10 {
            cached.measure("hello", &font);
        }
        assert_eq!(cached.cached_runs(), 1);
        assert_eq!(*cached.into_inner().0.borrow(), 1);
    }

    #[test]
    fn cache_keys_on_size_as_well_as_string() {
        let cached = CachingMeasure::new(StubMeasure);
        let a = cached.measure("hi", &FontSpec::new("Arial", 12.0));
        let b = cached.measure("hi", &FontSpec::new("Arial", 24.0));
        assert_eq!(cached.cached_runs(), 2);
        assert!((b.width - a.width * 2.0).abs() < 1e-4);
    }

    #[test]
    fn stub_advances_are_parallel_to_chars() {
        let m = StubMeasure.measure("Wil", &FontSpec::new("Arial", 10.0));
        assert_eq!(m.advances.len(), 3);
        assert!(m.advances[0] > m.advances[2], "W should be wider than l");
        assert!((m.width - m.advances.iter().sum::<f32>()).abs() < 1e-6);
    }

    #[test]
    fn approximate_metrics_scale_linearly_with_size() {
        let a = FontMetrics::approximate_for(10.0);
        let b = FontMetrics::approximate_for(20.0);
        assert!((b.line_height() - a.line_height() * 2.0).abs() < 1e-4);
    }
}
