//! `chart.xml` — the DrawingML charting part.
//!
//! Series values are read from the `<c:numCache>`/`<c:strCache>` blocks the authoring app
//! writes next to each spreadsheet reference. Those caches are the only reason an offline
//! viewer can draw a chart at all: the actual data lives in an embedded `.xlsx` that
//! would otherwise have to be unzipped, parsed and evaluated.

use quick_xml::events::{BytesStart, Event};

use crate::model::chart::{
    Axis, AxisKind, AxisPosition, Chart, Grouping, Legend, LegendPosition, Plot, PlotKind, Series,
};

use super::drawing::{children, parse_fill, parse_line};
use super::text::parse_text_body;
use super::xml::{attr, attr_bool, attr_f32, attr_u32, is, local_name, text_content, Reader};

use crate::model::chart::format_number;

pub fn parse_chart(xml: &[u8]) -> Chart {
    let mut chart = Chart::default();
    let mut reader = Reader::new(xml);
    let mut buf = Vec::new();

    let root = loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => break local_name(e.name().as_ref()).to_vec(),
            Ok(Event::Eof) | Err(_) => return chart,
            _ => {}
        }
    };

    children(&mut reader, &root, |r, e, empty| {
        match local_name(e.name().as_ref()) {
            b"chart" => {
                if empty {
                    return true;
                }
                parse_chart_body(r, &mut chart);
                true
            }
            b"spPr" => {
                if empty {
                    return true;
                }
                children(r, b"spPr", |r, p, p_empty| {
                    if let Some(f) = parse_fill(r, p, p_empty) {
                        chart.fill = f;
                        return !p_empty;
                    }
                    if local_name(p.name().as_ref()) == b"ln" {
                        chart.line = parse_line(r, p, p_empty);
                        return !p_empty;
                    }
                    false
                });
                true
            }
            _ => false,
        }
    });
    chart
}

fn parse_chart_body(r: &mut Reader<'_>, chart: &mut Chart) {
    children(r, b"chart", |r, e, empty| {
        match local_name(e.name().as_ref()) {
            b"title" => {
                if empty {
                    return true;
                }
                chart.title = parse_title(r);
                true
            }
            b"autoTitleDeleted" => {
                chart.auto_title_deleted = attr_bool(e, b"val").unwrap_or(true);
                false
            }
            b"plotArea" => {
                if empty {
                    return true;
                }
                parse_plot_area(r, chart);
                true
            }
            b"legend" => {
                if empty {
                    chart.legend = Some(Legend::default());
                    return true;
                }
                chart.legend = Some(parse_legend(r));
                true
            }
            b"dispBlanksAs" => {
                chart.show_gaps_as_zero = attr(e, b"val").as_deref() == Some("zero");
                false
            }
            _ => false,
        }
    });
}

/// A title is a rich text body; only its plain text is kept, since a chart title is
/// rendered as a single centred line.
fn parse_title(r: &mut Reader<'_>) -> Option<String> {
    let mut text = None;
    children(r, b"title", |r, e, empty| {
        if local_name(e.name().as_ref()) != b"tx" || empty {
            return false;
        }
        children(r, b"tx", |r, inner, inner_empty| {
            match local_name(inner.name().as_ref()) {
                b"rich" if !inner_empty => {
                    let body = parse_text_body(r, b"rich");
                    let plain = body.plain_text();
                    if !plain.trim().is_empty() {
                        text = Some(plain);
                    }
                    true
                }
                b"strRef" if !inner_empty => {
                    text = parse_string_cache(r, b"strRef").into_iter().next();
                    true
                }
                _ => false,
            }
        });
        true
    });
    text
}

