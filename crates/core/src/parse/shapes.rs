//! `<p:spTree>` — the shape tree shared by slides, layouts and masters.

use quick_xml::events::BytesStart;

use crate::model::geometry::Geometry;
use crate::model::shape::{
    GraphicContent, Picture, Placeholder, PlaceholderType, Shape, ShapeKind, ShapeTree,
};

use super::drawing::{
    children, parse_effects, parse_fill, parse_geometry, parse_group_xfrm, parse_line,
    parse_style_ref, parse_xfrm,
};
use super::text::parse_text_body;
use super::xml::{attr, attr_bool, attr_percent, attr_u32, is, local_name, r_attr, Reader};

/// Parses a `<p:spTree>` or `<p:grpSp>` body into a list of shapes.
pub fn parse_shape_tree(r: &mut Reader<'_>, container: &[u8]) -> ShapeTree {
    let mut tree = ShapeTree::default();
    children(r, container, |r, e, empty| {
        match local_name(e.name().as_ref()) {
            b"grpSpPr" => {
                if empty {
                    return true;
                }
                // The tree root's own group properties carry the slide-wide transform.
                let (_fill, _line, _fx, xfrm) = parse_group_props(r, b"grpSpPr");
                tree.root_transform = xfrm;
                true
            }
            b"sp" | b"pic" | b"grpSp" | b"cxnSp" | b"graphicFrame" => {
                if empty {
                    return true;
                }
                if let Some(shape) = parse_shape(r, e) {
                    tree.shapes.push(shape);
                }
                true
            }
            _ => false,
        }
    });
    tree
}

/// Dispatches on the shape element kind. Returns `None` only for elements that carry no
/// renderable content at all.
fn parse_shape(r: &mut Reader<'_>, e: &BytesStart<'_>) -> Option<Shape> {
    match local_name(e.name().as_ref()) {
        b"sp" => Some(parse_sp(r, b"sp", ShapeKind::Auto)),
        b"cxnSp" => Some(parse_sp(r, b"cxnSp", ShapeKind::Connector)),
        b"pic" => Some(parse_pic(r)),
        b"grpSp" => Some(parse_group(r)),
        b"graphicFrame" => Some(parse_graphic_frame(r)),
        _ => None,
    }
}

/// `<p:sp>` and `<p:cxnSp>` have the same shape: non-visual props, shape props, style,
/// text body.
fn parse_sp(r: &mut Reader<'_>, container: &[u8], kind: ShapeKind) -> Shape {
    let mut shape = Shape {
        kind,
        ..Default::default()
    };
    children(r, container, |r, e, empty| {
        match local_name(e.name().as_ref()) {
            b"nvSpPr" | b"nvCxnSpPr" => {
                if empty {
                    return true;
                }
                apply_non_visual(r, local_name(e.name().as_ref()), &mut shape);
                true
            }
            b"spPr" => {
                if empty {
                    return true;
                }
                apply_shape_props(r, b"spPr", &mut shape);
                true
            }
            b"style" => {
                if empty {
                    return true;
                }
                shape.style_ref = Some(parse_style_ref(r));
                true
            }
            b"txBody" => {
                if empty {
                    return true;
                }
                shape.text = Some(parse_text_body(r, b"txBody"));
                true
            }
            _ => false,
        }
    });
    shape
}

