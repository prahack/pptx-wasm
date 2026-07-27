//! Property resolution: shape → placeholder → layout → master → theme → hard default.
//!
//! This is where "renders" becomes "looks right". A title on a corporate template
//! typically specifies *nothing* — no position, no size, no font, no colour — and gets
//! all of it from the layout placeholder it is bound to, which in turn gets it from the
//! master, which gets its colours and fonts from the theme. Every lookup below walks that
//! chain in that order and stops at the first level that actually specified the property.

use std::rc::Rc;

use crate::dl::Color;
use crate::model::color::ColorRef;
use crate::model::fill::{Effects, Fill, Line};
use crate::model::geometry::Transform2D;
use crate::model::shape::{MasterTextStyle, Placeholder, Shape, ShapeKind, StyleRef};
use crate::model::text::{ListStyle, ParagraphProps, RunProps};
use crate::model::{Presentation, SlideChain};

/// Everything a shape needs to resolve its own properties.
pub struct Resolver<'a> {
    pub pres: &'a Presentation,
    pub chain: &'a SlideChain,
}

impl<'a> Resolver<'a> {
    pub fn new(pres: &'a Presentation, chain: &'a SlideChain) -> Self {
        Resolver { pres, chain }
    }

    /// The layout and master shapes this shape's placeholder binds to, nearest first.
    ///
    /// Returns an empty vector for a non-placeholder shape, which is what makes the rest
    /// of the resolution collapse to "whatever the shape said" for ordinary text boxes.
    pub fn placeholder_ancestors(&self, shape: &Shape) -> Vec<Rc<Shape>> {
        let Some(ph) = &shape.placeholder else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if let Some(layout) = &self.chain.layout {
            if let Some(found) = layout.tree.find_placeholder(ph) {
                out.push(Rc::new(found.clone()));
            }
        }
        if let Some(master) = &self.chain.master {
            if let Some(found) = master.tree.find_placeholder(ph) {
                out.push(Rc::new(found.clone()));
            }
        }
        out
    }

    /// The shape's position and size, inherited from its placeholder when absent.
    pub fn transform(&self, shape: &Shape, ancestors: &[Rc<Shape>]) -> Transform2D {
        if shape.transform.specified {
            return shape.transform;
        }
        for a in ancestors {
            if a.transform.specified {
                // Rotation and flips stay with the shape itself; only the box is inherited.
                return Transform2D {
                    rotation: shape.transform.rotation,
                    flip_h: shape.transform.flip_h,
                    flip_v: shape.transform.flip_v,
                    ..a.transform
                };
            }
        }
        shape.transform
    }

    /// The effective fill.
    ///
    /// Order: the shape's own `<a:solidFill>` etc., then its `<p:style><a:fillRef>` into
    /// the theme, then the placeholder chain, then nothing. The style reference sits
    /// *between* the shape and its placeholder because PowerPoint treats it as part of
    /// the shape's own formatting.
    pub fn fill(&self, shape: &Shape, ancestors: &[Rc<Shape>]) -> Fill {
        if shape.fill.is_specified() {
            return shape.fill.clone();
        }
        if let Some(f) = self.style_fill(shape.style_ref.as_ref()) {
            return f;
        }
        for a in ancestors {
            if a.fill.is_specified() {
                return a.fill.clone();
            }
            if let Some(f) = self.style_fill(a.style_ref.as_ref()) {
                return f;
            }
        }
        // Placeholders default to no fill; a bare autoshape defaults to the theme's
        // first fill style, which is what PowerPoint inserts a new shape with.
        if shape.placeholder.is_some() {
            Fill::NoFill
        } else {
            self.chain
                .theme
                .default_shape_fill
                .clone()
                .unwrap_or(Fill::NoFill)
        }
    }

    fn style_fill(&self, style: Option<&StyleRef>) -> Option<Fill> {
        let style = style?;
        let idx = style.fill_idx?;
        let fill = self.chain.theme.formats.fill(idx)?.clone();
        Some(substitute_placeholder_color(
            fill,
            style.fill_color.as_ref(),
        ))
    }

