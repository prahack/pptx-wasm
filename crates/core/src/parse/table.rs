//! `<a:tbl>` — table grid, rows, cells and their borders.

use crate::model::table::{Table, TableCell, TableProps, TableRow};
use crate::model::text::VerticalAnchor;

use super::drawing::{children, parse_fill, parse_line};
use super::text::parse_text_body;
use super::xml::{attr, attr_bool, attr_i64, attr_u32, is, local_name, Reader};

/// Parses the body of an `<a:tbl>` element the reader has just entered.
pub fn parse_table(r: &mut Reader<'_>) -> Table {
    let mut table = Table::default();
    children(r, b"tbl", |r, e, empty| {
        match local_name(e.name().as_ref()) {
            b"tblPr" => {
                table.props = parse_table_props(r, e, empty);
                !empty
            }
            b"tblGrid" => {
                if empty {
                    return true;
                }
                children(r, b"tblGrid", |_r, col, _ce| {
                    if is(col, b"gridCol") {
                        table.columns.push(attr_i64(col, b"w").unwrap_or(0));
                    }
                    false
                });
                true
            }
            b"tr" => {
                if empty {
                    return true;
                }
                table.rows.push(parse_row(r, e));
                true
            }
            _ => false,
        }
    });
    normalize(&mut table);
    table
}

fn parse_table_props(
    r: &mut Reader<'_>,
    e: &quick_xml::events::BytesStart<'_>,
    empty: bool,
) -> TableProps {
    let mut p = TableProps {
        first_row: attr_bool(e, b"firstRow").unwrap_or(false),
        last_row: attr_bool(e, b"lastRow").unwrap_or(false),
        first_col: attr_bool(e, b"firstCol").unwrap_or(false),
        last_col: attr_bool(e, b"lastCol").unwrap_or(false),
        band_row: attr_bool(e, b"bandRow").unwrap_or(false),
        band_col: attr_bool(e, b"bandCol").unwrap_or(false),
        rtl: attr_bool(e, b"rtl").unwrap_or(false),
        ..Default::default()
    };
    if empty {
        return p;
    }
    children(r, b"tblPr", |r, child, child_empty| {
        if let Some(f) = parse_fill(r, child, child_empty) {
            p.fill = f;
            return !child_empty;
        }
        if is(child, b"tableStyleId") && !child_empty {
            p.style_id = Some(super::xml::text_content(r, b"tableStyleId"));
            return true;
        }
        false
    });
    p
}

fn parse_row(r: &mut Reader<'_>, e: &quick_xml::events::BytesStart<'_>) -> TableRow {
    let mut row = TableRow {
        height: attr_i64(e, b"h").unwrap_or(0),
        cells: Vec::new(),
    };
    children(r, b"tr", |r, child, child_empty| {
        if !is(child, b"tc") {
            return false;
        }
        let mut cell = TableCell {
            grid_span: attr_u32(child, b"gridSpan").unwrap_or(1),
            row_span: attr_u32(child, b"rowSpan").unwrap_or(1),
            h_merge: attr_bool(child, b"hMerge").unwrap_or(false),
            v_merge: attr_bool(child, b"vMerge").unwrap_or(false),
            ..Default::default()
        };
        if !child_empty {
            parse_cell_body(r, &mut cell);
        }
        row.cells.push(cell);
        !child_empty
    });
    row
}

fn parse_cell_body(r: &mut Reader<'_>, cell: &mut TableCell) {
    children(r, b"tc", |r, e, empty| {
        match local_name(e.name().as_ref()) {
            b"txBody" => {
                if empty {
                    return true;
                }
                cell.text = Some(parse_text_body(r, b"txBody"));
                true
            }
            b"tcPr" => {
                if empty {
                    // Attributes still matter even with no children.
                    apply_cell_props_attrs(e, cell);
                    return true;
                }
                apply_cell_props_attrs(e, cell);
                children(r, b"tcPr", |r, inner, inner_empty| {
                    let name = local_name(inner.name().as_ref()).to_vec();
                    if let Some(f) = parse_fill(r, inner, inner_empty) {
                        cell.fill = f;
                        return !inner_empty;
                    }
                    let border = match name.as_slice() {
                        b"lnL" => Some(&mut cell.border_left),
                        b"lnR" => Some(&mut cell.border_right),
                        b"lnT" => Some(&mut cell.border_top),
                        b"lnB" => Some(&mut cell.border_bottom),
                        b"lnTlToBr" => Some(&mut cell.border_tl_br),
                        b"lnBlToTr" => Some(&mut cell.border_tr_bl),
                        _ => None,
                    };
                    match border {
                        Some(slot) => {
                            *slot = parse_line(r, inner, inner_empty);
                            !inner_empty
                        }
                        None => false,
                    }
                });
                true
            }
            _ => false,
        }
    });
}

fn apply_cell_props_attrs(e: &quick_xml::events::BytesStart<'_>, cell: &mut TableCell) {
    cell.margin_left = attr_i64(e, b"marL");
    cell.margin_right = attr_i64(e, b"marR");
    cell.margin_top = attr_i64(e, b"marT");
    cell.margin_bottom = attr_i64(e, b"marB");
    cell.vertical_anchor = attr(e, b"anchor")
        .as_deref()
        .and_then(VerticalAnchor::parse);
}