fn parse_pic(r: &mut Reader<'_>) -> Shape {
    let mut shape = Shape {
        kind: ShapeKind::Picture(Box::new(Picture {
            alpha: 1.0,
            ..Default::default()
        })),
        ..Default::default()
    };
    let mut picture = Picture {
        alpha: 1.0,
        ..Default::default()
    };
    children(r, b"pic", |r, e, empty| {
        match local_name(e.name().as_ref()) {
            b"nvPicPr" => {
                if empty {
                    return true;
                }
                apply_non_visual(r, b"nvPicPr", &mut shape);
                true
            }
            b"blipFill" => {
                if empty {
                    return true;
                }
                // Reuse the fill parser, then lift the image out of it: a picture's crop
                // and transparency live in exactly the same elements as a blip fill's.
                if let Some(crate::model::fill::Fill::Blip(b)) = parse_fill(r, e, empty) {
                    picture.embed_id = b.embed_id;
                    picture.src_rect = b.src_rect;
                    picture.alpha = b.alpha;
                }
                true
            }
            b"spPr" => {
                if empty {
                    return true;
                }
                apply_shape_props(r, b"spPr", &mut shape);
                true
            }
            b"style" => {
                if empty {
                    return true;
                }
                shape.style_ref = Some(parse_style_ref(r));
                true
            }
            _ => false,
        }
    });
    picture.alt_text = shape.description.clone();
    shape.kind = ShapeKind::Picture(Box::new(picture));
    shape
}

fn parse_group(r: &mut Reader<'_>) -> Shape {
    let mut shape = Shape {
        kind: ShapeKind::Group(Vec::new()),
        ..Default::default()
    };
    let mut kids: Vec<Shape> = Vec::new();
    children(r, b"grpSp", |r, e, empty| {
        match local_name(e.name().as_ref()) {
            b"nvGrpSpPr" => {
                if empty {
                    return true;
                }
                apply_non_visual(r, b"nvGrpSpPr", &mut shape);
                true
            }
            b"grpSpPr" => {
                if empty {
                    return true;
                }
                let (fill, line, fx, xfrm) = parse_group_props(r, b"grpSpPr");
                shape.fill = fill;
                if let Some(l) = line {
                    shape.line = l;
                }
                if let Some(f) = fx {
                    shape.effects = f;
                }
                if let Some(g) = xfrm {
                    shape.transform = g.xfrm;
                    shape.group_transform = Some(g);
                }
                true
            }
            b"sp" | b"pic" | b"grpSp" | b"cxnSp" | b"graphicFrame" => {
                if empty {
                    return true;
                }
                if let Some(child) = parse_shape(r, e) {
                    kids.push(child);
                }
                true
            }
            _ => false,
        }
    });
    shape.kind = ShapeKind::Group(kids);
    shape
}

fn parse_graphic_frame(r: &mut Reader<'_>) -> Shape {
    let mut shape = Shape {
        kind: ShapeKind::Graphic(Box::new(GraphicContent::Unsupported {
            kind: String::new(),
            fallback_image: None,
        })),
        ..Default::default()
    };
    let mut content: Option<GraphicContent> = None;
    children(r, b"graphicFrame", |r, e, empty| {
        match local_name(e.name().as_ref()) {
            b"nvGraphicFramePr" => {
                if empty {
                    return true;
                }
                apply_non_visual(r, b"nvGraphicFramePr", &mut shape);
                true
            }
            b"xfrm" => {
                shape.transform = parse_xfrm(r, e, empty);
                !empty
            }
            b"graphic" => {
                if empty {
                    return true;
                }
                content = parse_graphic(r);
                true
            }
            _ => false,
        }
    });
    shape.kind = ShapeKind::Graphic(Box::new(content.unwrap_or(GraphicContent::Unsupported {
        kind: String::new(),
        fallback_image: None,
    })));
    shape
}

fn parse_graphic(r: &mut Reader<'_>) -> Option<GraphicContent> {
    let mut out = None;
    children(r, b"graphic", |r, e, empty| {
        if !is(e, b"graphicData") {
            return false;
        }
        let uri = attr(e, b"uri").unwrap_or_default();
        if empty {
            // `<a:graphicData uri="..."/>` with no payload: the uri is all we get, and
            // it still tells the viewer (and the user) what could not be rendered.
            out = Some(GraphicContent::Unsupported {
                kind: uri,
                fallback_image: None,
            });
            return true;
        }
        children(
            r,
            b"graphicData",
            |r, inner, inner_empty| match local_name(inner.name().as_ref()) {
                b"tbl" if !inner_empty => {
                    out = Some(GraphicContent::Table(Box::new(super::table::parse_table(
                        r,
                    ))));
                    true
                }
                b"chart" => {
                    if let Some(id) = r_attr(inner, b"id") {
                        out = Some(GraphicContent::Chart(id));
                    }
                    false
                }
                _ => false,
            },
        );
        if out.is_none() {
            out = Some(GraphicContent::Unsupported {
                kind: uri,
                fallback_image: None,
            });
        }
        true
    });
    out
}

