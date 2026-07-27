//! `<p:txBody>` / `<a:txBody>` — body properties, list styles, paragraphs and runs.

use quick_xml::events::BytesStart;

use crate::model::text::AutoNumScheme;
use crate::model::text::{
    Autofit, BodyProps, BulletKind, Capitalization, ListStyle, Paragraph, ParagraphProps, Run,
    RunProps, Spacing, TextAlign, TextBody, TextDirection, UnderlineStyle, VerticalAnchor,
};

use super::drawing::{children, parse_color_container, parse_fill, parse_line};
use super::xml::{
    attr, attr_bool, attr_f32, attr_i32, attr_i64, attr_percent, attr_u32, is, local_name,
    text_content, Reader,
};

/// Parses a `<p:txBody>`, `<a:txBody>` or `<c:txPr>` element.
pub fn parse_text_body(r: &mut Reader<'_>, container: &[u8]) -> TextBody {
    let mut body = TextBody::default();
    children(r, container, |r, e, empty| {
        match local_name(e.name().as_ref()) {
            b"bodyPr" => {
                body.body = parse_body_props(r, e, empty);
                !empty
            }
            b"lstStyle" => {
                if empty {
                    return true;
                }
                body.list_style = parse_list_style(r, b"lstStyle");
                true
            }
            b"p" => {
                body.paragraphs.push(parse_paragraph(r, empty));
                !empty
            }
            _ => false,
        }
    });
    // A text body with no paragraphs still occupies its box; give it one so vertical
    // anchoring and autofit have something to measure.
    if body.paragraphs.is_empty() {
        body.paragraphs.push(Paragraph::default());
    }
    body
}

pub fn parse_body_props(r: &mut Reader<'_>, e: &BytesStart<'_>, empty: bool) -> BodyProps {
    let mut bp = BodyProps {
        anchor: attr(e, b"anchor").as_deref().and_then(VerticalAnchor::parse),
        anchor_center: attr_bool(e, b"anchorCtr"),
        left_inset: attr_i64(e, b"lIns"),
        top_inset: attr_i64(e, b"tIns"),
        right_inset: attr_i64(e, b"rIns"),
        bottom_inset: attr_i64(e, b"bIns"),
        wrap: attr(e, b"wrap").as_deref().map(|w| w != "none"),
        direction: attr(e, b"vert").as_deref().and_then(TextDirection::parse),
        rotation: attr_i32(e, b"rot"),
        columns: attr_u32(e, b"numCol"),
        column_gap: attr_i64(e, b"spcCol"),
        upright: attr_bool(e, b"upright"),
        autofit: None,
    };
    if empty {
        return bp;
    }
    children(r, b"bodyPr", |_r, child, _ce| {
        match local_name(child.name().as_ref()) {
            b"noAutofit" => bp.autofit = Some(Autofit::None),
            b"spAutoFit" => bp.autofit = Some(Autofit::ResizeShape),
            b"normAutofit" => {
                bp.autofit = Some(Autofit::Shrink {
                    // Absent means 100% / 0%, i.e. no shrink applied yet.
                    font_scale: attr_percent(child, b"fontScale").unwrap_or(1.0),
                    line_space_reduction: attr_percent(child, b"lnSpcReduction").unwrap_or(0.0),
                });
            }
            _ => {}
        }
        false
    });
    bp
}

/// Parses a `<a:lstStyle>` or `<p:titleStyle>`-shaped element: nine `<a:lvlNpPr>` children.
pub fn parse_list_style(r: &mut Reader<'_>, container: &[u8]) -> ListStyle {
    let mut ls = ListStyle::default();
    children(r, container, |r, e, empty| {
        let name = local_name(e.name().as_ref()).to_vec();
        let Some(level) = level_from_element(&name) else {
            // `<a:defPPr>` inside a list style applies to every level.
            if name == b"defPPr" {
                let props = parse_paragraph_props(r, e, empty);
                for lvl in 0..9u8 {
                    let mut p = props.clone();
                    p.level = lvl;
                    ls.set_level(lvl, p);
                }
                return !empty;
            }
            return false;
        };
        let mut props = parse_paragraph_props(r, e, empty);
        props.level = level;
        ls.set_level(level, props);
        !empty
    });
    ls
}

/// `lvl1pPr` → 0, `lvl9pPr` → 8.
fn level_from_element(name: &[u8]) -> Option<u8> {
    let s = std::str::from_utf8(name).ok()?;
    let digits = s.strip_prefix("lvl")?.strip_suffix("pPr")?;
    let n: u8 = digits.parse().ok()?;
    n.checked_sub(1).filter(|v| *v < 9)
}

