//! Unresolved colour references and the modifier stack OOXML applies to them.
//!
//! A colour in a deck is rarely a literal. It is usually "scheme colour `accent1`, tinted
//! 40%, luminance-modulated 60%", and the scheme it names depends on which master the
//! slide inherits from. So the model stores the *reference*, and layout resolves it once
//! the theme is known. Resolving earlier would bake in the wrong theme for any shape
//! that ends up on a different master.

use crate::dl::Color;

/// The twelve slots in a theme's colour scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SchemeColor {
    /// Text/background pairs. `dk1`/`lt1` are the "major" pair, `dk2`/`lt2` the minor.
    Dark1,
    Light1,
    Dark2,
    Light2,
    Accent1,
    Accent2,
    Accent3,
    Accent4,
    Accent5,
    Accent6,
    Hyperlink,
    FollowedHyperlink,
    /// `tx1`/`bg1`/`tx2`/`bg2` — the same slots seen through the master's colour map.
    Text1,
    Background1,
    Text2,
    Background2,
}

impl SchemeColor {
    pub fn parse(s: &str) -> Option<SchemeColor> {
        Some(match s {
            "dk1" => SchemeColor::Dark1,
            "lt1" => SchemeColor::Light1,
            "dk2" => SchemeColor::Dark2,
            "lt2" => SchemeColor::Light2,
            "accent1" => SchemeColor::Accent1,
            "accent2" => SchemeColor::Accent2,
            "accent3" => SchemeColor::Accent3,
            "accent4" => SchemeColor::Accent4,
            "accent5" => SchemeColor::Accent5,
            "accent6" => SchemeColor::Accent6,
            "hlink" => SchemeColor::Hyperlink,
            "folHlink" => SchemeColor::FollowedHyperlink,
            "tx1" => SchemeColor::Text1,
            "bg1" => SchemeColor::Background1,
            "tx2" => SchemeColor::Text2,
            "bg2" => SchemeColor::Background2,
            _ => return None,
        })
    }
}

/// One entry in the modifier stack. Order matters — OOXML applies them in document order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorMod {
    /// Absolute alpha, 0..1.
    Alpha(f32),
    /// Multiplies the current alpha.
    AlphaMod(f32),
    /// Adds to the current alpha.
    AlphaOff(f32),
    /// Mixes toward white by the given fraction.
    Tint(f32),
    /// Mixes toward black by the given fraction.
    Shade(f32),
    LumMod(f32),
    LumOff(f32),
    SatMod(f32),
    SatOff(f32),
    HueMod(f32),
    HueOff(f32),
    /// Replaces with the greyscale equivalent.
    Gray,
    /// Inverts each channel.
    Inverse,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ColorSpec {
    /// `<a:srgbClr val="RRGGBB"/>`
    Srgb(Color),
    /// `<a:schemeClr val="accent1"/>`
    Scheme(SchemeColor),
    /// `<a:sysClr val="windowText" lastClr="000000"/>` — we trust `lastClr`, since a
    /// browser has no access to the authoring machine's system palette.
    System(Color),
    /// `<a:prstClr val="red"/>`
    Preset(Color),
    /// `<a:hslClr hue=".." sat=".." lum=".."/>`
    Hsl { h: f32, s: f32, l: f32 },
    /// `<a:schemeClr val="phClr"/>` — "whatever colour the style is being applied with".
    /// Only meaningful inside a theme format scheme.
    Placeholder,
}

/// A colour reference plus its modifier stack.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorRef {
    pub spec: ColorSpec,
    pub mods: Vec<ColorMod>,
}

impl ColorRef {
    pub fn new(spec: ColorSpec) -> Self {
        ColorRef {
            spec,
            mods: Vec::new(),
        }
    }

    pub fn srgb(c: Color) -> Self {
        ColorRef::new(ColorSpec::Srgb(c))
    }

    pub fn scheme(s: SchemeColor) -> Self {
        ColorRef::new(ColorSpec::Scheme(s))
    }

    /// Resolves to a concrete colour.
    ///
    /// `scheme` supplies the theme slots; `placeholder` is the colour `phClr` stands for
    /// in the current style context (`None` outside a format scheme).
    pub fn resolve(&self, scheme: &dyn SchemeLookup, placeholder: Option<Color>) -> Color {
        let base = match &self.spec {
            ColorSpec::Srgb(c) | ColorSpec::System(c) | ColorSpec::Preset(c) => *c,
            ColorSpec::Scheme(s) => scheme.scheme_color(*s),
            ColorSpec::Hsl { h, s, l } => hsl_to_rgb(*h, *s, *l),
            ColorSpec::Placeholder => placeholder.unwrap_or(Color::BLACK),
        };
        apply_mods(base, &self.mods)
    }
}