/// `<p:nvSpPr>` and friends: id, name, hidden flag, description, placeholder binding.
fn apply_non_visual(r: &mut Reader<'_>, container: &[u8], shape: &mut Shape) {
    children(r, container, |r, e, empty| {
        match local_name(e.name().as_ref()) {
            b"cNvPr" => {
                shape.id = attr_u32(e, b"id").unwrap_or(0);
                shape.name = attr(e, b"name").unwrap_or_default();
                shape.description = attr(e, b"descr").unwrap_or_default();
                if attr_bool(e, b"hidden").unwrap_or(false) {
                    shape.hidden = true;
                }
                !empty
            }
            b"nvPr" => {
                if empty {
                    return true;
                }
                children(r, b"nvPr", |_r, inner, _ie| {
                    if is(inner, b"ph") {
                        shape.placeholder = Some(Placeholder {
                            kind: attr(inner, b"type")
                                .as_deref()
                                .map(PlaceholderType::parse)
                                .unwrap_or(PlaceholderType::Body),
                            index: attr_u32(inner, b"idx"),
                            has_custom_prompt: attr_bool(inner, b"hasCustomPrompt")
                                .unwrap_or(false),
                        });
                    }
                    false
                });
                true
            }
            _ => false,
        }
    });
}

/// `<p:spPr>` — transform, geometry, fill, outline, effects.
fn apply_shape_props(r: &mut Reader<'_>, container: &[u8], shape: &mut Shape) {
    children(r, container, |r, e, empty| {
        let name = local_name(e.name().as_ref()).to_vec();
        if let Some(g) = parse_geometry(r, e, empty) {
            shape.geometry = g;
            return !empty;
        }
        if let Some(f) = parse_fill(r, e, empty) {
            shape.fill = f;
            return !empty;
        }
        match name.as_slice() {
            b"xfrm" => {
                shape.transform = parse_xfrm(r, e, empty);
                !empty
            }
            b"ln" => {
                shape.line = parse_line(r, e, empty);
                !empty
            }
            b"effectLst" => {
                shape.effects = parse_effects(r, empty);
                !empty
            }
            _ => false,
        }
    });
}

/// `<p:grpSpPr>` — like `spPr` but its transform carries a child coordinate space.
fn parse_group_props(
    r: &mut Reader<'_>,
    container: &[u8],
) -> (
    crate::model::fill::Fill,
    Option<crate::model::fill::Line>,
    Option<crate::model::fill::Effects>,
    Option<crate::model::geometry::GroupTransform>,
) {
    let mut fill = crate::model::fill::Fill::Inherit;
    let mut line = None;
    let mut effects = None;
    let mut xfrm = None;
    children(r, container, |r, e, empty| {
        let name = local_name(e.name().as_ref()).to_vec();
        if let Some(f) = parse_fill(r, e, empty) {
            fill = f;
            return !empty;
        }
        match name.as_slice() {
            b"xfrm" => {
                xfrm = Some(parse_group_xfrm(r, e, empty));
                !empty
            }
            b"ln" => {
                line = Some(parse_line(r, e, empty));
                !empty
            }
            b"effectLst" => {
                effects = Some(parse_effects(r, empty));
                !empty
            }
            _ => false,
        }
    });
    (fill, line, effects, xfrm)
}

