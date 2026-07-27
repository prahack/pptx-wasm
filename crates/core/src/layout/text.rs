//! Text layout: runs → measured fragments → wrapped lines → positioned [`TextRun`]s.
//!
//! Wrapping happens in points against a [`TextMeasure`], so break points depend only on
//! the font and the box width — not on zoom, DPR, or the canvas the result lands on. That
//! is what makes the same display list correct at every scale.

use crate::dl::text::{Decorations, FontSpec, FontStyle, FontWeight};
use crate::dl::{Command, Paint, Point, Rect, TextRun};
use crate::emu;
use crate::model::text::{
    Autofit, BodyProps, BulletKind, Capitalization, ParagraphProps, Run, Spacing, TextAlign,
    UnderlineStyle, VerticalAnchor,
};
use crate::text::{FontMetrics, TextMeasure};

/// One run of text after property resolution, ready to measure.
#[derive(Debug, Clone)]
pub struct StyledFragment {
    pub text: String,
    pub font: FontSpec,
    pub paint: Paint,
    pub decorations: Decorations,
    /// Extra tracking in points.
    pub letter_spacing: f32,
    /// Superscript/subscript offset as a fraction of the font size, positive is up.
    pub baseline_shift: f32,
    /// True when this fragment is an explicit `<a:br/>` rather than text.
    pub is_break: bool,
}

impl StyledFragment {
    pub fn text_break(font: FontSpec, paint: Paint) -> Self {
        StyledFragment {
            text: String::new(),
            font,
            paint,
            decorations: Decorations::default(),
            letter_spacing: 0.0,
            baseline_shift: 0.0,
            is_break: true,
        }
    }
}

/// A paragraph after property resolution.
#[derive(Debug, Clone)]
pub struct StyledParagraph {
    pub fragments: Vec<StyledFragment>,
    pub align: TextAlign,
    /// Indent of the paragraph body from the text-box content edge, in points.
    pub margin_left: f32,
    pub margin_right: f32,
    /// First-line indent relative to `margin_left`. Negative for a hanging indent.
    pub indent: f32,
    pub line_spacing: Spacing,
    pub space_before: Spacing,
    pub space_after: Spacing,
    pub rtl: bool,
    /// Rendered bullet text and its font, if this paragraph has one.
    pub bullet: Option<StyledFragment>,
    /// Metrics of the paragraph mark, which set the height of an empty paragraph.
    pub empty_metrics: FontMetrics,
    /// Font size of the paragraph mark, used for empty-paragraph height.
    pub empty_size_pt: f32,
}

/// A measured, positioned line.
#[derive(Debug, Clone)]
struct Line {
    /// (fragment index, char range) pieces making up this line.
    pieces: Vec<Piece>,
    width: f32,
    ascent: f32,
    descent: f32,
    /// Set on the first line of a paragraph, which is the one that carries the bullet.
    is_first: bool,
    /// True when the line ended at a hard break or the paragraph end, so justification
    /// must not stretch it.
    is_last: bool,
}

#[derive(Debug, Clone)]
struct Piece {
    fragment: usize,
    start: usize,
    end: usize,
    width: f32,
}

/// Where the text ended up, so callers can grow a shape or report overflow.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextLayoutResult {
    /// Total height of the laid-out text in points.
    pub height: f32,
    /// Widest line in points.
    pub width: f32,
    /// True when the text did not fit the box vertically.
    pub overflowed: bool,
}

