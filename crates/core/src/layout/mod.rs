//! The layout engine: presentation model → [`DisplayList`].
//!
//! Layout is the only place EMUs become points, and the only producer of drawing
//! commands. It never touches a canvas, a font file, or a pixel — measurement comes in
//! through a [`TextMeasure`], and everything goes out as a display list.

pub mod geom;
pub mod inherit;
pub mod paint;
pub mod preset;
pub mod table;
pub mod text;

use std::rc::Rc;

use crate::dl::{Command, DisplayList, Fit, Paint, Path, Rect, Transform, View};
use crate::emu;
use crate::model::geometry::Transform2D;
use crate::model::shape::{Background, GraphicContent, Shape, ShapeKind, ShapeTree};
use crate::model::text::{BulletKind, Run};
use crate::model::{Presentation, SlideChain};
use crate::text::TextMeasure;

use inherit::Resolver;
use text::{StyledFragment, StyledParagraph};

/// Lays out a slide by index. Returns `None` when the index is out of range.
pub fn layout_slide(
    pres: &Presentation,
    index: usize,
    measure: &dyn TextMeasure,
) -> Option<DisplayList> {
    let chain = pres.chain_for(index)?;
    Some(layout_chain(pres, &chain, measure))
}

/// Lays out an already-resolved slide chain.
pub fn layout_chain(
    pres: &Presentation,
    chain: &SlideChain,
    measure: &dyn TextMeasure,
) -> DisplayList {
    let (w, h) = pres.slide_size_pt();
    let mut dl = DisplayList::new(w, h);
    let resolver = Resolver::new(pres, chain);
    let mut ctx = Ctx {
        pres,
        chain: chain.clone_refs(),
        resolver: &resolver,
        measure,
        slide_rect: Rect::new(0.0, 0.0, w, h),
    };

    emit_background(&mut ctx, &mut dl);

    // Master, then layout, then slide. Placeholders on the first two are skipped: in a
    // slideshow their prompt text is not rendered, only the slide's content in them.
    let show_master = chain.slide.show_master_shapes
        && chain
            .layout
            .as_ref()
            .map(|l| l.show_master_shapes)
            .unwrap_or(true);
    if show_master {
        if let Some(master) = &chain.master {
            let part = master.part_name.clone();
            emit_background_tree(&mut ctx, &master.tree, &part, &mut dl);
        }
    }
    if let Some(layout) = &chain.layout {
        let part = layout.part_name.clone();
        emit_background_tree(&mut ctx, &layout.tree, &part, &mut dl);
    }

    let slide_part = chain.slide.part_name.clone();
    let root = tree_transform(&chain.slide.tree);
    for shape in &chain.slide.tree.shapes {
        emit_shape(&mut ctx, shape, root, &slide_part, &mut dl);
    }

    debug_assert!(dl.is_balanced(), "layout leaked a Save without a Restore");
    dl
}

/// A [`View`] that fits a slide into a viewport.
pub fn view_for(viewport_w: f32, viewport_h: f32, dpr: f32, fit: Fit) -> View {
    View {
        viewport_w,
        viewport_h,
        dpr,
        fit,
        ..Default::default()
    }
}

struct Ctx<'a> {
    pres: &'a Presentation,
    chain: SlideChain,
    resolver: &'a Resolver<'a>,
    measure: &'a dyn TextMeasure,
    slide_rect: Rect,
}

impl SlideChain {
    /// Cheap clone: every field is behind an `Rc`.
    fn clone_refs(&self) -> SlideChain {
        SlideChain {
            slide: Rc::clone(&self.slide),
            layout: self.layout.as_ref().map(Rc::clone),
            master: self.master.as_ref().map(Rc::clone),
            theme: Rc::clone(&self.theme),
        }
    }
}

