//! Table styles.
//!
//! **The awkward fact this module exists for:** a `.pptx` that uses one of PowerPoint's
//! built-in table styles does not contain that style. `tableStyles.xml` is typically just
//! `<a:tblStyleLst def="{5C22544A-…}"/>` — a GUID and nothing else. The definition lives
//! inside PowerPoint. Every other renderer, LibreOffice included, hard-codes them.
//!
//! So do we. [`TableStyle::builtin`] synthesises the styles that actually appear in decks;
//! [`crate::parse::table_style`] parses any style the file *does* define, which takes
//! precedence. A GUID we do not recognise falls back to a plain grid rather than to
//! nothing, because an unstyled table with no borders is much harder to read than a
//! slightly wrong one.

use crate::model::color::{ColorMod, ColorRef, SchemeColor};
use crate::model::fill::{Fill, Line};

/// The formatting one style part contributes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PartStyle {
    pub fill: Fill,
    pub left: Line,
    pub right: Line,
    pub top: Line,
    pub bottom: Line,
    /// Borders between cells, which a part can style separately from its outer edges.
    pub inside_h: Line,
    pub inside_v: Line,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub color: Option<ColorRef>,
}

impl PartStyle {
    /// Fills anything unspecified here from `under`. Called from most specific to least.
    pub fn inherit_from(&mut self, under: &PartStyle) {
        if !self.fill.is_specified() {
            self.fill = under.fill.clone();
        }
        self.left.inherit_from(&under.left);
        self.right.inherit_from(&under.right);
        self.top.inherit_from(&under.top);
        self.bottom.inherit_from(&under.bottom);
        self.inside_h.inherit_from(&under.inside_h);
        self.inside_v.inherit_from(&under.inside_v);
        self.bold = self.bold.or(under.bold);
        self.italic = self.italic.or(under.italic);
        if self.color.is_none() {
            self.color = under.color.clone();
        }
    }
}

/// Where a cell sits, which decides which parts apply to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellPosition {
    pub row: usize,
    pub col: usize,
    pub row_count: usize,
    pub col_count: usize,
}