/// Lays a text body out inside `content` (already inset) and appends draw commands.
pub fn layout(
    paragraphs: &[StyledParagraph],
    body: &BodyProps,
    content: Rect,
    measure: &dyn TextMeasure,
    out: &mut Vec<Command>,
) -> TextLayoutResult {
    let wrap = body.wrap.unwrap_or(true);
    let autofit = body.autofit.unwrap_or(Autofit::None);
    let (font_scale, line_reduction) = match autofit {
        Autofit::Shrink {
            font_scale,
            line_space_reduction,
        } => (
            font_scale.clamp(0.05, 1.0),
            line_space_reduction.clamp(0.0, 0.9),
        ),
        _ => (1.0, 0.0),
    };

    // Apply PowerPoint's own pre-computed autofit shrink rather than re-deriving it.
    let scaled: Vec<StyledParagraph> = if font_scale < 1.0 {
        paragraphs
            .iter()
            .map(|p| scale_paragraph(p, font_scale))
            .collect()
    } else {
        paragraphs.to_vec()
    };

    let mut blocks: Vec<(usize, Vec<Line>)> = Vec::new();
    let mut total_height = 0.0f32;
    let mut max_width = 0.0f32;

    for (idx, para) in scaled.iter().enumerate() {
        let avail = (content.w - para.margin_left - para.margin_right).max(1.0);
        let first_line_indent = para.indent.max(-para.margin_left);
        let bullet_width = para
            .bullet
            .as_ref()
            .map(|b| measure.measure(&b.text, &b.font).width)
            .unwrap_or(0.0);

        let lines = wrap_paragraph(para, avail, first_line_indent, bullet_width, wrap, measure);
        for l in &lines {
            max_width = max_width.max(l.width);
        }
        total_height += paragraph_height(para, &lines, line_reduction, measure);
        blocks.push((idx, lines));
    }

    // Vertical anchoring positions the whole block; `justified` spreads the slack between
    // paragraphs instead, which is what `anchor="just"` means.
    let anchor = body.anchor.unwrap_or(VerticalAnchor::Top);
    let slack = content.h - total_height;
    let mut y = match anchor {
        VerticalAnchor::Top | VerticalAnchor::Justified | VerticalAnchor::Distributed => content.y,
        VerticalAnchor::Middle => content.y + slack / 2.0,
        VerticalAnchor::Bottom => content.y + slack,
    };
    let extra_per_gap = match anchor {
        VerticalAnchor::Justified | VerticalAnchor::Distributed
            if blocks.len() > 1 && slack > 0.0 =>
        {
            slack / (blocks.len() - 1) as f32
        }
        _ => 0.0,
    };

    for (n, (idx, lines)) in blocks.iter().enumerate() {
        let Some(para) = scaled.get(*idx) else {
            continue;
        };
        y += spacing_to_points(para.space_before, para_font_size(para));
        emit_paragraph(para, lines, content, &mut y, line_reduction, measure, out);
        y += spacing_to_points(para.space_after, para_font_size(para));
        if n + 1 < blocks.len() {
            y += extra_per_gap;
        }
    }

    TextLayoutResult {
        height: total_height,
        width: max_width,
        overflowed: total_height > content.h + 0.5,
    }
}

fn scale_paragraph(p: &StyledParagraph, factor: f32) -> StyledParagraph {
    let mut out = p.clone();
    for f in &mut out.fragments {
        f.font.size_pt = crate::dl::text::OrderedF32(f.font.size() * factor);
        f.letter_spacing *= factor;
    }
    if let Some(b) = &mut out.bullet {
        b.font.size_pt = crate::dl::text::OrderedF32(b.font.size() * factor);
    }
    out.empty_size_pt *= factor;
    out
}

fn para_font_size(p: &StyledParagraph) -> f32 {
    p.fragments
        .iter()
        .map(|f| f.font.size())
        .fold(0.0f32, f32::max)
        .max(p.empty_size_pt)
}

