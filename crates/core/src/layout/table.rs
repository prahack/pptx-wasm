//! Table layout: grid sizing, merged-cell spans, borders and in-cell text.

use crate::dl::{Command, FillRule, Paint, Path, Rect};
use crate::emu;
use crate::model::table::{Table, TableCell};
use crate::model::table_style::{CellPosition, PartStyle};
use crate::model::text::{Spacing, TextAlign, VerticalAnchor};
use crate::model::Presentation;
use crate::text::TextMeasure;

use super::inherit::Resolver;
use super::paint;
use super::text::{self as tlayout, StyledFragment, StyledParagraph};

/// Lays a table out inside `frame` and appends its commands.
///
/// Column widths come from the authored grid, scaled to the frame if they disagree with
/// it — PowerPoint resizes the frame and the grid together, but a hand-edited file can
/// have them out of step and stretching is less wrong than overflowing.
#[allow(clippy::too_many_arguments)]
pub fn layout_table(
    table: &Table,
    frame: Rect,
    resolver: &Resolver<'_>,
    pres: &Presentation,
    measure: &dyn TextMeasure,
    part: &str,
    out: &mut Vec<Command>,
) {
    if table.columns.is_empty() || table.rows.is_empty() {
        return;
    }
    // The table style supplies the fills, rules and header formatting that the cells
    // themselves almost never carry. See `model::table_style` for why it usually has to
    // be synthesised rather than read from the file.
    let styles = pres.table_styles();
    let style = styles.get(table.props.style_id.as_deref());

    let cols = column_edges(table, frame);
    let rows = row_edges(table, frame, resolver, measure, &style);

    let position = |r: usize, c: usize| CellPosition {
        row: r,
        col: c,
        row_count: table.row_count(),
        col_count: table.column_count(),
    };

    // Fills first, then all text, then all borders. Borders last so a neighbouring
    // cell's fill can never paint over a shared edge.
    for (r, row) in table.rows.iter().enumerate() {
        for (c, cell) in row.cells.iter().enumerate() {
            let Some(rect) = cell_rect(table, &cols, &rows, r, c) else {
                continue;
            };
            let part_style = style.resolve(position(r, c), &table.props);
            let fill = if cell.fill.is_specified() {
                cell.fill.clone()
            } else {
                part_style.fill.clone()
            };
            if let Some(p) = paint::fill_to_paint(&fill, rect, resolver, pres, part) {
                out.push(Command::FillPath {
                    path: Path::rect(rect),
                    paint: p,
                    rule: FillRule::NonZero,
                });
            }
        }
    }

    for (r, row) in table.rows.iter().enumerate() {
        for (c, cell) in row.cells.iter().enumerate() {
            let Some(rect) = cell_rect(table, &cols, &rows, r, c) else {
                continue;
            };
            let part_style = style.resolve(position(r, c), &table.props);
            emit_cell_text(cell, rect, resolver, measure, &part_style, out);
        }
    }

    for (r, row) in table.rows.iter().enumerate() {
        for (c, cell) in row.cells.iter().enumerate() {
            let Some(rect) = cell_rect(table, &cols, &rows, r, c) else {
                continue;
            };
            let part_style = style.resolve(position(r, c), &table.props);
            let edges = cell_edges(cell, &part_style, position(r, c));
            emit_cell_borders(&edges, rect, resolver, pres, part, out);
        }
    }
}

/// The four rules to draw around one cell.
///
/// A cell's own `<a:tcPr>` border wins; otherwise the style supplies an *outer* edge for
/// cells on the table's boundary and an *inner* one everywhere else. Getting that
/// distinction wrong is what makes a styled table draw its heavy outer border between
/// every pair of cells.
struct CellEdges {
    left: crate::model::fill::Line,
    right: crate::model::fill::Line,
    top: crate::model::fill::Line,
    bottom: crate::model::fill::Line,
    tl_br: crate::model::fill::Line,
    tr_bl: crate::model::fill::Line,
}