pub fn parse_paragraph(r: &mut Reader<'_>, empty: bool) -> Paragraph {
    let mut p = Paragraph::default();
    if empty {
        return p;
    }
    children(r, b"p", |r, child, child_empty| {
        match local_name(child.name().as_ref()) {
            b"pPr" => {
                p.props = parse_paragraph_props(r, child, child_empty);
                !child_empty
            }
            b"r" => {
                if child_empty {
                    return true;
                }
                p.runs.push(parse_run(r));
                true
            }
            b"br" => {
                let mut props = RunProps::default();
                if !child_empty {
                    children(r, b"br", |r, rp, rp_empty| {
                        if is(rp, b"rPr") {
                            props = parse_run_props(r, rp, rp_empty);
                            return !rp_empty;
                        }
                        false
                    });
                }
                p.runs.push(Run::Break { props });
                !child_empty
            }
            b"fld" => {
                if child_empty {
                    return true;
                }
                let kind = attr(child, b"type").unwrap_or_default();
                let (cached, props) = parse_field(r);
                p.runs.push(Run::Field {
                    kind,
                    cached,
                    props,
                });
                true
            }
            b"endParaRPr" => {
                p.end_props = Some(parse_run_props(r, child, child_empty));
                !child_empty
            }
            _ => false,
        }
    });
    p
}

fn parse_run(r: &mut Reader<'_>) -> Run {
    let mut props = RunProps::default();
    let mut text = String::new();
    children(r, b"r", |r, child, child_empty| {
        match local_name(child.name().as_ref()) {
            b"rPr" => {
                props = parse_run_props(r, child, child_empty);
                !child_empty
            }
            b"t" => {
                if child_empty {
                    return true;
                }
                text.push_str(&text_content(r, b"t"));
                true
            }
            _ => false,
        }
    });
    Run::Text { text, props }
}

fn parse_field(r: &mut Reader<'_>) -> (String, RunProps) {
    let mut props = RunProps::default();
    let mut cached = String::new();
    children(r, b"fld", |r, child, child_empty| {
        match local_name(child.name().as_ref()) {
            b"rPr" => {
                props = parse_run_props(r, child, child_empty);
                !child_empty
            }
            b"t" => {
                if child_empty {
                    return true;
                }
                cached.push_str(&text_content(r, b"t"));
                true
            }
            _ => false,
        }
    });
    (cached, props)
}

pub fn parse_paragraph_props(
    r: &mut Reader<'_>,
    e: &BytesStart<'_>,
    empty: bool,
) -> ParagraphProps {
    let mut p = ParagraphProps {
        level: attr_u32(e, b"lvl").unwrap_or(0).min(8) as u8,
        align: attr(e, b"algn").as_deref().and_then(TextAlign::parse),
        margin_left: attr_i64(e, b"marL"),
        margin_right: attr_i64(e, b"marR"),
        indent: attr_i64(e, b"indent"),
        default_tab_size: attr_i64(e, b"defTabSz"),
        rtl: attr_bool(e, b"rtl"),
        ..Default::default()
    };
    if empty {
        return p;
    }
    let container = local_name(e.name().as_ref()).to_vec();
    children(r, &container, |r, child, child_empty| {
        let name = local_name(child.name().as_ref()).to_vec();
        match name.as_slice() {
            b"lnSpc" => {
                p.line_spacing = parse_spacing(r, b"lnSpc", child_empty);
                !child_empty
            }
            b"spcBef" => {
                p.space_before = parse_spacing(r, b"spcBef", child_empty);
                !child_empty
            }
            b"spcAft" => {
                p.space_after = parse_spacing(r, b"spcAft", child_empty);
                !child_empty
            }
            b"buNone" => {
                p.bullet.kind = Some(BulletKind::None);
                false
            }
            b"buChar" => {
                p.bullet.kind = Some(BulletKind::Char(
                    attr(child, b"char").unwrap_or_else(|| "\u{2022}".into()),
                ));
                false
            }
            b"buAutoNum" => {
                let scheme = attr(child, b"type")
                    .as_deref()
                    .and_then(AutoNumScheme::parse)
                    .unwrap_or(AutoNumScheme::ArabicPeriod);
                p.bullet.kind = Some(BulletKind::AutoNum {
                    scheme,
                    start_at: attr_u32(child, b"startAt").unwrap_or(1),
                });
                false
            }
            b"buBlip" => {
                if child_empty {
                    return true;
                }
                let mut embed = None;
                children(r, b"buBlip", |_r, blip, _be| {
                    if is(blip, b"blip") {
                        embed = super::xml::r_attr(blip, b"embed");
                    }
                    false
                });
                if let Some(id) = embed {
                    p.bullet.kind = Some(BulletKind::Image(id));
                }
                true
            }
            b"buFont" => {
                p.bullet.font = attr(child, b"typeface");
                false
            }
            b"buSzPct" => {
                p.bullet.size_percent = attr_percent(child, b"val");
                false
            }
            b"buSzPts" => {
                p.bullet.size_points = attr_f32(child, b"val").map(|v| v / 100.0);
                false
            }
            b"buClrTx" => {
                p.bullet.follow_text_color = true;
                false
            }
            b"buClr" => {
                if child_empty {
                    return true;
                }
                p.bullet.color = parse_color_container(r, b"buClr");
                true
            }
            b"defRPr" => {
                p.default_run_props = Some(parse_run_props(r, child, child_empty));
                !child_empty
            }
            b"tabLst" => {
                if child_empty {
                    return true;
                }
                children(r, b"tabLst", |_r, tab, _te| {
                    if is(tab, b"tab") {
                        if let Some(pos) = attr_i64(tab, b"pos") {
                            p.tab_stops.push(pos);
                        }
                    }
                    false
                });
                true
            }
            _ => false,
        }
    });
    p
}