fn parse_legend(r: &mut Reader<'_>) -> Legend {
    let mut legend = Legend::default();
    children(r, b"legend", |r, e, empty| {
        match local_name(e.name().as_ref()) {
            b"legendPos" => {
                legend.position = attr(e, b"val")
                    .as_deref()
                    .map(LegendPosition::parse)
                    .unwrap_or_default();
                false
            }
            b"overlay" => {
                legend.overlay = attr_bool(e, b"val").unwrap_or(true);
                false
            }
            b"txPr" => {
                if empty {
                    return true;
                }
                legend.text = Some(Box::new(parse_text_body(r, b"txPr")));
                true
            }
            _ => false,
        }
    });
    legend
}

fn parse_plot_area(r: &mut Reader<'_>, chart: &mut Chart) {
    children(r, b"plotArea", |r, e, empty| {
        let name = local_name(e.name().as_ref()).to_vec();
        let kind = match name.as_slice() {
            b"barChart" => Some(PlotKind::Bar { horizontal: false }),
            b"bar3DChart" => Some(PlotKind::Bar { horizontal: false }),
            b"lineChart" | b"line3DChart" => Some(PlotKind::Line),
            b"pieChart" | b"pie3DChart" | b"ofPieChart" => Some(PlotKind::Pie),
            b"doughnutChart" => Some(PlotKind::Doughnut),
            b"areaChart" | b"area3DChart" => Some(PlotKind::Area),
            b"scatterChart" | b"bubbleChart" => Some(PlotKind::Scatter),
            _ => None,
        };
        if let Some(kind) = kind {
            if empty {
                return true;
            }
            chart.plots.push(parse_plot(r, &name, kind));
            return true;
        }

        let axis_kind = match name.as_slice() {
            b"catAx" => Some(AxisKind::Category),
            b"valAx" => Some(AxisKind::Value),
            b"dateAx" => Some(AxisKind::Date),
            b"serAx" => Some(AxisKind::Series),
            _ => None,
        };
        if let Some(axis_kind) = axis_kind {
            if empty {
                return true;
            }
            chart.axes.push(parse_axis(r, &name, axis_kind));
            return true;
        }

        if name == b"spPr" && !empty {
            children(r, b"spPr", |r, p, p_empty| {
                if let Some(f) = parse_fill(r, p, p_empty) {
                    chart.plot_area_fill = f;
                    return !p_empty;
                }
                false
            });
            return true;
        }
        false
    });
}

fn parse_plot(r: &mut Reader<'_>, container: &[u8], kind: PlotKind) -> Plot {
    let mut plot = Plot {
        kind,
        // Line and area charts default to overlaid rather than clustered.
        grouping: match kind {
            PlotKind::Bar { .. } => Grouping::Clustered,
            _ => Grouping::Standard,
        },
        ..Default::default()
    };
    children(r, container, |r, e, empty| {
        match local_name(e.name().as_ref()) {
            b"barDir" => {
                if attr(e, b"val").as_deref() == Some("bar") {
                    plot.kind = PlotKind::Bar { horizontal: true };
                }
                false
            }
            b"grouping" => {
                if let Some(v) = attr(e, b"val") {
                    plot.grouping = Grouping::parse(&v);
                }
                false
            }
            b"varyColors" => {
                plot.vary_colors = attr_bool(e, b"val").unwrap_or(true);
                false
            }
            b"gapWidth" => {
                plot.gap_width = attr_f32(e, b"val").unwrap_or(150.0);
                false
            }
            b"overlap" => {
                plot.overlap = attr_f32(e, b"val").unwrap_or(0.0);
                false
            }
            b"holeSize" => {
                plot.hole_percent = attr_f32(e, b"val").unwrap_or(50.0) / 100.0;
                false
            }
            b"axId" => {
                if let Some(id) = attr_u32(e, b"val") {
                    plot.axis_ids.push(id);
                }
                false
            }
            b"ser" => {
                if empty {
                    return true;
                }
                plot.series.push(parse_series(r));
                true
            }
            _ => false,
        }
    });
    plot.series.sort_by_key(|s| s.order);
    plot
}