/// Supplies theme colour-scheme slots. Implemented by the theme, wrapped by the master's
/// colour map so that `tx1`/`bg1` land on the right underlying slot.
pub trait SchemeLookup {
    fn scheme_color(&self, slot: SchemeColor) -> Color;
}

/// Applies a modifier stack in order.
pub fn apply_mods(base: Color, mods: &[ColorMod]) -> Color {
    let mut c = base;
    for m in mods {
        c = apply_mod(c, *m);
    }
    c
}

fn apply_mod(c: Color, m: ColorMod) -> Color {
    match m {
        ColorMod::Alpha(a) => Color { a: to_u8(a), ..c },
        ColorMod::AlphaMod(f) => c.with_alpha_factor(f),
        ColorMod::AlphaOff(o) => Color {
            a: to_u8(c.a as f32 / 255.0 + o),
            ..c
        },
        // ECMA-376: "a 10% tint is 10% of the input colour combined with 90% white", and
        // shade is the same against black. So the value is the fraction of the *original*
        // that survives, not the amount of white/black added — `shade val="60000"` is a
        // colour 60% as bright, not 40%.
        //
        // Crucially the mix happens in **linear** light, not on sRGB byte values. This is
        // not a subtlety we can skip: accent1 #4F81BD tinted 40% is #D0D8E7 in both
        // PowerPoint and LibreOffice, and #B9CBE0 if you lerp the sRGB bytes. Every
        // banded table and every "Lighter 40%" theme colour in a deck depends on it.
        ColorMod::Tint(t) => lerp_linear(c, 1.0, t.clamp(0.0, 1.0)),
        ColorMod::Shade(s) => lerp_linear(c, 0.0, s.clamp(0.0, 1.0)),
        ColorMod::Gray => {
            let y = luminance(c);
            Color {
                r: to_u8(y),
                g: to_u8(y),
                b: to_u8(y),
                a: c.a,
            }
        }
        ColorMod::Inverse => Color {
            r: 255 - c.r,
            g: 255 - c.g,
            b: 255 - c.b,
            a: c.a,
        },
        ColorMod::LumMod(f) => map_hsl(c, |h, s, l| (h, s, l * f)),
        ColorMod::LumOff(o) => map_hsl(c, |h, s, l| (h, s, l + o)),
        ColorMod::SatMod(f) => map_hsl(c, |h, s, l| (h, s * f, l)),
        ColorMod::SatOff(o) => map_hsl(c, |h, s, l| (h, s + o, l)),
        ColorMod::HueMod(f) => map_hsl(c, |h, s, l| ((h * f).rem_euclid(360.0), s, l)),
        ColorMod::HueOff(o) => map_hsl(c, |h, s, l| ((h + o).rem_euclid(360.0), s, l)),
    }
}

/// Mixes toward `toward` (0.0 = black, 1.0 = white) in linear light, keeping `factor` of
/// the original. See the note on [`ColorMod::Tint`] for why the linear step matters.
fn lerp_linear(c: Color, toward: f32, factor: f32) -> Color {
    let mix = |v: u8| {
        let linear = srgb_to_linear(v as f32 / 255.0);
        to_u8(linear_to_srgb(linear * factor + toward * (1.0 - factor)))
    };
    Color {
        r: mix(c.r),
        g: mix(c.g),
        b: mix(c.b),
        a: c.a,
    }
}

