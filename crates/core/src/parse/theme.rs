//! `theme1.xml` — colour scheme, font scheme and format scheme.

use crate::model::color::SchemeColor;
use crate::model::fill::{Effects, Fill, Line};
use crate::model::theme::{ColorScheme, FontCollection, FontScheme, FormatScheme, Theme};

use super::drawing::{children, parse_color_element, parse_effects, parse_fill, parse_line};
use super::text::parse_list_style;
use super::xml::{attr, local_name, Reader};

pub fn parse_theme(xml: &[u8]) -> Theme {
    let mut theme = Theme::default();
    let mut r = Reader::new(xml);
    let mut buf = Vec::new();
    let root = loop {
        match r.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(e)) => break local_name(e.name().as_ref()).to_vec(),
            Ok(quick_xml::events::Event::Eof) | Err(_) => return theme,
            _ => {}
        }
    };
    theme.name = root_name(xml).unwrap_or_default();

    children(&mut r, &root, |r, e, empty| {
        match local_name(e.name().as_ref()) {
            b"themeElements" => {
                if empty {
                    return true;
                }
                children(r, b"themeElements", |r, child, child_empty| {
                    if child_empty {
                        return true;
                    }
                    match local_name(child.name().as_ref()) {
                        b"clrScheme" => {
                            theme.colors = parse_color_scheme(r);
                            true
                        }
                        b"fontScheme" => {
                            theme.fonts = parse_font_scheme(r, child);
                            true
                        }
                        b"fmtScheme" => {
                            theme.formats = parse_format_scheme(r);
                            true
                        }
                        _ => false,
                    }
                });
                true
            }
            b"objectDefaults" => {
                if empty {
                    return true;
                }
                let (fill, line) = parse_object_defaults(r);
                theme.default_shape_fill = fill;
                theme.default_shape_line = line;
                true
            }
            _ => false,
        }
    });
    theme
}

fn root_name(xml: &[u8]) -> Option<String> {
    let mut r = Reader::new(xml);
    let mut buf = Vec::new();
    loop {
        match r.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(e)) => return attr(&e, b"name"),
            Ok(quick_xml::events::Event::Eof) | Err(_) => return None,
            _ => {}
        }
    }
}

fn parse_color_scheme(r: &mut Reader<'_>) -> ColorScheme {
    let mut scheme = ColorScheme::default();
    children(r, b"clrScheme", |r, e, empty| {
        let name = local_name(e.name().as_ref()).to_vec();
        let Some(slot) = SchemeColor::parse(&String::from_utf8_lossy(&name)) else {
            return false;
        };
        if empty {
            return true;
        }
        // Each slot wraps exactly one colour element. Theme colours cannot themselves be
        // scheme references, so resolving against the (default) scheme is safe here.
        let mut found = None;
        children(r, &name, |r, inner, inner_empty| {
            if found.is_none() {
                if let Some(c) = parse_color_element(r, inner, inner_empty) {
                    found = Some(c);
                    return !inner_empty;
                }
            }
            false
        });
        if let Some(c) = found {
            scheme.set(slot, c.resolve(&ColorScheme::default(), None));
        }
        true
    });
    scheme
}

fn parse_font_scheme(r: &mut Reader<'_>, e: &quick_xml::events::BytesStart<'_>) -> FontScheme {
    let mut fs = FontScheme {
        name: attr(e, b"name").unwrap_or_default(),
        ..Default::default()
    };
    children(r, b"fontScheme", |r, child, child_empty| {
        let slot = match local_name(child.name().as_ref()) {
            b"majorFont" => 0,
            b"minorFont" => 1,
            _ => return false,
        };
        if child_empty {
            return true;
        }
        let container = local_name(child.name().as_ref()).to_vec();
        let collection = parse_font_collection(r, &container);
        if slot == 0 {
            fs.major = collection;
        } else {
            fs.minor = collection;
        }
        true
    });
    fs
}

fn parse_font_collection(r: &mut Reader<'_>, container: &[u8]) -> FontCollection {
    let mut fc = FontCollection::default();
    children(r, container, |_r, e, _empty| {
        let typeface = attr(e, b"typeface").unwrap_or_default();
        match local_name(e.name().as_ref()) {
            b"latin" => fc.latin = typeface,
            b"ea" => fc.east_asian = typeface,
            b"cs" => fc.complex_script = typeface,
            b"font" => {
                if let Some(script) = attr(e, b"script") {
                    fc.scripts.push((script, typeface));
                }
            }
            _ => {}
        }
        false
    });
    fc
}

