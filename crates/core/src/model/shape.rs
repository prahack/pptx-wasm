//! Shapes as they appear in a `<p:spTree>`.

use super::fill::{Effects, Fill, Line};
use super::geometry::{Geometry, GroupTransform, Transform2D};
use super::table::Table;
use super::text::TextBody;

/// Placeholder types from `<p:ph type="..">`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PlaceholderType {
    Title,
    CenteredTitle,
    Subtitle,
    /// `body` — the workhorse content placeholder.
    #[default]
    Body,
    /// `obj` — content that could be a table/chart/picture.
    Object,
    Chart,
    Table,
    ClipArt,
    Diagram,
    Media,
    SlideImage,
    Picture,
    Header,
    Footer,
    SlideNumber,
    DateTime,
}

impl PlaceholderType {
    pub fn parse(s: &str) -> PlaceholderType {
        match s {
            "title" => PlaceholderType::Title,
            "ctrTitle" => PlaceholderType::CenteredTitle,
            "subTitle" => PlaceholderType::Subtitle,
            "body" => PlaceholderType::Body,
            "obj" => PlaceholderType::Object,
            "chart" => PlaceholderType::Chart,
            "tbl" => PlaceholderType::Table,
            "clipArt" => PlaceholderType::ClipArt,
            "dgm" => PlaceholderType::Diagram,
            "media" => PlaceholderType::Media,
            "sldImg" => PlaceholderType::SlideImage,
            "pic" => PlaceholderType::Picture,
            "hdr" => PlaceholderType::Header,
            "ftr" => PlaceholderType::Footer,
            "sldNum" => PlaceholderType::SlideNumber,
            "dt" => PlaceholderType::DateTime,
            _ => PlaceholderType::Body,
        }
    }

    /// Title-ish placeholders share a slot: a slide's `title` matches a layout's
    /// `ctrTitle` and vice versa, which is how a title-slide layout still binds.
    pub fn is_title(self) -> bool {
        matches!(self, PlaceholderType::Title | PlaceholderType::CenteredTitle)
    }

