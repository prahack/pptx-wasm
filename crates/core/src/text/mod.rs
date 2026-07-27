//! Text measurement — the seam that Spike A settled.
//!
//! Layout must know how wide a string is before it can wrap, but the core crate has no
//! font files and no rasteriser. So it asks a [`TextMeasure`]. The wasm crate supplies
//! one backed by Canvas2D `measureText` (the browser's own shaper, and therefore exactly
//! the metrics the Canvas2D renderer will draw with); a `cosmic-text` implementation
//! exists behind a feature flag for the deterministic-across-browsers path. Layout does
//! not know or care which it got.
//!
//! Everything here is in **points**, matching the display list.

mod measure;
mod metrics;

pub use measure::{MeasuredRun, TextMeasure};
pub use metrics::{CachingMeasure, FontMetrics, StubMeasure};

use crate::dl::FontSpec;

/// Fallback chain appended to every font request, so a missing face degrades to
/// something with sane metrics instead of the browser's last-resort font.
///
/// PowerPoint's own substitution table is far richer; this covers the faces that
/// actually appear in business decks and are absent on non-Windows machines.
pub fn fallbacks_for(family: &str) -> Vec<String> {
    let lower = family.to_ascii_lowercase();
    let mut chain: Vec<String> = Vec::new();

    // Metric-compatible substitutes first: these keep wrap points stable when the
    // original face is missing, which matters far more than exact glyph shapes.
    let metric_twins: &[&str] = match lower.as_str() {
        "calibri" => &["Carlito"],
        "cambria" => &["Caladea"],
        "arial" | "helvetica" => &["Liberation Sans", "Arimo", "Helvetica Neue"],
        "times new roman" | "times" => &["Liberation Serif", "Tinos"],
        "courier new" | "courier" => &["Liberation Mono", "Cousine"],
        "georgia" => &["Gelasio"],
        _ => &[],
    };
    chain.extend(metric_twins.iter().map(|s| (*s).to_string()));

    // Then a generic family, chosen by the shape of the requested face.
    let generic = if is_monospace_name(&lower) {
        "monospace"
    } else if is_serif_name(&lower) {
        "serif"
    } else {
        "sans-serif"
    };
    chain.push(generic.to_string());
    chain
}

fn is_serif_name(lower: &str) -> bool {
    const SERIF: &[&str] = &[
        "times", "georgia", "garamond", "book", "cambria", "constantia", "palatino",
        "serif", "roman", "minion", "caslon", "baskerville",
    ];
    SERIF.iter().any(|s| lower.contains(s)) && !lower.contains("sans")
}

fn is_monospace_name(lower: &str) -> bool {
    const MONO: &[&str] = &["courier", "consolas", "mono", "menlo", "code"];
    MONO.iter().any(|s| lower.contains(s))
}

/// Builds a [`FontSpec`] with the fallback chain already attached.
pub fn spec_with_fallbacks(family: &str, size_pt: f32) -> FontSpec {
    let mut spec = FontSpec::new(family, size_pt);
    spec.fallbacks = fallbacks_for(family);
    spec
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibri_falls_back_to_its_metric_twin_before_a_generic() {
        let chain = fallbacks_for("Calibri");
        assert_eq!(chain, vec!["Carlito".to_string(), "sans-serif".to_string()]);
    }

    #[test]
    fn serif_and_mono_faces_get_the_matching_generic() {
        assert_eq!(fallbacks_for("Book Antiqua").last().map(String::as_str), Some("serif"));
        assert_eq!(fallbacks_for("Consolas").last().map(String::as_str), Some("monospace"));
        assert_eq!(fallbacks_for("Whatever UI").last().map(String::as_str), Some("sans-serif"));
    }

    #[test]
    fn a_sans_face_with_a_serif_substring_is_still_sans() {
        // "Bookman Sans" contains "book" but is explicitly sans.
        assert_eq!(
            fallbacks_for("Bookman Sans").last().map(String::as_str),
            Some("sans-serif")
        );
    }
}
