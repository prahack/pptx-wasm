//! `ppt/tableStyles.xml`.
//!
//! Usually near-empty — see the note in [`crate::model::table_style`] — but decks that
//! define their own table styles do exist, and a defined style must beat the built-in.

use quick_xml::events::Event;

use crate::model::table_style::{PartStyle, TableStyle, TableStyles};

use super::drawing::{children, parse_color_container, parse_fill, parse_line};
use super::xml::{attr, local_name, Reader};

pub fn parse_table_styles(xml: &[u8]) -> TableStyles {
    let mut out = TableStyles::default();
    let mut reader = Reader::new(xml);
    let mut buf = Vec::new();

    let root = loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                out.default_id = attr(&e, b"def").unwrap_or_default();
                // A self-closing root carries the default and nothing else, which is the
                // overwhelmingly common case.
                if matches!(reader.read_event_into(&mut Vec::new()), Ok(Event::Eof)) {
                    return out;
                }
                break local_name(e.name().as_ref()).to_vec();
            }
            Ok(Event::Eof) | Err(_) => return out,
            _ => {}
        }
    };

    // Re-read from the start: the probe above consumed an event.
    let mut reader = Reader::new(xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(_)) => break,
            Ok(Event::Empty(_)) | Ok(Event::Eof) | Err(_) => return out,
            _ => {}
        }
    }

    children(&mut reader, &root, |r, e, empty| {
        if local_name(e.name().as_ref()) != b"tblStyle" || empty {
            return false;
        }
        let mut style = TableStyle {
            id: attr(e, b"styleId").unwrap_or_default(),
            name: attr(e, b"styleName").unwrap_or_default(),
            ..Default::default()
        };
        children(r, b"tblStyle", |r, part, part_empty| {
            if part_empty {
                return true;
            }
            let name = local_name(part.name().as_ref()).to_vec();
            let slot: Option<&mut PartStyle> = match name.as_slice() {
                b"wholeTbl" => Some(&mut style.whole_table),
                b"band1H" => Some(&mut style.band1_h),
                b"band2H" => Some(&mut style.band2_h),
                b"band1V" => Some(&mut style.band1_v),
                b"band2V" => Some(&mut style.band2_v),
                b"firstRow" => Some(&mut style.first_row),
                b"lastRow" => Some(&mut style.last_row),
                b"firstCol" => Some(&mut style.first_col),
                b"lastCol" => Some(&mut style.last_col),
                b"nwCell" => Some(&mut style.nw_cell),
                b"neCell" => Some(&mut style.ne_cell),
                b"swCell" => Some(&mut style.sw_cell),
                b"seCell" => Some(&mut style.se_cell),
                _ => None,
            };
            match slot {
                Some(target) => {
                    *target = parse_part(r, &name);
                    true
                }
                None => false,
            }
        });
        out.styles.push(style);
        true
    });
    out
}

/// One `<a:wholeTbl>`-shaped part: a text style and a cell style.
fn parse_part(r: &mut Reader<'_>, container: &[u8]) -> PartStyle {
    let mut part = PartStyle::default();
    children(r, container, |r, e, empty| {
        if empty {
            return true;
        }
        match local_name(e.name().as_ref()) {
            b"tcTxStyle" => {
                // Bold and italic are attributes here, not child elements.
                part.bold = bool_attr(e, b"b");
                part.italic = bool_attr(e, b"i");
                part.color = parse_color_container(r, b"tcTxStyle");
                true
            }
            b"tcStyle" => {
                children(r, b"tcStyle", |r, inner, inner_empty| {
                    match local_name(inner.name().as_ref()) {
                        b"tcBdr" => {
                            if inner_empty {
                                return true;
                            }
                            parse_borders(r, &mut part);
                            true
                        }
                        b"fill" => {
                            if inner_empty {
                                return true;
                            }
                            children(r, b"fill", |r, f, f_empty| {
                                if let Some(fill) = parse_fill(r, f, f_empty) {
                                    part.fill = fill;
                                    return !f_empty;
                                }
                                false
                            });
                            true
                        }
                        _ => false,
                    }
                });
                true
            }
            _ => false,
        }
    });
    part
}