fn emit_background(ctx: &mut Ctx<'_>, dl: &mut DisplayList) {
    let bg = ctx.chain.background().clone();
    let fill = match &bg {
        Background::Fill(f) => f.clone(),
        Background::Reference { idx, color } => {
            match ctx.chain.theme.formats.background_fill(*idx) {
                Some(f) => substitute(f.clone(), color),
                None => return,
            }
        }
        Background::Inherit => return,
    };
    let part = ctx.chain.slide.part_name.clone();
    let Some(p) = paint::fill_to_paint(&fill, ctx.slide_rect, ctx.resolver, ctx.pres, &part) else {
        return;
    };
    dl.push(Command::FillPath {
        path: Path::rect(ctx.slide_rect),
        paint: p,
        rule: crate::dl::FillRule::NonZero,
    });
}

/// Substitutes `phClr` in a background fill style with the colour the `<p:bgRef>` named.
fn substitute(fill: crate::model::fill::Fill, with: &crate::model::color::ColorRef) -> crate::model::fill::Fill {
    use crate::model::color::ColorSpec;
    use crate::model::fill::Fill;
    let swap = |c: &crate::model::color::ColorRef| {
        if matches!(c.spec, ColorSpec::Placeholder) {
            let mut out = with.clone();
            out.mods.extend(c.mods.iter().copied());
            out
        } else {
            c.clone()
        }
    };
    match fill {
        Fill::Solid(c) => Fill::Solid(swap(&c)),
        Fill::Gradient(mut g) => {
            for s in &mut g.stops {
                s.color = swap(&s.color);
            }
            Fill::Gradient(g)
        }
        other => other,
    }
}

fn emit_background_tree(ctx: &mut Ctx<'_>, tree: &ShapeTree, part: &str, dl: &mut DisplayList) {
    let root = tree_transform(tree);
    for shape in &tree.shapes {
        if !Resolver::is_background_shape(shape) {
            continue;
        }
        emit_shape(ctx, shape, root, part, dl);
    }
}

/// The transform a shape tree's own `<p:grpSpPr><a:xfrm>` imposes on its children.
///
/// Slides almost always leave this at identity, but layouts occasionally scale their
/// whole tree, and ignoring it puts every layout shape in the wrong place.
fn tree_transform(tree: &ShapeTree) -> Transform {
    match &tree.root_transform {
        Some(g) if g.xfrm.specified && g.child_extent_x != 0 && g.child_extent_y != 0 => {
            group_transform(g)
        }
        _ => Transform::IDENTITY,
    }
}

fn group_transform(g: &crate::model::geometry::GroupTransform) -> Transform {
    let (sx, sy) = g.child_scale();
    let child_off = Transform::translate(
        -emu::to_pt(g.child_offset_x),
        -emu::to_pt(g.child_offset_y),
    );
    let scale = Transform::scale(sx, sy);
    let place = Transform::translate(emu::to_pt(g.xfrm.offset_x), emu::to_pt(g.xfrm.offset_y));
    let mut t = child_off.then(&scale).then(&place);
    // The group rotates as a unit, about the centre of its own box.
    if g.xfrm.rotation != 0 || g.xfrm.flip_h || g.xfrm.flip_v {
        let box_rect = rect_of(&g.xfrm);
        t = t.then(&local_transform(&g.xfrm, box_rect));
    }
    t
}

fn rect_of(t: &Transform2D) -> Rect {
    Rect::new(
        emu::to_pt(t.offset_x),
        emu::to_pt(t.offset_y),
        emu::to_pt(t.extent_x),
        emu::to_pt(t.extent_y),
    )
}

/// Rotation and flips, applied about the centre of the shape's box.
fn local_transform(t: &Transform2D, box_rect: Rect) -> Transform {
    let c = box_rect.center();
    let mut out = Transform::IDENTITY;
    if t.flip_h || t.flip_v {
        let sx = if t.flip_h { -1.0 } else { 1.0 };
        let sy = if t.flip_v { -1.0 } else { 1.0 };
        out = out
            .then(&Transform::translate(-c.x, -c.y))
            .then(&Transform::scale(sx, sy))
            .then(&Transform::translate(c.x, c.y));
    }
    if t.rotation != 0 {
        out = out.then(&Transform::rotate_about(
            emu::angle_to_radians(t.rotation),
            c.x,
            c.y,
        ));
    }
    out
}