fn parse_series(r: &mut Reader<'_>) -> Series {
    let mut series = Series {
        marker: true,
        ..Default::default()
    };
    children(r, b"ser", |r, e, empty| {
        match local_name(e.name().as_ref()) {
            b"idx" => {
                series.index = attr_u32(e, b"val").unwrap_or(0);
                false
            }
            b"order" => {
                series.order = attr_u32(e, b"val").unwrap_or(0);
                false
            }
            b"tx" => {
                if empty {
                    return true;
                }
                series.name = parse_series_name(r);
                true
            }
            b"spPr" => {
                if empty {
                    return true;
                }
                children(r, b"spPr", |r, p, p_empty| {
                    if let Some(f) = parse_fill(r, p, p_empty) {
                        series.fill = f;
                        return !p_empty;
                    }
                    if local_name(p.name().as_ref()) == b"ln" {
                        series.line = parse_line(r, p, p_empty);
                        return !p_empty;
                    }
                    false
                });
                true
            }
            b"cat" | b"xVal" => {
                if empty {
                    return true;
                }
                let name = local_name(e.name().as_ref()).to_vec();
                // A category axis can hold either labels or numbers.
                let (strings, numbers) = parse_reference(r, &name);
                if !strings.is_empty() {
                    series.categories = strings;
                } else if !numbers.is_empty() {
                    if name == b"xVal" {
                        series.x_values = numbers;
                    } else {
                        series.categories = numbers
                            .iter()
                            .map(|v| v.map(format_number).unwrap_or_default())
                            .collect();
                    }
                }
                true
            }
            b"val" | b"yVal" => {
                if empty {
                    return true;
                }
                let name = local_name(e.name().as_ref()).to_vec();
                let (_, numbers) = parse_reference(r, &name);
                series.values = numbers;
                true
            }
            b"smooth" => {
                series.smooth = attr_bool(e, b"val").unwrap_or(true);
                false
            }
            b"marker" => {
                if empty {
                    return true;
                }
                children(r, b"marker", |_r, m, _me| {
                    if is(m, b"symbol") {
                        series.marker = attr(m, b"val").as_deref() != Some("none");
                    }
                    false
                });
                true
            }
            b"dPt" => {
                if empty {
                    return true;
                }
                let mut idx = 0u32;
                let mut fill = None;
                children(r, b"dPt", |r, d, d_empty| {
                    match local_name(d.name().as_ref()) {
                        b"idx" => {
                            idx = attr_u32(d, b"val").unwrap_or(0);
                            false
                        }
                        b"spPr" => {
                            if d_empty {
                                return true;
                            }
                            children(r, b"spPr", |r, p, p_empty| {
                                if let Some(f) = parse_fill(r, p, p_empty) {
                                    fill = Some(f);
                                    return !p_empty;
                                }
                                false
                            });
                            true
                        }
                        _ => false,
                    }
                });
                if let Some(f) = fill {
                    series.point_fills.push((idx, f));
                }
                true
            }
            _ => false,
        }
    });
    series
}

fn parse_series_name(r: &mut Reader<'_>) -> String {
    let mut name = String::new();
    children(r, b"tx", |r, e, empty| {
        match local_name(e.name().as_ref()) {
            b"strRef" if !empty => {
                name = parse_string_cache(r, b"strRef").into_iter().next().unwrap_or_default();
                true
            }
            b"v" if !empty => {
                name = text_content(r, b"v");
                true
            }
            b"rich" if !empty => {
                name = parse_text_body(r, b"rich").plain_text();
                true
            }
            _ => false,
        }
    });
    name
}

