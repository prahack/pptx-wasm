//! Tables (`<a:tbl>` inside a `<p:graphicFrame>`).
//!
//! The grid is stored exactly as authored: merged cells are still present as rows and
//! columns, marked with `h_merge`/`v_merge`, rather than being collapsed. Layout needs
//! the un-collapsed grid to compute spans, and a collapsed model cannot be un-collapsed.

use crate::emu::Emu;

use super::fill::{Fill, Line};
use super::text::TextBody;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TableCell {
    pub text: Option<TextBody>,
    /// Number of grid columns this cell spans; 1 for an unmerged cell.
    pub grid_span: u32,
    /// Number of grid rows this cell spans.
    pub row_span: u32,
    /// True when this cell is covered by a horizontal merge from its left.
    pub h_merge: bool,
    /// True when this cell is covered by a vertical merge from above.
    pub v_merge: bool,
    pub fill: Fill,
    pub border_left: Line,
    pub border_right: Line,
    pub border_top: Line,
    pub border_bottom: Line,
    /// Diagonal borders, rare but cheap to carry.
    pub border_tl_br: Line,
    pub border_tr_bl: Line,
    pub margin_left: Option<Emu>,
    pub margin_right: Option<Emu>,
    pub margin_top: Option<Emu>,
    pub margin_bottom: Option<Emu>,
    pub vertical_anchor: Option<super::text::VerticalAnchor>,
}

impl TableCell {
    /// ECMA-376 default cell margins: 0.1" horizontal, 0.05" vertical.
    pub fn resolved_margins(&self) -> (Emu, Emu, Emu, Emu) {
        (
            self.margin_left.unwrap_or(91_440),
            self.margin_top.unwrap_or(45_720),
            self.margin_right.unwrap_or(91_440),
            self.margin_bottom.unwrap_or(45_720),
        )
    }

    /// True when this cell is only a placeholder for a merge and draws no content.
    pub fn is_covered(&self) -> bool {
        self.h_merge || self.v_merge
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TableRow {
    /// Authored height in EMUs. Rows grow to fit their text, never shrink below this.
    pub height: Emu,
    pub cells: Vec<TableCell>,
}

/// `<a:tblPr>` — which parts of the table style are switched on.
#[derive(Debug, Clone, PartialEq)]
pub struct TableProps {
    pub first_row: bool,
    pub last_row: bool,
    pub first_col: bool,
    pub last_col: bool,
    pub band_row: bool,
    pub band_col: bool,
    pub rtl: bool,
    /// Relationship-independent id of the table style in `tableStyles.xml`.
    pub style_id: Option<String>,
    /// A fill declared directly on the table.
    pub fill: Fill,
}

impl Default for TableProps {
    fn default() -> Self {
        TableProps {
            first_row: false,
            last_row: false,
            first_col: false,
            last_col: false,
            band_row: false,
            band_col: false,
            rtl: false,
            style_id: None,
            fill: Fill::Inherit,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Table {
    pub props: TableProps,
    /// Column widths in EMUs, from `<a:gridCol>`.
    pub columns: Vec<Emu>,
    pub rows: Vec<TableRow>,
}

impl Table {
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn cell(&self, row: usize, col: usize) -> Option<&TableCell> {
        self.rows.get(row)?.cells.get(col)
    }

    /// Total authored width.
    pub fn total_width(&self) -> Emu {
        self.columns.iter().sum()
    }

    /// The grid rectangle a cell occupies, in (col, row, col_span, row_span) form,
    /// or `None` if the cell is covered by another cell's merge.
    ///
    /// `rowSpan` is authored only on the top-left cell of a vertical merge, so the span
    /// is taken from there; `gridSpan` likewise for horizontal merges.
    pub fn cell_span(&self, row: usize, col: usize) -> Option<(usize, usize, usize, usize)> {
        let c = self.cell(row, col)?;
        if c.is_covered() {
            return None;
        }
        let col_span = c.grid_span.max(1) as usize;
        let row_span = c.row_span.max(1) as usize;
        Some((col, row, col_span, row_span))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(grid_span: u32, row_span: u32, h: bool, v: bool) -> TableCell {
        TableCell {
            grid_span,
            row_span,
            h_merge: h,
            v_merge: v,
            ..Default::default()
        }
    }

    /// A 3x2 table where the first row's first two cells are merged horizontally
    /// and column 2 is merged vertically down both rows.
    fn merged_table() -> Table {
        Table {
            columns: vec![100, 100, 100],
            rows: vec![
                TableRow {
                    height: 50,
                    cells: vec![
                        cell(2, 1, false, false),
                        cell(1, 1, true, false),
                        cell(1, 2, false, false),
                    ],
                },
                TableRow {
                    height: 50,
                    cells: vec![
                        cell(1, 1, false, false),
                        cell(1, 1, false, false),
                        cell(1, 1, false, true),
                    ],
                },
            ],
            props: TableProps::default(),
        }
    }

    #[test]
    fn spans_come_from_the_originating_cell() {
        let t = merged_table();
        assert_eq!(
            t.cell_span(0, 0),
            Some((0, 0, 2, 1)),
            "horizontal merge of 2"
        );
        assert_eq!(t.cell_span(0, 2), Some((2, 0, 1, 2)), "vertical merge of 2");
    }

    #[test]
    fn covered_cells_report_no_span_so_they_draw_nothing() {
        let t = merged_table();
        assert_eq!(t.cell_span(0, 1), None, "covered by the hMerge to its left");
        assert_eq!(t.cell_span(1, 2), None, "covered by the vMerge above");
    }

    #[test]
    fn a_span_of_zero_is_treated_as_one() {
        let t = Table {
            columns: vec![100],
            rows: vec![TableRow {
                height: 10,
                cells: vec![cell(0, 0, false, false)],
            }],
            props: TableProps::default(),
        };
        assert_eq!(t.cell_span(0, 0), Some((0, 0, 1, 1)));
    }

    #[test]
    fn out_of_range_lookups_return_none() {
        let t = merged_table();
        assert!(t.cell(9, 0).is_none());
        assert!(t.cell(0, 9).is_none());
        assert!(t.cell_span(9, 9).is_none());
    }

    #[test]
    fn geometry_helpers_report_the_authored_grid() {
        let t = merged_table();
        assert_eq!(t.column_count(), 3);
        assert_eq!(t.row_count(), 2);
        assert_eq!(t.total_width(), 300);
    }

    #[test]
    fn default_cell_margins_are_the_ecma_values() {
        assert_eq!(
            TableCell::default().resolved_margins(),
            (91_440, 45_720, 91_440, 45_720)
        );
    }
}