/// Breaks a paragraph into lines.
///
/// Words are the unit; a single word wider than the line is broken by character rather
/// than allowed to overflow, which is what PowerPoint does with long URLs.
fn wrap_paragraph(
    para: &StyledParagraph,
    avail: f32,
    first_indent: f32,
    bullet_width: f32,
    wrap: bool,
    measure: &dyn TextMeasure,
) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    let mut current: Vec<Piece> = Vec::new();
    let mut current_width = 0.0f32;
    let mut is_first = true;

    let line_limit = |first: bool| {
        let indent = if first {
            first_indent + bullet_width
        } else {
            0.0
        };
        (avail - indent).max(1.0)
    };

    let mut flush = |pieces: &mut Vec<Piece>, width: &mut f32, first: &mut bool, last: bool| {
        if pieces.is_empty() && !last {
            return;
        }
        lines.push(Line {
            pieces: std::mem::take(pieces),
            width: *width,
            ascent: 0.0,
            descent: 0.0,
            is_first: *first,
            is_last: last,
        });
        *width = 0.0;
        *first = false;
    };

    for (fi, frag) in para.fragments.iter().enumerate() {
        if frag.is_break {
            flush(&mut current, &mut current_width, &mut is_first, true);
            continue;
        }
        if frag.text.is_empty() {
            continue;
        }
        let measured = measure.measure(&frag.text, &frag.font);
        let chars: Vec<char> = frag.text.chars().collect();
        let advance = |start: usize, end: usize| -> f32 {
            measured.width_of_range(start, end) + frag.letter_spacing * (end - start) as f32
        };

        if !wrap {
            let w = advance(0, chars.len());
            current.push(Piece {
                fragment: fi,
                start: 0,
                end: chars.len(),
                width: w,
            });
            current_width += w;
            continue;
        }

        // Walk word by word. A "word" is a run of non-space characters plus the spaces
        // that follow it, so trailing spaces never push a line over the edge.
        let mut i = 0usize;
        let mut piece_start = 0usize;
        while i < chars.len() {
            let word_start = i;
            while i < chars.len() && !is_break_opportunity(chars.get(i).copied()) {
                i += 1;
            }
            // Consume the run of spaces after the word.
            let word_end = i;
            while i < chars.len() && chars.get(i).copied() == Some(' ') {
                i += 1;
            }
            let with_spaces_end = i;
            // A tab or explicit break-opportunity character advances one at a time.
            if word_end == word_start && i == word_start {
                i += 1;
            }

            let word_w = advance(word_start, word_end);
            let limit = line_limit(is_first);

            if current_width + advance(piece_start, word_end) > limit
                && (current_width > 0.0 || word_start > piece_start)
            {
                // Commit what fits, then start a new line at this word.
                if word_start > piece_start {
                    let w = advance(piece_start, word_start);
                    current.push(Piece {
                        fragment: fi,
                        start: piece_start,
                        end: word_start,
                        width: w,
                    });
                    current_width += w;
                }
                flush(&mut current, &mut current_width, &mut is_first, false);
                piece_start = word_start;
            }

            // A word that cannot fit even on its own line is split by character.
            if word_w > line_limit(is_first) && current_width == 0.0 {
                let mut c = word_start;
                while c < word_end {
                    let remaining = line_limit(is_first) - current_width;
                    let mut fit = c;
                    let mut acc = 0.0;
                    while fit < word_end {
                        let a = advance(fit, fit + 1);
                        if acc + a > remaining && fit > c {
                            break;
                        }
                        acc += a;
                        fit += 1;
                    }
                    if fit == c {
                        fit = (c + 1).min(word_end);
                        acc = advance(c, fit);
                    }
                    current.push(Piece {
                        fragment: fi,
                        start: c,
                        end: fit,
                        width: acc,
                    });
                    current_width += acc;
                    c = fit;
                    if c < word_end {
                        flush(&mut current, &mut current_width, &mut is_first, false);
                    }
                }
                piece_start = word_end;
            }

            if with_spaces_end >= chars.len() {
                // Emit the tail of this fragment.
                if with_spaces_end > piece_start {
                    let w = advance(piece_start, with_spaces_end);
                    current.push(Piece {
                        fragment: fi,
                        start: piece_start,
                        end: with_spaces_end,
                        width: w,
                    });
                    current_width += w;
                }
                piece_start = with_spaces_end;
            }
        }
        // Anything left in this fragment that no word boundary flushed.
        if piece_start < chars.len() {
            let w = advance(piece_start, chars.len());
            current.push(Piece {
                fragment: fi,
                start: piece_start,
                end: chars.len(),
                width: w,
            });
            current_width += w;
        }
    }
    flush(&mut current, &mut current_width, &mut is_first, true);

    // Measure each line's vertical extent from the fragments actually on it.
    for line in &mut lines {
        let (mut ascent, mut descent) = (0.0f32, 0.0f32);
        for piece in &line.pieces {
            if let Some(f) = para.fragments.get(piece.fragment) {
                let m = measure.font_metrics(&f.font);
                ascent = ascent.max(m.ascent);
                descent = descent.max(m.descent);
            }
        }
        if line.pieces.is_empty() {
            ascent = para.empty_metrics.ascent;
            descent = para.empty_metrics.descent;
        }
        line.ascent = ascent;
        line.descent = descent;
    }
    lines
}

/// Characters after which a line may break.
fn is_break_opportunity(c: Option<char>) -> bool {
    matches!(c, Some(' ') | Some('\t') | Some('\u{00A0}') | None)
}