/// Reads a `<c:cat>`/`<c:val>`-shaped reference, returning whichever cache it held.
///
/// The `<c:f>` formula is deliberately ignored: it points into an embedded workbook this
/// viewer does not open. The cache beside it is what the authoring app last computed, and
/// is what every offline renderer draws.
fn parse_reference(r: &mut Reader<'_>, container: &[u8]) -> (Vec<String>, Vec<Option<f64>>) {
    let mut strings = Vec::new();
    let mut numbers = Vec::new();
    children(r, container, |r, e, empty| {
        if empty {
            return true;
        }
        match local_name(e.name().as_ref()) {
            b"strRef" => {
                strings = parse_string_cache(r, b"strRef");
                true
            }
            b"strLit" => {
                strings = parse_string_cache(r, b"strLit");
                true
            }
            b"numRef" => {
                numbers = parse_number_cache(r, b"numRef");
                true
            }
            b"numLit" => {
                numbers = parse_number_cache(r, b"numLit");
                true
            }
            b"multiLvlStrRef" => {
                // Multi-level categories are flattened to their innermost level, which is
                // what a single-row axis can show.
                strings = parse_string_cache(r, b"multiLvlStrRef");
                true
            }
            _ => false,
        }
    });
    (strings, numbers)
}

/// `<c:pt idx="n"><c:v>…</c:v></c:pt>` entries, placed at their declared index.
///
/// The indices matter: a cache omits empty cells rather than emitting blanks, so reading
/// the points in document order would silently shift every later category.
fn parse_string_cache(r: &mut Reader<'_>, container: &[u8]) -> Vec<String> {
    let mut points: Vec<(u32, String)> = Vec::new();
    let mut count = 0usize;
    collect_points(r, container, &mut count, &mut |idx, value| {
        points.push((idx, value.to_string()));
    });
    let len = count.max(points.iter().map(|(i, _)| *i as usize + 1).max().unwrap_or(0));
    let mut out = vec![String::new(); len];
    for (idx, value) in points {
        if let Some(slot) = out.get_mut(idx as usize) {
            *slot = value;
        }
    }
    out
}

fn parse_number_cache(r: &mut Reader<'_>, container: &[u8]) -> Vec<Option<f64>> {
    let mut points: Vec<(u32, Option<f64>)> = Vec::new();
    let mut count = 0usize;
    collect_points(r, container, &mut count, &mut |idx, value| {
        // A non-numeric cache entry is a gap, not a zero.
        points.push((idx, value.trim().parse::<f64>().ok().filter(|v| v.is_finite())));
    });
    let len = count.max(points.iter().map(|(i, _)| *i as usize + 1).max().unwrap_or(0));
    let mut out = vec![None; len];
    for (idx, value) in points {
        if let Some(slot) = out.get_mut(idx as usize) {
            *slot = value;
        }
    }
    out
}

/// Walks a reference's cache, calling `emit` for each `<c:pt>`.
fn collect_points(
    r: &mut Reader<'_>,
    container: &[u8],
    count: &mut usize,
    emit: &mut dyn FnMut(u32, &str),
) {
    children(r, container, |r, e, empty| {
        let name = local_name(e.name().as_ref()).to_vec();
        match name.as_slice() {
            b"strCache" | b"numCache" | b"multiLvlStrCache" | b"lvl" => {
                if empty {
                    return true;
                }
                collect_points(r, &name, count, emit);
                true
            }
            b"ptCount" => {
                *count = attr_u32(e, b"val").unwrap_or(0) as usize;
                false
            }
            b"pt" => {
                if empty {
                    return true;
                }
                let idx = attr_u32(e, b"idx").unwrap_or(0);
                let mut value = String::new();
                children(r, b"pt", |r, v, v_empty| {
                    if local_name(v.name().as_ref()) == b"v" && !v_empty {
                        value = text_content(r, b"v");
                        return true;
                    }
                    false
                });
                emit(idx, &value);
                true
            }
            _ => false,
        }
    });
}