    /// The effective outline, merged across the chain so a shape can override only its
    /// width while inheriting its colour.
    pub fn line(&self, shape: &Shape, ancestors: &[Rc<Shape>]) -> Line {
        let mut line = shape.line.clone();
        if let Some(l) = self.style_line(shape.style_ref.as_ref()) {
            line.inherit_from(&l);
        }
        for a in ancestors {
            line.inherit_from(&a.line);
            if let Some(l) = self.style_line(a.style_ref.as_ref()) {
                line.inherit_from(&l);
            }
        }
        if let Some(default) = &self.chain.theme.default_shape_line {
            line.inherit_from(default);
        }
        line
    }

    fn style_line(&self, style: Option<&StyleRef>) -> Option<Line> {
        let style = style?;
        let idx = style.line_idx?;
        let mut line = self.chain.theme.formats.line(idx)?.clone();
        line.fill = substitute_placeholder_color(line.fill, style.line_color.as_ref());
        Some(line)
    }

    pub fn effects(&self, shape: &Shape, ancestors: &[Rc<Shape>]) -> Effects {
        if !shape.effects.is_empty() {
            return shape.effects.clone();
        }
        if let Some(style) = &shape.style_ref {
            if let Some(idx) = style.effect_idx {
                if let Some(fx) = self.chain.theme.formats.effect(idx) {
                    if !fx.is_empty() {
                        return fx.clone();
                    }
                }
            }
        }
        for a in ancestors {
            if !a.effects.is_empty() {
                return a.effects.clone();
            }
        }
        Effects::default()
    }

    /// Resolves a colour reference in this slide's theme and colour-map context.
    pub fn color(&self, c: &ColorRef) -> Color {
        self.chain.resolve_color(c, None)
    }

    /// The merged paragraph properties for a paragraph at `level` in `shape`.
    ///
    /// `own` is the paragraph's own `<a:pPr>`; everything under it comes from the chain.
    pub fn paragraph_props(
        &self,
        shape: &Shape,
        ancestors: &[Rc<Shape>],
        own: &ParagraphProps,
        level: u8,
    ) -> ParagraphProps {
        let mut props = own.clone();
        props.level = level;

        // The shape's own <a:lstStyle> on its text body.
        if let Some(body) = &shape.text {
            if let Some(p) = body.list_style.level(level) {
                props.inherit_from(p);
            }
        }
        // Then each placeholder ancestor's list style.
        for a in ancestors {
            if let Some(body) = &a.text {
                if let Some(p) = body.list_style.level(level) {
                    props.inherit_from(p);
                }
            }
        }
        // Then the master's <p:txStyles> block for this placeholder kind.
        if let Some(master) = &self.chain.master {
            let style = match shape.placeholder.as_ref().map(|p| p.kind.text_style()) {
                Some(MasterTextStyle::Title) => &master.title_style,
                Some(MasterTextStyle::Body) => &master.body_style,
                Some(MasterTextStyle::Other) => &master.other_style,
                // A plain text box takes the "other" styles.
                None => &master.other_style,
            };
            if let Some(p) = style.level(level) {
                props.inherit_from(p);
            }
        }
        // Then the deck-wide default, then the theme's.
        if let Some(p) = self.pres.default_text_style.level(level) {
            props.inherit_from(p);
        }
        if let Some(p) = self.chain.theme.default_text_style.level(level) {
            props.inherit_from(p);
        }
        props.inherit_from(&hard_default_paragraph(level));
        props
    }

    /// The merged run properties for a run, given the already-merged paragraph props.
    pub fn run_props(
        &self,
        own: &RunProps,
        paragraph: &ParagraphProps,
        shape: &Shape,
        ancestors: &[Rc<Shape>],
    ) -> RunProps {
        let mut props = own.clone();
        if let Some(defaults) = &paragraph.default_run_props {
            props.inherit_from(defaults);
        }
        // A `<p:style><a:fontRef>` supplies the face and colour for styled autoshapes.
        for s in std::iter::once(shape).chain(ancestors.iter().map(|a| a.as_ref())) {
            if let Some(style) = &s.style_ref {
                if props.latin_font.is_none() {
                    props.latin_font = match style.font_kind.as_deref() {
                        Some("major") => Some("+mj-lt".to_string()),
                        Some("minor") => Some("+mn-lt".to_string()),
                        _ => None,
                    };
                }
                if !props.fill.is_specified() {
                    if let Some(c) = &style.font_color {
                        props.fill = Fill::Solid(c.clone());
                    }
                }
            }
        }
        props.inherit_from(&hard_default_run());
        props
    }