fn parse_borders(r: &mut Reader<'_>, part: &mut PartStyle) {
    children(r, b"tcBdr", |r, e, empty| {
        if empty {
            return true;
        }
        let name = local_name(e.name().as_ref()).to_vec();
        let slot = match name.as_slice() {
            b"left" => Some(&mut part.left),
            b"right" => Some(&mut part.right),
            b"top" => Some(&mut part.top),
            b"bottom" => Some(&mut part.bottom),
            b"insideH" => Some(&mut part.inside_h),
            b"insideV" => Some(&mut part.inside_v),
            _ => None,
        };
        let Some(slot) = slot else { return false };
        // Each edge wraps a single `<a:ln>`.
        children(r, &name, |r, ln, ln_empty| {
            if local_name(ln.name().as_ref()) == b"ln" {
                *slot = parse_line(r, ln, ln_empty);
                return !ln_empty;
            }
            false
        });
        true
    });
}

fn bool_attr(e: &quick_xml::events::BytesStart<'_>, name: &[u8]) -> Option<bool> {
    match attr(e, name)?.trim() {
        "on" | "1" | "true" => Some(true),
        "off" | "0" | "false" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::fill::Fill;

    #[test]
    fn the_common_near_empty_form_yields_only_a_default_id() {
        let xml = br#"<?xml version="1.0"?>
<a:tblStyleLst xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" def="{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}"/>"#;
        let styles = parse_table_styles(xml);
        assert_eq!(styles.default_id, "{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}");
        assert!(styles.styles.is_empty());
        // It still resolves, via the built-in table.
        assert_eq!(styles.get(None).name, "Medium Style 2");
    }

    #[test]
    fn a_defined_style_is_parsed_with_its_parts() {
        let xml = br#"<a:tblStyleLst def="{AAA}">
          <a:tblStyle styleId="{AAA}" styleName="House Style">
            <a:wholeTbl>
              <a:tcTxStyle b="off"><a:srgbClr val="333333"/></a:tcTxStyle>
              <a:tcStyle>
                <a:tcBdr>
                  <a:left><a:ln w="12700"><a:solidFill><a:srgbClr val="CCCCCC"/></a:solidFill></a:ln></a:left>
                  <a:insideH><a:ln w="6350"><a:solidFill><a:srgbClr val="EEEEEE"/></a:solidFill></a:ln></a:insideH>
                </a:tcBdr>
                <a:fill><a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill></a:fill>
              </a:tcStyle>
            </a:wholeTbl>
            <a:firstRow>
              <a:tcTxStyle b="on"><a:srgbClr val="FFFFFF"/></a:tcTxStyle>
              <a:tcStyle><a:fill><a:solidFill><a:srgbClr val="003366"/></a:solidFill></a:fill></a:tcStyle>
            </a:firstRow>
          </a:tblStyle>
        </a:tblStyleLst>"#;
        let styles = parse_table_styles(xml);
        assert_eq!(styles.styles.len(), 1);
        let s = styles.get(None);
        assert_eq!(s.name, "House Style");
        assert_eq!(s.whole_table.bold, Some(false));
        assert_eq!(s.whole_table.left.width, Some(12_700));
        assert_eq!(s.whole_table.inside_h.width, Some(6_350));
        assert!(matches!(s.whole_table.fill, Fill::Solid(_)));
        assert_eq!(s.first_row.bold, Some(true));
        assert!(matches!(s.first_row.fill, Fill::Solid(_)));
        // Unspecified edges stay inheritable rather than becoming invisible.
        assert!(s.whole_table.right.is_empty());
    }

    #[test]
    fn a_missing_or_broken_file_yields_an_empty_set_rather_than_a_panic() {
        assert!(parse_table_styles(b"").styles.is_empty());
        assert!(parse_table_styles(b"<a:tblStyleLst><a:tblStyle").styles.is_empty());
        // With no default and no definitions, lookups still return something drawable.
        assert_eq!(parse_table_styles(b"").get(None).name, "Table Grid");
    }
}
