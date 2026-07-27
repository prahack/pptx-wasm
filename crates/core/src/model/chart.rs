//! Charts (`chart.xml`, the DrawingML charting namespace).
//!
//! A chart part is its own document with its own vocabulary — it shares only colours,
//! fills and text runs with the rest of DrawingML. What it does *not* contain is a
//! drawing: `chart.xml` is data plus formatting, and every renderer computes the
//! geometry itself. That is why this module stops at "what was asked for" and
//! [`crate::layout::chart`] does the plotting.
//!
//! Values come from the `<c:numCache>`/`<c:strCache>` blocks that the authoring app
//! writes alongside the spreadsheet references. Caches are what make an offline viewer
//! possible at all: the workbook is an embedded `.xlsx` we would otherwise have to open
//! and evaluate.

use crate::model::fill::{Fill, Line};
use crate::model::text::TextBody;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlotKind {
    /// Vertical bars. `horizontal` makes it a bar-of-rows chart (`<c:barDir val="bar"/>`).
    Bar {
        horizontal: bool,
    },
    Line,
    Pie,
    /// A pie with a hole; `hole_percent` is 0..1 of the radius.
    Doughnut,
    Area,
    Scatter,
}

/// How multiple series share the category axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Grouping {
    /// Side by side. The default for a bar chart.
    #[default]
    Clustered,
    /// Each series stacked on the one before.
    Stacked,
    /// Stacked and normalised so every category totals 100%.
    PercentStacked,
    /// Drawn on top of each other, used by line and area charts.
    Standard,
}

impl Grouping {
    pub fn parse(s: &str) -> Grouping {
        match s {
            "stacked" => Grouping::Stacked,
            "percentStacked" => Grouping::PercentStacked,
            "clustered" => Grouping::Clustered,
            _ => Grouping::Standard,
        }
    }

    pub fn is_stacked(self) -> bool {
        matches!(self, Grouping::Stacked | Grouping::PercentStacked)
    }
}

/// One data series.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Series {
    /// `<c:idx>` — identity, used to pick the default colour from the accent cycle.
    pub index: u32,
    /// `<c:order>` — draw and legend order.
    pub order: u32,
    /// Cached series name from `<c:tx>`.
    pub name: String,
    /// Values from `<c:val>`. `None` is a gap in the data, which is not the same as zero
    /// — a line chart breaks across a gap and draws through a zero.
    pub values: Vec<Option<f64>>,
    /// Category labels from `<c:cat>`. May be shorter than `values`.
    pub categories: Vec<String>,
    /// X values for a scatter chart (`<c:xVal>`).
    pub x_values: Vec<Option<f64>>,
    pub fill: Fill,
    pub line: Line,
    /// Per-point overrides from `<c:dPt>`, keyed by point index. Pie charts colour every
    /// slice this way.
    pub point_fills: Vec<(u32, Fill)>,
    /// `<c:smooth>` on a line series.
    pub smooth: bool,
    /// `<c:marker><c:symbol val="none"/>` suppresses markers on a line series.
    pub marker: bool,
}

impl Series {
    /// The fill for point `i`: its own override if it has one, else the series fill.
    pub fn fill_for_point(&self, i: u32) -> &Fill {
        self.point_fills
            .iter()
            .find(|(idx, _)| *idx == i)
            .map(|(_, f)| f)
            .unwrap_or(&self.fill)
    }

    /// Largest and smallest value present, ignoring gaps.
    pub fn extent(&self) -> Option<(f64, f64)> {
        let mut it = self.values.iter().flatten().copied();
        let first = it.next()?;
        Some(it.fold((first, first), |(lo, hi), v| (lo.min(v), hi.max(v))))
    }
}

