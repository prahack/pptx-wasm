//! Slide, slide layout and slide master parts.
//!
//! All three share a `<p:cSld>` containing a `<p:spTree>` and an optional background;
//! they differ in what else they carry and in which part they point at next.

use crate::model::shape::{Background, ColorMap, Slide, SlideLayout, SlideMaster};
use crate::model::Presentation;
use crate::opc::rel_type;

use super::drawing::children;
use super::shapes::{parse_background, parse_shape_tree};
use super::text::parse_list_style;
use super::xml::{attr, local_name, Reader};

/// Reads the `<p:cSld>` common to all three part kinds.
fn parse_common_slide_data(
    r: &mut Reader<'_>,
) -> (crate::model::shape::ShapeTree, Background) {
    let mut tree = crate::model::shape::ShapeTree::default();
    let mut background = Background::Inherit;
    children(r, b"cSld", |r, e, empty| {
        match local_name(e.name().as_ref()) {
            b"bg" => {
                if empty {
                    return true;
                }
                background = parse_background(r);
                true
            }
            b"spTree" => {
                if empty {
                    return true;
                }
                tree = parse_shape_tree(r, b"spTree");
                true
            }
            _ => false,
        }
    });
    (tree, background)
}

/// Enters the document element and hands the reader over, so each part parser can share
/// the same "find the root, then walk its children" preamble.
fn with_root<T>(xml: &[u8], f: impl FnOnce(&mut Reader<'_>, &[u8]) -> T, default: T) -> T {
    let mut r = Reader::new(xml);
    let mut buf = Vec::new();
    loop {
        match r.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                return f(&mut r, &name);
            }
            Ok(quick_xml::events::Event::Eof) | Err(_) => return default,
            _ => {}
        }
        buf.clear();
    }
}

pub fn parse_slide(pres: &Presentation, part_name: &str, xml: &[u8]) -> Slide {
    let mut slide = Slide {
        part_name: part_name.to_string(),
        show_master_shapes: true,
        ..Default::default()
    };
    with_root(
        xml,
        |r, root| {
            children(r, root, |r, e, empty| {
                match local_name(e.name().as_ref()) {
                    b"cSld" => {
                        if empty {
                            return true;
                        }
                        let (tree, bg) = parse_common_slide_data(r);
                        slide.tree = tree;
                        slide.background = bg;
                        true
                    }
                    _ => false,
                }
            });
        },
        (),
    );
    // `showMasterSp` is an attribute of the root element, which `children` has already
    // stepped past, so it is read separately.
    slide.show_master_shapes = root_bool(xml, b"showMasterSp").unwrap_or(true);
    slide.layout_part = pres
        .package()
        .resolve_single(part_name, rel_type::SLIDE_LAYOUT);
    slide.notes = pres
        .package()
        .resolve_single(part_name, rel_type::NOTES_SLIDE)
        .and_then(|p| pres.package().part(&p))
        .map(|bytes| notes_text(&bytes));
    slide
}

pub fn parse_layout(pres: &Presentation, part_name: &str, xml: &[u8]) -> SlideLayout {
    let mut layout = SlideLayout {
        part_name: part_name.to_string(),
        show_master_shapes: true,
        ..Default::default()
    };
    with_root(
        xml,
        |r, root| {
            children(r, root, |r, e, empty| {
                if local_name(e.name().as_ref()) == b"cSld" {
                    if empty {
                        return true;
                    }
                    let (tree, bg) = parse_common_slide_data(r);
                    layout.tree = tree;
                    layout.background = bg;
                    return true;
                }
                false
            });
        },
        (),
    );
    layout.layout_type = root_attr(xml, b"type").unwrap_or_default();
    layout.show_master_shapes = root_bool(xml, b"showMasterSp").unwrap_or(true);
    layout.master_part = pres
        .package()
        .resolve_single(part_name, rel_type::SLIDE_MASTER);
    layout
}

