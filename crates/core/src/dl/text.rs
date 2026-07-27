//! Text in the display list.
//!
//! A `TextRun` carries both the source string *and* per-cluster advances. That looks
//! redundant, but it is what lets one display list serve both backends: the Canvas2D
//! backend hands the string to the browser's shaper and trusts the advances only for
//! decoration extents, while the WebGPU backend positions glyphs itself from the
//! advances. Whoever produced the advances is also the authority on line breaking, so
//! the two backends break lines identically even though they rasterise differently.

use super::geom::Point;
use super::paint::Paint;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FontWeight {
    #[default]
    Regular,
    Bold,
}

impl FontWeight {
    /// CSS numeric weight, for the Canvas2D `font` shorthand.
    pub fn css_value(self) -> u16 {
        match self {
            FontWeight::Regular => 400,
            FontWeight::Bold => 700,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
}

/// A fully resolved font request: the theme lookup and the latin/ea/cs script choice
/// have already happened. `family` is the primary face; `fallbacks` is the chain to try
/// after it, ending with a generic family so the request can never fail outright.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FontSpec {
    pub family: String,
    pub fallbacks: Vec<String>,
    /// Size in points.
    pub size_pt: OrderedF32,
    pub weight: FontWeight,
    pub style: FontStyle,
}

impl FontSpec {
    pub fn new(family: impl Into<String>, size_pt: f32) -> Self {
        FontSpec {
            family: family.into(),
            fallbacks: Vec::new(),
            size_pt: OrderedF32(size_pt),
            weight: FontWeight::Regular,
            style: FontStyle::Normal,
        }
    }

    pub fn size(&self) -> f32 {
        self.size_pt.0
    }

    /// The CSS `font` shorthand, e.g. `italic 700 18px "Calibri", Arial, sans-serif`.
    ///
    /// Sizes are emitted in `px` with the point value substituted directly. That is
    /// deliberate: measurement happens in the same unit system the display list uses,
    /// so advances come back in points and stay zoom-independent. The renderer's view
    /// transform is what turns points into device pixels.
    pub fn to_css(&self) -> String {
        let mut families = String::new();
        for family in std::iter::once(&self.family).chain(self.fallbacks.iter()) {
            if !families.is_empty() {
                families.push_str(", ");
            }
            if family
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
            {
                families.push_str(family);
            } else {
                families.push('"');
                families.push_str(&family.replace('"', ""));
                families.push('"');
            }
        }
        let style = match self.style {
            FontStyle::Italic => "italic ",
            FontStyle::Normal => "",
        };
        format!(
            "{style}{} {}px {families}",
            self.weight.css_value(),
            self.size_pt.0
        )
    }
}

/// `f32` with total ordering and hashing, so a [`FontSpec`] can key a metrics cache.
/// NaN is not reachable here — sizes come from parsed integers — but the wrapper still
/// normalises it rather than trusting that.
#[derive(Debug, Clone, Copy)]
pub struct OrderedF32(pub f32);

impl PartialEq for OrderedF32 {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits() || self.0 == other.0
    }
}
impl Eq for OrderedF32 {}
impl std::hash::Hash for OrderedF32 {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let v = if self.0.is_nan() { 0.0 } else { self.0 };
        v.to_bits().hash(state);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Decorations {
    pub underline: bool,
    pub strikethrough: bool,
}

impl Decorations {
    pub fn any(&self) -> bool {
        self.underline || self.strikethrough
    }
}

/// One horizontal run of text sharing a single font and paint.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    pub text: String,
    pub font: FontSpec,
    /// Left end of the baseline, in slide points.
    pub origin: Point,
    pub paint: Paint,
    /// Per-`char` advance in points, parallel to `text.chars()`. Empty when the producer
    /// had no measurer available, in which case only the Canvas2D backend can draw it.
    pub advances: Vec<f32>,
    /// Total advance width in points. Always populated — decorations and alignment use it.
    pub width: f32,
    pub decorations: Decorations,
    /// Extra tracking added after each character, in points.
    pub letter_spacing: f32,
}

impl TextRun {
    pub fn has_glyph_positions(&self) -> bool {
        !self.advances.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_shorthand_quotes_only_families_that_need_it() {
        let mut f = FontSpec::new("Calibri", 18.0);
        f.fallbacks = vec!["Times New Roman".into(), "sans-serif".into()];
        f.weight = FontWeight::Bold;
        f.style = FontStyle::Italic;
        assert_eq!(
            f.to_css(),
            "italic 700 18px Calibri, \"Times New Roman\", sans-serif"
        );
    }

    #[test]
    fn font_spec_is_usable_as_a_cache_key() {
        use std::collections::HashMap;
        let mut m: HashMap<FontSpec, u32> = HashMap::new();
        m.insert(FontSpec::new("Arial", 12.0), 1);
        assert_eq!(m.get(&FontSpec::new("Arial", 12.0)), Some(&1));
        assert_eq!(m.get(&FontSpec::new("Arial", 12.5)), None);
    }

    #[test]
    fn embedded_quotes_cannot_break_out_of_the_family_list() {
        let f = FontSpec::new("Ev\"il, monospace; x", 10.0);
        assert_eq!(f.to_css(), "400 10px \"Evil, monospace; x\"");
    }
}