/// `<a:lnSpc>`/`<a:spcBef>`/`<a:spcAft>` wrap either `<a:spcPct>` or `<a:spcPts>`.
fn parse_spacing(r: &mut Reader<'_>, container: &[u8], empty: bool) -> Option<Spacing> {
    if empty {
        return None;
    }
    let mut out = None;
    children(r, container, |_r, child, _ce| {
        match local_name(child.name().as_ref()) {
            b"spcPct" => out = attr_percent(child, b"val").map(Spacing::Percent),
            // `spcPts` is in hundredths of a point.
            b"spcPts" => out = attr_f32(child, b"val").map(|v| Spacing::Points(v / 100.0)),
            _ => {}
        }
        false
    });
    out
}

pub fn parse_run_props(r: &mut Reader<'_>, e: &BytesStart<'_>, empty: bool) -> RunProps {
    let mut rp = RunProps {
        size: attr_i32(e, b"sz"),
        bold: attr_bool(e, b"b"),
        italic: attr_bool(e, b"i"),
        underline: attr(e, b"u").as_deref().and_then(UnderlineStyle::parse),
        strikethrough: attr(e, b"strike").as_deref().map(|s| s != "noStrike"),
        letter_spacing: attr_i32(e, b"spc"),
        caps: attr(e, b"cap").as_deref().map(|c| match c {
            "all" => Capitalization::All,
            "small" => Capitalization::Small,
            _ => Capitalization::None,
        }),
        // `baseline` is a percentage of the font size in thousandths of a percent.
        baseline: attr_percent(e, b"baseline"),
        language: attr(e, b"lang"),
        ..Default::default()
    };
    if empty {
        return rp;
    }
    let container = local_name(e.name().as_ref()).to_vec();
    children(r, &container, |r, child, child_empty| {
        let name = local_name(child.name().as_ref()).to_vec();
        if let Some(f) = parse_fill(r, child, child_empty) {
            rp.fill = f;
            return !child_empty;
        }
        match name.as_slice() {
            b"latin" => {
                rp.latin_font = typeface(child);
                false
            }
            b"ea" => {
                rp.ea_font = typeface(child);
                false
            }
            b"cs" => {
                rp.cs_font = typeface(child);
                false
            }
            b"sym" => {
                rp.symbol_font = typeface(child);
                false
            }
            b"ln" => {
                rp.outline = Some(parse_line(r, child, child_empty));
                !child_empty
            }
            b"highlight" => {
                if child_empty {
                    return true;
                }
                rp.highlight = parse_color_container(r, b"highlight");
                true
            }
            b"uFill" => {
                if child_empty {
                    return true;
                }
                // The underline's own colour lives inside a nested fill.
                let mut inner = None;
                children(r, b"uFill", |r, f, fe| {
                    if local_name(f.name().as_ref()) == b"solidFill" && !fe {
                        inner = parse_color_container(r, b"solidFill");
                        return true;
                    }
                    false
                });
                rp.underline_color = inner;
                true
            }
            b"hlinkClick" => {
                rp.hyperlink = super::xml::r_attr(child, b"id");
                !child_empty
            }
            _ => false,
        }
    });
    rp
}