fn parse_axis(r: &mut Reader<'_>, container: &[u8], kind: AxisKind) -> Axis {
    let mut axis = Axis {
        kind,
        position: match kind {
            AxisKind::Value => AxisPosition::Left,
            _ => AxisPosition::Bottom,
        },
        ..Default::default()
    };
    children(r, container, |r, e, empty| {
        match local_name(e.name().as_ref()) {
            b"axId" => {
                axis.id = attr_u32(e, b"val").unwrap_or(0);
                false
            }
            b"delete" => {
                axis.deleted = attr_bool(e, b"val").unwrap_or(true);
                false
            }
            b"axPos" => {
                if let Some(p) = attr(e, b"val").as_deref().and_then(AxisPosition::parse) {
                    axis.position = p;
                }
                false
            }
            b"majorGridlines" => {
                axis.major_gridlines = true;
                !empty
            }
            b"minorGridlines" => {
                axis.minor_gridlines = true;
                !empty
            }
            b"numFmt" => {
                axis.number_format = attr(e, b"formatCode");
                false
            }
            b"majorUnit" => {
                axis.major_unit = attr_f32(e, b"val").map(|v| v as f64);
                false
            }
            b"tickLblPos" => {
                axis.labels_visible = attr(e, b"val").as_deref() != Some("none");
                false
            }
            b"scaling" => {
                if empty {
                    return true;
                }
                children(r, b"scaling", |_r, s, _se| {
                    match local_name(s.name().as_ref()) {
                        b"min" => axis.min = attr_f32(s, b"val").map(|v| v as f64),
                        b"max" => axis.max = attr_f32(s, b"val").map(|v| v as f64),
                        _ => {}
                    }
                    false
                });
                true
            }
            b"title" => {
                if empty {
                    return true;
                }
                axis.title = parse_title(r);
                true
            }
            b"spPr" => {
                if empty {
                    return true;
                }
                children(r, b"spPr", |r, p, p_empty| {
                    if local_name(p.name().as_ref()) == b"ln" {
                        axis.line = parse_line(r, p, p_empty);
                        return !p_empty;
                    }
                    false
                });
                true
            }
            b"txPr" => {
                if empty {
                    return true;
                }
                axis.text = Some(Box::new(parse_text_body(r, b"txPr")));
                true
            }
            _ => false,
        }
    });
    axis
}