/// `<p:bg>` — a slide, layout or master background.
pub fn parse_background(r: &mut Reader<'_>) -> crate::model::shape::Background {
    use crate::model::shape::Background;
    let mut bg = Background::Inherit;
    children(r, b"bg", |r, e, empty| {
        match local_name(e.name().as_ref()) {
            b"bgPr" => {
                if empty {
                    return true;
                }
                let mut fill = crate::model::fill::Fill::Inherit;
                children(r, b"bgPr", |r, inner, inner_empty| {
                    if let Some(f) = parse_fill(r, inner, inner_empty) {
                        fill = f;
                        return !inner_empty;
                    }
                    false
                });
                bg = Background::Fill(fill);
                true
            }
            b"bgRef" => {
                let idx = attr_u32(e, b"idx").unwrap_or(0);
                let color = if empty {
                    None
                } else {
                    super::drawing::parse_color_container(r, b"bgRef")
                };
                bg = Background::Reference {
                    idx,
                    color: color.unwrap_or_else(|| {
                        crate::model::color::ColorRef::scheme(
                            crate::model::color::SchemeColor::Background1,
                        )
                    }),
                };
                !empty
            }
            _ => false,
        }
    });
    bg
}

/// Picture-specific alpha, used when a `<a:blip>` sits outside a `blipFill`.
pub fn blip_alpha(e: &BytesStart<'_>) -> f32 {
    attr_percent(e, b"amt").unwrap_or(1.0)
}