fn emit_shape(
    ctx: &mut Ctx<'_>,
    shape: &Shape,
    parent: Transform,
    part: &str,
    dl: &mut DisplayList,
) {
    if shape.hidden {
        return;
    }
    let ancestors = ctx.resolver.placeholder_ancestors(shape);
    let xfrm = ctx.resolver.transform(shape, &ancestors);
    let box_rect = rect_of(&xfrm);

    if let ShapeKind::Group(children) = &shape.kind {
        let Some(g) = &shape.group_transform else {
            for c in children {
                emit_shape(ctx, c, parent, part, dl);
            }
            return;
        };
        let inner = group_transform(g).then(&parent);
        dl.push(Command::Save);
        for c in children {
            emit_shape(ctx, c, inner, part, dl);
        }
        dl.push(Command::Restore);
        return;
    }

    // A shape with no size draws nothing, but a *connector* legitimately has a zero
    // extent on one axis, so only both being zero is a skip.
    if box_rect.w == 0.0 && box_rect.h == 0.0 {
        return;
    }

    let local = local_transform(&xfrm, box_rect).then(&parent);
    let concat = !matches!(local, t if t == Transform::IDENTITY);
    dl.push(Command::Save);
    if concat {
        dl.push(Command::Concat(local));
    }

    match &shape.kind {
        ShapeKind::Picture(pic) => emit_picture(ctx, shape, pic, box_rect, part, dl),
        ShapeKind::Graphic(g) => emit_graphic(ctx, shape, g, box_rect, part, dl),
        _ => emit_geometry(ctx, shape, &ancestors, box_rect, part, dl),
    }

    if let Some(body) = &shape.text {
        if !body.is_empty() {
            emit_text(ctx, shape, &ancestors, box_rect, dl);
        }
    }

    dl.push(Command::Restore);
}

fn emit_geometry(
    ctx: &mut Ctx<'_>,
    shape: &Shape,
    ancestors: &[Rc<Shape>],
    box_rect: Rect,
    part: &str,
    dl: &mut DisplayList,
) {
    let fill = ctx.resolver.fill(shape, ancestors);
    let line = ctx.resolver.line(shape, ancestors);
    let paths = geom::evaluate(&shape.geometry, box_rect.w, box_rect.h);

    // Geometry is generated at the origin; move it to the shape's position.
    let offset = Transform::translate(box_rect.x, box_rect.y);
    let fill_paint = paint::fill_to_paint(&fill, box_rect, ctx.resolver, ctx.pres, part);
    let stroke = paint::line_to_stroke(&line, box_rect, ctx.resolver, ctx.pres, part);

    for ep in paths {
        let mut path = ep.path;
        path.transform(&offset);
        if ep.fill {
            if let Some(p) = &fill_paint {
                dl.push(Command::FillPath {
                    path: path.clone(),
                    paint: p.clone(),
                    rule: geom::FILL_RULE,
                });
            }
        }
        if ep.stroke {
            if let Some(s) = &stroke {
                dl.push(Command::StrokePath { path, stroke: s.clone() });
            }
        }
    }
}