impl CellPosition {
    pub fn is_first_row(&self) -> bool {
        self.row == 0
    }
    pub fn is_last_row(&self) -> bool {
        self.row_count > 0 && self.row == self.row_count - 1
    }
    pub fn is_first_col(&self) -> bool {
        self.col == 0
    }
    pub fn is_last_col(&self) -> bool {
        self.col_count > 0 && self.col == self.col_count - 1
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TableStyle {
    pub id: String,
    pub name: String,
    pub whole_table: PartStyle,
    pub band1_h: PartStyle,
    pub band2_h: PartStyle,
    pub band1_v: PartStyle,
    pub band2_v: PartStyle,
    pub first_row: PartStyle,
    pub last_row: PartStyle,
    pub first_col: PartStyle,
    pub last_col: PartStyle,
    pub nw_cell: PartStyle,
    pub ne_cell: PartStyle,
    pub sw_cell: PartStyle,
    pub se_cell: PartStyle,
}

impl TableStyle {
    /// The style for one cell, honouring which parts the table has switched on.
    ///
    /// Applied least-specific first so that later parts override earlier ones, which is
    /// the order ECMA-376 §20.1.4.2.26 specifies: whole table, then column bands, then
    /// row bands, then first/last column, then first/last row, then the corner cells.
    pub fn resolve(&self, pos: CellPosition, props: &super::table::TableProps) -> PartStyle {
        let mut parts: Vec<&PartStyle> = vec![&self.whole_table];

        // Banding counts only the rows and columns that are not header/footer, so a
        // table with a header row starts its first band on the row below it.
        if props.band_col {
            let first = usize::from(props.first_col);
            let last = usize::from(props.last_col);
            if pos.col >= first && pos.col + last < pos.col_count {
                let band = (pos.col - first) % 2;
                parts.push(if band == 0 {
                    &self.band1_v
                } else {
                    &self.band2_v
                });
            }
        }
        if props.band_row {
            let first = usize::from(props.first_row);
            let last = usize::from(props.last_row);
            if pos.row >= first && pos.row + last < pos.row_count {
                let band = (pos.row - first) % 2;
                parts.push(if band == 0 {
                    &self.band1_h
                } else {
                    &self.band2_h
                });
            }
        }
        if props.first_col && pos.is_first_col() {
            parts.push(&self.first_col);
        }
        if props.last_col && pos.is_last_col() {
            parts.push(&self.last_col);
        }
        if props.first_row && pos.is_first_row() {
            parts.push(&self.first_row);
            if props.first_col && pos.is_first_col() {
                parts.push(&self.nw_cell);
            }
            if props.last_col && pos.is_last_col() {
                parts.push(&self.ne_cell);
            }
        }
        if props.last_row && pos.is_last_row() {
            parts.push(&self.last_row);
            if props.first_col && pos.is_first_col() {
                parts.push(&self.sw_cell);
            }
            if props.last_col && pos.is_last_col() {
                parts.push(&self.se_cell);
            }
        }

        // Merge most-specific first, since `inherit_from` only fills gaps.
        let mut out = PartStyle::default();
        for part in parts.iter().rev() {
            out.inherit_from(part);
        }
        out
    }

    /// A built-in style, by GUID.
    ///
    /// Covers the "Medium Style 2 - Accent N" family — PowerPoint's default for an
    /// inserted table, and by far the most common style in the wild — plus the plain
    /// grid. Anything else falls back to [`TableStyle::plain_grid`].
    pub fn builtin(id: &str) -> Option<TableStyle> {
        let upper = id.to_ascii_uppercase();
        let accent = match upper.trim_matches(|c| c == '{' || c == '}') {
            "5C22544A-7EE6-4342-B048-85BDC9FD1C3A" => SchemeColor::Accent1,
            "21E4AEA4-8DFA-4A89-87EB-49C32662AFE0" => SchemeColor::Accent2,
            "F5AB1C69-6EDB-4FF4-983F-18BD219EF322" => SchemeColor::Accent3,
            "00A15C55-8517-42AA-B614-E9B94910E393" => SchemeColor::Accent4,
            "7DF18680-E054-41AD-8BC1-D1AEF772440D" => SchemeColor::Accent5,
            "93296810-A885-4BE3-A3E7-6D5BEEA58F35" => SchemeColor::Accent6,
            "5940675A-B579-460E-94D1-54222C63F5DA" => return Some(Self::plain_grid()),
            _ => return None,
        };
        Some(Self::medium_style_2(id, accent))
    }

    /// PowerPoint's "Medium Style 2 - Accent N".
    ///
    /// Whole table tinted 20%, alternate rows tinted 40%, a solid accent header with
    /// white bold text, and white rules throughout.
    pub fn medium_style_2(id: &str, accent: SchemeColor) -> TableStyle {
        let tinted = |t: f32| {
            Fill::Solid(ColorRef {
                spec: crate::model::color::ColorSpec::Scheme(accent),
                mods: vec![ColorMod::Tint(t)],
            })
        };
        let white_rule = || Line {
            width: Some(12_700),
            fill: Fill::Solid(ColorRef::scheme(SchemeColor::Light1)),
            ..Default::default()
        };
        let bordered = |fill: Fill| PartStyle {
            fill,
            left: white_rule(),
            right: white_rule(),
            top: white_rule(),
            bottom: white_rule(),
            inside_h: white_rule(),
            inside_v: white_rule(),
            ..Default::default()
        };

        TableStyle {
            id: id.to_string(),
            name: "Medium Style 2".to_string(),
            whole_table: bordered(tinted(0.2)),
            band1_h: PartStyle {
                fill: tinted(0.4),
                ..Default::default()
            },
            band1_v: PartStyle {
                fill: tinted(0.4),
                ..Default::default()
            },
            first_row: PartStyle {
                fill: Fill::Solid(ColorRef::scheme(accent)),
                bold: Some(true),
                color: Some(ColorRef::scheme(SchemeColor::Light1)),
                ..Default::default()
            },
            last_row: PartStyle {
                fill: Fill::Solid(ColorRef::scheme(SchemeColor::Light1)),
                bold: Some(true),
                top: Line {
                    width: Some(25_400),
                    fill: Fill::Solid(ColorRef::scheme(accent)),
                    ..Default::default()
                },
                ..Default::default()
            },
            first_col: PartStyle {
                fill: Fill::Solid(ColorRef::scheme(accent)),
                bold: Some(true),
                color: Some(ColorRef::scheme(SchemeColor::Light1)),
                ..Default::default()
            },
            last_col: PartStyle {
                fill: Fill::Solid(ColorRef::scheme(accent)),
                bold: Some(true),
                color: Some(ColorRef::scheme(SchemeColor::Light1)),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// "No Style, Table Grid": thin dark rules, no fills. Also the fallback for a style
    /// we do not recognise.
    pub fn plain_grid() -> TableStyle {
        let rule = || Line {
            width: Some(12_700),
            fill: Fill::Solid(ColorRef {
                spec: crate::model::color::ColorSpec::Scheme(SchemeColor::Text1),
                mods: vec![ColorMod::Tint(0.4)],
            }),
            ..Default::default()
        };
        TableStyle {
            id: String::new(),
            name: "Table Grid".to_string(),
            whole_table: PartStyle {
                fill: Fill::NoFill,
                left: rule(),
                right: rule(),
                top: rule(),
                bottom: rule(),
                inside_h: rule(),
                inside_v: rule(),
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

/// Every style a deck defines, plus its default.
#[derive(Debug, Clone, Default)]
pub struct TableStyles {
    pub default_id: String,
    pub styles: Vec<TableStyle>,
}

impl TableStyles {
    /// Looks a style up: the deck's own definition first, then the built-in table, then
    /// a plain grid.
    pub fn get(&self, id: Option<&str>) -> TableStyle {
        let id = id.unwrap_or(&self.default_id);
        if let Some(defined) = self.styles.iter().find(|s| eq_guid(&s.id, id)) {
            return defined.clone();
        }
        TableStyle::builtin(id).unwrap_or_else(|| {
            if id.is_empty() {
                TableStyle::plain_grid()
            } else {
                log::debug!("table style {id} is not defined in the deck and is not built in");
                TableStyle::plain_grid()
            }
        })
    }
}

/// GUID comparison that ignores case and surrounding braces, both of which vary between
/// producers for the same style.
fn eq_guid(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        s.trim()
            .trim_matches(|c| c == '{' || c == '}')
            .to_ascii_uppercase()
    };
    norm(a) == norm(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::table::TableProps;

    fn pos(row: usize, col: usize) -> CellPosition {
        CellPosition {
            row,
            col,
            row_count: 5,
            col_count: 4,
        }
    }

    fn header_and_bands() -> TableProps {
        TableProps {
            first_row: true,
            band_row: true,
            ..Default::default()
        }
    }

    #[test]
    fn the_powerpoint_default_style_guid_is_recognised() {
        let s = TableStyle::builtin("{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}").expect("builtin");
        assert_eq!(s.name, "Medium Style 2");
        // Case and braces vary between producers.
        assert!(TableStyle::builtin("5c22544a-7ee6-4342-b048-85bdc9fd1c3a").is_some());
    }

    #[test]
    fn an_unknown_guid_is_not_built_in_but_still_resolves_to_a_grid() {
        assert!(TableStyle::builtin("{00000000-0000-0000-0000-000000000000}").is_none());
        let styles = TableStyles {
            default_id: "{00000000-0000-0000-0000-000000000000}".into(),
            styles: Vec::new(),
        };
        let resolved = styles.get(None);
        assert_eq!(resolved.name, "Table Grid");
        assert!(
            !resolved.whole_table.top.is_empty(),
            "a fallback must still draw rules"
        );
    }

    #[test]
    fn a_style_defined_in_the_deck_beats_the_built_in_table() {
        let mut custom = TableStyle::plain_grid();
        custom.id = "{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}".into();
        custom.name = "Custom".into();
        let styles = TableStyles {
            default_id: custom.id.clone(),
            styles: vec![custom],
        };
        assert_eq!(styles.get(None).name, "Custom");
    }

    #[test]
    fn the_header_row_wins_over_the_banding_underneath_it() {
        let style = TableStyle::medium_style_2("x", SchemeColor::Accent1);
        let props = header_and_bands();
        let header = style.resolve(pos(0, 0), &props);
        match header.fill {
            Fill::Solid(ref c) => {
                assert!(c.mods.is_empty(), "the header is solid accent, untinted")
            }
            ref other => panic!("expected a solid header fill, got {other:?}"),
        }
        assert_eq!(header.bold, Some(true));
        assert!(header.color.is_some(), "the header has its own text colour");
    }

    #[test]
    fn banding_starts_below_the_header_and_alternates() {
        let style = TableStyle::medium_style_2("x", SchemeColor::Accent1);
        let props = header_and_bands();
        let tint_of = |row: usize| match style.resolve(pos(row, 0), &props).fill {
            Fill::Solid(c) => c.mods.first().copied(),
            _ => None,
        };
        // Row 1 is the first banded row, so it gets band1 (40%); row 2 falls through to
        // the whole-table 20%; row 3 is band1 again.
        assert_eq!(tint_of(1), Some(ColorMod::Tint(0.4)));
        assert_eq!(tint_of(2), Some(ColorMod::Tint(0.2)));
        assert_eq!(tint_of(3), Some(ColorMod::Tint(0.4)));
    }

    #[test]
    fn banding_is_off_when_the_table_does_not_ask_for_it() {
        let style = TableStyle::medium_style_2("x", SchemeColor::Accent1);
        let props = TableProps {
            first_row: true,
            band_row: false,
            ..Default::default()
        };
        let tint_of = |row: usize| match style.resolve(pos(row, 0), &props).fill {
            Fill::Solid(c) => c.mods.first().copied(),
            _ => None,
        };
        assert_eq!(tint_of(1), Some(ColorMod::Tint(0.2)));
        assert_eq!(tint_of(2), Some(ColorMod::Tint(0.2)));
    }

    #[test]
    fn every_cell_gets_the_whole_tables_rules() {
        let style = TableStyle::medium_style_2("x", SchemeColor::Accent1);
        let props = header_and_bands();
        for (row, col) in [(0, 0), (2, 1), (4, 3)] {
            let s = style.resolve(pos(row, col), &props);
            assert!(!s.top.is_empty(), "cell {row},{col} has no top rule");
            assert!(!s.inside_v.is_empty(), "cell {row},{col} has no inner rule");
        }
    }

    #[test]
    fn the_last_row_overrides_the_band_it_sits_in() {
        let style = TableStyle::medium_style_2("x", SchemeColor::Accent1);
        let props = TableProps {
            first_row: true,
            last_row: true,
            band_row: true,
            ..Default::default()
        };
        let last = style.resolve(pos(4, 0), &props);
        assert_eq!(last.bold, Some(true));
        assert_eq!(
            last.top.width,
            Some(25_400),
            "the total row gets a heavier rule"
        );
    }

    #[test]
    fn part_styles_merge_without_overwriting_what_is_already_set() {
        let mut a = PartStyle {
            bold: Some(false),
            ..Default::default()
        };
        let b = PartStyle {
            bold: Some(true),
            italic: Some(true),
            ..Default::default()
        };
        a.inherit_from(&b);
        assert_eq!(a.bold, Some(false), "an explicit value must survive");
        assert_eq!(a.italic, Some(true));
    }
}