/// sRGB transfer function, inverse. Input and output are 0..1.
pub fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// sRGB transfer function. Input and output are 0..1.
pub fn linear_to_srgb(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

fn to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn luminance(c: Color) -> f32 {
    (0.299 * c.r as f32 + 0.587 * c.g as f32 + 0.114 * c.b as f32) / 255.0
}

fn map_hsl(c: Color, f: impl Fn(f32, f32, f32) -> (f32, f32, f32)) -> Color {
    let (h, s, l) = rgb_to_hsl(c);
    let (h, s, l) = f(h, s, l);
    let out = hsl_to_rgb(h, s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
    Color { a: c.a, ..out }
}

/// Hue in degrees 0..360, saturation and lightness 0..1.
pub fn rgb_to_hsl(c: Color) -> (f32, f32, f32) {
    let (r, g, b) = (c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d.abs() < f32::EPSILON {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    ((h * 60.0).rem_euclid(360.0), s, l)
}

pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> Color {
    if s <= 0.0 {
        let v = to_u8(l);
        return Color::rgb(v, v, v);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let hk = h.rem_euclid(360.0) / 360.0;
    let f = |mut t: f32| {
        t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    Color::rgb(
        to_u8(f(hk + 1.0 / 3.0)),
        to_u8(f(hk)),
        to_u8(f(hk - 1.0 / 3.0)),
    )
}

/// The DrawingML preset colour names.
pub fn preset_color(name: &str) -> Option<Color> {
    // The full list is 140 CSS-ish names; these are the ones that appear in decks.
    Some(match name {
        "black" => Color::rgb(0, 0, 0),
        "white" => Color::rgb(255, 255, 255),
        "red" => Color::rgb(255, 0, 0),
        "green" => Color::rgb(0, 128, 0),
        "blue" => Color::rgb(0, 0, 255),
        "yellow" => Color::rgb(255, 255, 0),
        "cyan" | "aqua" => Color::rgb(0, 255, 255),
        "magenta" | "fuchsia" => Color::rgb(255, 0, 255),
        "gray" | "grey" => Color::rgb(128, 128, 128),
        "darkGray" => Color::rgb(169, 169, 169),
        "lightGray" => Color::rgb(211, 211, 211),
        "orange" => Color::rgb(255, 165, 0),
        "purple" => Color::rgb(128, 0, 128),
        "brown" => Color::rgb(165, 42, 42),
        "pink" => Color::rgb(255, 192, 203),
        "gold" => Color::rgb(255, 215, 0),
        "silver" => Color::rgb(192, 192, 192),
        "navy" => Color::rgb(0, 0, 128),
        "teal" => Color::rgb(0, 128, 128),
        "olive" => Color::rgb(128, 128, 0),
        "maroon" => Color::rgb(128, 0, 0),
        "lime" => Color::rgb(0, 255, 0),
        "darkBlue" => Color::rgb(0, 0, 139),
        "darkRed" => Color::rgb(139, 0, 0),
        "darkGreen" => Color::rgb(0, 100, 0),
        "lightBlue" => Color::rgb(173, 216, 230),
        "lightGreen" => Color::rgb(144, 238, 144),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedScheme;
    impl SchemeLookup for FixedScheme {
        fn scheme_color(&self, slot: SchemeColor) -> Color {
            match slot {
                SchemeColor::Accent1 => Color::rgb(0x44, 0x72, 0xC4),
                SchemeColor::Dark1 | SchemeColor::Text1 => Color::BLACK,
                SchemeColor::Light1 | SchemeColor::Background1 => Color::WHITE,
                _ => Color::rgb(0x80, 0x80, 0x80),
            }
        }
    }

    #[test]
    fn scheme_names_round_trip() {
        assert_eq!(SchemeColor::parse("accent3"), Some(SchemeColor::Accent3));
        assert_eq!(
            SchemeColor::parse("folHlink"),
            Some(SchemeColor::FollowedHyperlink)
        );
        assert_eq!(SchemeColor::parse("bogus"), None);
    }

    #[test]
    fn scheme_colours_resolve_through_the_lookup() {
        let r = ColorRef::scheme(SchemeColor::Accent1);
        assert_eq!(r.resolve(&FixedScheme, None), Color::rgb(0x44, 0x72, 0xC4));
    }

    #[test]
    fn tint_moves_toward_white_and_shade_toward_black() {
        let base = Color::rgb(100, 100, 100);
        let tinted = apply_mods(base, &[ColorMod::Tint(0.5)]);
        let shaded = apply_mods(base, &[ColorMod::Shade(0.5)]);
        assert!(tinted.r > base.r, "tint should lighten: {}", tinted.r);
        assert!(shaded.r < base.r, "shade should darken: {}", shaded.r);
        // The value is the fraction of the original kept, so 0% of either is the
        // extreme and 100% is a no-op.
        assert_eq!(apply_mods(base, &[ColorMod::Tint(0.0)]), Color::WHITE);
        assert_eq!(apply_mods(base, &[ColorMod::Shade(0.0)]), Color::BLACK);
        assert_eq!(apply_mods(base, &[ColorMod::Shade(1.0)]), base);
        assert_eq!(apply_mods(base, &[ColorMod::Tint(1.0)]), base);
    }

    #[test]
    fn shade_keeps_the_given_fraction_of_the_original_in_linear_light() {
        // `<a:shade val="60000"/>` on white keeps 60% of the *linear* value, which is far
        // brighter than 60% of the sRGB byte (153) because of the transfer curve.
        let c = apply_mods(Color::WHITE, &[ColorMod::Shade(0.6)]);
        assert_eq!(c.r, 203);
    }

    fn assert_close(actual: Color, expected: Color, tolerance: i16) {
        let d = |a: u8, b: u8| (a as i16 - b as i16).abs();
        assert!(
            d(actual.r, expected.r) <= tolerance
                && d(actual.g, expected.g) <= tolerance
                && d(actual.b, expected.b) <= tolerance,
            "{actual:?} is not within {tolerance} of {expected:?}"
        );
    }

    /// Sampled from a LibreOffice render of a deck using the built-in
    /// "Medium Style 2 - Accent 1" table style, whose banded rows are the Office accent1
    /// tinted 40% and 20%.
    ///
    /// This is the evidence that tint is a *linear-light* operation: an sRGB-space lerp
    /// gives #B9CBE0 and #DCE2EF, which are wrong by 20-30 per channel. The ±1 tolerance
    /// is the two implementations' rounding, not a difference in the formula.
    #[test]
    fn tint_matches_powerpoints_own_output_for_the_office_accent1() {
        let accent1 = Color::rgb(0x4F, 0x81, 0xBD);
        assert_close(
            apply_mods(accent1, &[ColorMod::Tint(0.4)]),
            Color::rgb(0xD0, 0xD8, 0xE7),
            1,
        );
        assert_close(
            apply_mods(accent1, &[ColorMod::Tint(0.2)]),
            Color::rgb(0xE9, 0xEC, 0xF3),
            1,
        );
        // The sRGB-space answer would have been these, and is nowhere near.
        let srgb_lerp = Color::rgb(0xB9, 0xCB, 0xE0);
        let linear = apply_mods(accent1, &[ColorMod::Tint(0.4)]);
        assert!(
            (linear.r as i16 - srgb_lerp.r as i16).abs() > 15,
            "the linear and sRGB answers must be distinguishable"
        );
    }

    #[test]
    fn the_srgb_transfer_function_round_trips() {
        for v in [0.0f32, 0.002, 0.04, 0.5, 0.9, 1.0] {
            let back = linear_to_srgb(srgb_to_linear(v));
            assert!((back - v).abs() < 1e-4, "{v} -> {back}");
        }
    }

    #[test]
    fn the_common_accent1_luminance_pair_lightens() {
        // The exact stack PowerPoint writes for "Accent 1, Lighter 40%".
        let r = ColorRef {
            spec: ColorSpec::Scheme(SchemeColor::Accent1),
            mods: vec![ColorMod::LumMod(0.6), ColorMod::LumOff(0.4)],
        };
        let out = r.resolve(&FixedScheme, None);
        let base = Color::rgb(0x44, 0x72, 0xC4);
        assert!(
            luminance(out) > luminance(base),
            "expected lighter, got {out:?} from {base:?}"
        );
    }

    #[test]
    fn alpha_modifiers_compose_in_document_order() {
        let c = apply_mods(
            Color::rgb(1, 2, 3),
            &[ColorMod::Alpha(0.5), ColorMod::AlphaMod(0.5)],
        );
        assert_eq!(c.a, 64);
    }

    #[test]
    fn hsl_round_trips_within_a_quantisation_step() {
        for c in [
            Color::rgb(0x44, 0x72, 0xC4),
            Color::rgb(255, 0, 0),
            Color::rgb(18, 200, 77),
            Color::rgb(128, 128, 128),
        ] {
            let (h, s, l) = rgb_to_hsl(c);
            let back = hsl_to_rgb(h, s, l);
            assert!(
                (back.r as i16 - c.r as i16).abs() <= 1
                    && (back.g as i16 - c.g as i16).abs() <= 1
                    && (back.b as i16 - c.b as i16).abs() <= 1,
                "{c:?} -> hsl({h},{s},{l}) -> {back:?}"
            );
        }
    }

    #[test]
    fn placeholder_without_a_context_colour_degrades_to_black() {
        let r = ColorRef::new(ColorSpec::Placeholder);
        assert_eq!(r.resolve(&FixedScheme, None), Color::BLACK);
        assert_eq!(r.resolve(&FixedScheme, Some(Color::WHITE)), Color::WHITE);
    }

    #[test]
    fn greyscale_preserves_alpha() {
        let c = apply_mods(Color::rgba(255, 0, 0, 128), &[ColorMod::Gray]);
        assert_eq!(c.a, 128);
        assert_eq!(c.r, c.g);
        assert_eq!(c.g, c.b);
    }
}