fn emit_picture(
    ctx: &mut Ctx<'_>,
    shape: &Shape,
    pic: &crate::model::shape::Picture,
    box_rect: Rect,
    part: &str,
    dl: &mut DisplayList,
) {
    let Some(rid) = pic.embed_id.as_deref() else {
        return;
    };
    let Some(id) = ctx.pres.intern_image(part, rid) else {
        log::debug!("picture {} references {rid}, which does not resolve", shape.name);
        return;
    };
    let [l, t, r, b] = pic.src_rect;
    let src = Rect::new(l, t, (1.0 - l - r).max(0.0), (1.0 - t - b).max(0.0));
    if src.is_empty() {
        return;
    }

    // A picture can carry its own geometry, in which case it is clipped to it — that is
    // how "picture in a circle" is expressed.
    let clipped = !matches!(shape.geometry, crate::model::Geometry::None)
        && shape.geometry.preset_name() != Some("rect");
    if clipped {
        let paths = geom::evaluate(&shape.geometry, box_rect.w, box_rect.h);
        if let Some(first) = paths.into_iter().next() {
            let mut path = first.path;
            path.transform(&Transform::translate(box_rect.x, box_rect.y));
            dl.push(Command::Save);
            dl.push(Command::ClipPath {
                path,
                rule: geom::FILL_RULE,
            });
        }
    }
    dl.push(Command::DrawImage {
        image: id,
        src,
        dst: box_rect,
        opacity: pic.alpha.clamp(0.0, 1.0),
    });
    if clipped {
        dl.push(Command::Restore);
    }

    // A picture's outline is drawn on top of it.
    let line = ctx.resolver.line(shape, &[]);
    if let Some(stroke) = paint::line_to_stroke(&line, box_rect, ctx.resolver, ctx.pres, part) {
        let mut path = geom::evaluate(&shape.geometry, box_rect.w, box_rect.h)
            .into_iter()
            .next()
            .map(|p| p.path)
            .unwrap_or_else(|| Path::rect(Rect::new(0.0, 0.0, box_rect.w, box_rect.h)));
        path.transform(&Transform::translate(box_rect.x, box_rect.y));
        dl.push(Command::StrokePath { path, stroke });
    }
}

fn emit_graphic(
    ctx: &mut Ctx<'_>,
    shape: &Shape,
    content: &GraphicContent,
    box_rect: Rect,
    part: &str,
    dl: &mut DisplayList,
) {
    match content {
        GraphicContent::Table(t) => table::layout_table(
            t,
            box_rect,
            ctx.resolver,
            ctx.pres,
            ctx.measure,
            part,
            &mut dl.commands,
        ),
        GraphicContent::Chart(rid) => {
            // Charts arrive in M5b. Until then a frame is drawn so the slide's structure
            // is still legible instead of a blank gap.
            log::debug!("chart {rid} on {} not rendered yet", shape.name);
            placeholder_frame(box_rect, dl);
        }
        GraphicContent::Unsupported { kind, .. } => {
            log::debug!("graphic {kind:?} on {} is not supported", shape.name);
            let _ = part;
            placeholder_frame(box_rect, dl);
        }
    }
}

fn placeholder_frame(box_rect: Rect, dl: &mut DisplayList) {
    dl.push(Command::StrokePath {
        path: Path::rect(box_rect),
        stroke: crate::dl::Stroke {
            paint: Paint::Solid(crate::dl::Color::rgba(0x99, 0x99, 0x99, 0x80)),
            width: 0.75,
            dash: vec![3.0, 3.0],
            ..Default::default()
        },
    });
}

fn emit_text(
    ctx: &mut Ctx<'_>,
    shape: &Shape,
    ancestors: &[Rc<Shape>],
    box_rect: Rect,
    dl: &mut DisplayList,
) {
    let Some(body) = &shape.text else { return };
    let styled = build_paragraphs(ctx, shape, ancestors, body);

    // Custom geometry can narrow where text goes; presets use the whole box.
    let inner = geom::text_rect(&shape.geometry, box_rect.w, box_rect.h);
    let text_box = Rect::new(
        box_rect.x + inner.x,
        box_rect.y + inner.y,
        inner.w,
        inner.h,
    );
    let content = text::content_rect(text_box, &body.body);

    // Text is clipped to its shape only when it would otherwise escape; PowerPoint lets
    // overflowing text spill, so clipping unconditionally would be wrong.
    let mut commands = Vec::new();
    let result = text::layout(&styled, &body.body, content, ctx.measure, &mut commands);
    if result.overflowed {
        log::trace!("text overflows {} by {:.1}pt", shape.name, result.height - content.h);
    }
    dl.commands.extend(commands);
}