/// True when an element name is a chart part this build can draw.
pub fn is_supported_plot(name: &BytesStart<'_>) -> bool {
    matches!(
        local_name(name.name().as_ref()),
        b"barChart" | b"lineChart" | b"pieChart" | b"doughnutChart" | b"areaChart" | b"scatterChart"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAR: &[u8] = br##"<?xml version="1.0"?>
<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"
              xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <c:chart>
    <c:title><c:tx><c:rich><a:bodyPr/><a:p><a:r><a:t>Revenue by region</a:t></a:r></a:p></c:rich></c:tx></c:title>
    <c:autoTitleDeleted val="0"/>
    <c:plotArea>
      <c:barChart>
        <c:barDir val="col"/>
        <c:grouping val="clustered"/>
        <c:varyColors val="0"/>
        <c:ser>
          <c:idx val="0"/>
          <c:order val="1"/>
          <c:tx><c:strRef><c:f>Sheet1!$B$1</c:f><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>2023</c:v></c:pt></c:strCache></c:strRef></c:tx>
          <c:spPr><a:solidFill><a:srgbClr val="4472C4"/></a:solidFill></c:spPr>
          <c:cat><c:strRef><c:f>Sheet1!$A$2:$A$4</c:f><c:strCache><c:ptCount val="3"/>
            <c:pt idx="0"><c:v>North</c:v></c:pt>
            <c:pt idx="1"><c:v>South</c:v></c:pt>
            <c:pt idx="2"><c:v>East</c:v></c:pt>
          </c:strCache></c:strRef></c:cat>
          <c:val><c:numRef><c:f>Sheet1!$B$2:$B$4</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="3"/>
            <c:pt idx="0"><c:v>1204</c:v></c:pt>
            <c:pt idx="2"><c:v>1455</c:v></c:pt>
          </c:numCache></c:numRef></c:val>
        </c:ser>
        <c:ser>
          <c:idx val="1"/>
          <c:order val="0"/>
          <c:tx><c:strRef><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>2024</c:v></c:pt></c:strCache></c:strRef></c:tx>
          <c:val><c:numRef><c:numCache><c:ptCount val="3"/>
            <c:pt idx="0"><c:v>1388</c:v></c:pt>
            <c:pt idx="1"><c:v>1022</c:v></c:pt>
            <c:pt idx="2"><c:v>1390</c:v></c:pt>
          </c:numCache></c:numRef></c:val>
        </c:ser>
        <c:gapWidth val="219"/>
        <c:overlap val="-27"/>
        <c:axId val="111"/>
        <c:axId val="222"/>
      </c:barChart>
      <c:catAx>
        <c:axId val="111"/>
        <c:scaling/>
        <c:delete val="0"/>
        <c:axPos val="b"/>
      </c:catAx>
      <c:valAx>
        <c:axId val="222"/>
        <c:scaling><c:min val="0"/><c:max val="2000"/></c:scaling>
        <c:delete val="0"/>
        <c:axPos val="l"/>
        <c:majorGridlines/>
        <c:numFmt formatCode="#,##0"/>
      </c:valAx>
    </c:plotArea>
    <c:legend><c:legendPos val="b"/><c:overlay val="0"/></c:legend>
    <c:dispBlanksAs val="gap"/>
  </c:chart>
</c:chartSpace>"##;

    #[test]
    fn parses_title_plots_series_and_axes() {
        let chart = parse_chart(BAR);
        assert_eq!(chart.title.as_deref(), Some("Revenue by region"));
        assert_eq!(chart.plots.len(), 1);
        let plot = chart.plots.first().expect("plot");
        assert_eq!(plot.kind, PlotKind::Bar { horizontal: false });
        assert_eq!(plot.grouping, Grouping::Clustered);
        assert_eq!(plot.gap_width, 219.0);
        assert_eq!(plot.overlap, -27.0);
        assert_eq!(chart.axes.len(), 2);
        assert_eq!(chart.legend.map(|l| l.position), Some(LegendPosition::Bottom));
    }

    #[test]
    fn series_are_sorted_by_their_declared_order_not_document_order() {
        let chart = parse_chart(BAR);
        let plot = chart.plots.first().expect("plot");
        let names: Vec<_> = plot.series.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["2024", "2023"], "c:order must win over document order");
    }

    #[test]
    fn a_cache_gap_is_placed_by_index_rather_than_shifting_later_points() {
        let chart = parse_chart(BAR);
        let plot = chart.plots.first().expect("plot");
        let s2023 = plot.series.iter().find(|s| s.name == "2023").expect("2023");
        // The cache omits idx=1 entirely; 1455 must stay at index 2.
        assert_eq!(s2023.values, vec![Some(1204.0), None, Some(1455.0)]);
        assert_eq!(s2023.categories, vec!["North", "South", "East"]);
    }

    #[test]
    fn axis_scaling_and_gridlines_are_read() {
        let chart = parse_chart(BAR);
        let value = chart.axes.iter().find(|a| a.kind == AxisKind::Value).expect("value axis");
        assert_eq!(value.min, Some(0.0));
        assert_eq!(value.max, Some(2000.0));
        assert!(value.major_gridlines);
        assert_eq!(value.number_format.as_deref(), Some("#,##0"));
        assert_eq!(value.position, AxisPosition::Left);
        assert!(!value.deleted);
    }

    #[test]
    fn a_horizontal_bar_chart_is_distinguished_from_a_column_chart() {
        let xml = br##"<c:chartSpace><c:chart><c:plotArea><c:barChart>
            <c:barDir val="bar"/><c:grouping val="stacked"/>
            <c:ser><c:idx val="0"/><c:val><c:numRef><c:numCache><c:ptCount val="1"/><c:pt idx="0"><c:v>5</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>
        </c:barChart></c:plotArea></c:chart></c:chartSpace>"##;
        let chart = parse_chart(xml);
        let plot = chart.plots.first().expect("plot");
        assert_eq!(plot.kind, PlotKind::Bar { horizontal: true });
        assert_eq!(plot.grouping, Grouping::Stacked);
    }

    #[test]
    fn pie_slices_take_their_colours_from_data_point_overrides() {
        let xml = br##"<c:chartSpace><c:chart><c:plotArea><c:pieChart>
            <c:varyColors val="1"/>
            <c:ser><c:idx val="0"/>
              <c:dPt><c:idx val="0"/><c:spPr><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></c:spPr></c:dPt>
              <c:dPt><c:idx val="1"/><c:spPr><a:solidFill><a:srgbClr val="00FF00"/></a:solidFill></c:spPr></c:dPt>
              <c:val><c:numRef><c:numCache><c:ptCount val="2"/><c:pt idx="0"><c:v>60</c:v></c:pt><c:pt idx="1"><c:v>40</c:v></c:pt></c:numCache></c:numRef></c:val>
            </c:ser>
        </c:pieChart></c:plotArea></c:chart></c:chartSpace>"##;
        let chart = parse_chart(xml);
        let plot = chart.plots.first().expect("plot");
        assert_eq!(plot.kind, PlotKind::Pie);
        assert!(plot.vary_colors);
        let s = plot.series.first().expect("series");
        assert_eq!(s.point_fills.len(), 2);
        assert!(matches!(s.fill_for_point(0), crate::model::fill::Fill::Solid(_)));
    }

    #[test]
    fn a_line_series_records_smoothing_and_marker_suppression() {
        let xml = br##"<c:chartSpace><c:chart><c:plotArea><c:lineChart>
            <c:ser><c:idx val="0"/>
              <c:marker><c:symbol val="none"/></c:marker>
              <c:smooth val="1"/>
              <c:val><c:numRef><c:numCache><c:ptCount val="1"/><c:pt idx="0"><c:v>1</c:v></c:pt></c:numCache></c:numRef></c:val>
            </c:ser>
        </c:lineChart></c:plotArea></c:chart></c:chartSpace>"##;
        let chart = parse_chart(xml);
        let s = chart.plots.first().and_then(|p| p.series.first()).expect("series");
        assert!(s.smooth);
        assert!(!s.marker);
    }

    #[test]
    fn numeric_categories_are_formatted_as_labels() {
        let xml = br##"<c:chartSpace><c:chart><c:plotArea><c:barChart>
            <c:ser><c:idx val="0"/>
              <c:cat><c:numRef><c:numCache><c:ptCount val="2"/><c:pt idx="0"><c:v>2023</c:v></c:pt><c:pt idx="1"><c:v>2024</c:v></c:pt></c:numCache></c:numRef></c:cat>
              <c:val><c:numRef><c:numCache><c:ptCount val="2"/><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt></c:numCache></c:numRef></c:val>
            </c:ser>
        </c:barChart></c:plotArea></c:chart></c:chartSpace>"##;
        let chart = parse_chart(xml);
        let s = chart.plots.first().and_then(|p| p.series.first()).expect("series");
        assert_eq!(s.categories, vec!["2023", "2024"]);
    }

    #[test]
    fn a_malformed_or_empty_chart_yields_an_empty_chart_rather_than_a_panic() {
        assert!(parse_chart(b"").is_empty());
        assert!(parse_chart(b"<c:chartSpace><c:chart><c:plotArea><c:barChart").is_empty());
    }

    #[allow(dead_code)]
    #[test]
    fn axis_values_are_formatted_by_their_format_code() {
        assert_eq!(crate::model::chart::format_axis_value_for(1500.0, Some("#,##0")), "1,500");
        assert_eq!(crate::model::chart::format_axis_value_for(-1234567.0, Some("#,##0")), "-1,234,567");
        assert_eq!(crate::model::chart::format_axis_value_for(0.255, Some("0%")), "25.5%");
        assert_eq!(crate::model::chart::format_axis_value_for(0.25, Some("0%")), "25%");
        assert_eq!(crate::model::chart::format_axis_value_for(3.14159, Some("0.00")), "3.14");
        assert_eq!(crate::model::chart::format_axis_value_for(42.0, None), "42");
        assert_eq!(crate::model::chart::format_axis_value_for(42.5, None), "42.5");
    }
}