fn parse_format_scheme(r: &mut Reader<'_>) -> FormatScheme {
    let mut fs = FormatScheme::default();
    children(r, b"fmtScheme", |r, e, empty| {
        if empty {
            return true;
        }
        let name = local_name(e.name().as_ref()).to_vec();
        match name.as_slice() {
            b"fillStyleLst" | b"bgFillStyleLst" => {
                let mut list = Vec::new();
                children(r, &name, |r, child, child_empty| {
                    if let Some(f) = parse_fill(r, child, child_empty) {
                        list.push(f);
                        return !child_empty;
                    }
                    false
                });
                if name == b"fillStyleLst" {
                    fs.fill_styles = list;
                } else {
                    fs.background_fill_styles = list;
                }
                true
            }
            b"lnStyleLst" => {
                children(r, b"lnStyleLst", |r, child, child_empty| {
                    if local_name(child.name().as_ref()) == b"ln" {
                        fs.line_styles.push(parse_line(r, child, child_empty));
                        return !child_empty;
                    }
                    false
                });
                true
            }
            b"effectStyleLst" => {
                children(r, b"effectStyleLst", |r, style, style_empty| {
                    if local_name(style.name().as_ref()) != b"effectStyle" {
                        return false;
                    }
                    if style_empty {
                        fs.effect_styles.push(Effects::default());
                        return true;
                    }
                    let mut fx = Effects::default();
                    children(r, b"effectStyle", |r, inner, inner_empty| {
                        if local_name(inner.name().as_ref()) == b"effectLst" {
                            fx = parse_effects(r, inner_empty);
                            return !inner_empty;
                        }
                        false
                    });
                    fs.effect_styles.push(fx);
                    true
                });
                true
            }
            _ => false,
        }
    });
    fs
}

fn parse_object_defaults(r: &mut Reader<'_>) -> (Option<Fill>, Option<Line>) {
    let mut fill = None;
    let mut line = None;
    children(r, b"objectDefaults", |r, e, empty| {
        if local_name(e.name().as_ref()) != b"spDef" || empty {
            return false;
        }
        children(r, b"spDef", |r, child, child_empty| {
            if local_name(child.name().as_ref()) != b"spPr" || child_empty {
                return false;
            }
            children(r, b"spPr", |r, prop, prop_empty| {
                if let Some(f) = parse_fill(r, prop, prop_empty) {
                    fill = Some(f);
                    return !prop_empty;
                }
                if local_name(prop.name().as_ref()) == b"ln" {
                    line = Some(parse_line(r, prop, prop_empty));
                    return !prop_empty;
                }
                false
            });
            true
        });
        true
    });
    (fill, line)
}

