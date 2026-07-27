//! The theme part: colour scheme, font scheme, and the format scheme that `<p:style>`
//! references index into.

use crate::dl::Color;

use super::color::{ColorRef, SchemeColor, SchemeLookup};
use super::fill::{Effects, Fill, Line};

#[derive(Debug, Clone, PartialEq)]
pub struct ColorScheme {
    pub dark1: Color,
    pub light1: Color,
    pub dark2: Color,
    pub light2: Color,
    pub accent1: Color,
    pub accent2: Color,
    pub accent3: Color,
    pub accent4: Color,
    pub accent5: Color,
    pub accent6: Color,
    pub hyperlink: Color,
    pub followed_hyperlink: Color,
}

impl Default for ColorScheme {
    /// The Office 2013+ default theme, which is what a deck with a missing or unparseable
    /// theme part should look like rather than a pile of black rectangles.
    fn default() -> Self {
        ColorScheme {
            dark1: Color::rgb(0x00, 0x00, 0x00),
            light1: Color::rgb(0xFF, 0xFF, 0xFF),
            dark2: Color::rgb(0x44, 0x54, 0x6A),
            light2: Color::rgb(0xE7, 0xE6, 0xE6),
            accent1: Color::rgb(0x44, 0x72, 0xC4),
            accent2: Color::rgb(0xED, 0x7D, 0x31),
            accent3: Color::rgb(0xA5, 0xA5, 0xA5),
            accent4: Color::rgb(0xFF, 0xC0, 0x00),
            accent5: Color::rgb(0x5B, 0x9B, 0xD5),
            accent6: Color::rgb(0x70, 0xAD, 0x47),
            hyperlink: Color::rgb(0x05, 0x63, 0xC1),
            followed_hyperlink: Color::rgb(0x95, 0x4F, 0x72),
        }
    }
}

impl ColorScheme {
    pub fn get(&self, slot: SchemeColor) -> Color {
        match slot {
            SchemeColor::Dark1 | SchemeColor::Text1 => self.dark1,
            SchemeColor::Light1 | SchemeColor::Background1 => self.light1,
            SchemeColor::Dark2 | SchemeColor::Text2 => self.dark2,
            SchemeColor::Light2 | SchemeColor::Background2 => self.light2,
            SchemeColor::Accent1 => self.accent1,
            SchemeColor::Accent2 => self.accent2,
            SchemeColor::Accent3 => self.accent3,
            SchemeColor::Accent4 => self.accent4,
            SchemeColor::Accent5 => self.accent5,
            SchemeColor::Accent6 => self.accent6,
            SchemeColor::Hyperlink => self.hyperlink,
            SchemeColor::FollowedHyperlink => self.followed_hyperlink,
        }
    }

    pub fn set(&mut self, slot: SchemeColor, c: Color) {
        match slot {
            SchemeColor::Dark1 | SchemeColor::Text1 => self.dark1 = c,
            SchemeColor::Light1 | SchemeColor::Background1 => self.light1 = c,
            SchemeColor::Dark2 | SchemeColor::Text2 => self.dark2 = c,
            SchemeColor::Light2 | SchemeColor::Background2 => self.light2 = c,
            SchemeColor::Accent1 => self.accent1 = c,
            SchemeColor::Accent2 => self.accent2 = c,
            SchemeColor::Accent3 => self.accent3 = c,
            SchemeColor::Accent4 => self.accent4 = c,
            SchemeColor::Accent5 => self.accent5 = c,
            SchemeColor::Accent6 => self.accent6 = c,
            SchemeColor::Hyperlink => self.hyperlink = c,
            SchemeColor::FollowedHyperlink => self.followed_hyperlink = c,
        }
    }
}

impl SchemeLookup for ColorScheme {
    fn scheme_color(&self, slot: SchemeColor) -> Color {
        self.get(slot)
    }
}

/// A colour scheme seen through a master's colour map. This is the lookup layout uses:
/// resolving `tx1` against the raw scheme would ignore masters that swap the pairs, and
/// dark-background templates do exactly that.
pub struct MappedScheme<'a> {
    pub scheme: &'a ColorScheme,
    pub map: super::shape::ColorMap,
}

impl SchemeLookup for MappedScheme<'_> {
    fn scheme_color(&self, slot: SchemeColor) -> Color {
        self.scheme.get(self.map.resolve(slot))
    }
}

/// One entry in `<a:fontScheme>`: a latin/ea/cs triple plus per-script overrides.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FontCollection {
    pub latin: String,
    pub east_asian: String,
    pub complex_script: String,
    /// `<a:font script="Hang" typeface="맑은 고딕"/>` entries, keyed by script tag.
    pub scripts: Vec<(String, String)>,
}