/// Squares off the grid.
///
/// Decks do exist whose `<a:tblGrid>` disagrees with the number of `<a:tc>` elements in a
/// row. Layout indexes rows and columns together, so pad both to the widest row rather
/// than let a short row silently drop cells.
fn normalize(table: &mut Table) {
    let widest = table.rows.iter().map(|r| r.cells.len()).max().unwrap_or(0);
    let cols = table.columns.len().max(widest);
    if cols > table.columns.len() {
        // Give invented columns the average width so the table still spans its frame.
        let avg = if table.columns.is_empty() {
            914_400
        } else {
            table.total_width() / table.columns.len() as i64
        };
        table.columns.resize(cols, avg);
    }
    for row in &mut table.rows {
        if row.cells.len() < cols {
            row.cells.resize(cols, TableCell::default());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quick_xml::events::Event;

    fn table(xml: &str) -> Table {
        let mut r = Reader::new(xml.as_bytes());
        let mut buf = Vec::new();
        loop {
            match r.read_event_into(&mut buf) {
                Ok(Event::Start(_)) => return parse_table(&mut r),
                Ok(Event::Eof) => panic!("no element"),
                _ => {}
            }
        }
    }

    #[test]
    fn parses_the_grid_rows_and_cell_text() {
        let t = table(
            r#"<a:tbl>
                 <a:tblPr firstRow="1" bandRow="1"><a:tableStyleId>{5C22544A}</a:tableStyleId></a:tblPr>
                 <a:tblGrid><a:gridCol w="3048000"/><a:gridCol w="3048000"/></a:tblGrid>
                 <a:tr h="370840">
                   <a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:t>A1</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc>
                   <a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:t>B1</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc>
                 </a:tr>
               </a:tbl>"#,
        );
        assert_eq!(t.columns, vec![3_048_000, 3_048_000]);
        assert_eq!(t.rows.len(), 1);
        assert_eq!(t.rows[0].height, 370_840);
        assert!(t.props.first_row && t.props.band_row);
        assert_eq!(t.props.style_id.as_deref(), Some("{5C22544A}"));
        assert_eq!(
            t.cell(0, 1)
                .and_then(|c| c.text.as_ref())
                .map(|t| t.plain_text()),
            Some("B1".into())
        );
    }

    #[test]
    fn merges_are_recorded_on_both_the_origin_and_the_covered_cells() {
        let t = table(
            r#"<a:tbl>
                 <a:tblGrid><a:gridCol w="100"/><a:gridCol w="100"/></a:tblGrid>
                 <a:tr h="10">
                   <a:tc gridSpan="2"><a:txBody><a:bodyPr/><a:p><a:r><a:t>wide</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc>
                   <a:tc hMerge="1"><a:txBody><a:bodyPr/><a:p/></a:txBody><a:tcPr/></a:tc>
                 </a:tr>
               </a:tbl>"#,
        );
        assert_eq!(t.cell_span(0, 0), Some((0, 0, 2, 1)));
        assert_eq!(t.cell_span(0, 1), None);
    }

    #[test]
    fn cell_borders_and_fill_are_read_from_the_cell_properties() {
        let t = table(
            r#"<a:tbl>
                 <a:tblGrid><a:gridCol w="100"/></a:tblGrid>
                 <a:tr h="10"><a:tc>
                   <a:txBody><a:bodyPr/><a:p/></a:txBody>
                   <a:tcPr marL="0" anchor="ctr">
                     <a:lnB w="12700"><a:solidFill><a:srgbClr val="000000"/></a:solidFill></a:lnB>
                     <a:solidFill><a:srgbClr val="EEEEEE"/></a:solidFill>
                   </a:tcPr>
                 </a:tc></a:tr>
               </a:tbl>"#,
        );
        let c = t.cell(0, 0).expect("cell");
        assert_eq!(c.border_bottom.width, Some(12_700));
        assert!(
            c.border_top.is_empty(),
            "unspecified borders stay inheritable"
        );
        assert!(matches!(c.fill, crate::model::fill::Fill::Solid(_)));
        assert_eq!(c.margin_left, Some(0));
        assert_eq!(c.vertical_anchor, Some(VerticalAnchor::Middle));
    }

    #[test]
    fn a_row_shorter_than_the_grid_is_padded_rather_than_truncating_the_table() {
        let t = table(
            r#"<a:tbl>
                 <a:tblGrid><a:gridCol w="100"/><a:gridCol w="100"/><a:gridCol w="100"/></a:tblGrid>
                 <a:tr h="10"><a:tc><a:txBody><a:bodyPr/><a:p/></a:txBody><a:tcPr/></a:tc></a:tr>
               </a:tbl>"#,
        );
        assert_eq!(t.column_count(), 3);
        assert_eq!(t.rows[0].cells.len(), 3);
        assert!(t.cell(0, 2).is_some());
    }

    #[test]
    fn more_cells_than_grid_columns_grows_the_grid() {
        let t = table(
            r#"<a:tbl>
                 <a:tblGrid><a:gridCol w="100"/></a:tblGrid>
                 <a:tr h="10">
                   <a:tc><a:txBody><a:bodyPr/><a:p/></a:txBody><a:tcPr/></a:tc>
                   <a:tc><a:txBody><a:bodyPr/><a:p/></a:txBody><a:tcPr/></a:tc>
                 </a:tr>
               </a:tbl>"#,
        );
        assert_eq!(t.column_count(), 2);
        assert_eq!(t.columns.get(1), Some(&100));
    }

    #[test]
    fn an_empty_table_element_does_not_panic() {
        let t = table(r#"<a:tbl></a:tbl>"#);
        assert_eq!(t.row_count(), 0);
        assert_eq!(t.column_count(), 0);
    }
}