fn cell_edges(cell: &TableCell, style: &PartStyle, pos: CellPosition) -> CellEdges {
    let pick = |own: &crate::model::fill::Line,
                outer: &crate::model::fill::Line,
                inner: &crate::model::fill::Line,
                is_outer: bool| {
        if !own.is_empty() {
            return own.clone();
        }
        let from_style = if is_outer { outer } else { inner };
        if from_style.is_empty() && is_outer {
            // A style that only defines inner rules still needs an edge on the boundary.
            inner.clone()
        } else {
            from_style.clone()
        }
    };
    CellEdges {
        left: pick(
            &cell.border_left,
            &style.left,
            &style.inside_v,
            pos.is_first_col(),
        ),
        right: pick(
            &cell.border_right,
            &style.right,
            &style.inside_v,
            pos.is_last_col(),
        ),
        top: pick(
            &cell.border_top,
            &style.top,
            &style.inside_h,
            pos.is_first_row(),
        ),
        bottom: pick(
            &cell.border_bottom,
            &style.bottom,
            &style.inside_h,
            pos.is_last_row(),
        ),
        tl_br: cell.border_tl_br.clone(),
        tr_bl: cell.border_tr_bl.clone(),
    }
}

/// Cumulative column x positions, `columns.len() + 1` entries.
fn column_edges(table: &Table, frame: Rect) -> Vec<f32> {
    let total: i64 = table.columns.iter().sum();
    let scale = if total > 0 {
        frame.w / emu::to_pt(total)
    } else {
        1.0
    };
    let mut edges = Vec::with_capacity(table.columns.len() + 1);
    let mut x = frame.x;
    edges.push(x);
    for w in &table.columns {
        x += emu::to_pt(*w) * scale;
        edges.push(x);
    }
    edges
}

/// Cumulative row y positions. Rows grow to fit their text but never shrink below the
/// authored height, which is what PowerPoint does.
fn row_edges(
    table: &Table,
    frame: Rect,
    resolver: &Resolver<'_>,
    measure: &dyn TextMeasure,
    style: &crate::model::table_style::TableStyle,
) -> Vec<f32> {
    let mut heights: Vec<f32> = table
        .rows
        .iter()
        .map(|r| emu::to_pt(r.height).max(0.0))
        .collect();

    let cols = column_edges(table, frame);
    for (r, row) in table.rows.iter().enumerate() {
        for (c, cell) in row.cells.iter().enumerate() {
            // Only single-row cells contribute to their row's minimum height; a spanning
            // cell's text is shared across the rows it covers.
            if cell.is_covered() || cell.row_span.max(1) > 1 {
                continue;
            }
            let Some(text) = &cell.text else { continue };
            let width = column_width(&cols, c, cell.grid_span.max(1) as usize);
            let (ml, mt, mr, mb) = cell.resolved_margins();
            let content_w = (width - emu::to_pt(ml) - emu::to_pt(mr)).max(1.0);
            let part_style = style.resolve(
                CellPosition {
                    row: r,
                    col: c,
                    row_count: table.row_count(),
                    col_count: table.column_count(),
                },
                &table.props,
            );
            let paragraphs = cell_paragraphs(text, resolver, &part_style);
            let mut scratch = Vec::new();
            let result = tlayout::layout(
                &paragraphs,
                &text.body,
                Rect::new(0.0, 0.0, content_w, f32::MAX / 4.0),
                measure,
                &mut scratch,
            );
            let needed = result.height + emu::to_pt(mt) + emu::to_pt(mb);
            if let Some(h) = heights.get_mut(r) {
                *h = h.max(needed);
            }
        }
    }

    let mut edges = Vec::with_capacity(heights.len() + 1);
    let mut y = frame.y;
    edges.push(y);
    for h in &heights {
        y += h;
        edges.push(y);
    }
    edges
}

fn column_width(edges: &[f32], start: usize, span: usize) -> f32 {
    let a = edges.get(start).copied().unwrap_or(0.0);
    let b = edges
        .get((start + span).min(edges.len().saturating_sub(1)))
        .copied()
        .unwrap_or(a);
    (b - a).max(0.0)
}

/// The rectangle a cell occupies, or `None` if it is covered by a merge.
fn cell_rect(table: &Table, cols: &[f32], rows: &[f32], r: usize, c: usize) -> Option<Rect> {
    let (col, row, col_span, row_span) = table.cell_span(r, c)?;
    let x0 = cols.get(col).copied()?;
    let x1 = cols
        .get((col + col_span).min(cols.len().saturating_sub(1)))
        .copied()?;
    let y0 = rows.get(row).copied()?;
    let y1 = rows
        .get((row + row_span).min(rows.len().saturating_sub(1)))
        .copied()?;
    Some(Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)))
}