/// `<p:defaultTextStyle>` from `presentation.xml`, which shares the list-style shape.
pub fn parse_default_text_style(r: &mut Reader<'_>) -> crate::model::text::ListStyle {
    parse_list_style(r, b"defaultTextStyle")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dl::Color;

    const THEME: &[u8] = br#"<?xml version="1.0"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Ion Boardroom">
  <a:themeElements>
    <a:clrScheme name="Ion Boardroom">
      <a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1>
      <a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>
      <a:dk2><a:srgbClr val="1F2C3C"/></a:dk2>
      <a:lt2><a:srgbClr val="EEEEEE"/></a:lt2>
      <a:accent1><a:srgbClr val="B31166"/></a:accent1>
      <a:accent2><a:srgbClr val="E33D6F"/></a:accent2>
      <a:accent3><a:srgbClr val="E45F3C"/></a:accent3>
      <a:accent4><a:srgbClr val="E9943A"/></a:accent4>
      <a:accent5><a:srgbClr val="9B6BF2"/></a:accent5>
      <a:accent6><a:srgbClr val="D53DD0"/></a:accent6>
      <a:hlink><a:srgbClr val="55C1FF"/></a:hlink>
      <a:folHlink><a:srgbClr val="8E82EA"/></a:folHlink>
    </a:clrScheme>
    <a:fontScheme name="Ion Boardroom">
      <a:majorFont>
        <a:latin typeface="Century Gothic"/><a:ea typeface=""/><a:cs typeface=""/>
        <a:font script="Jpan" typeface="Yu Gothic UI"/>
      </a:majorFont>
      <a:minorFont>
        <a:latin typeface="Century Gothic"/><a:ea typeface=""/><a:cs typeface=""/>
      </a:minorFont>
    </a:fontScheme>
    <a:fmtScheme name="Ion Boardroom">
      <a:fillStyleLst>
        <a:solidFill><a:schemeClr val="phClr"/></a:solidFill>
        <a:gradFill rotWithShape="1">
          <a:gsLst>
            <a:gs pos="0"><a:schemeClr val="phClr"><a:tint val="60000"/></a:schemeClr></a:gs>
            <a:gs pos="100000"><a:schemeClr val="phClr"><a:shade val="80000"/></a:schemeClr></a:gs>
          </a:gsLst>
          <a:lin ang="5400000" scaled="0"/>
        </a:gradFill>
      </a:fillStyleLst>
      <a:lnStyleLst>
        <a:ln w="6350"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln>
        <a:ln w="12700"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln>
      </a:lnStyleLst>
      <a:effectStyleLst>
        <a:effectStyle><a:effectLst/></a:effectStyle>
        <a:effectStyle><a:effectLst><a:outerShdw blurRad="57150" dist="19050" dir="5400000"><a:srgbClr val="000000"><a:alpha val="63000"/></a:srgbClr></a:outerShdw></a:effectLst></a:effectStyle>
      </a:effectStyleLst>
      <a:bgFillStyleLst>
        <a:solidFill><a:schemeClr val="phClr"/></a:solidFill>
      </a:bgFillStyleLst>
    </a:fmtScheme>
  </a:themeElements>
</a:theme>"#;

    #[test]
    fn colour_scheme_slots_are_all_read() {
        let t = parse_theme(THEME);
        assert_eq!(t.name, "Ion Boardroom");
        assert_eq!(t.colors.accent1, Color::rgb(0xB3, 0x11, 0x66));
        assert_eq!(t.colors.accent6, Color::rgb(0xD5, 0x3D, 0xD0));
        assert_eq!(t.colors.hyperlink, Color::rgb(0x55, 0xC1, 0xFF));
        assert_eq!(t.colors.dark2, Color::rgb(0x1F, 0x2C, 0x3C));
    }

    #[test]
    fn sysclr_falls_back_to_its_lastclr() {
        let t = parse_theme(THEME);
        assert_eq!(t.colors.dark1, Color::BLACK);
        assert_eq!(t.colors.light1, Color::WHITE);
    }

    #[test]
    fn font_scheme_captures_latin_and_script_specific_faces() {
        let t = parse_theme(THEME);
        assert_eq!(t.fonts.name, "Ion Boardroom");
        assert_eq!(t.fonts.major.latin, "Century Gothic");
        assert_eq!(t.fonts.minor.latin, "Century Gothic");
        assert!(t.fonts.major.for_script("Jpan").is_some());
    }

    #[test]
    fn format_scheme_lists_are_indexable_one_based() {
        let t = parse_theme(THEME);
        assert_eq!(t.formats.fill_styles.len(), 2);
        assert!(matches!(t.formats.fill(1), Some(Fill::Solid(_))));
        assert!(matches!(t.formats.fill(2), Some(Fill::Gradient(_))));
        assert_eq!(t.formats.line(2).and_then(|l| l.width), Some(12_700));
        assert!(t.formats.effect(1).map(|e| e.is_empty()).unwrap_or(false));
        assert!(t.formats.effect(2).map(|e| e.outer_shadow.is_some()).unwrap_or(false));
        assert!(matches!(t.formats.background_fill(1001), Some(Fill::Solid(_))));
    }

    #[test]
    fn phclr_survives_parsing_so_it_can_be_substituted_later() {
        let t = parse_theme(THEME);
        match t.formats.fill(1) {
            Some(Fill::Solid(c)) => {
                assert_eq!(c.spec, crate::model::color::ColorSpec::Placeholder);
            }
            other => panic!("expected a phClr solid fill, got {other:?}"),
        }
    }

    #[test]
    fn a_missing_or_broken_theme_falls_back_to_the_office_defaults() {
        let t = parse_theme(b"");
        assert_eq!(t.colors.accent1, ColorScheme::default().accent1);
        let t = parse_theme(b"<a:theme><a:themeElements><a:clrScheme");
        assert_eq!(t.colors.accent1, ColorScheme::default().accent1);
    }
}