    /// Which `<p:txStyles>` block on the master supplies this placeholder's defaults.
    pub fn text_style(self) -> MasterTextStyle {
        if self.is_title() {
            MasterTextStyle::Title
        } else {
            match self {
                PlaceholderType::Body
                | PlaceholderType::Subtitle
                | PlaceholderType::Object
                | PlaceholderType::Table
                | PlaceholderType::Chart
                | PlaceholderType::Diagram
                | PlaceholderType::ClipArt
                | PlaceholderType::Media
                | PlaceholderType::Picture
                | PlaceholderType::SlideImage => MasterTextStyle::Body,
                _ => MasterTextStyle::Other,
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MasterTextStyle {
    Title,
    Body,
    Other,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Placeholder {
    pub kind: PlaceholderType,
    /// `idx` — the tie-breaker when a layout has several placeholders of one type.
    pub index: Option<u32>,
    /// `<p:ph hasCustomPrompt="1">`, which suppresses the layout's prompt text.
    pub has_custom_prompt: bool,
}

impl Placeholder {
    /// Whether this placeholder should bind to `other` on a layout or master.
    ///
    /// PowerPoint matches on index first and falls back to type. Getting the priority
    /// backwards makes two body placeholders on a layout swap their formatting, which is
    /// exactly the kind of failure M4 exists to prevent.
    pub fn matches(&self, other: &Placeholder) -> bool {
        match (self.index, other.index) {
            (Some(a), Some(b)) if a == b => true,
            _ => {
                self.kind == other.kind
                    || (self.kind.is_title() && other.kind.is_title())
                    // A slide's `body` binds to a layout's `obj` and vice versa.
                    || (self.is_content_like() && other.is_content_like())
            }
        }
    }

    fn is_content_like(&self) -> bool {
        matches!(
            self.kind,
            PlaceholderType::Body
                | PlaceholderType::Object
                | PlaceholderType::Table
                | PlaceholderType::Chart
                | PlaceholderType::Diagram
        )
    }
}

/// A `<p:pic>` — a picture and its crop.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Picture {
    /// Relationship id of the image part.
    pub embed_id: Option<String>,
    /// Source crop as 0..1 insets from left/top/right/bottom.
    pub src_rect: [f32; 4],
    /// `<a:alphaModFix>` transparency, 0..1.
    pub alpha: f32,
    /// Alternative text, exposed to the accessibility layer.
    pub alt_text: String,
}

/// Content of a `<p:graphicFrame>`. Diagrams and OLE objects are recognised but not
/// rendered natively — they carry a fallback picture in the file, which is what we draw.
#[derive(Debug, Clone, PartialEq)]
pub enum GraphicContent {
    Table(Box<Table>),
    /// Relationship id of the chart part.
    Chart(String),
    /// Anything else, with the relationship id of its fallback image if it has one.
    Unsupported {
        kind: String,
        fallback_image: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ShapeKind {
    /// `<p:sp>` — an autoshape, possibly with text.
    Auto,
    /// `<p:pic>`
    Picture(Box<Picture>),
    /// `<p:grpSp>` — children are in the group's child coordinate space.
    Group(Vec<Shape>),
    /// `<p:cxnSp>` — a connector. Rendered like an autoshape but never filled.
    Connector,
    /// `<p:graphicFrame>`
    Graphic(Box<GraphicContent>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub id: u32,
    pub name: String,
    pub kind: ShapeKind,
    pub transform: Transform2D,
    /// Present only for groups; carries the child coordinate space.
    pub group_transform: Option<GroupTransform>,
    pub geometry: Geometry,
    pub fill: Fill,
    pub line: Line,
    pub effects: Effects,
    pub text: Option<TextBody>,
    pub placeholder: Option<Placeholder>,
    /// `<p:cNvPr hidden="1">` or a `<p:nvPr>` with no visible content.
    pub hidden: bool,
    /// `<p:style>` — indices into the theme's format scheme.
    pub style_ref: Option<StyleRef>,
    /// Alt text from `<p:cNvPr descr>`.
    pub description: String,
}

impl Default for Shape {
    fn default() -> Self {
        Shape {
            id: 0,
            name: String::new(),
            kind: ShapeKind::Auto,
            transform: Transform2D::default(),
            group_transform: None,
            geometry: Geometry::None,
            fill: Fill::Inherit,
            line: Line::default(),
            effects: Effects::default(),
            text: None,
            placeholder: None,
            hidden: false,
            style_ref: None,
            description: String::new(),
        }
    }
}

impl Shape {
    /// Depth-first walk over this shape and its descendants.
    pub fn walk(&self, f: &mut impl FnMut(&Shape)) {
        f(self);
        if let ShapeKind::Group(children) = &self.kind {
            for c in children {
                c.walk(f);
            }
        }
    }

    pub fn is_group(&self) -> bool {
        matches!(self.kind, ShapeKind::Group(_))
    }

    /// All text on this shape and its descendants.
    pub fn plain_text(&self) -> String {
        let mut parts = Vec::new();
        self.walk(&mut |s| {
            if let Some(t) = &s.text {
                let text = t.plain_text();
                if !text.trim().is_empty() {
                    parts.push(text);
                }
            }
        });
        parts.join("\n")
    }
}

/// `<p:style>` — references into the theme's format scheme plus a font and colour ref.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StyleRef {
    /// 1-based index into `<a:fillStyleLst>`; 0 means no fill style.
    pub fill_idx: Option<u32>,
    pub fill_color: Option<super::color::ColorRef>,
    pub line_idx: Option<u32>,
    pub line_color: Option<super::color::ColorRef>,
    pub effect_idx: Option<u32>,
    pub effect_color: Option<super::color::ColorRef>,
    /// `<a:fontRef idx="minor|major|none">`
    pub font_kind: Option<String>,
    pub font_color: Option<super::color::ColorRef>,
}

/// A slide's background, which may come from the slide, its layout, or its master.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Background {
    /// `<p:bgPr>` — an explicit fill.
    Fill(Fill),
    /// `<p:bgRef idx="1001">` — an index into the theme's background fill styles.
    Reference {
        idx: u32,
        color: super::color::ColorRef,
    },
    #[default]
    Inherit,
}

/// A shape tree plus the properties that apply to all of it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ShapeTree {
    pub shapes: Vec<Shape>,
    /// The `<p:grpSpPr>` on the tree root, which can carry a transform for the whole slide.
    pub root_transform: Option<GroupTransform>,
}

impl ShapeTree {
    /// Depth-first walk over every shape in the tree.
    pub fn walk(&self, f: &mut impl FnMut(&Shape)) {
        for s in &self.shapes {
            s.walk(f);
        }
    }

    /// The first shape bound to a matching placeholder, searching nested groups too.
    ///
    /// An index match anywhere in the tree beats a type-only match earlier in it —
    /// otherwise a layout with `body idx="1"` and `body idx="2"` binds both of the
    /// slide's body placeholders to the first one.
    pub fn find_placeholder(&self, want: &Placeholder) -> Option<&Shape> {
        fn search<'a>(
            shapes: &'a [Shape],
            want: &Placeholder,
            by_index: &mut Option<&'a Shape>,
            by_type: &mut Option<&'a Shape>,
        ) {
            for s in shapes {
                if let Some(ph) = &s.placeholder {
                    if by_index.is_none() && want.index.is_some() && ph.index == want.index {
                        *by_index = Some(s);
                    }
                    if by_type.is_none() && want.matches(ph) {
                        *by_type = Some(s);
                    }
                }
                if let ShapeKind::Group(kids) = &s.kind {
                    search(kids, want, by_index, by_type);
                }
            }
        }
        let mut by_index = None;
        let mut by_type = None;
        search(&self.shapes, want, &mut by_index, &mut by_type);
        by_index.or(by_type)
    }

    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }
}

/// A slide, its layout link, and its own overrides.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Slide {
    /// Part name, used as the cache key and for diagnostics.
    pub part_name: String,
    pub tree: ShapeTree,
    pub background: Background,
    /// Part name of this slide's layout, if it has one.
    pub layout_part: Option<String>,
    /// `<p:sld showMasterSp="0">` suppresses the master's non-placeholder shapes.
    pub show_master_shapes: bool,
    /// Speaker notes, if a notes slide is attached.
    pub notes: Option<String>,
}

/// A slide layout.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SlideLayout {
    pub part_name: String,
    pub tree: ShapeTree,
    pub background: Background,
    pub master_part: Option<String>,
    /// `<p:sldLayout type="..">`, e.g. `title`, `obj`, `blank`.
    pub layout_type: String,
    pub show_master_shapes: bool,
}