    /// Resolves a typeface name, expanding the `+mj-`/`+mn-` theme references.
    ///
    /// `+mn-lt` means "the minor (body) latin font of the current theme". Failing to
    /// expand these is the single most visible theme bug: every run falls back to the
    /// browser default and the whole deck changes character.
    pub fn font_family(&self, requested: Option<&str>) -> String {
        let fonts = &self.chain.theme.fonts;
        let name = requested.unwrap_or("+mn-lt");
        let resolved = match name {
            "+mj-lt" => &fonts.major.latin,
            "+mn-lt" => &fonts.minor.latin,
            "+mj-ea" => &fonts.major.east_asian,
            "+mn-ea" => &fonts.minor.east_asian,
            "+mj-cs" => &fonts.major.complex_script,
            "+mn-cs" => &fonts.minor.complex_script,
            other => return other.to_string(),
        };
        if resolved.is_empty() {
            // A theme with an empty slot (common for `ea`/`cs`) falls back to the latin
            // minor font rather than to nothing.
            let fallback = &fonts.minor.latin;
            if fallback.is_empty() {
                "Calibri".to_string()
            } else {
                fallback.clone()
            }
        } else {
            resolved.clone()
        }
    }

    /// The list style a placeholder contributes, used by table and chart text too.
    pub fn placeholder_list_style(&self, ph: &Placeholder) -> ListStyle {
        let mut ls = ListStyle::default();
        if let Some(layout) = &self.chain.layout {
            if let Some(s) = layout.tree.find_placeholder(ph) {
                if let Some(body) = &s.text {
                    ls.inherit_from(&body.list_style);
                }
            }
        }
        if let Some(master) = &self.chain.master {
            if let Some(s) = master.tree.find_placeholder(ph) {
                if let Some(body) = &s.text {
                    ls.inherit_from(&body.list_style);
                }
            }
        }
        ls
    }

    /// Whether a shape from the layout or master should be drawn beneath the slide.
    ///
    /// Placeholders are skipped: in a slideshow PowerPoint draws the slide's content in
    /// them, never the layout's prompt text ("Click to edit Master title style").
    pub fn is_background_shape(shape: &Shape) -> bool {
        if shape.hidden {
            return false;
        }
        if shape.placeholder.is_some() {
            return false;
        }
        // An empty group contributes nothing but costs a transform push.
        !matches!(&shape.kind, ShapeKind::Group(kids) if kids.is_empty())
    }
}

/// Replaces `phClr` inside a theme style fill with the colour the style reference named.
fn substitute_placeholder_color(fill: Fill, with: Option<&ColorRef>) -> Fill {
    let Some(with) = with else { return fill };
    fn subst(c: &ColorRef, with: &ColorRef) -> ColorRef {
        if !matches!(c.spec, crate::model::color::ColorSpec::Placeholder) {
            return c.clone();
        }
        // Keep the theme style's own modifiers and stack them on the named colour, which
        // is how "accent1, shaded 50%" comes out of a `lnRef` plus a `shade` in the style.
        let mut out = with.clone();
        out.mods.extend(c.mods.iter().copied());
        out
    }
    match fill {
        Fill::Solid(c) => Fill::Solid(subst(&c, with)),
        Fill::Gradient(mut g) => {
            for stop in &mut g.stops {
                stop.color = subst(&stop.color, with);
            }
            Fill::Gradient(g)
        }
        Fill::Pattern(mut p) => {
            p.foreground = subst(&p.foreground, with);
            p.background = subst(&p.background, with);
            Fill::Pattern(p)
        }
        other => other,
    }
}

/// The bottom of the paragraph chain: ECMA-376's own defaults.
fn hard_default_paragraph(level: u8) -> ParagraphProps {
    use crate::model::text::{Spacing, TextAlign};
    ParagraphProps {
        level,
        align: Some(TextAlign::Left),
        margin_left: Some(0),
        margin_right: Some(0),
        indent: Some(0),
        default_tab_size: Some(914_400),
        line_spacing: Some(Spacing::Percent(1.0)),
        space_before: Some(Spacing::Points(0.0)),
        space_after: Some(Spacing::Points(0.0)),
        rtl: Some(false),
        ..Default::default()
    }
}

