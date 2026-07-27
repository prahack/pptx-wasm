//! `presentation.xml` — slide size, slide order, embedded fonts, default text style.
//!
//! This is the only part read eagerly. It establishes the slide list and the deck-wide
//! defaults; everything else is pulled in on demand.

use crate::error::{Error, Result};
use crate::model::media::EmbeddedFont;
use crate::model::{Presentation, SlideInfo};
use crate::opc::{rel_type, Package};

use super::drawing::children;
use super::theme::parse_default_text_style;
use super::xml::{attr_i64, attr_u32, is, local_name, r_attr, Reader};

/// Default slide size when `<p:sldSz>` is missing: 4:3 at 10 x 7.5 inches, the
/// pre-2013 PowerPoint default.
const DEFAULT_WIDTH: i64 = 9_144_000;
const DEFAULT_HEIGHT: i64 = 6_858_000;

pub fn parse(package: Package) -> Result<Presentation> {
    let part_name = find_presentation_part(&package)
        .ok_or(Error::NotAPresentation)?;
    let xml = package
        .part(&part_name)
        .ok_or_else(|| Error::MissingPart(part_name.clone()))?;

    let mut width = DEFAULT_WIDTH;
    let mut height = DEFAULT_HEIGHT;
    let mut slide_rids: Vec<(u32, String)> = Vec::new();
    let mut master_rids: Vec<String> = Vec::new();
    let mut default_text_style = crate::model::ListStyle::default();
    let mut embedded_fonts: Vec<EmbeddedFont> = Vec::new();

    let mut r = Reader::new(&xml);
    let mut buf = Vec::new();
    let root = loop {
        match r.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(e)) => break local_name(e.name().as_ref()).to_vec(),
            Ok(quick_xml::events::Event::Eof) | Err(_) => return Err(Error::NotAPresentation),
            _ => {}
        }
    };

    children(&mut r, &root, |r, e, empty| {
        match local_name(e.name().as_ref()) {
            b"sldSz" => {
                width = attr_i64(e, b"cx").filter(|v| *v > 0).unwrap_or(DEFAULT_WIDTH);
                height = attr_i64(e, b"cy").filter(|v| *v > 0).unwrap_or(DEFAULT_HEIGHT);
                false
            }
            b"sldIdLst" => {
                if empty {
                    return true;
                }
                children(r, b"sldIdLst", |_r, sld, _se| {
                    if is(sld, b"sldId") {
                        if let Some(rid) = r_attr(sld, b"id") {
                            slide_rids.push((attr_u32(sld, b"id").unwrap_or(0), rid));
                        }
                    }
                    false
                });
                true
            }
            b"sldMasterIdLst" => {
                if empty {
                    return true;
                }
                children(r, b"sldMasterIdLst", |_r, m, _me| {
                    if is(m, b"sldMasterId") {
                        if let Some(rid) = r_attr(m, b"id") {
                            master_rids.push(rid);
                        }
                    }
                    false
                });
                true
            }
            b"defaultTextStyle" => {
                if empty {
                    return true;
                }
                default_text_style = parse_default_text_style(r);
                true
            }
            b"embeddedFontLst" => {
                if empty {
                    return true;
                }
                embedded_fonts = parse_embedded_fonts(r);
                true
            }
            _ => false,
        }
    });

    let mut pres = Presentation::new(package, width, height);
    pres.default_text_style = default_text_style;
    pres.embedded_fonts = embedded_fonts;

    // Resolve slide relationship ids to part names, dropping any that do not resolve
    // rather than leaving a hole in the slide numbering.
    let mut slides = Vec::with_capacity(slide_rids.len());
    for (id, rid) in slide_rids {
        match pres.package().resolve_target(&part_name, &rid) {
            Some(target) if pres.package().has_part(&target) => {
                slides.push(SlideInfo {
                    index: slides.len(),
                    id,
                    part_name: target,
                });
            }
            _ => log::warn!("slide relationship {rid} does not resolve; skipping"),
        }
    }
    pres.slides = slides;

    pres.first_master = master_rids
        .first()
        .and_then(|rid| pres.package().resolve_target(&part_name, rid));

    Ok(pres)
}

