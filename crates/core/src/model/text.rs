//! The text model: body → paragraphs → runs, with properties still unresolved.
//!
//! Every property is an `Option`. That is the whole point: a `None` means "not specified
//! here, ask the next level up", and the resolution chain in `layout::inherit` depends on
//! being able to tell an unset property from one explicitly set to a default value.

use crate::emu::Emu;

use super::color::ColorRef;
use super::fill::{Fill, Line};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
    Justify,
    /// `dist` — justify including the last line.
    Distributed,
    /// `thaiDist`, treated as distributed.
    ThaiDistributed,
}

impl TextAlign {
    pub fn parse(s: &str) -> Option<TextAlign> {
        Some(match s {
            "l" => TextAlign::Left,
            "ctr" => TextAlign::Center,
            "r" => TextAlign::Right,
            "just" | "justLow" => TextAlign::Justify,
            "dist" => TextAlign::Distributed,
            "thaiDist" => TextAlign::ThaiDistributed,
            _ => return None,
        })
    }

    /// Whether the last line of a paragraph is stretched too.
    pub fn justifies_last_line(self) -> bool {
        matches!(self, TextAlign::Distributed | TextAlign::ThaiDistributed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerticalAnchor {
    #[default]
    Top,
    Middle,
    Bottom,
    /// `just`/`dist` distribute the paragraphs across the box.
    Justified,
    Distributed,
}

impl VerticalAnchor {
    pub fn parse(s: &str) -> Option<VerticalAnchor> {
        Some(match s {
            "t" => VerticalAnchor::Top,
            "ctr" => VerticalAnchor::Middle,
            "b" => VerticalAnchor::Bottom,
            "just" => VerticalAnchor::Justified,
            "dist" => VerticalAnchor::Distributed,
            _ => return None,
        })
    }
}

/// Text flow direction inside the box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextDirection {
    #[default]
    Horizontal,
    /// `vert` — rotated 90° clockwise.
    Vertical90,
    /// `vert270` — rotated 90° anticlockwise.
    Vertical270,
    /// `wordArtVert`, stacked upright characters.
    Stacked,
}

impl TextDirection {
    pub fn parse(s: &str) -> Option<TextDirection> {
        Some(match s {
            "horz" => TextDirection::Horizontal,
            "vert" | "eaVert" | "mongolianVert" => TextDirection::Vertical90,
            "vert270" => TextDirection::Vertical270,
            "wordArtVert" | "wordArtVertRtl" => TextDirection::Stacked,
            _ => return None,
        })
    }

    /// Whether the text box's width and height swap for layout purposes.
    pub fn is_rotated(self) -> bool {
        matches!(self, TextDirection::Vertical90 | TextDirection::Vertical270)
    }
}

/// How the box reacts when its text does not fit.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Autofit {
    /// `<a:noAutofit/>` — text overflows.
    #[default]
    None,
    /// `<a:normAutofit fontScale=".." lnSpcReduction=".."/>` — PowerPoint has already
    /// computed the shrink factors and written them into the file, so we apply them
    /// rather than re-deriving them. Re-deriving would disagree with the authoring app
    /// on any deck whose fonts we substitute.
    Shrink {
        font_scale: f32,
        line_space_reduction: f32,
    },
    /// `<a:spAutoFit/>` — the shape grows to fit; the stored extent is already correct
    /// for the authored text, so this behaves like `None` at render time.
    ResizeShape,
}

/// Spacing before/after a paragraph, or between its lines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Spacing {
    /// Fraction of the line height, 1.0 = single.
    Percent(f32),
    /// Absolute, in points.
    Points(f32),
}

/// `<a:bodyPr>` — the geometry of the text box itself.
#[derive(Debug, Clone, PartialEq)]
pub struct BodyProps {
    pub anchor: Option<VerticalAnchor>,
    /// `anchorCtr` centres the text block horizontally within the box, independently of
    /// paragraph alignment.
    pub anchor_center: Option<bool>,
    pub left_inset: Option<Emu>,
    pub top_inset: Option<Emu>,
    pub right_inset: Option<Emu>,
    pub bottom_inset: Option<Emu>,
    pub wrap: Option<bool>,
    pub autofit: Option<Autofit>,
    pub direction: Option<TextDirection>,
    /// Extra rotation of the text within the shape, in 60000ths of a degree.
    pub rotation: Option<i32>,
    /// Number of columns (`numCol`) and the gap between them.
    pub columns: Option<u32>,
    pub column_gap: Option<Emu>,
    /// `upright` keeps text horizontal even when the shape is rotated.
    pub upright: Option<bool>,
}