fn hard_default_run() -> RunProps {
    use crate::model::text::{Capitalization, UnderlineStyle};
    RunProps {
        // 18pt is the ECMA-376 default character size.
        size: Some(1800),
        bold: Some(false),
        italic: Some(false),
        underline: Some(UnderlineStyle::None),
        strikethrough: Some(false),
        letter_spacing: Some(0),
        caps: Some(Capitalization::None),
        baseline: Some(0.0),
        latin_font: Some("+mn-lt".to_string()),
        fill: Fill::Solid(ColorRef::scheme(crate::model::color::SchemeColor::Text1)),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::color::{ColorMod, ColorSpec, SchemeColor};
    use crate::model::shape::{PlaceholderType, ShapeTree, SlideLayout, SlideMaster};
    use crate::model::text::{TextAlign, TextBody};
    use crate::model::theme::Theme;
    use crate::model::Slide;

    fn placeholder(kind: PlaceholderType, index: Option<u32>) -> Placeholder {
        Placeholder {
            kind,
            index,
            has_custom_prompt: false,
        }
    }

    fn empty_presentation() -> Presentation {
        use std::io::{Cursor, Write};
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            w.start_file("[Content_Types].xml", opts).expect("start");
            w.write_all(
                br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#,
            )
            .expect("write");
            w.finish().expect("finish");
        }
        let pkg = crate::opc::Package::open(buf).expect("open");
        Presentation::new(pkg, 12_192_000, 6_858_000)
    }

    struct Fixture {
        pres: Presentation,
        chain: SlideChain,
    }

    fn fixture(slide: Slide, layout: SlideLayout, master: SlideMaster, theme: Theme) -> Fixture {
        Fixture {
            pres: empty_presentation(),
            chain: SlideChain {
                slide: Rc::new(slide),
                layout: Some(Rc::new(layout)),
                master: Some(Rc::new(master)),
                theme: Rc::new(theme),
            },
        }
    }

    #[test]
    fn a_title_with_no_xfrm_takes_its_box_from_the_layout_placeholder() {
        let slide_shape = Shape {
            id: 2,
            placeholder: Some(placeholder(PlaceholderType::Title, None)),
            ..Default::default()
        };
        let layout = SlideLayout {
            tree: ShapeTree {
                shapes: vec![Shape {
                    id: 10,
                    placeholder: Some(placeholder(PlaceholderType::Title, None)),
                    transform: Transform2D {
                        offset_x: 838_200,
                        offset_y: 365_125,
                        extent_x: 10_515_600,
                        extent_y: 1_325_563,
                        specified: true,
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                root_transform: None,
            },
            ..Default::default()
        };
        let f = fixture(
            Slide::default(),
            layout,
            SlideMaster::default(),
            Theme::default(),
        );
        let r = Resolver::new(&f.pres, &f.chain);
        let ancestors = r.placeholder_ancestors(&slide_shape);
        assert_eq!(ancestors.len(), 1);
        let t = r.transform(&slide_shape, &ancestors);
        assert_eq!(t.offset_x, 838_200);
        assert_eq!(t.extent_y, 1_325_563);
    }

    #[test]
    fn a_shape_that_specifies_its_own_box_ignores_the_placeholder() {
        let shape = Shape {
            placeholder: Some(placeholder(PlaceholderType::Title, None)),
            transform: Transform2D {
                offset_x: 1,
                extent_x: 2,
                specified: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let layout = SlideLayout {
            tree: ShapeTree {
                shapes: vec![Shape {
                    placeholder: Some(placeholder(PlaceholderType::Title, None)),
                    transform: Transform2D {
                        offset_x: 999,
                        specified: true,
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                root_transform: None,
            },
            ..Default::default()
        };
        let f = fixture(
            Slide::default(),
            layout,
            SlideMaster::default(),
            Theme::default(),
        );
        let r = Resolver::new(&f.pres, &f.chain);
        let ancestors = r.placeholder_ancestors(&shape);
        assert_eq!(r.transform(&shape, &ancestors).offset_x, 1);
    }

    #[test]
    fn rotation_stays_with_the_shape_even_when_the_box_is_inherited() {
        let shape = Shape {
            placeholder: Some(placeholder(PlaceholderType::Body, None)),
            transform: Transform2D {
                rotation: 2_700_000,
                flip_h: true,
                specified: false,
                ..Default::default()
            },
            ..Default::default()
        };
        let layout = SlideLayout {
            tree: ShapeTree {
                shapes: vec![Shape {
                    placeholder: Some(placeholder(PlaceholderType::Body, None)),
                    transform: Transform2D {
                        offset_x: 500,
                        extent_x: 100,
                        rotation: 0,
                        specified: true,
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                root_transform: None,
            },
            ..Default::default()
        };
        let f = fixture(
            Slide::default(),
            layout,
            SlideMaster::default(),
            Theme::default(),
        );
        let r = Resolver::new(&f.pres, &f.chain);
        let ancestors = r.placeholder_ancestors(&shape);
        let t = r.transform(&shape, &ancestors);
        assert_eq!(t.offset_x, 500, "box comes from the layout");
        assert_eq!(t.rotation, 2_700_000, "rotation stays with the slide shape");
        assert!(t.flip_h);
    }

    #[test]
    fn a_style_reference_pulls_its_fill_from_the_theme_with_phclr_substituted() {
        let mut theme = Theme::default();
        theme.formats.fill_styles = vec![Fill::Solid(ColorRef {
            spec: ColorSpec::Placeholder,
            mods: vec![ColorMod::Shade(0.5)],
        })];
        let shape = Shape {
            style_ref: Some(StyleRef {
                fill_idx: Some(1),
                fill_color: Some(ColorRef::scheme(SchemeColor::Accent2)),
                ..Default::default()
            }),
            ..Default::default()
        };
        let f = fixture(
            Slide::default(),
            SlideLayout::default(),
            SlideMaster::default(),
            theme,
        );
        let r = Resolver::new(&f.pres, &f.chain);
        match r.fill(&shape, &[]) {
            Fill::Solid(c) => {
                assert_eq!(c.spec, ColorSpec::Scheme(SchemeColor::Accent2));
                assert_eq!(
                    c.mods,
                    vec![ColorMod::Shade(0.5)],
                    "the style's own modifier survives"
                );
            }
            other => panic!("expected a solid fill, got {other:?}"),
        }
    }

    #[test]
    fn a_shapes_own_fill_beats_its_style_reference() {
        let mut theme = Theme::default();
        theme.formats.fill_styles = vec![Fill::Solid(ColorRef::scheme(SchemeColor::Accent1))];
        let shape = Shape {
            fill: Fill::Solid(ColorRef::srgb(Color::rgb(1, 2, 3))),
            style_ref: Some(StyleRef {
                fill_idx: Some(1),
                ..Default::default()
            }),
            ..Default::default()
        };
        let f = fixture(
            Slide::default(),
            SlideLayout::default(),
            SlideMaster::default(),
            theme,
        );
        let r = Resolver::new(&f.pres, &f.chain);
        match r.fill(&shape, &[]) {
            Fill::Solid(c) => assert_eq!(c.spec, ColorSpec::Srgb(Color::rgb(1, 2, 3))),
            other => panic!("expected the shape's own fill, got {other:?}"),
        }
    }

    #[test]
    fn an_explicit_nofill_is_not_overridden_by_the_layout() {
        let shape = Shape {
            fill: Fill::NoFill,
            placeholder: Some(placeholder(PlaceholderType::Body, None)),
            ..Default::default()
        };
        let layout = SlideLayout {
            tree: ShapeTree {
                shapes: vec![Shape {
                    placeholder: Some(placeholder(PlaceholderType::Body, None)),
                    fill: Fill::Solid(ColorRef::srgb(Color::WHITE)),
                    ..Default::default()
                }],
                root_transform: None,
            },
            ..Default::default()
        };
        let f = fixture(
            Slide::default(),
            layout,
            SlideMaster::default(),
            Theme::default(),
        );
        let r = Resolver::new(&f.pres, &f.chain);
        let ancestors = r.placeholder_ancestors(&shape);
        assert_eq!(r.fill(&shape, &ancestors), Fill::NoFill);
    }

    #[test]
    fn paragraph_properties_fall_through_to_the_master_body_style() {
        let mut master = SlideMaster::default();
        master.body_style.set_level(
            1,
            ParagraphProps {
                align: Some(TextAlign::Right),
                margin_left: Some(742_950),
                ..Default::default()
            },
        );
        let shape = Shape {
            placeholder: Some(placeholder(PlaceholderType::Body, None)),
            text: Some(TextBody::default()),
            ..Default::default()
        };
        let f = fixture(
            Slide::default(),
            SlideLayout::default(),
            master,
            Theme::default(),
        );
        let r = Resolver::new(&f.pres, &f.chain);
        let props = r.paragraph_props(&shape, &[], &ParagraphProps::default(), 1);
        assert_eq!(props.align, Some(TextAlign::Right));
        assert_eq!(props.margin_left, Some(742_950));
    }

    #[test]
    fn the_paragraphs_own_alignment_wins_over_every_inherited_level() {
        let mut master = SlideMaster::default();
        master.body_style.set_level(
            0,
            ParagraphProps {
                align: Some(TextAlign::Right),
                ..Default::default()
            },
        );
        let shape = Shape {
            placeholder: Some(placeholder(PlaceholderType::Body, None)),
            ..Default::default()
        };
        let f = fixture(
            Slide::default(),
            SlideLayout::default(),
            master,
            Theme::default(),
        );
        let r = Resolver::new(&f.pres, &f.chain);
        let own = ParagraphProps {
            align: Some(TextAlign::Center),
            ..Default::default()
        };
        assert_eq!(
            r.paragraph_props(&shape, &[], &own, 0).align,
            Some(TextAlign::Center)
        );
    }

    #[test]
    fn hard_defaults_apply_when_nothing_in_the_chain_says_anything() {
        let f = fixture(
            Slide::default(),
            SlideLayout::default(),
            SlideMaster::default(),
            Theme::default(),
        );
        let r = Resolver::new(&f.pres, &f.chain);
        let props = r.paragraph_props(&Shape::default(), &[], &ParagraphProps::default(), 0);
        assert_eq!(props.align, Some(TextAlign::Left));
        let run = r.run_props(&RunProps::default(), &props, &Shape::default(), &[]);
        assert_eq!(run.size, Some(1800));
        assert_eq!(run.bold, Some(false));
        assert_eq!(run.latin_font.as_deref(), Some("+mn-lt"));
    }

    #[test]
    fn theme_font_references_expand_to_the_real_typeface() {
        let mut theme = Theme::default();
        theme.fonts.major.latin = "Century Gothic".into();
        theme.fonts.minor.latin = "Verdana".into();
        let f = fixture(
            Slide::default(),
            SlideLayout::default(),
            SlideMaster::default(),
            theme,
        );
        let r = Resolver::new(&f.pres, &f.chain);
        assert_eq!(r.font_family(Some("+mj-lt")), "Century Gothic");
        assert_eq!(r.font_family(Some("+mn-lt")), "Verdana");
        assert_eq!(r.font_family(Some("Georgia")), "Georgia");
        assert_eq!(
            r.font_family(None),
            "Verdana",
            "no request means the body font"
        );
    }

    #[test]
    fn an_empty_theme_slot_falls_back_to_the_minor_latin_font() {
        let mut theme = Theme::default();
        theme.fonts.minor.latin = "Verdana".into();
        theme.fonts.major.east_asian = String::new();
        let f = fixture(
            Slide::default(),
            SlideLayout::default(),
            SlideMaster::default(),
            theme,
        );
        let r = Resolver::new(&f.pres, &f.chain);
        assert_eq!(r.font_family(Some("+mj-ea")), "Verdana");
    }

    #[test]
    fn layout_and_master_placeholders_are_not_drawn_as_background_shapes() {
        assert!(!Resolver::is_background_shape(&Shape {
            placeholder: Some(placeholder(PlaceholderType::Title, None)),
            ..Default::default()
        }));
        assert!(!Resolver::is_background_shape(&Shape {
            hidden: true,
            ..Default::default()
        }));
        assert!(Resolver::is_background_shape(&Shape::default()));
    }
}