fn emit_cell_text(
    cell: &TableCell,
    rect: Rect,
    resolver: &Resolver<'_>,
    measure: &dyn TextMeasure,
    style: &PartStyle,
    out: &mut Vec<Command>,
) {
    let Some(text) = &cell.text else { return };
    if text.is_empty() {
        return;
    }
    let (ml, mt, mr, mb) = cell.resolved_margins();
    let content = Rect::new(
        rect.x + emu::to_pt(ml),
        rect.y + emu::to_pt(mt),
        (rect.w - emu::to_pt(ml) - emu::to_pt(mr)).max(0.0),
        (rect.h - emu::to_pt(mt) - emu::to_pt(mb)).max(0.0),
    );
    let paragraphs = cell_paragraphs(text, resolver, style);
    let mut body = text.body.clone();
    // The cell's own anchor overrides the text body's.
    if let Some(a) = cell.vertical_anchor {
        body.anchor = Some(a);
    }
    body.anchor = body.anchor.or(Some(VerticalAnchor::Top));
    // Insets are the cell's margins, already applied to `content`.
    body.left_inset = Some(0);
    body.right_inset = Some(0);
    body.top_inset = Some(0);
    body.bottom_inset = Some(0);
    tlayout::layout(&paragraphs, &body, content, measure, out);
}

/// Cell text uses the deck's default text style rather than a placeholder chain, since a
/// table cell is not a placeholder.
fn cell_paragraphs(
    text: &crate::model::TextBody,
    resolver: &Resolver<'_>,
    style: &PartStyle,
) -> Vec<StyledParagraph> {
    let mut out = Vec::with_capacity(text.paragraphs.len());
    for para in &text.paragraphs {
        let level = para.props.level.min(8);
        let mut props = para.props.clone();
        if let Some(p) = text.list_style.level(level) {
            props.inherit_from(p);
        }
        if let Some(p) = resolver.pres.default_text_style.level(level) {
            props.inherit_from(p);
        }

        let mut fragments = Vec::new();
        for run in &para.runs {
            let raw = run.text();
            let mut rp = run.props().clone();
            if let Some(defaults) = &props.default_run_props {
                rp.inherit_from(defaults);
            }
            let family = resolver.font_family(rp.latin_font.as_deref());
            // Table text defaults to 18pt like everything else, but cells are small, so
            // an unstyled table reads better at the ECMA default than at a guess.
            let size = rp.size_points().unwrap_or(18.0);
            // The table style supplies bold and colour for header and total rows; the
            // run's own properties still win where it set them.
            let font = tlayout::font_spec(
                &family,
                size,
                rp.bold.or(style.bold).unwrap_or(false),
                rp.italic.or(style.italic).unwrap_or(false),
            );
            let paint = match (&rp.fill, &style.color) {
                (crate::model::fill::Fill::Solid(c), _) => Paint::Solid(resolver.color(c)),
                (_, Some(c)) => Paint::Solid(resolver.color(c)),
                _ => Paint::Solid(crate::dl::Color::BLACK),
            };
            if matches!(run, crate::model::Run::Break { .. }) {
                fragments.push(StyledFragment::text_break(font, paint));
                continue;
            }
            if raw.is_empty() {
                continue;
            }
            fragments.push(StyledFragment {
                text: raw.to_string(),
                font,
                paint,
                decorations: tlayout::decorations(rp.underline, rp.strikethrough),
                letter_spacing: rp.letter_spacing.unwrap_or(0) as f32 / 100.0,
                baseline_shift: rp.baseline.unwrap_or(0.0),
                is_break: false,
            });
        }

        let size = fragments.first().map(|f| f.font.size()).unwrap_or(18.0);
        out.push(StyledParagraph {
            fragments,
            align: props.align.unwrap_or(TextAlign::Left),
            margin_left: emu::to_pt(props.margin_left.unwrap_or(0)),
            margin_right: emu::to_pt(props.margin_right.unwrap_or(0)),
            indent: emu::to_pt(props.indent.unwrap_or(0)),
            line_spacing: props.line_spacing.unwrap_or(Spacing::Percent(1.0)),
            space_before: props.space_before.unwrap_or(Spacing::Points(0.0)),
            space_after: props.space_after.unwrap_or(Spacing::Points(0.0)),
            rtl: props.rtl.unwrap_or(false),
            bullet: None,
            empty_metrics: crate::text::FontMetrics::approximate_for(size),
            empty_size_pt: size,
        });
    }
    out
}