/// A slide master.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SlideMaster {
    pub part_name: String,
    pub tree: ShapeTree,
    pub background: Background,
    pub theme_part: Option<String>,
    /// `<p:txStyles>` — the per-level defaults for title, body and other placeholders.
    pub title_style: super::text::ListStyle,
    pub body_style: super::text::ListStyle,
    pub other_style: super::text::ListStyle,
    /// `<p:clrMap>` — maps `tx1`/`bg1`/… onto the theme's `dk1`/`lt1`/… slots.
    pub color_map: ColorMap,
}

/// `<p:clrMap bg1="lt1" tx1="dk1" .../>`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorMap {
    pub background1: super::color::SchemeColor,
    pub text1: super::color::SchemeColor,
    pub background2: super::color::SchemeColor,
    pub text2: super::color::SchemeColor,
}

impl Default for ColorMap {
    fn default() -> Self {
        use super::color::SchemeColor as S;
        ColorMap {
            background1: S::Light1,
            text1: S::Dark1,
            background2: S::Light2,
            text2: S::Dark2,
        }
    }
}

impl ColorMap {
    /// Applies the map, turning a `tx1`-style reference into the theme slot it names.
    pub fn resolve(&self, slot: super::color::SchemeColor) -> super::color::SchemeColor {
        use super::color::SchemeColor as S;
        match slot {
            S::Background1 => self.background1,
            S::Text1 => self.text1,
            S::Background2 => self.background2,
            S::Text2 => self.text2,
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ph(kind: PlaceholderType, index: Option<u32>) -> Placeholder {
        Placeholder {
            kind,
            index,
            has_custom_prompt: false,
        }
    }

    #[test]
    fn title_variants_bind_to_each_other() {
        assert!(ph(PlaceholderType::Title, None).matches(&ph(PlaceholderType::CenteredTitle, None)));
        assert!(PlaceholderType::CenteredTitle.is_title());
        assert_eq!(PlaceholderType::CenteredTitle.text_style(), MasterTextStyle::Title);
    }

    #[test]
    fn body_and_obj_placeholders_are_interchangeable() {
        assert!(ph(PlaceholderType::Body, None).matches(&ph(PlaceholderType::Object, None)));
        assert!(ph(PlaceholderType::Object, None).matches(&ph(PlaceholderType::Table, None)));
    }

    #[test]
    fn a_title_does_not_bind_to_a_footer() {
        assert!(!ph(PlaceholderType::Title, None).matches(&ph(PlaceholderType::Footer, None)));
        assert_eq!(PlaceholderType::Footer.text_style(), MasterTextStyle::Other);
    }

    #[test]
    fn matching_index_beats_a_mismatched_type() {
        assert!(ph(PlaceholderType::Body, Some(2)).matches(&ph(PlaceholderType::Picture, Some(2))));
    }

    #[test]
    fn two_body_placeholders_bind_by_index_not_document_order() {
        let tree = ShapeTree {
            shapes: vec![
                Shape {
                    id: 1,
                    name: "first".into(),
                    placeholder: Some(ph(PlaceholderType::Body, Some(1))),
                    ..Default::default()
                },
                Shape {
                    id: 2,
                    name: "second".into(),
                    placeholder: Some(ph(PlaceholderType::Body, Some(2))),
                    ..Default::default()
                },
            ],
            root_transform: None,
        };
        let found = tree
            .find_placeholder(&ph(PlaceholderType::Body, Some(2)))
            .expect("should find idx=2");
        assert_eq!(found.id, 2, "must not fall back to the first body placeholder");
    }

    #[test]
    fn placeholder_lookup_descends_into_groups() {
        let tree = ShapeTree {
            shapes: vec![Shape {
                id: 1,
                kind: ShapeKind::Group(vec![Shape {
                    id: 7,
                    placeholder: Some(ph(PlaceholderType::Title, None)),
                    ..Default::default()
                }]),
                ..Default::default()
            }],
            root_transform: None,
        };
        assert_eq!(
            tree.find_placeholder(&ph(PlaceholderType::Title, None)).map(|s| s.id),
            Some(7)
        );
    }

    #[test]
    fn colour_map_redirects_the_text_and_background_slots() {
        use super::super::color::SchemeColor as S;
        let map = ColorMap {
            background1: S::Dark1,
            text1: S::Light1,
            background2: S::Dark2,
            text2: S::Light2,
        };
        assert_eq!(map.resolve(S::Background1), S::Dark1);
        assert_eq!(map.resolve(S::Text1), S::Light1);
        // Accents pass through untouched.
        assert_eq!(map.resolve(S::Accent3), S::Accent3);
    }

    #[test]
    fn walking_a_group_visits_children_after_the_parent() {
        let s = Shape {
            id: 1,
            kind: ShapeKind::Group(vec![
                Shape {
                    id: 2,
                    ..Default::default()
                },
                Shape {
                    id: 3,
                    kind: ShapeKind::Group(vec![Shape {
                        id: 4,
                        ..Default::default()
                    }]),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        };
        let mut seen = Vec::new();
        s.walk(&mut |sh| seen.push(sh.id));
        assert_eq!(seen, vec![1, 2, 3, 4]);
    }
}