/// True when a shape has no geometry of its own and should fall back to its box.
pub fn needs_default_geometry(shape: &Shape) -> bool {
    matches!(shape.geometry, Geometry::None) && matches!(shape.kind, ShapeKind::Auto)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::fill::Fill;
    use quick_xml::events::Event;

    fn tree(xml: &str) -> ShapeTree {
        let mut r = Reader::new(xml.as_bytes());
        let mut buf = Vec::new();
        loop {
            match r.read_event_into(&mut buf) {
                Ok(Event::Start(e)) => {
                    let name = local_name(e.name().as_ref()).to_vec();
                    return parse_shape_tree(&mut r, &name);
                }
                Ok(Event::Eof) => panic!("no element in {xml}"),
                _ => {}
            }
        }
    }

    const RECT: &str = r#"<p:spTree>
        <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
        <p:grpSpPr/>
        <p:sp>
          <p:nvSpPr>
            <p:cNvPr id="2" name="Rectangle 1" descr="a red box"/>
            <p:cNvSpPr/>
            <p:nvPr/>
          </p:nvSpPr>
          <p:spPr>
            <a:xfrm><a:off x="914400" y="457200"/><a:ext cx="1828800" cy="914400"/></a:xfrm>
            <a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
            <a:solidFill><a:srgbClr val="FF0000"/></a:solidFill>
            <a:ln w="12700"><a:solidFill><a:srgbClr val="000000"/></a:solidFill></a:ln>
          </p:spPr>
          <p:txBody><a:bodyPr/><a:p><a:r><a:t>Hi</a:t></a:r></a:p></p:txBody>
        </p:sp>
      </p:spTree>"#;

    #[test]
    fn parses_an_autoshape_with_geometry_fill_line_and_text() {
        let t = tree(RECT);
        assert_eq!(t.shapes.len(), 1);
        let s = t.shapes.first().expect("shape");
        assert_eq!(s.id, 2);
        assert_eq!(s.name, "Rectangle 1");
        assert_eq!(s.description, "a red box");
        assert_eq!(s.transform.offset_x, 914_400);
        assert_eq!(s.transform.extent_x, 1_828_800);
        assert_eq!(s.geometry.preset_name(), Some("rect"));
        assert!(matches!(s.fill, Fill::Solid(_)));
        assert_eq!(s.line.width, Some(12_700));
        assert_eq!(s.text.as_ref().map(|t| t.plain_text()), Some("Hi".into()));
    }

    #[test]
    fn the_tree_root_group_props_do_not_become_a_shape() {
        let t = tree(RECT);
        assert_eq!(
            t.shapes.len(),
            1,
            "the root <p:nvGrpSpPr> must not be a shape"
        );
    }

    #[test]
    fn placeholders_are_captured_with_type_and_index() {
        let t = tree(
            r#"<p:spTree><p:sp>
                 <p:nvSpPr><p:cNvPr id="2" name="Title"/><p:cNvSpPr/>
                   <p:nvPr><p:ph type="ctrTitle" idx="4"/></p:nvPr>
                 </p:nvSpPr>
                 <p:spPr/>
               </p:sp></p:spTree>"#,
        );
        let ph = t
            .shapes
            .first()
            .and_then(|s| s.placeholder.clone())
            .expect("ph");
        assert_eq!(ph.kind, PlaceholderType::CenteredTitle);
        assert_eq!(ph.index, Some(4));
    }

    #[test]
    fn a_shape_with_no_xfrm_is_marked_unspecified_so_it_inherits() {
        let t = tree(
            r#"<p:spTree><p:sp>
                 <p:nvSpPr><p:cNvPr id="2" name="Title"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
                 <p:spPr/>
               </p:sp></p:spTree>"#,
        );
        assert!(!t.shapes.first().expect("shape").transform.specified);
    }

    #[test]
    fn pictures_carry_their_relationship_crop_and_alpha() {
        let t = tree(
            r#"<p:spTree><p:pic>
                 <p:nvPicPr><p:cNvPr id="5" name="Photo" descr="alt text"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>
                 <p:blipFill>
                   <a:blip r:embed="rId2"><a:alphaModFix amt="80000"/></a:blip>
                   <a:srcRect l="5000" t="10000" r="0" b="0"/>
                   <a:stretch><a:fillRect/></a:stretch>
                 </p:blipFill>
                 <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></a:xfrm></p:spPr>
               </p:pic></p:spTree>"#,
        );
        let s = t.shapes.first().expect("shape");
        match &s.kind {
            ShapeKind::Picture(p) => {
                assert_eq!(p.embed_id.as_deref(), Some("rId2"));
                assert_eq!(p.src_rect[0], 0.05);
                assert_eq!(p.src_rect[1], 0.1);
                assert!((p.alpha - 0.8).abs() < 1e-6);
                assert_eq!(p.alt_text, "alt text");
            }
            other => panic!("expected picture, got {other:?}"),
        }
    }

    #[test]
    fn groups_nest_and_keep_their_child_coordinate_space() {
        let t = tree(
            r#"<p:spTree><p:grpSp>
                 <p:nvGrpSpPr><p:cNvPr id="9" name="Group"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
                 <p:grpSpPr>
                   <a:xfrm rot="1200000">
                     <a:off x="100" y="200"/><a:ext cx="1000" cy="500"/>
                     <a:chOff x="0" y="0"/><a:chExt cx="2000" cy="1000"/>
                   </a:xfrm>
                 </p:grpSpPr>
                 <p:sp><p:nvSpPr><p:cNvPr id="10" name="Inner"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
                   <p:spPr><a:prstGeom prst="ellipse"><a:avLst/></a:prstGeom></p:spPr>
                 </p:sp>
               </p:grpSp></p:spTree>"#,
        );
        let g = t.shapes.first().expect("group");
        assert_eq!(g.id, 9);
        assert_eq!(g.transform.rotation, 1_200_000);
        let gt = g.group_transform.expect("group transform");
        assert_eq!(gt.child_extent_x, 2000);
        assert_eq!(gt.child_scale(), (0.5, 0.5));
        match &g.kind {
            ShapeKind::Group(kids) => {
                assert_eq!(kids.len(), 1);
                assert_eq!(kids.first().map(|k| k.id), Some(10));
            }
            other => panic!("expected group, got {other:?}"),
        }
    }

    #[test]
    fn connectors_parse_as_their_own_kind() {
        let t = tree(
            r#"<p:spTree><p:cxnSp>
                 <p:nvCxnSpPr><p:cNvPr id="3" name="Connector"/><p:cNvCxnSpPr/><p:nvPr/></p:nvCxnSpPr>
                 <p:spPr><a:prstGeom prst="straightConnector1"><a:avLst/></a:prstGeom></p:spPr>
               </p:cxnSp></p:spTree>"#,
        );
        let s = t.shapes.first().expect("shape");
        assert!(matches!(s.kind, ShapeKind::Connector));
        assert!(s.geometry.is_line_like());
    }

    #[test]
    fn hidden_shapes_are_flagged_rather_than_dropped() {
        let t = tree(
            r#"<p:spTree><p:sp>
                 <p:nvSpPr><p:cNvPr id="2" name="Ghost" hidden="1"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
                 <p:spPr/>
               </p:sp></p:spTree>"#,
        );
        assert!(t.shapes.first().expect("shape").hidden);
    }

    #[test]
    fn a_chart_graphic_frame_records_its_relationship() {
        let t = tree(
            r#"<p:spTree><p:graphicFrame>
                 <p:nvGraphicFramePr><p:cNvPr id="4" name="Chart"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
                 <p:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></p:xfrm>
                 <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart">
                   <c:chart r:id="rId3"/>
                 </a:graphicData></a:graphic>
               </p:graphicFrame></p:spTree>"#,
        );
        match &t.shapes.first().expect("shape").kind {
            ShapeKind::Graphic(g) => match g.as_ref() {
                GraphicContent::Chart(id) => assert_eq!(id, "rId3"),
                other => panic!("expected chart, got {other:?}"),
            },
            other => panic!("expected graphic frame, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_graphic_frame_keeps_its_uri_and_does_not_panic() {
        let t = tree(
            r#"<p:spTree><p:graphicFrame>
                 <p:nvGraphicFramePr><p:cNvPr id="4" name="SmartArt"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
                 <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/diagram"/></a:graphic>
               </p:graphicFrame></p:spTree>"#,
        );
        match &t.shapes.first().expect("shape").kind {
            ShapeKind::Graphic(g) => match g.as_ref() {
                GraphicContent::Unsupported { kind, .. } => assert!(kind.contains("diagram")),
                other => panic!("expected unsupported, got {other:?}"),
            },
            other => panic!("expected graphic frame, got {other:?}"),
        }
    }

    #[test]
    fn truncated_shape_tree_keeps_the_shapes_it_finished() {
        let t = tree(
            r#"<p:spTree>
                 <p:sp><p:nvSpPr><p:cNvPr id="2" name="Done"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr/></p:sp>
                 <p:sp><p:nvSpPr><p:cNvPr id="3" name="Trunc"#,
        );
        assert!(!t.shapes.is_empty());
        assert_eq!(t.shapes.first().map(|s| s.id), Some(2));
    }

    #[test]
    fn background_fill_and_reference_forms_both_parse() {
        fn bg(xml: &str) -> crate::model::shape::Background {
            let mut r = Reader::new(xml.as_bytes());
            let mut buf = Vec::new();
            loop {
                match r.read_event_into(&mut buf) {
                    Ok(Event::Start(_)) => return parse_background(&mut r),
                    Ok(Event::Eof) => panic!("no element"),
                    _ => {}
                }
            }
        }
        match bg(
            r#"<p:bg><p:bgPr><a:solidFill><a:srgbClr val="123456"/></a:solidFill></p:bgPr></p:bg>"#,
        ) {
            crate::model::shape::Background::Fill(Fill::Solid(_)) => {}
            other => panic!("expected a solid background fill, got {other:?}"),
        }
        match bg(r#"<p:bg><p:bgRef idx="1001"><a:schemeClr val="bg1"/></p:bgRef></p:bg>"#) {
            crate::model::shape::Background::Reference { idx, .. } => assert_eq!(idx, 1001),
            other => panic!("expected a background reference, got {other:?}"),
        }
    }
}