/// `<a:latin typeface="+mn-lt"/>` — a `+mj-`/`+mn-` prefix means "the theme's major or
/// minor font", resolved later against the theme.
fn typeface(e: &BytesStart<'_>) -> Option<String> {
    let t = attr(e, b"typeface")?;
    if t.is_empty() {
        return None;
    }
    Some(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::fill::Fill;
    use quick_xml::events::Event;

    fn body(xml: &str) -> TextBody {
        let mut r = Reader::new(xml.as_bytes());
        let mut buf = Vec::new();
        loop {
            match r.read_event_into(&mut buf) {
                Ok(Event::Start(e)) => {
                    let name = local_name(e.name().as_ref()).to_vec();
                    return parse_text_body(&mut r, &name);
                }
                Ok(Event::Eof) => panic!("no element"),
                _ => {}
            }
        }
    }

    #[test]
    fn parses_paragraphs_runs_and_text() {
        let b = body(
            r#"<p:txBody><a:bodyPr/><a:p><a:r><a:t>Hello </a:t></a:r><a:r><a:t>world</a:t></a:r></a:p></p:txBody>"#,
        );
        assert_eq!(b.paragraphs.len(), 1);
        assert_eq!(b.plain_text(), "Hello world");
    }

    #[test]
    fn significant_whitespace_inside_a_run_survives() {
        let b = body(r#"<p:txBody><a:p><a:r><a:t>  two  spaces  </a:t></a:r></a:p></p:txBody>"#);
        assert_eq!(b.plain_text(), "  two  spaces  ");
    }

    #[test]
    fn run_properties_cover_the_common_character_formatting() {
        let b = body(
            r#"<p:txBody><a:p><a:r>
                 <a:rPr lang="en-US" sz="2400" b="1" i="1" u="sng" spc="150">
                   <a:solidFill><a:srgbClr val="FF0000"/></a:solidFill>
                   <a:latin typeface="Georgia"/>
                 </a:rPr>
                 <a:t>styled</a:t>
               </a:r></a:p></p:txBody>"#,
        );
        let run = b.paragraphs.first().and_then(|p| p.runs.first()).expect("run");
        let rp = run.props();
        assert_eq!(rp.size, Some(2400));
        assert_eq!(rp.size_points(), Some(24.0));
        assert_eq!(rp.bold, Some(true));
        assert_eq!(rp.italic, Some(true));
        assert_eq!(rp.underline, Some(UnderlineStyle::Single));
        assert_eq!(rp.letter_spacing, Some(150));
        assert_eq!(rp.latin_font.as_deref(), Some("Georgia"));
        assert!(matches!(rp.fill, Fill::Solid(_)));
    }

    #[test]
    fn paragraph_properties_cover_alignment_indent_and_spacing() {
        let b = body(
            r#"<p:txBody><a:p>
                 <a:pPr lvl="2" algn="ctr" marL="457200" indent="-228600">
                   <a:lnSpc><a:spcPct val="150000"/></a:lnSpc>
                   <a:spcBef><a:spcPts val="1200"/></a:spcBef>
                   <a:buChar char="&#8226;"/>
                   <a:buFont typeface="Arial"/>
                 </a:pPr>
                 <a:r><a:t>x</a:t></a:r>
               </a:p></p:txBody>"#,
        );
        let p = &b.paragraphs.first().expect("paragraph").props;
        assert_eq!(p.level, 2);
        assert_eq!(p.align, Some(TextAlign::Center));
        assert_eq!(p.margin_left, Some(457_200));
        assert_eq!(p.indent, Some(-228_600));
        assert_eq!(p.line_spacing, Some(Spacing::Percent(1.5)));
        assert_eq!(p.space_before, Some(Spacing::Points(12.0)));
        assert_eq!(p.bullet.font.as_deref(), Some("Arial"));
        assert!(matches!(p.bullet.kind, Some(BulletKind::Char(_))));
    }

    #[test]
    fn bullet_suppression_is_distinct_from_no_bullet_element() {
        let with_none = body(r#"<p:txBody><a:p><a:pPr><a:buNone/></a:pPr></a:p></p:txBody>"#);
        assert_eq!(
            with_none.paragraphs.first().expect("p").props.bullet.kind,
            Some(BulletKind::None)
        );
        let without = body(r#"<p:txBody><a:p><a:pPr/></a:p></p:txBody>"#);
        assert_eq!(without.paragraphs.first().expect("p").props.bullet.kind, None);
    }

    #[test]
    fn auto_numbering_keeps_its_scheme_and_start() {
        let b = body(
            r#"<p:txBody><a:p><a:pPr><a:buAutoNum type="alphaLcParenR" startAt="3"/></a:pPr></a:p></p:txBody>"#,
        );
        match &b.paragraphs.first().expect("p").props.bullet.kind {
            Some(BulletKind::AutoNum { scheme, start_at }) => {
                assert_eq!(*scheme, AutoNumScheme::AlphaLcParenR);
                assert_eq!(*start_at, 3);
            }
            other => panic!("expected autonum, got {other:?}"),
        }
    }

    #[test]
    fn line_breaks_become_break_runs() {
        let b = body(
            r#"<p:txBody><a:p><a:r><a:t>a</a:t></a:r><a:br/><a:r><a:t>b</a:t></a:r></a:p></p:txBody>"#,
        );
        let p = b.paragraphs.first().expect("p");
        assert_eq!(p.runs.len(), 3);
        assert!(matches!(p.runs.get(1), Some(Run::Break { .. })));
        assert_eq!(p.plain_text(), "a\nb");
    }

    #[test]
    fn fields_keep_the_cached_text_powerpoint_wrote() {
        let b = body(
            r#"<p:txBody><a:p><a:fld id="{GUID}" type="slidenum"><a:rPr lang="en-US"/><a:t>7</a:t></a:fld></a:p></p:txBody>"#,
        );
        match b.paragraphs.first().and_then(|p| p.runs.first()) {
            Some(Run::Field { kind, cached, .. }) => {
                assert_eq!(kind, "slidenum");
                assert_eq!(cached, "7");
            }
            other => panic!("expected field, got {other:?}"),
        }
    }

    #[test]
    fn body_properties_and_autofit_scale() {
        let b = body(
            r#"<p:txBody><a:bodyPr anchor="ctr" lIns="0" tIns="45720" wrap="none">
                 <a:normAutofit fontScale="70000" lnSpcReduction="20000"/>
               </a:bodyPr><a:p/></p:txBody>"#,
        );
        assert_eq!(b.body.anchor, Some(VerticalAnchor::Middle));
        assert_eq!(b.body.left_inset, Some(0));
        assert_eq!(b.body.wrap, Some(false));
        match b.body.autofit {
            Some(Autofit::Shrink {
                font_scale,
                line_space_reduction,
            }) => {
                assert!((font_scale - 0.7).abs() < 1e-6);
                assert!((line_space_reduction - 0.2).abs() < 1e-6);
            }
            other => panic!("expected shrink autofit, got {other:?}"),
        }
    }

    #[test]
    fn list_styles_map_lvl_n_ppr_onto_zero_based_levels() {
        let mut r = Reader::new(
            br#"<a:lstStyle>
                  <a:lvl1pPr marL="0" algn="l"/>
                  <a:lvl3pPr marL="800100" algn="ctr"/>
                </a:lstStyle>"#,
        );
        let mut buf = Vec::new();
        let _ = r.read_event_into(&mut buf);
        let ls = parse_list_style(&mut r, b"lstStyle");
        assert_eq!(ls.level(0).and_then(|p| p.align), Some(TextAlign::Left));
        assert!(ls.level(1).is_none());
        assert_eq!(ls.level(2).and_then(|p| p.margin_left), Some(800_100));
    }

    #[test]
    fn an_empty_body_still_has_one_paragraph_so_it_occupies_space() {
        let b = body(r#"<p:txBody><a:bodyPr/><a:lstStyle/></p:txBody>"#);
        assert_eq!(b.paragraphs.len(), 1);
        assert!(b.is_empty());
    }

    #[test]
    fn malformed_text_body_does_not_panic() {
        let b = body(r#"<p:txBody><a:p><a:r><a:t>unclosed"#);
        assert!(!b.paragraphs.is_empty());
    }

    #[test]
    fn level_element_names_parse_only_in_range() {
        assert_eq!(level_from_element(b"lvl1pPr"), Some(0));
        assert_eq!(level_from_element(b"lvl9pPr"), Some(8));
        assert_eq!(level_from_element(b"lvl0pPr"), None);
        assert_eq!(level_from_element(b"lvl10pPr"), None);
        assert_eq!(level_from_element(b"defPPr"), None);
    }
}