fn build_paragraphs(
    ctx: &Ctx<'_>,
    shape: &Shape,
    ancestors: &[Rc<Shape>],
    body: &crate::model::TextBody,
) -> Vec<StyledParagraph> {
    let mut out = Vec::with_capacity(body.paragraphs.len());
    // Auto-numbered lists restart per level whenever a shallower paragraph intervenes.
    let mut counters = [0u32; 9];

    for para in &body.paragraphs {
        let level = para.props.level.min(8);
        let props = ctx
            .resolver
            .paragraph_props(shape, ancestors, &para.props, level);

        let mut fragments = Vec::new();
        for run in &para.runs {
            match run {
                Run::Break { props: rp } => {
                    let resolved = ctx.resolver.run_props(rp, &props, shape, ancestors);
                    let (font, paint, _, _) = run_style(ctx, &resolved);
                    fragments.push(StyledFragment::text_break(font, paint));
                }
                other => {
                    let raw = other.text();
                    if raw.is_empty() {
                        continue;
                    }
                    let resolved = ctx.resolver.run_props(other.props(), &props, shape, ancestors);
                    let (font, paint, decorations, letter_spacing) = run_style(ctx, &resolved);
                    let caps = resolved.caps.unwrap_or(crate::model::text::Capitalization::None);
                    fragments.push(StyledFragment {
                        text: text::apply_caps(raw, caps),
                        font,
                        paint,
                        decorations,
                        letter_spacing,
                        baseline_shift: resolved.baseline.unwrap_or(0.0),
                        is_break: false,
                    });
                }
            }
        }

        // The paragraph mark decides an empty paragraph's height.
        let end_props = ctx.resolver.run_props(
            para.end_props.as_ref().unwrap_or(&Default::default()),
            &props,
            shape,
            ancestors,
        );
        let (end_font, end_paint, _, _) = run_style(ctx, &end_props);

        // Advance the auto-number counter only for paragraphs that actually show one.
        let index_in_list = if matches!(props.bullet.kind, Some(BulletKind::AutoNum { .. })) {
            let n = counters.get(level as usize).copied().unwrap_or(0);
            if let Some(slot) = counters.get_mut(level as usize) {
                *slot = n + 1;
            }
            // Deeper levels restart under a new parent item.
            for deeper in (level as usize + 1)..9 {
                if let Some(slot) = counters.get_mut(deeper) {
                    *slot = 0;
                }
            }
            n
        } else {
            0
        };

        let bullet_font = fragments
            .first()
            .map(|f| f.font.clone())
            .unwrap_or_else(|| end_font.clone());
        let bullet_paint = fragments
            .first()
            .map(|f| f.paint.clone())
            .unwrap_or_else(|| end_paint.clone());
        let bullet_paint = match (&props.bullet.color, props.bullet.follow_text_color) {
            (Some(c), false) => Paint::Solid(ctx.resolver.color(c)),
            _ => bullet_paint,
        };
        let bullet = text::bullet_fragment(&props, index_in_list, &bullet_font, &bullet_paint);

        out.push(StyledParagraph {
            fragments,
            align: props.align.unwrap_or_default(),
            margin_left: emu::to_pt(props.margin_left.unwrap_or(0)),
            margin_right: emu::to_pt(props.margin_right.unwrap_or(0)),
            indent: emu::to_pt(props.indent.unwrap_or(0)),
            line_spacing: props
                .line_spacing
                .unwrap_or(crate::model::text::Spacing::Percent(1.0)),
            space_before: props
                .space_before
                .unwrap_or(crate::model::text::Spacing::Points(0.0)),
            space_after: props
                .space_after
                .unwrap_or(crate::model::text::Spacing::Points(0.0)),
            rtl: props.rtl.unwrap_or(false),
            bullet,
            empty_metrics: ctx.measure.font_metrics(&end_font),
            empty_size_pt: end_font.size(),
        });
    }
    out
}