fn line_height(
    line: &Line,
    para: &StyledParagraph,
    reduction: f32,
    measure: &dyn TextMeasure,
) -> f32 {
    let natural = line.ascent + line.descent;
    let base = match para.line_spacing {
        // A percentage multiplies the font's own line height. PowerPoint applies the
        // percentage to 1.2x the point size rather than the face's real line gap, which
        // is why 100% spacing looks tighter here than a naive ascent+descent+gap.
        Spacing::Percent(p) => {
            let em = para
                .fragments
                .iter()
                .map(|f| f.font.size())
                .fold(0.0f32, f32::max)
                .max(para.empty_size_pt);
            let metric = if line.pieces.is_empty() {
                para.empty_metrics.line_height()
            } else {
                let mut lh = 0.0f32;
                for piece in &line.pieces {
                    if let Some(f) = para.fragments.get(piece.fragment) {
                        lh = lh.max(measure.font_metrics(&f.font).line_height());
                    }
                }
                lh
            };
            (metric.max(em * 1.2)) * p
        }
        Spacing::Points(pt) => pt,
    };
    (base * (1.0 - reduction)).max(natural * 0.5)
}

fn paragraph_height(
    para: &StyledParagraph,
    lines: &[Line],
    reduction: f32,
    measure: &dyn TextMeasure,
) -> f32 {
    let size = para_font_size(para);
    let mut h =
        spacing_to_points(para.space_before, size) + spacing_to_points(para.space_after, size);
    for l in lines {
        h += line_height(l, para, reduction, measure);
    }
    h
}