impl Default for BodyProps {
    fn default() -> Self {
        BodyProps {
            anchor: None,
            anchor_center: None,
            left_inset: None,
            top_inset: None,
            right_inset: None,
            bottom_inset: None,
            wrap: None,
            autofit: None,
            direction: None,
            rotation: None,
            columns: None,
            column_gap: None,
            upright: None,
        }
    }
}

impl BodyProps {
    /// ECMA-376 defaults: 0.1" left/right, 0.05" top/bottom.
    pub const DEFAULT_LEFT_INSET: Emu = 91_440;
    pub const DEFAULT_TOP_INSET: Emu = 45_720;
    pub const DEFAULT_RIGHT_INSET: Emu = 91_440;
    pub const DEFAULT_BOTTOM_INSET: Emu = 45_720;

    pub fn resolved_insets(&self) -> (Emu, Emu, Emu, Emu) {
        (
            self.left_inset.unwrap_or(Self::DEFAULT_LEFT_INSET),
            self.top_inset.unwrap_or(Self::DEFAULT_TOP_INSET),
            self.right_inset.unwrap_or(Self::DEFAULT_RIGHT_INSET),
            self.bottom_inset.unwrap_or(Self::DEFAULT_BOTTOM_INSET),
        )
    }

    /// Fills unspecified fields from `parent`.
    pub fn inherit_from(&mut self, parent: &BodyProps) {
        self.anchor = self.anchor.or(parent.anchor);
        self.anchor_center = self.anchor_center.or(parent.anchor_center);
        self.left_inset = self.left_inset.or(parent.left_inset);
        self.top_inset = self.top_inset.or(parent.top_inset);
        self.right_inset = self.right_inset.or(parent.right_inset);
        self.bottom_inset = self.bottom_inset.or(parent.bottom_inset);
        self.wrap = self.wrap.or(parent.wrap);
        self.autofit = self.autofit.or(parent.autofit);
        self.direction = self.direction.or(parent.direction);
        self.rotation = self.rotation.or(parent.rotation);
        self.columns = self.columns.or(parent.columns);
        self.column_gap = self.column_gap.or(parent.column_gap);
        self.upright = self.upright.or(parent.upright);
    }
}

/// Automatic numbering schemes. Only the ones PowerPoint's bullet UI can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoNumScheme {
    ArabicPeriod,
    ArabicParenR,
    ArabicParenBoth,
    AlphaLcPeriod,
    AlphaUcPeriod,
    AlphaLcParenR,
    AlphaUcParenR,
    RomanLcPeriod,
    RomanUcPeriod,
    RomanLcParenR,
    RomanUcParenR,
}

impl AutoNumScheme {
    pub fn parse(s: &str) -> Option<AutoNumScheme> {
        Some(match s {
            "arabicPeriod" => AutoNumScheme::ArabicPeriod,
            "arabicParenR" => AutoNumScheme::ArabicParenR,
            "arabicParenBoth" => AutoNumScheme::ArabicParenBoth,
            "alphaLcPeriod" => AutoNumScheme::AlphaLcPeriod,
            "alphaUcPeriod" => AutoNumScheme::AlphaUcPeriod,
            "alphaLcParenR" => AutoNumScheme::AlphaLcParenR,
            "alphaUcParenR" => AutoNumScheme::AlphaUcParenR,
            "romanLcPeriod" => AutoNumScheme::RomanLcPeriod,
            "romanUcPeriod" => AutoNumScheme::RomanUcPeriod,
            "romanLcParenR" => AutoNumScheme::RomanLcParenR,
            "romanUcParenR" => AutoNumScheme::RomanUcParenR,
            _ => return None,
        })
    }