/// One plot — a chart type applied to a set of series. A chart can hold several (a bar
/// chart with a line overlaid is two plots sharing one axis pair).
#[derive(Debug, Clone, PartialEq)]
pub struct Plot {
    pub kind: PlotKind,
    pub grouping: Grouping,
    pub series: Vec<Series>,
    /// `<c:gapWidth>` as a percentage of the bar width. 150 is PowerPoint's default.
    pub gap_width: f32,
    /// `<c:overlap>` as a percentage; negative separates clustered bars.
    pub overlap: f32,
    /// Doughnut hole as a fraction of the radius.
    pub hole_percent: f32,
    /// Axis ids this plot is drawn against, in `<c:axId>` order.
    pub axis_ids: Vec<u32>,
    /// `<c:varyColors>` — colour each point differently rather than each series. Implied
    /// for pie charts, which have one series.
    pub vary_colors: bool,
}

impl Default for Plot {
    fn default() -> Self {
        Plot {
            kind: PlotKind::Bar { horizontal: false },
            grouping: Grouping::default(),
            series: Vec::new(),
            gap_width: 150.0,
            overlap: 0.0,
            hole_percent: 0.5,
            axis_ids: Vec::new(),
            vary_colors: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisPosition {
    Left,
    Right,
    Top,
    Bottom,
}

impl AxisPosition {
    pub fn parse(s: &str) -> Option<AxisPosition> {
        Some(match s {
            "l" => AxisPosition::Left,
            "r" => AxisPosition::Right,
            "t" => AxisPosition::Top,
            "b" => AxisPosition::Bottom,
            _ => return None,
        })
    }

    pub fn is_vertical(self) -> bool {
        matches!(self, AxisPosition::Left | AxisPosition::Right)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisKind {
    /// `<c:catAx>` — discrete labels.
    Category,
    /// `<c:valAx>` — a continuous numeric scale.
    Value,
    /// `<c:dateAx>`, treated as a category axis with formatted labels.
    Date,
    /// `<c:serAx>`, only meaningful on 3-D charts.
    Series,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Axis {
    pub id: u32,
    pub kind: AxisKind,
    pub position: AxisPosition,
    /// `<c:delete val="1"/>` — present in the file but not drawn.
    pub deleted: bool,
    /// Manual scale bounds; `None` means "derive from the data".
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub major_unit: Option<f64>,
    pub major_gridlines: bool,
    pub minor_gridlines: bool,
    /// `<c:numFmt formatCode="0.0%">`
    pub number_format: Option<String>,
    pub title: Option<String>,
    pub line: Line,
    /// Where tick labels go relative to the axis (`<c:tickLblPos>`).
    pub labels_visible: bool,
    /// Text formatting for the tick labels.
    pub text: Option<Box<TextBody>>,
}

impl Default for Axis {
    fn default() -> Self {
        Axis {
            id: 0,
            kind: AxisKind::Value,
            position: AxisPosition::Left,
            deleted: false,
            min: None,
            max: None,
            major_unit: None,
            major_gridlines: false,
            minor_gridlines: false,
            number_format: None,
            title: None,
            line: Line::default(),
            labels_visible: true,
            text: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LegendPosition {
    Left,
    Right,
    Top,
    #[default]
    Bottom,
    TopRight,
}

impl LegendPosition {
    pub fn parse(s: &str) -> LegendPosition {
        match s {
            "l" => LegendPosition::Left,
            "r" => LegendPosition::Right,
            "t" => LegendPosition::Top,
            "tr" => LegendPosition::TopRight,
            _ => LegendPosition::Bottom,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Legend {
    pub position: LegendPosition,
    /// `<c:overlay val="1"/>` — draw over the plot rather than beside it.
    pub overlay: bool,
    pub text: Option<Box<TextBody>>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Chart {
    pub title: Option<String>,
    /// `<c:autoTitleDeleted val="1"/>` — suppresses the automatic single-series title.
    pub auto_title_deleted: bool,
    pub plots: Vec<Plot>,
    pub axes: Vec<Axis>,
    pub legend: Option<Legend>,
    /// Fill of the whole chart area.
    pub fill: Fill,
    pub line: Line,
    /// Fill of the plot area alone.
    pub plot_area_fill: Fill,
    /// `<c:dispBlanksAs>` — how gaps are drawn.
    pub show_gaps_as_zero: bool,
}

impl Chart {
    pub fn axis(&self, id: u32) -> Option<&Axis> {
        self.axes.iter().find(|a| a.id == id)
    }

    /// The category (or date) axis a plot is drawn against.
    pub fn category_axis(&self, plot: &Plot) -> Option<&Axis> {
        plot.axis_ids
            .iter()
            .filter_map(|id| self.axis(*id))
            .find(|a| matches!(a.kind, AxisKind::Category | AxisKind::Date))
    }

    /// The value axis a plot is drawn against.
    pub fn value_axis(&self, plot: &Plot) -> Option<&Axis> {
        plot.axis_ids
            .iter()
            .filter_map(|id| self.axis(*id))
            .find(|a| a.kind == AxisKind::Value)
    }

    /// Category labels for the chart, taken from the first series that has any.
    pub fn categories(&self) -> Vec<String> {
        for plot in &self.plots {
            for series in &plot.series {
                if !series.categories.is_empty() {
                    return series.categories.clone();
                }
            }
        }
        // No cached categories: fall back to 1-based ordinals so the axis still reads.
        let n = self
            .plots
            .iter()
            .flat_map(|p| p.series.iter())
            .map(|s| s.values.len())
            .max()
            .unwrap_or(0);
        (1..=n).map(|i| i.to_string()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.plots.iter().all(|p| p.series.is_empty())
    }

    /// Every series in draw order, paired with its plot.
    pub fn series(&self) -> impl Iterator<Item = (&Plot, &Series)> {
        self.plots
            .iter()
            .flat_map(|p| p.series.iter().map(move |s| (p, s)))
    }

    /// The value range a plot's axis has to span.
    ///
    /// Stacked plots need the range of the *sums*, not of the individual values, or the
    /// top of the stack falls outside the axis.
    pub fn value_range(&self, plot: &Plot) -> (f64, f64) {
        if plot.grouping == Grouping::PercentStacked {
            return (0.0, 1.0);
        }
        if plot.grouping.is_stacked() {
            let points = plot
                .series
                .iter()
                .map(|s| s.values.len())
                .max()
                .unwrap_or(0);
            let (mut lo, mut hi) = (0.0f64, 0.0f64);
            for i in 0..points {
                let (mut pos, mut neg) = (0.0f64, 0.0f64);
                for s in &plot.series {
                    match s.values.get(i).copied().flatten() {
                        Some(v) if v >= 0.0 => pos += v,
                        Some(v) => neg += v,
                        None => {}
                    }
                }
                hi = hi.max(pos);
                lo = lo.min(neg);
            }
            return (lo, hi);
        }
        let mut extents = plot.series.iter().filter_map(Series::extent);
        let Some(first) = extents.next() else {
            return (0.0, 1.0);
        };
        extents.fold(first, |(lo, hi), (l, h)| (lo.min(l), hi.max(h)))
    }
}

/// A "nice" axis scale covering `min..max`.
///
/// Charts look wrong when the axis stops at the exact data maximum, so this rounds
/// outward to a 1/2/5×10ⁿ step — which is what every charting library does and what
/// PowerPoint's automatic scaling produces.
///
/// `anchor_zero` decides whether the axis is forced to include zero. It must be true for
/// bars and areas, where *length* encodes the value: an axis starting at 900 makes 950
/// and 1000 look like 1:2. It must be false for lines and scatters, where *position*
/// encodes the value and anchoring at zero flattens the series into a straight line at
/// the top of the plot. PowerPoint and LibreOffice both draw this distinction.
pub fn nice_scale(min: f64, max: f64, target_ticks: usize, anchor_zero: bool) -> (f64, f64, f64) {
    let (lo, hi) = if anchor_zero {
        (min.min(0.0), max.max(0.0))
    } else {
        (min, max)
    };
    // `hi - lo` overflows to infinity for a range near f64's limits, and every derived
    // value after that is NaN. Cached chart data is not trustworthy enough to assume it
    // is sane, so the span is bounded before any arithmetic touches it.
    const LIMIT: f64 = 1e15;
    let (lo, hi) = (lo.clamp(-LIMIT, LIMIT), hi.clamp(-LIMIT, LIMIT));
    if !(lo.is_finite() && hi.is_finite())
        || !(hi - lo).is_finite()
        || (hi - lo).abs() < f64::EPSILON
    {
        return (0.0, 1.0, 0.5);
    }
    let target = target_ticks.max(2) as f64;
    let raw_step = (hi - lo) / target;
    let magnitude = 10f64.powf(raw_step.abs().log10().floor());
    let normalised = raw_step / magnitude;
    let step = magnitude
        * if normalised <= 1.0 {
            1.0
        } else if normalised <= 2.0 {
            2.0
        } else if normalised <= 5.0 {
            5.0
        } else {
            10.0
        };
    let scaled_lo = (lo / step).floor() * step;
    let scaled_hi = (hi / step).ceil() * step;
    if !(scaled_lo.is_finite() && scaled_hi.is_finite() && step.is_finite() && step > 0.0) {
        return (0.0, 1.0, 0.5);
    }
    (scaled_lo, scaled_hi, step)
}

/// Formats a cached number for use as a category label.
///
/// Lives here rather than in the parser because layout needs it for axis ticks, and
/// layout must not depend on the parsing layer.
pub fn format_number(v: f64) -> String {
    if v.fract().abs() < f64::EPSILON && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let s = format!("{v:.4}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Formats an axis tick value, honouring the common `formatCode` shapes.
///
/// A full number-format engine is out of scope; percentages and thousands separators
/// cover what actually appears on a business chart's axis.
pub fn format_axis_value_for(v: f64, format: Option<&str>) -> String {
    let Some(fmt) = format else {
        return format_number(v);
    };
    if fmt.contains('%') {
        let pct = v * 100.0;
        return if pct.fract().abs() < 1e-9 {
            format!("{}%", pct as i64)
        } else {
            format!("{pct:.1}%")
        };
    }
    let decimals = fmt
        .split_once('.')
        .map(|(_, rest)| rest.chars().take_while(|c| *c == '0' || *c == '#').count())
        .unwrap_or(0);
    let rendered = format!("{v:.decimals$}");
    if fmt.contains("#,#") || fmt.contains("#,0") {
        group_thousands(&rendered)
    } else {
        rendered
    }
}

fn group_thousands(s: &str) -> String {
    let (sign, rest) = match s.strip_prefix('-') {
        Some(r) => ("-", r),
        None => ("", s),
    };
    let (int, frac) = match rest.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (rest, None),
    };
    let mut grouped = String::with_capacity(int.len() + int.len() / 3);
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    match frac {
        Some(f) => format!("{sign}{grouped}.{f}"),
        None => format!("{sign}{grouped}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(values: &[f64]) -> Series {
        Series {
            values: values.iter().map(|v| Some(*v)).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_series_extent_ignores_gaps() {
        let s = Series {
            values: vec![Some(3.0), None, Some(-1.0), Some(7.0)],
            ..Default::default()
        };
        assert_eq!(s.extent(), Some((-1.0, 7.0)));
        assert_eq!(Series::default().extent(), None);
    }

    #[test]
    fn per_point_fills_override_the_series_fill() {
        let s = Series {
            fill: Fill::NoFill,
            point_fills: vec![(1, Fill::Group)],
            ..Default::default()
        };
        assert_eq!(*s.fill_for_point(0), Fill::NoFill);
        assert_eq!(*s.fill_for_point(1), Fill::Group);
    }

    #[test]
    fn a_clustered_range_spans_the_widest_series() {
        let chart = Chart {
            plots: vec![Plot {
                series: vec![series(&[1.0, 5.0]), series(&[-2.0, 3.0])],
                ..Default::default()
            }],
            ..Default::default()
        };
        let plot = chart.plots.first().expect("plot");
        assert_eq!(chart.value_range(plot), (-2.0, 5.0));
    }

    #[test]
    fn a_stacked_range_uses_the_sums_not_the_individual_values() {
        let chart = Chart {
            plots: vec![Plot {
                grouping: Grouping::Stacked,
                series: vec![series(&[3.0, 1.0]), series(&[4.0, 1.0])],
                ..Default::default()
            }],
            ..Default::default()
        };
        let plot = chart.plots.first().expect("plot");
        // The tallest column is 3+4, not the tallest single value.
        assert_eq!(chart.value_range(plot), (0.0, 7.0));
    }

    #[test]
    fn a_percent_stacked_range_is_always_zero_to_one() {
        let chart = Chart {
            plots: vec![Plot {
                grouping: Grouping::PercentStacked,
                series: vec![series(&[30.0]), series(&[70.0])],
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(
            chart.value_range(chart.plots.first().expect("plot")),
            (0.0, 1.0)
        );
    }

    #[test]
    fn categories_come_from_the_first_series_that_has_them() {
        let chart = Chart {
            plots: vec![Plot {
                series: vec![
                    Series {
                        values: vec![Some(1.0)],
                        ..Default::default()
                    },
                    Series {
                        categories: vec!["Q1".into(), "Q2".into()],
                        values: vec![Some(1.0), Some(2.0)],
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(chart.categories(), vec!["Q1".to_string(), "Q2".to_string()]);
    }

    #[test]
    fn categories_fall_back_to_ordinals_when_nothing_is_cached() {
        let chart = Chart {
            plots: vec![Plot {
                series: vec![series(&[1.0, 2.0, 3.0])],
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(chart.categories(), vec!["1", "2", "3"]);
    }

    #[test]
    fn axes_are_matched_to_a_plot_by_id_and_kind() {
        let chart = Chart {
            plots: vec![Plot {
                axis_ids: vec![10, 20],
                ..Default::default()
            }],
            axes: vec![
                Axis {
                    id: 10,
                    kind: AxisKind::Category,
                    position: AxisPosition::Bottom,
                    ..Default::default()
                },
                Axis {
                    id: 20,
                    kind: AxisKind::Value,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let plot = chart.plots.first().expect("plot");
        assert_eq!(chart.category_axis(plot).map(|a| a.id), Some(10));
        assert_eq!(chart.value_axis(plot).map(|a| a.id), Some(20));
    }

    #[test]
    fn nice_scale_rounds_outward_to_a_readable_step() {
        let (lo, hi, step) = nice_scale(0.0, 1610.0, 5, true);
        assert_eq!(lo, 0.0);
        assert!(hi >= 1610.0, "the scale must contain the data: {hi}");
        assert_eq!(step, 500.0);
        assert_eq!(hi, 2000.0);
    }

    #[test]
    fn nice_scale_anchors_a_positive_series_at_zero_when_asked() {
        // Bars measured from 900 rather than 0 would all look the same length.
        let (lo, _, _) = nice_scale(900.0, 1000.0, 5, true);
        assert_eq!(lo, 0.0);
    }

    #[test]
    fn an_unanchored_scale_tracks_the_data_instead_of_reaching_to_zero() {
        // A line chart of 12.4..14.6 must not be squashed against the top of a 0..15 axis.
        let (lo, hi, _) = nice_scale(12.4, 14.6, 5, false);
        assert!((12.0..=12.4).contains(&lo), "lo={lo}");
        assert!((14.6..=15.0).contains(&hi), "hi={hi}");
    }

    #[test]
    fn nice_scale_keeps_negative_data_in_range() {
        let (lo, hi, _) = nice_scale(-40.0, 90.0, 5, true);
        assert!(lo <= -40.0 && hi >= 90.0, "{lo}..{hi}");
    }

    #[test]
    fn nice_scale_survives_degenerate_input() {
        for (a, b) in [(0.0, 0.0), (5.0, 5.0), (f64::NAN, 1.0)] {
            let (lo, hi, step) = nice_scale(a, b, 5, true);
            assert!(lo.is_finite() && hi.is_finite() && step > 0.0, "{a}..{b}");
            assert!(hi > lo);
        }
    }
}