fn emit_cell_borders(
    edges: &CellEdges,
    rect: Rect,
    resolver: &Resolver<'_>,
    pres: &Presentation,
    part: &str,
    out: &mut Vec<Command>,
) {
    let mut edge = |line: &crate::model::fill::Line, x0: f32, y0: f32, x1: f32, y1: f32| {
        if line.is_empty() {
            return;
        }
        let Some(stroke) = paint::line_to_stroke(line, rect, resolver, pres, part) else {
            return;
        };
        let mut p = Path::new();
        p.move_to(x0, y0).line_to(x1, y1);
        out.push(Command::StrokePath { path: p, stroke });
    };
    edge(&edges.top, rect.x, rect.y, rect.right(), rect.y);
    edge(
        &edges.bottom,
        rect.x,
        rect.bottom(),
        rect.right(),
        rect.bottom(),
    );
    edge(&edges.left, rect.x, rect.y, rect.x, rect.bottom());
    edge(
        &edges.right,
        rect.right(),
        rect.y,
        rect.right(),
        rect.bottom(),
    );
    edge(&edges.tl_br, rect.x, rect.y, rect.right(), rect.bottom());
    edge(&edges.tr_bl, rect.right(), rect.y, rect.x, rect.bottom());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::color::ColorRef;
    use crate::model::fill::{Fill, Line};
    use crate::model::shape::{SlideLayout, SlideMaster};
    use crate::model::table::{TableProps, TableRow};
    use crate::model::theme::Theme;
    use crate::model::{Slide, SlideChain, TextBody};
    use crate::text::StubMeasure;
    use std::rc::Rc;

    struct Env {
        pres: Presentation,
        chain: SlideChain,
    }

    fn env() -> Env {
        use std::io::{Cursor, Write};
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            w.start_file("[Content_Types].xml", opts).expect("s");
            w.write_all(
                br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#,
            )
            .expect("w");
            w.finish().expect("f");
        }
        let pkg = crate::opc::Package::open(buf).expect("open");
        Env {
            pres: Presentation::new(pkg, 1, 1),
            chain: SlideChain {
                slide: Rc::new(Slide::default()),
                layout: Some(Rc::new(SlideLayout::default())),
                master: Some(Rc::new(SlideMaster::default())),
                theme: Rc::new(Theme::default()),
            },
        }
    }

    fn text_cell(s: &str) -> TableCell {
        let body = TextBody {
            paragraphs: vec![crate::model::Paragraph {
                runs: vec![crate::model::Run::Text {
                    text: s.to_string(),
                    props: Default::default(),
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        TableCell {
            text: Some(body),
            grid_span: 1,
            row_span: 1,
            ..Default::default()
        }
    }

    /// A 2x2 table, 100pt columns and 20pt rows, in EMUs.
    fn grid() -> Table {
        Table {
            props: TableProps::default(),
            columns: vec![emu::from_pt(100.0), emu::from_pt(100.0)],
            rows: vec![
                TableRow {
                    height: emu::from_pt(20.0),
                    cells: vec![text_cell("A1"), text_cell("B1")],
                },
                TableRow {
                    height: emu::from_pt(20.0),
                    cells: vec![text_cell("A2"), text_cell("B2")],
                },
            ],
        }
    }

    fn run(table: &Table, frame: Rect) -> Vec<Command> {
        let e = env();
        let r = Resolver::new(&e.pres, &e.chain);
        let mut out = Vec::new();
        layout_table(table, frame, &r, &e.pres, &StubMeasure, "p", &mut out);
        out
    }

    fn text_positions(cmds: &[Command]) -> Vec<(String, f32, f32)> {
        cmds.iter()
            .filter_map(|c| match c {
                Command::DrawText(r) => Some((r.text.clone(), r.origin.x, r.origin.y)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn cells_are_positioned_on_the_authored_grid() {
        let cmds = run(&grid(), Rect::new(0.0, 0.0, 200.0, 40.0));
        let texts = text_positions(&cmds);
        assert_eq!(texts.len(), 4);
        let a1 = texts.iter().find(|t| t.0 == "A1").expect("A1");
        let b1 = texts.iter().find(|t| t.0 == "B1").expect("B1");
        let a2 = texts.iter().find(|t| t.0 == "A2").expect("A2");
        assert!(b1.1 > a1.1 + 90.0, "B1 should be a column to the right");
        assert!(a2.2 > a1.2 + 10.0, "A2 should be a row below");
    }

    #[test]
    fn columns_are_scaled_when_the_grid_disagrees_with_the_frame() {
        // Authored 200pt wide, drawn into a 400pt frame.
        let cmds = run(&grid(), Rect::new(0.0, 0.0, 400.0, 40.0));
        let texts = text_positions(&cmds);
        let a1 = texts.iter().find(|t| t.0 == "A1").expect("A1");
        let b1 = texts.iter().find(|t| t.0 == "B1").expect("B1");
        assert!(b1.1 - a1.1 > 190.0, "columns should stretch to the frame");
    }

    #[test]
    fn a_merged_cell_spans_both_columns_and_its_partner_draws_nothing() {
        let mut t = grid();
        if let Some(row) = t.rows.get_mut(0) {
            if let Some(c) = row.cells.get_mut(0) {
                c.grid_span = 2;
            }
            if let Some(c) = row.cells.get_mut(1) {
                c.h_merge = true;
            }
        }
        let cmds = run(&t, Rect::new(0.0, 0.0, 200.0, 40.0));
        let texts = text_positions(&cmds);
        assert!(
            !texts.iter().any(|x| x.0 == "B1"),
            "the covered cell must not draw: {texts:?}"
        );
        assert_eq!(texts.len(), 3);
    }

    #[test]
    fn a_vertically_merged_cell_covers_both_rows() {
        let mut t = grid();
        if let Some(row) = t.rows.get_mut(0) {
            if let Some(c) = row.cells.get_mut(1) {
                c.row_span = 2;
            }
        }
        if let Some(row) = t.rows.get_mut(1) {
            if let Some(c) = row.cells.get_mut(1) {
                c.v_merge = true;
            }
        }
        let e = env();
        let r = Resolver::new(&e.pres, &e.chain);
        let cols = column_edges(&t, Rect::new(0.0, 0.0, 200.0, 40.0));
        let rows = row_edges(
            &t,
            Rect::new(0.0, 0.0, 200.0, 40.0),
            &r,
            &StubMeasure,
            &crate::model::table_style::TableStyle::plain_grid(),
        );
        let merged = cell_rect(&t, &cols, &rows, 0, 1).expect("merged cell");
        assert!(
            merged.h > 30.0,
            "merged cell should span both rows: {merged:?}"
        );
        assert!(cell_rect(&t, &cols, &rows, 1, 1).is_none());
    }

    #[test]
    fn rows_grow_to_fit_text_that_wraps() {
        let mut t = grid();
        if let Some(row) = t.rows.get_mut(0) {
            if let Some(c) = row.cells.get_mut(0) {
                *c = text_cell("a fairly long sentence that will certainly need several lines");
            }
        }
        let e = env();
        let r = Resolver::new(&e.pres, &e.chain);
        let frame = Rect::new(0.0, 0.0, 200.0, 40.0);
        let rows = row_edges(
            &t,
            frame,
            &r,
            &StubMeasure,
            &crate::model::table_style::TableStyle::plain_grid(),
        );
        let first_row_height =
            rows.get(1).copied().unwrap_or(0.0) - rows.first().copied().unwrap_or(0.0);
        assert!(
            first_row_height > 20.0,
            "row should grow past its authored 20pt, got {first_row_height}"
        );
    }

    #[test]
    fn cell_fills_and_borders_are_emitted() {
        let mut t = grid();
        if let Some(row) = t.rows.get_mut(0) {
            if let Some(c) = row.cells.get_mut(0) {
                c.fill = Fill::Solid(ColorRef::srgb(crate::dl::Color::rgb(1, 2, 3)));
                c.border_bottom = Line {
                    width: Some(12_700),
                    fill: Fill::Solid(ColorRef::srgb(crate::dl::Color::BLACK)),
                    ..Default::default()
                };
            }
        }
        let cmds = run(&t, Rect::new(0.0, 0.0, 200.0, 40.0));
        // This deck declares no table style, so the fallback is the plain grid: rules on
        // every cell, no fills of its own. The one fill is therefore the cell's own.
        let fills: Vec<_> = cmds
            .iter()
            .filter_map(|c| match c {
                Command::FillPath { paint, .. } => Some(paint.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(fills, vec![Paint::Solid(crate::dl::Color::rgb(1, 2, 3))]);
        let strokes = cmds
            .iter()
            .filter(|c| matches!(c, Command::StrokePath { .. }))
            .count();
        assert_eq!(strokes, 16, "four edges on each of four cells");
    }

    #[test]
    fn a_cells_own_border_overrides_the_table_styles() {
        let mut t = grid();
        if let Some(row) = t.rows.get_mut(0) {
            if let Some(c) = row.cells.get_mut(0) {
                c.border_top = Line {
                    width: Some(50_800), // 4pt, unmistakably not the style's rule
                    fill: Fill::Solid(ColorRef::srgb(crate::dl::Color::rgb(255, 0, 0))),
                    ..Default::default()
                };
            }
        }
        let cmds = run(&t, Rect::new(0.0, 0.0, 200.0, 40.0));
        let widths: Vec<f32> = cmds
            .iter()
            .filter_map(|c| match c {
                Command::StrokePath { stroke, .. } => Some(stroke.width),
                _ => None,
            })
            .collect();
        assert!(
            widths.contains(&4.0),
            "expected the 4pt override, got {widths:?}"
        );
    }

    #[test]
    fn the_built_in_header_style_fills_the_first_row_and_bolds_its_text() {
        let mut t = grid();
        t.props.first_row = true;
        t.props.band_row = true;
        t.props.style_id = Some("{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}".into());
        let cmds = run(&t, Rect::new(0.0, 0.0, 200.0, 40.0));

        // The header row's fill is the untinted accent; the body rows are tints of it, so
        // the header must be the most saturated fill on the table.
        let header_fill = cmds.iter().find_map(|c| match c {
            Command::FillPath {
                paint: Paint::Solid(c),
                path,
                ..
            } if path.bounds().y < 1.0 => Some(*c),
            _ => None,
        });
        assert_eq!(
            header_fill,
            Some(crate::dl::Color::rgb(0x44, 0x72, 0xC4)),
            "the header should be the theme accent1"
        );

        let header_text = cmds.iter().find_map(|c| match c {
            Command::DrawText(r) if r.text == "A1" => Some(r.clone()),
            _ => None,
        });
        let run = header_text.expect("header text");
        assert!(
            run.font.to_css().contains("700"),
            "header text should be bold"
        );
        assert_eq!(
            run.paint,
            Paint::Solid(crate::dl::Color::WHITE),
            "header text should take the style's colour"
        );
    }

    #[test]
    fn borders_are_emitted_after_every_fill_so_shared_edges_survive() {
        let mut t = grid();
        for row in &mut t.rows {
            for c in &mut row.cells {
                c.fill = Fill::Solid(ColorRef::srgb(crate::dl::Color::WHITE));
                c.border_top = Line {
                    width: Some(12_700),
                    fill: Fill::Solid(ColorRef::srgb(crate::dl::Color::BLACK)),
                    ..Default::default()
                };
            }
        }
        let cmds = run(&t, Rect::new(0.0, 0.0, 200.0, 40.0));
        let last_fill = cmds
            .iter()
            .rposition(|c| matches!(c, Command::FillPath { .. }))
            .unwrap_or(0);
        let first_stroke = cmds
            .iter()
            .position(|c| matches!(c, Command::StrokePath { .. }))
            .unwrap_or(usize::MAX);
        assert!(
            first_stroke > last_fill,
            "all fills must precede all borders"
        );
    }

    #[test]
    fn an_empty_table_emits_nothing() {
        let t = Table::default();
        assert!(run(&t, Rect::new(0.0, 0.0, 100.0, 100.0)).is_empty());
    }

    #[test]
    fn a_table_with_zero_width_columns_does_not_divide_by_zero() {
        let t = Table {
            columns: vec![0, 0],
            rows: vec![TableRow {
                height: 0,
                cells: vec![text_cell("x"), text_cell("y")],
            }],
            props: TableProps::default(),
        };
        let cmds = run(&t, Rect::new(0.0, 0.0, 100.0, 100.0));
        for c in &cmds {
            if let Command::DrawText(r) = c {
                assert!(r.origin.x.is_finite() && r.origin.y.is_finite());
            }
        }
    }
}