impl FontCollection {
    pub fn for_script(&self, script: &str) -> Option<&str> {
        self.scripts
            .iter()
            .find(|(s, _)| s == script)
            .map(|(_, f)| f.as_str())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FontScheme {
    pub name: String,
    pub major: FontCollection,
    pub minor: FontCollection,
}

impl Default for FontScheme {
    fn default() -> Self {
        FontScheme {
            name: "Office".into(),
            major: FontCollection {
                latin: "Calibri Light".into(),
                ..Default::default()
            },
            minor: FontCollection {
                latin: "Calibri".into(),
                ..Default::default()
            },
        }
    }
}

/// `<a:fmtScheme>` — the numbered fill/line/effect styles a `<p:style>` points into.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FormatScheme {
    pub fill_styles: Vec<Fill>,
    pub line_styles: Vec<Line>,
    pub effect_styles: Vec<Effects>,
    pub background_fill_styles: Vec<Fill>,
}

impl FormatScheme {
    /// `<a:fillRef idx="2">` is 1-based, and index 0 means "no fill from the style".
    pub fn fill(&self, idx: u32) -> Option<&Fill> {
        if idx == 0 {
            return None;
        }
        self.fill_styles.get((idx - 1) as usize)
    }

    pub fn line(&self, idx: u32) -> Option<&Line> {
        if idx == 0 {
            return None;
        }
        self.line_styles.get((idx - 1) as usize)
    }

    pub fn effect(&self, idx: u32) -> Option<&Effects> {
        if idx == 0 {
            return None;
        }
        self.effect_styles.get((idx - 1) as usize)
    }

    /// Background fill styles are referenced with a 1001-based index (`<p:bgRef idx="1001">`).
    pub fn background_fill(&self, idx: u32) -> Option<&Fill> {
        if idx < 1001 {
            return self.fill(idx);
        }
        self.background_fill_styles.get((idx - 1001) as usize)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Theme {
    pub name: String,
    pub colors: ColorScheme,
    pub fonts: FontScheme,
    pub formats: FormatScheme,
    /// `<a:objectDefaults>` — per-shape-type default properties.
    pub default_shape_fill: Option<Fill>,
    pub default_shape_line: Option<Line>,
    pub default_text_style: super::text::ListStyle,
}

impl Theme {
    /// Resolves a colour reference against this theme, honouring a master's colour map.
    pub fn resolve_color(
        &self,
        c: &ColorRef,
        map: super::shape::ColorMap,
        placeholder: Option<Color>,
    ) -> Color {
        let mapped = MappedScheme {
            scheme: &self.colors,
            map,
        };
        c.resolve(&mapped, placeholder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::shape::ColorMap;

    #[test]
    fn default_theme_is_office_not_black() {
        let t = Theme::default();
        assert_eq!(t.colors.accent1, Color::rgb(0x44, 0x72, 0xC4));
        assert_eq!(t.colors.light1, Color::WHITE);
        assert_eq!(t.fonts.minor.latin, "Calibri");
    }

    #[test]
    fn a_swapped_colour_map_flips_text_and_background() {
        let mut theme = Theme::default();
        theme.colors.dark1 = Color::rgb(1, 1, 1);
        theme.colors.light1 = Color::rgb(254, 254, 254);
        let swapped = ColorMap {
            background1: SchemeColor::Dark1,
            text1: SchemeColor::Light1,
            background2: SchemeColor::Dark2,
            text2: SchemeColor::Light2,
        };
        let bg = ColorRef::scheme(SchemeColor::Background1);
        assert_eq!(theme.resolve_color(&bg, swapped, None), Color::rgb(1, 1, 1));
        assert_eq!(
            theme.resolve_color(&bg, ColorMap::default(), None),
            Color::rgb(254, 254, 254)
        );
    }

    #[test]
    fn format_scheme_indices_are_one_based_and_zero_means_none() {
        let fs = FormatScheme {
            fill_styles: vec![Fill::NoFill, Fill::Solid(ColorRef::scheme(SchemeColor::Accent1))],
            ..Default::default()
        };
        assert_eq!(fs.fill(0), None);
        assert_eq!(fs.fill(1), Some(&Fill::NoFill));
        assert!(matches!(fs.fill(2), Some(Fill::Solid(_))));
        assert_eq!(fs.fill(3), None, "out of range must not panic");
    }

    #[test]
    fn background_fill_refs_use_the_thousand_and_one_base() {
        let fs = FormatScheme {
            background_fill_styles: vec![Fill::NoFill, Fill::Group],
            ..Default::default()
        };
        assert_eq!(fs.background_fill(1001), Some(&Fill::NoFill));
        assert_eq!(fs.background_fill(1002), Some(&Fill::Group));
        assert_eq!(fs.background_fill(1003), None);
    }

    #[test]
    fn script_specific_fonts_are_looked_up_by_tag() {
        let fc = FontCollection {
            latin: "Calibri".into(),
            scripts: vec![("Hang".into(), "Malgun Gothic".into())],
            ..Default::default()
        };
        assert_eq!(fc.for_script("Hang"), Some("Malgun Gothic"));
        assert_eq!(fc.for_script("Arab"), None);
    }
}