    /// Renders `n` (1-based) in this scheme.
    pub fn format(self, n: u32) -> String {
        use AutoNumScheme::*;
        let body = match self {
            ArabicPeriod | ArabicParenR | ArabicParenBoth => n.to_string(),
            AlphaLcPeriod | AlphaLcParenR => alpha(n, false),
            AlphaUcPeriod | AlphaUcParenR => alpha(n, true),
            RomanLcPeriod | RomanLcParenR => roman(n, false),
            RomanUcPeriod | RomanUcParenR => roman(n, true),
        };
        match self {
            ArabicPeriod | AlphaLcPeriod | AlphaUcPeriod | RomanLcPeriod | RomanUcPeriod => {
                format!("{body}.")
            }
            ArabicParenR | AlphaLcParenR | AlphaUcParenR | RomanLcParenR | RomanUcParenR => {
                format!("{body})")
            }
            ArabicParenBoth => format!("({body})"),
        }
    }
}

/// 1 → a, 26 → z, 27 → aa (spreadsheet-column style, which is what PowerPoint uses).
fn alpha(n: u32, upper: bool) -> String {
    if n == 0 {
        return String::new();
    }
    let base = if upper { b'A' } else { b'a' };
    let mut n = n;
    let mut out = Vec::new();
    while n > 0 {
        let rem = ((n - 1) % 26) as u8;
        out.push(base + rem);
        n = (n - 1) / 26;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

fn roman(n: u32, upper: bool) -> String {
    const TABLE: &[(u32, &str)] = &[
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut n = n;
    let mut out = String::new();
    for (value, sym) in TABLE {
        while n >= *value {
            out.push_str(sym);
            n -= value;
        }
    }
    if upper {
        out.to_ascii_uppercase()
    } else {
        out
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BulletKind {
    /// `<a:buNone/>`
    None,
    /// `<a:buChar char="•"/>`
    Char(String),
    /// `<a:buAutoNum type=".." startAt=".."/>`
    AutoNum {
        scheme: AutoNumScheme,
        start_at: u32,
    },
    /// `<a:buBlip>` — a picture bullet, by relationship id.
    Image(String),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Bullet {
    pub kind: Option<BulletKind>,
    /// `<a:buFont typeface=".."/>` — bullet glyphs usually come from Wingdings/Arial
    /// rather than the paragraph's own font.
    pub font: Option<String>,
    /// `<a:buSzPct>` as a fraction, or `<a:buSzPts>` in points.
    pub size_percent: Option<f32>,
    pub size_points: Option<f32>,
    pub color: Option<ColorRef>,
    /// True when `<a:buClrTx/>` said to follow the text colour.
    pub follow_text_color: bool,
}

impl Bullet {
    pub fn is_empty(&self) -> bool {
        self.kind.is_none()
            && self.font.is_none()
            && self.size_percent.is_none()
            && self.size_points.is_none()
            && self.color.is_none()
            && !self.follow_text_color
    }

    pub fn inherit_from(&mut self, parent: &Bullet) {
        if self.kind.is_none() {
            self.kind = parent.kind.clone();
        }
        if self.font.is_none() {
            self.font = parent.font.clone();
        }
        self.size_percent = self.size_percent.or(parent.size_percent);
        self.size_points = self.size_points.or(parent.size_points);
        if self.color.is_none() && !self.follow_text_color {
            self.color = parent.color.clone();
            self.follow_text_color = parent.follow_text_color;
        }
    }
}

/// `<a:pPr>`
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParagraphProps {
    /// Outline level, 0-based (`lvl="0"` is the first).
    pub level: u8,
    pub align: Option<TextAlign>,
    /// Left margin, i.e. the indent of the whole paragraph body.
    pub margin_left: Option<Emu>,
    pub margin_right: Option<Emu>,
    /// First-line indent relative to `margin_left`. Negative gives a hanging indent,
    /// which is how every bulleted list in OOXML is expressed.
    pub indent: Option<Emu>,
    pub default_tab_size: Option<Emu>,
    pub line_spacing: Option<Spacing>,
    pub space_before: Option<Spacing>,
    pub space_after: Option<Spacing>,
    pub bullet: Bullet,
    /// Right-to-left paragraph.
    pub rtl: Option<bool>,
    /// Properties applied to runs in this paragraph that do not override them.
    pub default_run_props: Option<RunProps>,
    /// Explicit tab stops, in EMUs from the text-box left edge.
    pub tab_stops: Vec<Emu>,
}

impl ParagraphProps {
    pub fn inherit_from(&mut self, parent: &ParagraphProps) {
        self.align = self.align.or(parent.align);
        self.margin_left = self.margin_left.or(parent.margin_left);
        self.margin_right = self.margin_right.or(parent.margin_right);
        self.indent = self.indent.or(parent.indent);
        self.default_tab_size = self.default_tab_size.or(parent.default_tab_size);
        self.line_spacing = self.line_spacing.or(parent.line_spacing);
        self.space_before = self.space_before.or(parent.space_before);
        self.space_after = self.space_after.or(parent.space_after);
        self.rtl = self.rtl.or(parent.rtl);
        self.bullet.inherit_from(&parent.bullet);
        if self.tab_stops.is_empty() {
            self.tab_stops = parent.tab_stops.clone();
        }
        match (&mut self.default_run_props, &parent.default_run_props) {
            (Some(child), Some(p)) => child.inherit_from(p),
            (None, Some(p)) => self.default_run_props = Some(p.clone()),
            _ => {}
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnderlineStyle {
    #[default]
    None,
    Single,
    Double,
    Heavy,
    Dotted,
    Dashed,
    Wavy,
    /// Any of the many `words`-suffixed variants: underline skips spaces.
    Words,
}

impl UnderlineStyle {
    pub fn parse(s: &str) -> Option<UnderlineStyle> {
        Some(match s {
            "none" => UnderlineStyle::None,
            "sng" => UnderlineStyle::Single,
            "dbl" => UnderlineStyle::Double,
            "heavy" | "thick" => UnderlineStyle::Heavy,
            "dotted" | "dottedHeavy" => UnderlineStyle::Dotted,
            "dash" | "dashHeavy" | "dashLong" | "dashLongHeavy" => UnderlineStyle::Dashed,
            "wavy" | "wavyHeavy" | "wavyDbl" => UnderlineStyle::Wavy,
            "words" => UnderlineStyle::Words,
            // Every remaining preset is some flavour of single underline.
            _ => UnderlineStyle::Single,
        })
    }

    pub fn is_visible(self) -> bool {
        !matches!(self, UnderlineStyle::None)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Capitalization {
    #[default]
    None,
    All,
    Small,
}

/// `<a:rPr>`
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RunProps {
    /// Size in hundredths of a point, as OOXML stores it (`sz="1800"` = 18pt).
    pub size: Option<i32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<UnderlineStyle>,
    pub strikethrough: Option<bool>,
    /// Character spacing in hundredths of a point; may be negative.
    pub letter_spacing: Option<i32>,
    pub caps: Option<Capitalization>,
    /// Superscript/subscript as a percentage of the font size; positive is up.
    pub baseline: Option<f32>,
    /// `<a:latin typeface=".."/>` and friends. Kept separate because a single run can
    /// legitimately use different faces for Latin, East Asian and complex-script text.
    pub latin_font: Option<String>,
    pub ea_font: Option<String>,
    pub cs_font: Option<String>,
    /// `<a:symbol typeface=".."/>` — forces a symbol face for the whole run.
    pub symbol_font: Option<String>,
    /// Text colour, expressed as a fill so gradient-filled text is representable.
    pub fill: Fill,
    /// Outline on the glyphs themselves.
    pub outline: Option<Line>,
    pub highlight: Option<ColorRef>,
    pub underline_color: Option<ColorRef>,
    /// Relationship id of a hyperlink on this run.
    pub hyperlink: Option<String>,
    /// Language tag, used to pick a script-appropriate font.
    pub language: Option<String>,
}

impl RunProps {
    pub fn size_points(&self) -> Option<f32> {
        self.size.map(|s| s as f32 / 100.0)
    }

    pub fn inherit_from(&mut self, parent: &RunProps) {
        self.size = self.size.or(parent.size);
        self.bold = self.bold.or(parent.bold);
        self.italic = self.italic.or(parent.italic);
        self.underline = self.underline.or(parent.underline);
        self.strikethrough = self.strikethrough.or(parent.strikethrough);
        self.letter_spacing = self.letter_spacing.or(parent.letter_spacing);
        self.caps = self.caps.or(parent.caps);
        self.baseline = self.baseline.or(parent.baseline);
        if self.latin_font.is_none() {
            self.latin_font = parent.latin_font.clone();
        }
        if self.ea_font.is_none() {
            self.ea_font = parent.ea_font.clone();
        }
        if self.cs_font.is_none() {
            self.cs_font = parent.cs_font.clone();
        }
        if self.symbol_font.is_none() {
            self.symbol_font = parent.symbol_font.clone();
        }
        if !self.fill.is_specified() {
            self.fill = parent.fill.clone();
        }
        if self.outline.is_none() {
            self.outline = parent.outline.clone();
        }
        if self.highlight.is_none() {
            self.highlight = parent.highlight.clone();
        }
        if self.underline_color.is_none() {
            self.underline_color = parent.underline_color.clone();
        }
        if self.language.is_none() {
            self.language = parent.language.clone();
        }
    }
}

/// A run is either text, an explicit break, or a field whose value was cached by the
/// authoring app (slide number, date). Fields render their cached text — recomputing a
/// slide number is right, recomputing a date is not.
#[derive(Debug, Clone, PartialEq)]
pub enum Run {
    Text {
        text: String,
        props: RunProps,
    },
    /// `<a:br/>`
    Break {
        props: RunProps,
    },
    Field {
        /// `slidenum`, `datetime1`, etc.
        kind: String,
        /// Text PowerPoint last rendered for this field.
        cached: String,
        props: RunProps,
    },
}

impl Run {
    pub fn props(&self) -> &RunProps {
        match self {
            Run::Text { props, .. } | Run::Break { props } | Run::Field { props, .. } => props,
        }
    }

    pub fn props_mut(&mut self) -> &mut RunProps {
        match self {
            Run::Text { props, .. } | Run::Break { props } | Run::Field { props, .. } => props,
        }
    }

    /// The characters this run contributes to the laid-out line.
    pub fn text(&self) -> &str {
        match self {
            Run::Text { text, .. } => text,
            Run::Field { cached, .. } => cached,
            Run::Break { .. } => "",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Paragraph {
    pub props: ParagraphProps,
    pub runs: Vec<Run>,
    /// `<a:endParaRPr>` — the formatting of the paragraph mark. It decides the height of
    /// an otherwise empty paragraph, which is why an empty `<a:p>` still takes up space.
    pub end_props: Option<RunProps>,
}

impl Paragraph {
    pub fn is_empty(&self) -> bool {
        self.runs.iter().all(|r| r.text().is_empty())
    }

    pub fn plain_text(&self) -> String {
        let mut s = String::new();
        for r in &self.runs {
            match r {
                Run::Break { .. } => s.push('\n'),
                other => s.push_str(other.text()),
            }
        }
        s
    }
}

/// `<p:txBody>` / `<a:txBody>`
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextBody {
    pub body: BodyProps,
    pub paragraphs: Vec<Paragraph>,
    /// `<a:lstStyle>` on the body itself: per-level defaults that sit between the
    /// paragraph's own properties and the placeholder's list style.
    pub list_style: ListStyle,
}

impl TextBody {
    pub fn is_empty(&self) -> bool {
        self.paragraphs.iter().all(Paragraph::is_empty)
    }

    pub fn plain_text(&self) -> String {
        self.paragraphs
            .iter()
            .map(Paragraph::plain_text)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Nine levels of paragraph defaults, as found in `<a:lstStyle>`, `<p:txStyles>` and the
/// theme's default text style.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ListStyle {
    pub levels: [Option<ParagraphProps>; 9],
}

impl ListStyle {
    pub fn level(&self, lvl: u8) -> Option<&ParagraphProps> {
        self.levels.get(lvl as usize).and_then(Option::as_ref)
    }

    pub fn set_level(&mut self, lvl: u8, props: ParagraphProps) {
        if let Some(slot) = self.levels.get_mut(lvl as usize) {
            *slot = Some(props);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.levels.iter().all(Option::is_none)
    }

    /// Merges `parent` underneath `self`, level by level.
    pub fn inherit_from(&mut self, parent: &ListStyle) {
        for (lvl, slot) in self.levels.iter_mut().enumerate() {
            let Some(p) = parent.levels.get(lvl).and_then(Option::as_ref) else {
                continue;
            };
            match slot {
                Some(child) => child.inherit_from(p),
                None => *slot = Some(p.clone()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment_and_anchor_names_parse() {
        assert_eq!(TextAlign::parse("ctr"), Some(TextAlign::Center));
        assert_eq!(TextAlign::parse("just"), Some(TextAlign::Justify));
        assert_eq!(TextAlign::parse("xx"), None);
        assert_eq!(VerticalAnchor::parse("b"), Some(VerticalAnchor::Bottom));
    }

    #[test]
    fn default_insets_are_the_ecma_values() {
        let (l, t, r, b) = BodyProps::default().resolved_insets();
        assert_eq!((l, t, r, b), (91_440, 45_720, 91_440, 45_720));
    }

    #[test]
    fn explicit_zero_inset_survives_defaulting() {
        let bp = BodyProps {
            left_inset: Some(0),
            ..Default::default()
        };
        assert_eq!(bp.resolved_insets().0, 0);
    }

    #[test]
    fn run_props_inherit_only_unset_fields() {
        let mut child = RunProps {
            size: Some(2400),
            ..Default::default()
        };
        let parent = RunProps {
            size: Some(1800),
            bold: Some(true),
            latin_font: Some("Calibri".into()),
            ..Default::default()
        };
        child.inherit_from(&parent);
        assert_eq!(child.size, Some(2400));
        assert_eq!(child.bold, Some(true));
        assert_eq!(child.latin_font.as_deref(), Some("Calibri"));
    }

    #[test]
    fn an_explicit_false_beats_an_inherited_true() {
        let mut child = RunProps {
            bold: Some(false),
            ..Default::default()
        };
        child.inherit_from(&RunProps {
            bold: Some(true),
            ..Default::default()
        });
        assert_eq!(child.bold, Some(false), "explicit b=\"0\" must not be overwritten");
    }

    #[test]
    fn arabic_alpha_and_roman_numbering() {
        assert_eq!(AutoNumScheme::ArabicPeriod.format(3), "3.");
        assert_eq!(AutoNumScheme::ArabicParenBoth.format(12), "(12)");
        assert_eq!(AutoNumScheme::AlphaLcParenR.format(1), "a)");
        assert_eq!(AutoNumScheme::AlphaLcParenR.format(27), "aa)");
        assert_eq!(AutoNumScheme::AlphaUcPeriod.format(26), "Z.");
        assert_eq!(AutoNumScheme::RomanUcPeriod.format(4), "IV.");
        assert_eq!(AutoNumScheme::RomanLcPeriod.format(1994), "mcmxciv.");
    }

    #[test]
    fn list_styles_merge_level_by_level() {
        let mut child = ListStyle::default();
        child.set_level(
            0,
            ParagraphProps {
                align: Some(TextAlign::Center),
                ..Default::default()
            },
        );
        let mut parent = ListStyle::default();
        parent.set_level(
            0,
            ParagraphProps {
                align: Some(TextAlign::Left),
                margin_left: Some(342_900),
                ..Default::default()
            },
        );
        parent.set_level(
            1,
            ParagraphProps {
                margin_left: Some(742_950),
                ..Default::default()
            },
        );
        child.inherit_from(&parent);

        let l0 = child.level(0).expect("level 0");
        assert_eq!(l0.align, Some(TextAlign::Center), "child alignment wins");
        assert_eq!(l0.margin_left, Some(342_900), "margin comes from parent");
        assert_eq!(child.level(1).and_then(|p| p.margin_left), Some(742_950));
        assert!(child.level(8).is_none());
    }

    #[test]
    fn paragraph_plain_text_turns_breaks_into_newlines() {
        let p = Paragraph {
            runs: vec![
                Run::Text {
                    text: "a".into(),
                    props: RunProps::default(),
                },
                Run::Break {
                    props: RunProps::default(),
                },
                Run::Text {
                    text: "b".into(),
                    props: RunProps::default(),
                },
            ],
            ..Default::default()
        };
        assert_eq!(p.plain_text(), "a\nb");
        assert!(!p.is_empty());
    }

    #[test]
    fn level_index_beyond_nine_is_none_rather_than_a_panic() {
        let mut ls = ListStyle::default();
        ls.set_level(200, ParagraphProps::default());
        assert!(ls.level(200).is_none());
        assert!(ls.is_empty());
    }
}