/// Turns resolved run properties into the display-list font, paint and decorations.
fn run_style(
    ctx: &Ctx<'_>,
    props: &crate::model::RunProps,
) -> (crate::dl::FontSpec, Paint, crate::dl::Decorations, f32) {
    let family = ctx.resolver.font_family(props.latin_font.as_deref());
    let size = props.size_points().unwrap_or(18.0);
    let font = text::font_spec(
        &family,
        size,
        props.bold.unwrap_or(false),
        props.italic.unwrap_or(false),
    );
    let paint = paint::fill_to_paint(
        &props.fill,
        Rect::new(0.0, 0.0, size, size),
        ctx.resolver,
        ctx.pres,
        &ctx.chain.slide.part_name,
    )
    .unwrap_or(Paint::Solid(crate::dl::Color::BLACK));
    let decorations = text::decorations(props.underline, props.strikethrough);
    // `spc` is in hundredths of a point.
    let letter_spacing = props.letter_spacing.unwrap_or(0) as f32 / 100.0;
    (font, paint, decorations, letter_spacing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::StubMeasure;

    /// Builds a one-slide deck from raw slide XML.
    fn deck_with_slide(slide_xml: &str) -> Presentation {
        use std::io::{Cursor, Write};
        let ct = br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
          <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
          <Default Extension="xml" ContentType="application/xml"/>
          <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
        </Types>"#;
        let root_rels = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
          <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>
        </Relationships>"#;
        let pres_rels = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
          <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
        </Relationships>"#;
        let pres = br#"<p:presentation xmlns:p="p" xmlns:r="r">
          <p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst>
          <p:sldSz cx="12192000" cy="6858000"/>
        </p:presentation>"#;

        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            for (name, data) in [
                ("[Content_Types].xml", &ct[..]),
                ("_rels/.rels", &root_rels[..]),
                ("ppt/presentation.xml", &pres[..]),
                ("ppt/_rels/presentation.xml.rels", &pres_rels[..]),
                ("ppt/slides/slide1.xml", slide_xml.as_bytes()),
            ] {
                w.start_file(name, opts).expect("start");
                w.write_all(data).expect("write");
            }
            w.finish().expect("finish");
        }
        crate::open(buf).expect("open")
    }

    fn fills(dl: &DisplayList) -> Vec<(Rect, Paint)> {
        dl.commands
            .iter()
            .filter_map(|c| match c {
                Command::FillPath { path, paint, .. } => Some((path.bounds(), paint.clone())),
                _ => None,
            })
            .collect()
    }

    const RECT_SLIDE: &str = r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree>
        <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
        <p:grpSpPr/>
        <p:sp>
          <p:nvSpPr><p:cNvPr id="2" name="Rect"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
          <p:spPr>
            <a:xfrm><a:off x="914400" y="914400"/><a:ext cx="1828800" cy="914400"/></a:xfrm>
            <a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
            <a:solidFill><a:srgbClr val="FF0000"/></a:solidFill>
          </p:spPr>
        </p:sp>
      </p:spTree></p:cSld></p:sld>"#;

    #[test]
    fn a_rectangle_lands_at_its_emu_position_converted_to_points() {
        let pres = deck_with_slide(RECT_SLIDE);
        let dl = layout_slide(&pres, 0, &StubMeasure).expect("layout");
        assert_eq!(dl.width_pt, 960.0);
        assert_eq!(dl.height_pt, 540.0);
        let f = fills(&dl);
        assert_eq!(f.len(), 1);
        // 914400 EMU = 72pt, 1828800 = 144pt, 914400 = 72pt.
        assert_eq!(f[0].0, Rect::new(72.0, 72.0, 144.0, 72.0));
        assert_eq!(f[0].1, Paint::Solid(crate::dl::Color::rgb(255, 0, 0)));
    }

    #[test]
    fn the_display_list_is_save_restore_balanced() {
        let pres = deck_with_slide(RECT_SLIDE);
        let dl = layout_slide(&pres, 0, &StubMeasure).expect("layout");
        assert!(dl.is_balanced());
    }

    #[test]
    fn an_out_of_range_slide_index_returns_none() {
        let pres = deck_with_slide(RECT_SLIDE);
        assert!(layout_slide(&pres, 5, &StubMeasure).is_none());
    }

    #[test]
    fn text_is_emitted_inside_its_shape() {
        let slide = r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree>
            <p:sp>
              <p:nvSpPr><p:cNvPr id="2" name="TextBox"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
              <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="3657600" cy="1828800"/></a:xfrm></p:spPr>
              <p:txBody><a:bodyPr/><a:p><a:r><a:rPr sz="1800"/><a:t>Hello</a:t></a:r></a:p></p:txBody>
            </p:sp>
          </p:spTree></p:cSld></p:sld>"#;
        let pres = deck_with_slide(slide);
        let dl = layout_slide(&pres, 0, &StubMeasure).expect("layout");
        let runs: Vec<_> = dl.text_runs().collect();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "Hello");
        assert_eq!(runs[0].font.size(), 18.0);
        // Inside the 288pt x 144pt box, past the default insets.
        assert!(runs[0].origin.x > 7.0 && runs[0].origin.x < 20.0);
        assert!(runs[0].origin.y > 0.0 && runs[0].origin.y < 144.0);
        assert_eq!(dl.plain_text(), "Hello");
    }

    #[test]
    fn a_rotated_shape_gets_a_concat_command_rather_than_baked_coordinates() {
        let slide = r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree>
            <p:sp>
              <p:nvSpPr><p:cNvPr id="2" name="Rot"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
              <p:spPr>
                <a:xfrm rot="2700000"><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm>
                <a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
                <a:solidFill><a:srgbClr val="00FF00"/></a:solidFill>
              </p:spPr>
            </p:sp>
          </p:spTree></p:cSld></p:sld>"#;
        let pres = deck_with_slide(slide);
        let dl = layout_slide(&pres, 0, &StubMeasure).expect("layout");
        let concats: Vec<_> = dl
            .commands
            .iter()
            .filter_map(|c| match c {
                Command::Concat(t) => Some(*t),
                _ => None,
            })
            .collect();
        assert_eq!(concats.len(), 1);
        // 45 degrees: a and d are cos(45), b is sin(45).
        assert!((concats[0].a - 0.7071).abs() < 0.001, "{:?}", concats[0]);
        assert!((concats[0].b - 0.7071).abs() < 0.001);
        // The unrotated geometry is still emitted at its authored position.
        assert_eq!(fills(&dl)[0].0, Rect::new(0.0, 0.0, 72.0, 72.0));
    }

    #[test]
    fn a_group_maps_children_through_its_child_coordinate_space() {
        let slide = r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree>
            <p:grpSp>
              <p:nvGrpSpPr><p:cNvPr id="9" name="G"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
              <p:grpSpPr><a:xfrm>
                <a:off x="914400" y="0"/><a:ext cx="914400" cy="914400"/>
                <a:chOff x="0" y="0"/><a:chExt cx="1828800" cy="1828800"/>
              </a:xfrm></p:grpSpPr>
              <p:sp>
                <p:nvSpPr><p:cNvPr id="10" name="Inner"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
                <p:spPr>
                  <a:xfrm><a:off x="0" y="0"/><a:ext cx="1828800" cy="1828800"/></a:xfrm>
                  <a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
                  <a:solidFill><a:srgbClr val="0000FF"/></a:solidFill>
                </p:spPr>
              </p:sp>
            </p:grpSp>
          </p:spTree></p:cSld></p:sld>"#;
        let pres = deck_with_slide(slide);
        let dl = layout_slide(&pres, 0, &StubMeasure).expect("layout");
        let concats: Vec<_> = dl
            .commands
            .iter()
            .filter_map(|c| match c {
                Command::Concat(t) => Some(*t),
                _ => None,
            })
            .collect();
        assert_eq!(concats.len(), 1, "the group contributes one transform");
        // Child space is twice the group box, so children scale by 0.5 and shift by 72pt.
        let t = concats[0];
        assert!((t.a - 0.5).abs() < 1e-5, "{t:?}");
        assert!((t.e - 72.0).abs() < 1e-4, "{t:?}");
        assert!(dl.is_balanced());
    }

    #[test]
    fn a_hidden_shape_emits_nothing() {
        let slide = r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree>
            <p:sp>
              <p:nvSpPr><p:cNvPr id="2" name="Ghost" hidden="1"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
              <p:spPr>
                <a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm>
                <a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
                <a:solidFill><a:srgbClr val="FF0000"/></a:solidFill>
              </p:spPr>
            </p:sp>
          </p:spTree></p:cSld></p:sld>"#;
        let pres = deck_with_slide(slide);
        let dl = layout_slide(&pres, 0, &StubMeasure).expect("layout");
        assert!(fills(&dl).is_empty());
    }

    #[test]
    fn a_zero_sized_shape_is_skipped_without_emitting_an_unbalanced_save() {
        let slide = r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree>
            <p:sp>
              <p:nvSpPr><p:cNvPr id="2" name="Zero"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
              <p:spPr>
                <a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></a:xfrm>
                <a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
                <a:solidFill><a:srgbClr val="FF0000"/></a:solidFill>
              </p:spPr>
            </p:sp>
          </p:spTree></p:cSld></p:sld>"#;
        let pres = deck_with_slide(slide);
        let dl = layout_slide(&pres, 0, &StubMeasure).expect("layout");
        assert!(dl.is_balanced());
        assert!(fills(&dl).is_empty());
    }

    #[test]
    fn a_slide_background_fills_the_whole_slide_first() {
        let slide = r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld>
            <p:bg><p:bgPr><a:solidFill><a:srgbClr val="123456"/></a:solidFill></p:bgPr></p:bg>
            <p:spTree/>
          </p:cSld></p:sld>"#;
        let pres = deck_with_slide(slide);
        let dl = layout_slide(&pres, 0, &StubMeasure).expect("layout");
        let f = fills(&dl);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].0, Rect::new(0.0, 0.0, 960.0, 540.0));
        assert_eq!(f[0].1, Paint::Solid(crate::dl::Color::rgb(0x12, 0x34, 0x56)));
    }

    #[test]
    fn re_laying_out_the_same_slide_produces_an_identical_display_list() {
        let pres = deck_with_slide(RECT_SLIDE);
        let a = layout_slide(&pres, 0, &StubMeasure).expect("a");
        let b = layout_slide(&pres, 0, &StubMeasure).expect("b");
        assert_eq!(a, b, "layout must be deterministic");
    }

    #[test]
    fn the_view_transform_is_what_applies_zoom_not_the_layout() {
        let pres = deck_with_slide(RECT_SLIDE);
        let dl = layout_slide(&pres, 0, &StubMeasure).expect("layout");
        let small = view_for(480.0, 270.0, 1.0, Fit::Contain).transform_for(dl.width_pt, dl.height_pt);
        let large = view_for(1920.0, 1080.0, 1.0, Fit::Contain).transform_for(dl.width_pt, dl.height_pt);
        assert!((small.a - 0.5).abs() < 1e-6);
        assert!((large.a - 2.0).abs() < 1e-6);
        // The display list itself is unchanged by either.
        assert_eq!(fills(&dl)[0].0, Rect::new(72.0, 72.0, 144.0, 72.0));
    }
}