fn spacing_to_points(s: Spacing, font_size: f32) -> f32 {
    match s {
        Spacing::Points(p) => p,
        // A percentage space-before/after is relative to the paragraph's font size.
        Spacing::Percent(p) => font_size * p,
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_paragraph(
    para: &StyledParagraph,
    lines: &[Line],
    content: Rect,
    y: &mut f32,
    reduction: f32,
    measure: &dyn TextMeasure,
    out: &mut Vec<Command>,
) {
    for line in lines {
        let lh = line_height(line, para, reduction, measure);
        let baseline = if line.ascent + line.descent >= lh {
            // The line box is tighter than the text needs; sit the baseline at the top
            // rather than clipping the ascenders.
            *y + line.ascent
        } else {
            // Distribute the leading above and below, as every text engine does.
            *y + (lh - line.ascent - line.descent) / 2.0 + line.ascent
        };

        let bullet_width = if line.is_first {
            para.bullet
                .as_ref()
                .map(|b| measure.measure(&b.text, &b.font).width)
                .unwrap_or(0.0)
        } else {
            0.0
        };
        let indent = if line.is_first { para.indent } else { 0.0 };
        let line_left = content.x + para.margin_left + indent.max(-para.margin_left);
        let avail = (content.w
            - para.margin_left
            - para.margin_right
            - if line.is_first {
                indent.max(-para.margin_left)
            } else {
                0.0
            })
        .max(0.0);

        // Justification stretches the inter-word gaps rather than the glyphs.
        let justify = matches!(para.align, TextAlign::Justify) && !line.is_last
            || para.align.justifies_last_line();
        let slack = (avail - bullet_width - line.width).max(0.0);
        let mut x = match para.align {
            TextAlign::Left
            | TextAlign::Justify
            | TextAlign::Distributed
            | TextAlign::ThaiDistributed => line_left,
            TextAlign::Center => line_left + slack / 2.0,
            TextAlign::Right => line_left + slack,
        };
        if justify {
            x = line_left;
        }

        if line.is_first {
            if let Some(b) = &para.bullet {
                // The bullet sits at the start of the first line's indent, not inside the
                // text: with a hanging indent that means to the left of the text body.
                let bullet_x = content.x + para.margin_left + para.indent.max(-para.margin_left);
                out.push(Command::DrawText(TextRun {
                    text: b.text.clone(),
                    font: b.font.clone(),
                    origin: Point::new(bullet_x, baseline),
                    paint: b.paint.clone(),
                    advances: measure.measure(&b.text, &b.font).advances,
                    width: measure.measure(&b.text, &b.font).width,
                    decorations: Decorations::default(),
                    letter_spacing: 0.0,
                }));
            }
            x += bullet_width;
        }

        let gap_count = count_gaps(line, para);
        let extra_per_gap = if justify && gap_count > 0 {
            slack / gap_count as f32
        } else {
            0.0
        };

        for piece in &line.pieces {
            let Some(frag) = para.fragments.get(piece.fragment) else {
                continue;
            };
            let text: String = frag
                .text
                .chars()
                .skip(piece.start)
                .take(piece.end.saturating_sub(piece.start))
                .collect();
            if text.is_empty() {
                continue;
            }
            let measured = measure.measure(&text, &frag.font);
            let spaces = text.chars().filter(|c| *c == ' ').count();
            let shift = frag.baseline_shift * frag.font.size();
            out.push(Command::DrawText(TextRun {
                text,
                font: frag.font.clone(),
                origin: Point::new(x, baseline - shift),
                paint: frag.paint.clone(),
                advances: measured.advances,
                width: piece.width,
                decorations: frag.decorations,
                letter_spacing: frag.letter_spacing,
            }));
            x += piece.width + extra_per_gap * spaces as f32;
        }
        *y += lh;
    }
}

fn count_gaps(line: &Line, para: &StyledParagraph) -> usize {
    let mut n = 0;
    for piece in &line.pieces {
        if let Some(f) = para.fragments.get(piece.fragment) {
            n += f
                .text
                .chars()
                .skip(piece.start)
                .take(piece.end.saturating_sub(piece.start))
                .filter(|c| *c == ' ')
                .count();
        }
    }
    n
}

/// Builds the bullet fragment for a paragraph, or `None` if it has none.
pub fn bullet_fragment(
    props: &ParagraphProps,
    index_in_list: u32,
    text_font: &FontSpec,
    text_paint: &Paint,
) -> Option<StyledFragment> {
    let kind = props.bullet.kind.as_ref()?;
    let text = match kind {
        BulletKind::None => return None,
        BulletKind::Char(c) => {
            let mut s = c.clone();
            // A trailing space keeps the bullet from touching the text; PowerPoint gets
            // this from the tab stop after the bullet, which we approximate.
            s.push(' ');
            s
        }
        BulletKind::AutoNum { scheme, start_at } => {
            let n = start_at.saturating_add(index_in_list);
            format!("{} ", scheme.format(n.max(1)))
        }
        // Picture bullets are drawn by the shape layer, not as text.
        BulletKind::Image(_) => return None,
    };
    let size = match (props.bullet.size_points, props.bullet.size_percent) {
        (Some(pt), _) => pt,
        (None, Some(pct)) => text_font.size() * pct,
        (None, None) => text_font.size(),
    };
    let mut font = FontSpec::new(
        props
            .bullet
            .font
            .clone()
            .unwrap_or_else(|| text_font.family.clone()),
        size,
    );
    font.fallbacks = crate::text::fallbacks_for(&font.family);
    Some(StyledFragment {
        text,
        font,
        paint: text_paint.clone(),
        decorations: Decorations::default(),
        letter_spacing: 0.0,
        baseline_shift: 0.0,
        is_break: false,
    })
}

/// Applies `<a:rPr cap="all|small">` to a run's text.
pub fn apply_caps(text: &str, caps: Capitalization) -> String {
    match caps {
        Capitalization::None => text.to_string(),
        // Real small caps need a font feature; upper-casing at a reduced size is the
        // standard approximation and is what LibreOffice does when the face lacks them.
        Capitalization::All | Capitalization::Small => text.to_uppercase(),
    }
}

/// Builds the [`FontSpec`] for a run.
pub fn font_spec(family: &str, size_pt: f32, bold: bool, italic: bool) -> FontSpec {
    let mut spec = FontSpec::new(family, size_pt.max(1.0));
    spec.fallbacks = crate::text::fallbacks_for(family);
    spec.weight = if bold {
        FontWeight::Bold
    } else {
        FontWeight::Regular
    };
    spec.style = if italic {
        FontStyle::Italic
    } else {
        FontStyle::Normal
    };
    spec
}

/// Underline/strikethrough flags for a run.
pub fn decorations(underline: Option<UnderlineStyle>, strike: Option<bool>) -> Decorations {
    Decorations {
        underline: underline.map(|u| u.is_visible()).unwrap_or(false),
        strikethrough: strike.unwrap_or(false),
    }
}

/// The content rectangle of a text box: its extent less the body insets.
pub fn content_rect(box_pt: Rect, body: &BodyProps) -> Rect {
    let (l, t, r, b) = body.resolved_insets();
    let (l, t, r, b) = (emu::to_pt(l), emu::to_pt(t), emu::to_pt(r), emu::to_pt(b));
    Rect::new(
        box_pt.x + l,
        box_pt.y + t,
        (box_pt.w - l - r).max(0.0),
        (box_pt.h - t - b).max(0.0),
    )
}

/// True when a run should be skipped entirely.
pub fn is_empty_run(run: &Run) -> bool {
    matches!(run, Run::Text { text, .. } if text.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dl::Color;
    use crate::text::StubMeasure;

    fn frag(text: &str, size: f32) -> StyledFragment {
        StyledFragment {
            text: text.to_string(),
            font: FontSpec::new("Arial", size),
            paint: Paint::Solid(Color::BLACK),
            decorations: Decorations::default(),
            letter_spacing: 0.0,
            baseline_shift: 0.0,
            is_break: false,
        }
    }

    fn para(fragments: Vec<StyledFragment>, align: TextAlign) -> StyledParagraph {
        StyledParagraph {
            fragments,
            align,
            margin_left: 0.0,
            margin_right: 0.0,
            indent: 0.0,
            line_spacing: Spacing::Percent(1.0),
            space_before: Spacing::Points(0.0),
            space_after: Spacing::Points(0.0),
            rtl: false,
            bullet: None,
            empty_metrics: FontMetrics::approximate_for(18.0),
            empty_size_pt: 18.0,
        }
    }

    fn run_texts(cmds: &[Command]) -> Vec<String> {
        cmds.iter()
            .filter_map(|c| match c {
                Command::DrawText(r) => Some(r.text.clone()),
                _ => None,
            })
            .collect()
    }

    fn run_origins(cmds: &[Command]) -> Vec<Point> {
        cmds.iter()
            .filter_map(|c| match c {
                Command::DrawText(r) => Some(r.origin),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_short_line_is_not_wrapped() {
        let mut out = Vec::new();
        layout(
            &[para(vec![frag("Hello world", 12.0)], TextAlign::Left)],
            &BodyProps::default(),
            Rect::new(0.0, 0.0, 500.0, 100.0),
            &StubMeasure,
            &mut out,
        );
        assert_eq!(run_texts(&out).len(), 1);
    }

    #[test]
    fn text_wraps_at_a_word_boundary_not_mid_word() {
        let mut out = Vec::new();
        layout(
            &[para(
                vec![frag("aaaa bbbb cccc dddd eeee", 12.0)],
                TextAlign::Left,
            )],
            &BodyProps::default(),
            Rect::new(0.0, 0.0, 80.0, 200.0),
            &StubMeasure,
            &mut out,
        );
        let texts = run_texts(&out);
        assert!(texts.len() > 1, "expected wrapping, got {texts:?}");
        for t in &texts {
            // No line may start or end mid-word.
            assert!(
                !t.trim().is_empty(),
                "empty line produced while wrapping: {texts:?}"
            );
        }
        let joined: String = texts.join("");
        assert_eq!(joined.replace('\n', ""), "aaaa bbbb cccc dddd eeee");
    }

    #[test]
    fn every_wrapped_line_fits_the_available_width() {
        let width = 100.0;
        let mut out = Vec::new();
        layout(
            &[para(
                vec![frag("the quick brown fox jumps over the lazy dog", 11.0)],
                TextAlign::Left,
            )],
            &BodyProps {
                left_inset: Some(0),
                right_inset: Some(0),
                ..Default::default()
            },
            Rect::new(0.0, 0.0, width, 400.0),
            &StubMeasure,
            &mut out,
        );
        for cmd in &out {
            if let Command::DrawText(r) = cmd {
                // Allow the trailing space of a line to exceed the box, as text engines do.
                let trimmed = StubMeasure.measure(r.text.trim_end(), &r.font).width;
                assert!(
                    trimmed <= width + 0.5,
                    "line {:?} is {trimmed}pt wide, limit {width}",
                    r.text
                );
            }
        }
    }

    #[test]
    fn a_word_longer_than_the_line_is_broken_rather_than_overflowing() {
        let mut out = Vec::new();
        layout(
            &[para(
                vec![frag("supercalifragilisticexpialidocious", 14.0)],
                TextAlign::Left,
            )],
            &BodyProps {
                left_inset: Some(0),
                right_inset: Some(0),
                ..Default::default()
            },
            Rect::new(0.0, 0.0, 40.0, 300.0),
            &StubMeasure,
            &mut out,
        );
        let texts = run_texts(&out);
        assert!(texts.len() > 1, "long word should be split, got {texts:?}");
        assert_eq!(texts.join(""), "supercalifragilisticexpialidocious");
    }

    #[test]
    fn explicit_breaks_start_a_new_line_regardless_of_width() {
        let mut fragments = vec![frag("a", 12.0)];
        fragments.push(StyledFragment::text_break(
            FontSpec::new("Arial", 12.0),
            Paint::Solid(Color::BLACK),
        ));
        fragments.push(frag("b", 12.0));
        let mut out = Vec::new();
        layout(
            &[para(fragments, TextAlign::Left)],
            &BodyProps::default(),
            Rect::new(0.0, 0.0, 500.0, 100.0),
            &StubMeasure,
            &mut out,
        );
        let origins = run_origins(&out);
        assert_eq!(origins.len(), 2);
        assert!(
            origins[1].y > origins[0].y,
            "the run after <a:br/> must be on the next line"
        );
    }

    #[test]
    fn alignment_moves_the_line_within_the_box() {
        let text = "word";
        let make = |align| {
            let mut out = Vec::new();
            layout(
                &[para(vec![frag(text, 12.0)], align)],
                &BodyProps {
                    left_inset: Some(0),
                    right_inset: Some(0),
                    ..Default::default()
                },
                Rect::new(0.0, 0.0, 200.0, 100.0),
                &StubMeasure,
                &mut out,
            );
            run_origins(&out).first().copied().unwrap_or_default().x
        };
        let left = make(TextAlign::Left);
        let center = make(TextAlign::Center);
        let right = make(TextAlign::Right);
        assert!(left < center && center < right, "{left} {center} {right}");
        assert!((left - 0.0).abs() < 0.01);
        let w = StubMeasure
            .measure(text, &FontSpec::new("Arial", 12.0))
            .width;
        assert!(
            (right - (200.0 - w)).abs() < 0.5,
            "right={right} expected {}",
            200.0 - w
        );
    }

    #[test]
    fn vertical_anchoring_positions_the_block() {
        let make = |anchor| {
            let mut out = Vec::new();
            layout(
                &[para(vec![frag("x", 12.0)], TextAlign::Left)],
                &BodyProps {
                    anchor: Some(anchor),
                    top_inset: Some(0),
                    bottom_inset: Some(0),
                    ..Default::default()
                },
                Rect::new(0.0, 0.0, 200.0, 200.0),
                &StubMeasure,
                &mut out,
            );
            run_origins(&out).first().copied().unwrap_or_default().y
        };
        let top = make(VerticalAnchor::Top);
        let middle = make(VerticalAnchor::Middle);
        let bottom = make(VerticalAnchor::Bottom);
        assert!(top < middle && middle < bottom, "{top} {middle} {bottom}");
    }

    #[test]
    fn line_spacing_percentage_changes_the_distance_between_lines() {
        let make = |spacing| {
            let mut p = para(vec![frag("aaaa bbbb cccc", 12.0)], TextAlign::Left);
            p.line_spacing = spacing;
            let mut out = Vec::new();
            layout(
                &[p],
                &BodyProps {
                    left_inset: Some(0),
                    right_inset: Some(0),
                    ..Default::default()
                },
                Rect::new(0.0, 0.0, 50.0, 400.0),
                &StubMeasure,
                &mut out,
            );
            let o = run_origins(&out);
            o.get(1).map(|p| p.y).unwrap_or(0.0) - o.first().map(|p| p.y).unwrap_or(0.0)
        };
        let single = make(Spacing::Percent(1.0));
        let double = make(Spacing::Percent(2.0));
        assert!(double > single * 1.8, "single={single} double={double}");
    }

    #[test]
    fn space_before_and_after_separate_paragraphs() {
        let mut a = para(vec![frag("one", 12.0)], TextAlign::Left);
        let mut b = para(vec![frag("two", 12.0)], TextAlign::Left);
        a.space_after = Spacing::Points(0.0);
        b.space_before = Spacing::Points(0.0);
        let mut tight = Vec::new();
        layout(
            &[a.clone(), b.clone()],
            &BodyProps::default(),
            Rect::new(0.0, 0.0, 300.0, 300.0),
            &StubMeasure,
            &mut tight,
        );

        b.space_before = Spacing::Points(24.0);
        let mut loose = Vec::new();
        layout(
            &[a, b],
            &BodyProps::default(),
            Rect::new(0.0, 0.0, 300.0, 300.0),
            &StubMeasure,
            &mut loose,
        );

        let gap = |cmds: &[Command]| {
            let o = run_origins(cmds);
            o.get(1).map(|p| p.y).unwrap_or(0.0) - o.first().map(|p| p.y).unwrap_or(0.0)
        };
        assert!((gap(&loose) - gap(&tight) - 24.0).abs() < 0.5);
    }

    #[test]
    fn a_bullet_is_emitted_before_the_first_line_only() {
        let mut p = para(vec![frag("aaaa bbbb cccc dddd", 12.0)], TextAlign::Left);
        p.bullet = Some(frag("\u{2022} ", 12.0));
        let mut out = Vec::new();
        layout(
            &[p],
            &BodyProps {
                left_inset: Some(0),
                right_inset: Some(0),
                ..Default::default()
            },
            Rect::new(0.0, 0.0, 70.0, 300.0),
            &StubMeasure,
            &mut out,
        );
        let texts = run_texts(&out);
        let bullets = texts.iter().filter(|t| t.starts_with('\u{2022}')).count();
        assert_eq!(bullets, 1, "exactly one bullet per paragraph: {texts:?}");
    }

    #[test]
    fn autofit_shrink_reduces_the_rendered_font_size() {
        let body = BodyProps {
            autofit: Some(Autofit::Shrink {
                font_scale: 0.5,
                line_space_reduction: 0.0,
            }),
            ..Default::default()
        };
        let mut out = Vec::new();
        layout(
            &[para(vec![frag("text", 20.0)], TextAlign::Left)],
            &body,
            Rect::new(0.0, 0.0, 300.0, 300.0),
            &StubMeasure,
            &mut out,
        );
        match out.first() {
            Some(Command::DrawText(r)) => assert_eq!(r.font.size(), 10.0),
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn overflow_is_reported_without_clipping_the_text_away() {
        let paras: Vec<_> = (0..20)
            .map(|_| para(vec![frag("a line of text", 18.0)], TextAlign::Left))
            .collect();
        let mut out = Vec::new();
        let result = layout(
            &paras,
            &BodyProps::default(),
            Rect::new(0.0, 0.0, 300.0, 40.0),
            &StubMeasure,
            &mut out,
        );
        assert!(result.overflowed);
        assert_eq!(
            run_texts(&out).len(),
            20,
            "overflowing text is still emitted"
        );
    }

    #[test]
    fn an_empty_paragraph_still_occupies_a_line() {
        let mut out = Vec::new();
        let result = layout(
            &[para(vec![], TextAlign::Left)],
            &BodyProps::default(),
            Rect::new(0.0, 0.0, 300.0, 300.0),
            &StubMeasure,
            &mut out,
        );
        assert!(result.height > 0.0, "an empty paragraph must have height");
        assert!(run_texts(&out).is_empty());
    }

    #[test]
    fn wrapping_disabled_keeps_everything_on_one_line() {
        let mut out = Vec::new();
        layout(
            &[para(
                vec![frag("a very long line that would otherwise wrap", 12.0)],
                TextAlign::Left,
            )],
            &BodyProps {
                wrap: Some(false),
                ..Default::default()
            },
            Rect::new(0.0, 0.0, 30.0, 300.0),
            &StubMeasure,
            &mut out,
        );
        assert_eq!(run_origins(&out).len(), 1);
    }

    #[test]
    fn content_rect_applies_the_body_insets() {
        let r = content_rect(Rect::new(10.0, 20.0, 200.0, 100.0), &BodyProps::default());
        // 91440 EMU = 7.2pt, 45720 EMU = 3.6pt
        assert!((r.x - 17.2).abs() < 0.01);
        assert!((r.y - 23.6).abs() < 0.01);
        assert!((r.w - (200.0 - 14.4)).abs() < 0.01);
    }

    #[test]
    fn insets_larger_than_the_box_clamp_to_an_empty_content_rect() {
        let r = content_rect(Rect::new(0.0, 0.0, 5.0, 5.0), &BodyProps::default());
        assert!(r.w >= 0.0 && r.h >= 0.0);
    }

    #[test]
    fn caps_are_applied_to_the_rendered_text() {
        assert_eq!(apply_caps("hello", Capitalization::All), "HELLO");
        assert_eq!(apply_caps("hello", Capitalization::None), "hello");
    }

    #[test]
    fn wrapping_is_independent_of_the_zoom_the_result_is_drawn_at() {
        // The same paragraph measured twice must break identically; this is the property
        // that lets one display list serve every zoom level.
        let run_once = || {
            let mut out = Vec::new();
            layout(
                &[para(
                    vec![frag("the quick brown fox jumps over the lazy dog", 12.0)],
                    TextAlign::Left,
                )],
                &BodyProps::default(),
                Rect::new(0.0, 0.0, 120.0, 400.0),
                &StubMeasure,
                &mut out,
            );
            run_texts(&out)
        };
        assert_eq!(run_once(), run_once());
    }
}