pub fn parse_master(pres: &Presentation, part_name: &str, xml: &[u8]) -> SlideMaster {
    let mut master = SlideMaster {
        part_name: part_name.to_string(),
        ..Default::default()
    };
    with_root(
        xml,
        |r, root| {
            children(r, root, |r, e, empty| {
                match local_name(e.name().as_ref()) {
                    b"cSld" => {
                        if empty {
                            return true;
                        }
                        let (tree, bg) = parse_common_slide_data(r);
                        master.tree = tree;
                        master.background = bg;
                        true
                    }
                    b"clrMap" => {
                        master.color_map = parse_color_map(e);
                        !empty
                    }
                    b"txStyles" => {
                        if empty {
                            return true;
                        }
                        children(r, b"txStyles", |r, style, style_empty| {
                            if style_empty {
                                return true;
                            }
                            match local_name(style.name().as_ref()) {
                                b"titleStyle" => {
                                    master.title_style = parse_list_style(r, b"titleStyle");
                                    true
                                }
                                b"bodyStyle" => {
                                    master.body_style = parse_list_style(r, b"bodyStyle");
                                    true
                                }
                                b"otherStyle" => {
                                    master.other_style = parse_list_style(r, b"otherStyle");
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
        },
        (),
    );
    master.theme_part = pres.package().resolve_single(part_name, rel_type::THEME);
    master
}

fn parse_color_map(e: &quick_xml::events::BytesStart<'_>) -> ColorMap {
    use crate::model::color::SchemeColor as S;
    let slot = |name: &[u8], default: S| {
        attr(e, name)
            .as_deref()
            .and_then(S::parse)
            .unwrap_or(default)
    };
    ColorMap {
        background1: slot(b"bg1", S::Light1),
        text1: slot(b"tx1", S::Dark1),
        background2: slot(b"bg2", S::Light2),
        text2: slot(b"tx2", S::Dark2),
    }
}

/// Reads an attribute off the document element without a full parse.
fn root_attr(xml: &[u8], name: &[u8]) -> Option<String> {
    let mut r = Reader::new(xml);
    let mut buf = Vec::new();
    loop {
        match r.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(e)) | Ok(quick_xml::events::Event::Empty(e)) => {
                return attr(&e, name);
            }
            Ok(quick_xml::events::Event::Eof) | Err(_) => return None,
            _ => {}
        }
        buf.clear();
    }
}

fn root_bool(xml: &[u8], name: &[u8]) -> Option<bool> {
    match root_attr(xml, name)?.trim() {
        "1" | "true" => Some(true),
        "0" | "false" => Some(false),
        _ => None,
    }
}

/// Plain text of a notes slide's body placeholder.
fn notes_text(xml: &[u8]) -> String {
    let mut out = Vec::new();
    let mut r = Reader::new(xml);
    let mut buf = Vec::new();
    loop {
        match r.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(e)) => {
                if local_name(e.name().as_ref()) == b"t" {
                    out.push(super::xml::text_content(&mut r, b"t"));
                }
            }
            Ok(quick_xml::events::Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out.join("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::color::SchemeColor;

    /// A presentation over an in-memory package, so slide parsing can resolve
    /// relationships without touching the filesystem.
    fn presentation_with(parts: &[(&str, &[u8])]) -> Presentation {
        use std::io::{Cursor, Write};
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            w.start_file("[Content_Types].xml", opts).expect("start");
            w.write_all(br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>"#)
                .expect("write");
            for (name, data) in parts {
                w.start_file(*name, opts).expect("start");
                w.write_all(data).expect("write");
            }
            w.finish().expect("finish");
        }
        let pkg = crate::opc::Package::open(buf).expect("open");
        Presentation::new(pkg, 12_192_000, 6_858_000)
    }

    const SLIDE_XML: &[u8] = br#"<?xml version="1.0"?>
<p:sld xmlns:p="p" xmlns:a="a" showMasterSp="0">
  <p:cSld>
    <p:bg><p:bgPr><a:solidFill><a:srgbClr val="102030"/></a:solidFill></p:bgPr></p:bg>
    <p:spTree>
      <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
      <p:grpSpPr/>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="838200" y="365125"/><a:ext cx="10515600" cy="1325563"/></a:xfrm></p:spPr>
        <p:txBody><a:bodyPr/><a:p><a:r><a:t>Deck title</a:t></a:r></a:p></p:txBody>
      </p:sp>
    </p:spTree>
  </p:cSld>
</p:sld>"#;

    const SLIDE_RELS: &[u8] = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>
</Relationships>"#;

    #[test]
    fn slide_parses_tree_background_and_layout_link() {
        let pres = presentation_with(&[
            ("ppt/slides/slide1.xml", SLIDE_XML),
            ("ppt/slides/_rels/slide1.xml.rels", SLIDE_RELS),
        ]);
        let slide = parse_slide(&pres, "ppt/slides/slide1.xml", SLIDE_XML);
        assert_eq!(slide.tree.shapes.len(), 1);
        assert_eq!(
            slide.tree.shapes.first().map(|s| s.plain_text()),
            Some("Deck title".into())
        );
        assert!(matches!(slide.background, Background::Fill(_)));
        assert_eq!(
            slide.layout_part.as_deref(),
            Some("ppt/slideLayouts/slideLayout1.xml")
        );
        assert!(!slide.show_master_shapes, "showMasterSp=\"0\" must be honoured");
    }

    #[test]
    fn show_master_shapes_defaults_to_true_when_absent() {
        let pres = presentation_with(&[]);
        let xml = br#"<p:sld><p:cSld><p:spTree/></p:cSld></p:sld>"#;
        let slide = parse_slide(&pres, "ppt/slides/slide1.xml", xml);
        assert!(slide.show_master_shapes);
        assert_eq!(slide.layout_part, None, "a slide with no rels has no layout");
    }

    #[test]
    fn layout_records_its_type_and_master_link() {
        let rels = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
          <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/>
        </Relationships>"#;
        let xml = br#"<p:sldLayout type="title" preserve="1"><p:cSld name="Title Slide"><p:spTree/></p:cSld></p:sldLayout>"#;
        let pres = presentation_with(&[(
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
            rels,
        )]);
        let layout = parse_layout(&pres, "ppt/slideLayouts/slideLayout1.xml", xml);
        assert_eq!(layout.layout_type, "title");
        assert_eq!(
            layout.master_part.as_deref(),
            Some("ppt/slideMasters/slideMaster1.xml")
        );
    }

    #[test]
    fn master_parses_colour_map_and_text_styles() {
        let xml = br#"<p:sldMaster>
          <p:cSld><p:spTree/></p:cSld>
          <p:clrMap bg1="dk1" tx1="lt1" bg2="dk2" tx2="lt2" accent1="accent1"/>
          <p:txStyles>
            <p:titleStyle><a:lvl1pPr algn="ctr"><a:defRPr sz="4400"/></a:lvl1pPr></p:titleStyle>
            <p:bodyStyle><a:lvl1pPr marL="342900" indent="-342900"><a:defRPr sz="2800"/></a:lvl1pPr>
                         <a:lvl2pPr marL="742950"><a:defRPr sz="2400"/></a:lvl2pPr></p:bodyStyle>
            <p:otherStyle><a:lvl1pPr><a:defRPr sz="1800"/></a:lvl1pPr></p:otherStyle>
          </p:txStyles>
        </p:sldMaster>"#;
        let pres = presentation_with(&[]);
        let m = parse_master(&pres, "ppt/slideMasters/slideMaster1.xml", xml);

        assert_eq!(m.color_map.background1, SchemeColor::Dark1);
        assert_eq!(m.color_map.text1, SchemeColor::Light1);

        let title_l1 = m.title_style.level(0).expect("title level 1");
        assert_eq!(
            title_l1.default_run_props.as_ref().and_then(|r| r.size),
            Some(4400)
        );
        let body_l2 = m.body_style.level(1).expect("body level 2");
        assert_eq!(body_l2.margin_left, Some(742_950));
        assert_eq!(
            m.other_style
                .level(0)
                .and_then(|p| p.default_run_props.as_ref())
                .and_then(|r| r.size),
            Some(1800)
        );
    }

    #[test]
    fn a_default_colour_map_is_used_when_the_element_is_missing() {
        let pres = presentation_with(&[]);
        let m = parse_master(&pres, "m.xml", br#"<p:sldMaster><p:cSld><p:spTree/></p:cSld></p:sldMaster>"#);
        assert_eq!(m.color_map, ColorMap::default());
    }

    #[test]
    fn notes_text_is_attached_when_a_notes_slide_exists() {
        let notes = br#"<p:notes><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>Remember the demo</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:notes>"#;
        let rels = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
          <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide" Target="../notesSlides/notesSlide1.xml"/>
        </Relationships>"#;
        let pres = presentation_with(&[
            ("ppt/slides/_rels/slide1.xml.rels", rels),
            ("ppt/notesSlides/notesSlide1.xml", notes),
        ]);
        let slide = parse_slide(&pres, "ppt/slides/slide1.xml", SLIDE_XML);
        assert_eq!(slide.notes.as_deref(), Some("Remember the demo"));
    }

    #[test]
    fn a_slide_that_is_not_xml_yields_an_empty_slide_rather_than_an_error() {
        let pres = presentation_with(&[]);
        let slide = parse_slide(&pres, "ppt/slides/slide1.xml", b"\x00\x01\x02 not xml");
        assert!(slide.tree.is_empty());
        assert_eq!(slide.background, Background::Inherit);
    }
}
