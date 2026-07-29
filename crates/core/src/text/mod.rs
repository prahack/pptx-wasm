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

    // Then a concrete, widely-packaged face of the same shape, and only then a generic.
    //
    // The concrete step is what keeps the chain deterministic. A generic family is
    // resolved by the browser, and Chromium, Firefox and WebKit do not resolve it the
    // same way even on the same machine — so a face whose metric twin is missing lands on
    // three different fonts, measures three different advances, and wraps in three
    // different places. That is not hypothetical: Georgia's only twin is Gelasio, which
    // is not packaged for Ubuntu at all, so `["Gelasio", "serif"]` fell through to the
    // generic on CI and Chromium broke a line one word later than the other two.
    //
    // These are not metric twins of the requested face and are not pretending to be. They
    // are a floor: the same wrong-but-identical answer everywhere beats three different
    // ones, because a deck that reflows depending on the reader's browser is worse than
    // one whose line breaks are uniformly a little off.
    let (concrete, generic): (&[&str], &str) = if is_monospace_name(&lower) {
        (&["Liberation Mono", "Cousine"], "monospace")
    } else if is_serif_name(&lower) {
        (&["Liberation Serif", "Tinos"], "serif")
    } else {
        (&["Liberation Sans", "Arimo"], "sans-serif")
    };
    for name in concrete {
        if !chain.iter().any(|c| c == name) {
            chain.push((*name).to_string());
        }
    }
    chain.push(generic.to_string());
    chain
}

fn is_serif_name(lower: &str) -> bool {
    const SERIF: &[&str] = &[
        "times",
        "georgia",
        "garamond",
        "book",
        "cambria",
        "constantia",
        "palatino",
        "serif",
        "roman",
        "minion",
        "caslon",
        "baskerville",
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
    #[test]
    fn every_chain_ends_in_a_concrete_face_before_its_generic() {
        // A generic family is resolved by the browser, and the three engines do not agree.
        // Whatever the requested face, the chain must name something real first, or a
        // machine missing that face wraps differently in each browser.
        for family in [
            "Calibri",
            "Cambria",
            "Arial",
            "Times New Roman",
            "Courier New",
            "Georgia",
            "Some Corporate Font",
            "Consolas",
            "Palatino",
        ] {
            let chain = fallbacks_for(family);
            let generic = chain.last().expect("a chain is never empty");
            assert!(
                matches!(generic.as_str(), "serif" | "sans-serif" | "monospace"),
                "{family} ends with {generic}, not a generic",
            );
            let before = chain
                .get(chain.len().wrapping_sub(2))
                .unwrap_or_else(|| panic!("{family} has only a generic to fall back on"));
            assert!(
                !matches!(before.as_str(), "serif" | "sans-serif" | "monospace"),
                "{family} has no concrete face before its generic",
            );
        }
    }

    #[test]
    fn georgia_falls_through_to_a_face_that_actually_exists() {
        // Gelasio is Georgia's metric twin and is not packaged for Ubuntu, so on Linux
        // this chain has to reach something installable. It failing to do so is what made
        // Chromium break an m2 line one word later than Firefox and WebKit on CI.
        let chain = fallbacks_for("Georgia");
        assert_eq!(chain.first().map(String::as_str), Some("Gelasio"));
        assert!(
            chain
                .iter()
                .any(|f| f == "Liberation Serif" || f == "Tinos"),
            "no obtainable serif in {chain:?}",
        );
    }

    #[test]
    fn a_metric_twin_is_never_repeated_by_the_concrete_step() {
        // Times' twins are already Liberation Serif and Tinos; appending them again would
        // be harmless but sloppy, and the list is read by humans debugging wrap points.
        let chain = fallbacks_for("Times New Roman");
        let mut seen = chain.clone();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), chain.len(), "duplicates in {chain:?}");
    }

    use super::*;

    #[test]
    fn calibri_falls_back_to_its_metric_twin_before_a_generic() {
        // The order is the point, not the length: the metric twin must come first, so a
        // machine that has it keeps Calibri's wrap points, and the generic must come last.
        // Asserting the exact list instead pins the tail as well, and the tail is allowed
        // to grow — a concrete face was added between the two precisely because a generic
        // is resolved differently by each browser.
        let chain = fallbacks_for("Calibri");
        assert_eq!(chain.first().map(String::as_str), Some("Carlito"));
        assert_eq!(chain.last().map(String::as_str), Some("sans-serif"));
        assert!(
            chain.iter().position(|f| f == "Carlito")
                < chain.iter().position(|f| f == "sans-serif"),
            "the twin must be tried before the generic: {chain:?}",
        );
    }

    #[test]
    fn serif_and_mono_faces_get_the_matching_generic() {
        assert_eq!(
            fallbacks_for("Book Antiqua").last().map(String::as_str),
            Some("serif")
        );
        assert_eq!(
            fallbacks_for("Consolas").last().map(String::as_str),
            Some("monospace")
        );
        assert_eq!(
            fallbacks_for("Whatever UI").last().map(String::as_str),
            Some("sans-serif")
        );
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