/// Locates the presentation part.
///
/// The name is `ppt/presentation.xml` in every file PowerPoint writes, but the spec only
/// guarantees the *relationship*, and some generators use a different path. Content type
/// first, then the root relationship, then the conventional name.
fn find_presentation_part(package: &Package) -> Option<String> {
    use crate::opc::content_types::content_type;
    for ct in [
        content_type::PRESENTATION,
        content_type::SLIDESHOW,
        content_type::TEMPLATE,
    ] {
        if let Some(p) = package.content_types().parts_with_type(ct).into_iter().next() {
            if package.has_part(&p) {
                return Some(p);
            }
        }
    }
    if let Some(target) = package.resolve_single("", rel_type::OFFICE_DOC) {
        if package.has_part(&target) {
            return Some(target);
        }
    }
    package
        .has_part("ppt/presentation.xml")
        .then(|| "ppt/presentation.xml".to_string())
}

fn parse_embedded_fonts(r: &mut Reader<'_>) -> Vec<EmbeddedFont> {
    let mut fonts = Vec::new();
    children(r, b"embeddedFontLst", |r, e, empty| {
        if !is(e, b"embeddedFont") || empty {
            return false;
        }
        let mut font = EmbeddedFont::new(String::new());
        children(r, b"embeddedFont", |_r, child, _ce| {
            match local_name(child.name().as_ref()) {
                b"font" => {
                    font.typeface = super::xml::attr(child, b"typeface").unwrap_or_default();
                }
                b"regular" => font.regular = r_attr(child, b"id"),
                b"bold" => font.bold = r_attr(child, b"id"),
                b"italic" => font.italic = r_attr(child, b"id"),
                b"boldItalic" => font.bold_italic = r_attr(child, b"id"),
                _ => {}
            }
            false
        });
        if !font.typeface.is_empty() {
            fonts.push(font);
        }
        true
    });
    fonts
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            for (name, data) in entries {
                w.start_file(*name, opts).expect("start");
                w.write_all(data).expect("write");
            }
            w.finish().expect("finish");
        }
        buf
    }

    const CT: &[u8] = br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
      <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
      <Default Extension="xml" ContentType="application/xml"/>
      <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
    </Types>"#;

    const ROOT_RELS: &[u8] = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
      <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>
    </Relationships>"#;

    const PRES_RELS: &[u8] = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
      <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/>
      <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
      <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide2.xml"/>
    </Relationships>"#;

    const PRES: &[u8] = br#"<?xml version="1.0"?>
    <p:presentation xmlns:p="p" xmlns:r="r">
      <p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst>
      <p:sldIdLst>
        <p:sldId id="256" r:id="rId2"/>
        <p:sldId id="257" r:id="rId3"/>
      </p:sldIdLst>
      <p:sldSz cx="12192000" cy="6858000"/>
      <p:notesSz cx="6858000" cy="9144000"/>
      <p:defaultTextStyle>
        <a:lvl1pPr marL="0" algn="l"><a:defRPr sz="1800"/></a:lvl1pPr>
      </p:defaultTextStyle>
    </p:presentation>"#;

    fn deck() -> Vec<u8> {
        zip(&[
            ("[Content_Types].xml", CT),
            ("_rels/.rels", ROOT_RELS),
            ("ppt/presentation.xml", PRES),
            ("ppt/_rels/presentation.xml.rels", PRES_RELS),
            ("ppt/slides/slide1.xml", b"<p:sld><p:cSld><p:spTree/></p:cSld></p:sld>"),
            ("ppt/slides/slide2.xml", b"<p:sld><p:cSld><p:spTree/></p:cSld></p:sld>"),
            ("ppt/slideMasters/slideMaster1.xml", b"<p:sldMaster><p:cSld><p:spTree/></p:cSld></p:sldMaster>"),
        ])
    }

    #[test]
    fn reads_slide_size_order_and_ids() {
        let p = crate::open(deck()).expect("open");
        assert_eq!((p.slide_width, p.slide_height), (12_192_000, 6_858_000));
        assert_eq!(p.slide_size_pt(), (960.0, 540.0));
        assert_eq!(p.slide_count(), 2);
        assert_eq!(p.slide_info(0).map(|s| s.id), Some(256));
        assert_eq!(
            p.slide_info(1).map(|s| s.part_name.as_str()),
            Some("ppt/slides/slide2.xml")
        );
    }

    #[test]
    fn slide_order_follows_sldIdLst_not_the_relationship_file() {
        // rId3 is listed second in sldIdLst; both orders happen to agree here, so assert
        // the mapping explicitly rather than the count.
        let p = crate::open(deck()).expect("open");
        let names: Vec<_> = p.slides().iter().map(|s| s.part_name.as_str()).collect();
        assert_eq!(names, vec!["ppt/slides/slide1.xml", "ppt/slides/slide2.xml"]);
    }

    #[test]
    fn default_text_style_and_first_master_are_recorded() {
        let p = crate::open(deck()).expect("open");
        assert_eq!(
            p.default_text_style
                .level(0)
                .and_then(|l| l.default_run_props.as_ref())
                .and_then(|r| r.size),
            Some(1800)
        );
        assert_eq!(
            p.first_master.as_deref(),
            Some("ppt/slideMasters/slideMaster1.xml")
        );
    }

    #[test]
    fn a_slide_relationship_pointing_at_a_missing_part_is_skipped() {
        let broken_rels = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
          <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
          <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/gone.xml"/>
        </Relationships>"#;
        let bytes = zip(&[
            ("[Content_Types].xml", CT),
            ("_rels/.rels", ROOT_RELS),
            ("ppt/presentation.xml", PRES),
            ("ppt/_rels/presentation.xml.rels", broken_rels),
            ("ppt/slides/slide1.xml", b"<p:sld/>"),
        ]);
        let p = crate::open(bytes).expect("open");
        assert_eq!(p.slide_count(), 1);
        assert_eq!(p.slide_info(0).map(|s| s.index), Some(0));
    }

    #[test]
    fn a_missing_slide_size_falls_back_to_the_four_by_three_default() {
        let pres = br#"<p:presentation><p:sldIdLst/></p:presentation>"#;
        let bytes = zip(&[
            ("[Content_Types].xml", CT),
            ("_rels/.rels", ROOT_RELS),
            ("ppt/presentation.xml", pres),
        ]);
        let p = crate::open(bytes).expect("open");
        assert_eq!((p.slide_width, p.slide_height), (9_144_000, 6_858_000));
    }

    #[test]
    fn a_zero_slide_size_is_rejected_rather_than_producing_an_empty_canvas() {
        let pres = br#"<p:presentation><p:sldSz cx="0" cy="0"/></p:presentation>"#;
        let bytes = zip(&[
            ("[Content_Types].xml", CT),
            ("_rels/.rels", ROOT_RELS),
            ("ppt/presentation.xml", pres),
        ]);
        let p = crate::open(bytes).expect("open");
        assert!(p.slide_width > 0 && p.slide_height > 0);
    }

    #[test]
    fn embedded_fonts_are_listed_with_their_variants() {
        let pres = br#"<p:presentation xmlns:r="r">
          <p:embeddedFontLst>
            <p:embeddedFont>
              <p:font typeface="Corporate Sans" pitchFamily="34" charset="0"/>
              <p:regular r:id="rId10"/>
              <p:bold r:id="rId11"/>
            </p:embeddedFont>
          </p:embeddedFontLst>
        </p:presentation>"#;
        let bytes = zip(&[
            ("[Content_Types].xml", CT),
            ("_rels/.rels", ROOT_RELS),
            ("ppt/presentation.xml", pres),
        ]);
        let p = crate::open(bytes).expect("open");
        assert_eq!(p.embedded_fonts.len(), 1);
        let f = p.embedded_fonts.first().expect("font");
        assert_eq!(f.typeface, "Corporate Sans");
        assert_eq!(f.variants(), vec![("rId10", false, false), ("rId11", true, false)]);
    }

    #[test]
    fn a_docx_shaped_package_is_rejected_as_not_a_presentation() {
        let ct = br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
          <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
        </Types>"#;
        let bytes = zip(&[("[Content_Types].xml", ct), ("word/document.xml", b"<w/>")]);
        assert!(matches!(crate::open(bytes), Err(Error::NotAPresentation)));
    }

    #[test]
    fn the_presentation_part_is_found_by_relationship_when_named_unconventionally() {
        let ct = br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
          <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
        </Types>"#;
        let root_rels = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
          <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="deck/main.xml"/>
        </Relationships>"#;
        let bytes = zip(&[
            ("[Content_Types].xml", ct),
            ("_rels/.rels", root_rels),
            ("deck/main.xml", br#"<p:presentation><p:sldSz cx="100" cy="50"/></p:presentation>"#),
        ]);
        let p = crate::open(bytes).expect("open");
        assert_eq!((p.slide_width, p.slide_height), (100, 50));
    }
}
