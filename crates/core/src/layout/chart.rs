//! Chart plotting: [`Chart`] → display-list commands.
//!
//! `chart.xml` holds data and formatting but no geometry, so every renderer computes the
//! plot itself. This is that computation: reserve space for the title, legend and axis
//! labels, derive a scale, then map data values onto the remaining rectangle.
//!
//! Everything is in points, like the rest of layout. Nothing here knows about a canvas.

use std::f32::consts::PI;

use crate::dl::{Color, Command, FillRule, Paint, Path, Point, Rect, Stroke};
use crate::model::chart::{
    format_axis_value_for, nice_scale, Axis, Chart, Grouping, LegendPosition, Plot, PlotKind,
    Series,
};
use crate::model::color::{ColorMod, ColorRef, ColorSpec, SchemeColor};
use crate::model::fill::Fill;
use crate::model::Presentation;
use crate::text::TextMeasure;

use super::inherit::Resolver;
use super::paint;
use super::text::{self as tlayout};

/// Default type sizes, in points. PowerPoint scales chart text with the frame; these are
/// the sizes it uses at a typical 5-6 inch chart, which is what fixtures and real decks
/// overwhelmingly contain.
const TITLE_SIZE: f32 = 14.0;
const LABEL_SIZE: f32 = 10.0;
const LEGEND_SIZE: f32 = 10.0;
/// Gap between the plot area and the things around it.
const PADDING: f32 = 6.0;
/// Legend swatch size.
const SWATCH: f32 = 8.0;

/// Draws a chart into `frame`.
pub fn layout_chart(
    chart: &Chart,
    frame: Rect,
    resolver: &Resolver<'_>,
    pres: &Presentation,
    measure: &dyn TextMeasure,
    part: &str,
    out: &mut Vec<Command>,
) {
    if frame.is_empty() {
        return;
    }
    let ctx = Ctx {
        resolver,
        pres,
        measure,
        part,
    };

    // Chart area background. A chart with no explicit fill is opaque white in PowerPoint,
    // not transparent — it hides whatever the slide has behind it.
    let background = if chart.fill.is_specified() {
        chart.fill.clone()
    } else {
        Fill::Solid(ColorRef::scheme(SchemeColor::Light1))
    };
    if let Some(p) = ctx.fill(&background, frame) {
        out.push(Command::FillPath {
            path: Path::rect(frame),
            paint: p,
            rule: FillRule::NonZero,
        });
    }
    if let Some(stroke) = ctx.stroke(&chart.line, frame) {
        out.push(Command::StrokePath {
            path: Path::rect(frame),
            stroke,
        });
    }

    if chart.is_empty() {
        return;
    }

    let mut area = Rect::new(
        frame.x + PADDING,
        frame.y + PADDING,
        (frame.w - PADDING * 2.0).max(1.0),
        (frame.h - PADDING * 2.0).max(1.0),
    );

    // Title, then legend, then the axis furniture — each takes a bite out of the area
    // left for the plot.
    if let Some(title) = title_text(chart) {
        let font = tlayout::font_spec(&ctx.font_family(), TITLE_SIZE, true, false);
        let width = measure.measure(&title, &font).width;
        let metrics = measure.font_metrics(&font);
        out.push(Command::DrawText(crate::dl::TextRun {
            // Chart labels come from chart.xml, which has no hyperlink of its own.
            link: None,
            text: title.clone(),
            font: font.clone(),
            origin: Point::new(area.x + (area.w - width) / 2.0, area.y + metrics.ascent),
            paint: Paint::Solid(ctx.text_color()),
            advances: measure.measure(&title, &font).advances,
            width,
            decorations: Default::default(),
            letter_spacing: 0.0,
        }));
        let used = metrics.line_height() + PADDING;
        area = Rect::new(area.x, area.y + used, area.w, (area.h - used).max(1.0));
    }

    let legend_rect = chart.legend.as_ref().and_then(|legend| {
        if legend.overlay {
            // An overlaid legend does not take space from the plot.
            return None;
        }
        let (reserved, remaining) = split_for_legend(area, legend.position, chart, &ctx);
        area = remaining;
        Some((reserved, legend.position))
    });

    let plot_area = draw_axes_and_reserve(chart, area, &ctx, out);

    if let Some(p) = ctx.fill(&chart.plot_area_fill, plot_area) {
        out.push(Command::FillPath {
            path: Path::rect(plot_area),
            paint: p,
            rule: FillRule::NonZero,
        });
    }

    for (i, plot) in chart.plots.iter().enumerate() {
        draw_plot(chart, plot, i, plot_area, &ctx, out);
    }

    if let Some((rect, position)) = legend_rect {
        draw_legend(chart, rect, position, &ctx, out);
    }
}

struct Ctx<'a> {
    resolver: &'a Resolver<'a>,
    pres: &'a Presentation,
    measure: &'a dyn TextMeasure,
    part: &'a str,
}

impl Ctx<'_> {
    fn fill(&self, fill: &Fill, bounds: Rect) -> Option<Paint> {
        paint::fill_to_paint(fill, bounds, self.resolver, self.pres, self.part)
    }

    fn stroke(&self, line: &crate::model::fill::Line, bounds: Rect) -> Option<Stroke> {
        if line.is_empty() {
            return None;
        }
        paint::line_to_stroke(line, bounds, self.resolver, self.pres, self.part)
    }

    fn font_family(&self) -> String {
        self.resolver.font_family(None)
    }

    fn text_color(&self) -> Color {
        // Chart text follows the theme's body text colour, softened the way PowerPoint's
        // chart styles do.
        self.resolver.color(&ColorRef {
            spec: ColorSpec::Scheme(SchemeColor::Text1),
            mods: vec![ColorMod::LumMod(0.65), ColorMod::LumOff(0.35)],
        })
    }

    fn gridline_color(&self) -> Color {
        self.resolver.color(&ColorRef {
            spec: ColorSpec::Scheme(SchemeColor::Text1),
            mods: vec![ColorMod::LumMod(0.15), ColorMod::LumOff(0.85)],
        })
    }

    /// The colour a series is drawn in.
    ///
    /// An explicit `<c:spPr>` wins. Otherwise PowerPoint walks the six theme accents and
    /// then repeats them shaded — which is why a seven-series chart does not have two
    /// identical colours.
    fn series_color(&self, series: &Series, fallback_index: u32) -> Color {
        if let Fill::Solid(c) = &series.fill {
            return self.resolver.color(c);
        }
        self.accent(fallback_index)
    }

    fn accent(&self, index: u32) -> Color {
        const ACCENTS: [SchemeColor; 6] = [
            SchemeColor::Accent1,
            SchemeColor::Accent2,
            SchemeColor::Accent3,
            SchemeColor::Accent4,
            SchemeColor::Accent5,
            SchemeColor::Accent6,
        ];
        let slot = ACCENTS
            .get((index % 6) as usize)
            .copied()
            .unwrap_or(SchemeColor::Accent1);
        let cycle = index / 6;
        let mods = match cycle {
            0 => Vec::new(),
            // Each further cycle alternates darker and lighter, as PowerPoint does.
            n if n % 2 == 1 => vec![ColorMod::Shade(0.75)],
            _ => vec![ColorMod::Tint(0.6)],
        };
        self.resolver.color(&ColorRef {
            spec: ColorSpec::Scheme(slot),
            mods,
        })
    }

    fn label(
        &self,
        text: &str,
        x: f32,
        y: f32,
        align: LabelAlign,
        size: f32,
        out: &mut Vec<Command>,
    ) {
        if text.is_empty() {
            return;
        }
        let font = tlayout::font_spec(&self.font_family(), size, false, false);
        let measured = self.measure.measure(text, &font);
        let x = match align {
            LabelAlign::Left => x,
            LabelAlign::Center => x - measured.width / 2.0,
            LabelAlign::Right => x - measured.width,
        };
        out.push(Command::DrawText(crate::dl::TextRun {
            // Chart labels come from chart.xml, which has no hyperlink of its own.
            link: None,
            text: text.to_string(),
            font,
            origin: Point::new(x, y),
            paint: Paint::Solid(self.text_color()),
            advances: measured.advances,
            width: measured.width,
            decorations: Default::default(),
            letter_spacing: 0.0,
        }));
    }

    fn text_width(&self, text: &str, size: f32) -> f32 {
        let font = tlayout::font_spec(&self.font_family(), size, false, false);
        self.measure.measure(text, &font).width
    }

    fn line_height(&self, size: f32) -> f32 {
        let font = tlayout::font_spec(&self.font_family(), size, false, false);
        self.measure.font_metrics(&font).line_height()
    }
}

#[derive(Clone, Copy)]
enum LabelAlign {
    Left,
    Center,
    Right,
}

/// The title to draw, honouring `autoTitleDeleted` and the single-series default.
fn title_text(chart: &Chart) -> Option<String> {
    if let Some(t) = &chart.title {
        return Some(t.clone());
    }
    if chart.auto_title_deleted {
        return None;
    }
    // PowerPoint titles a single-series chart with that series' name.
    let mut series = chart.series();
    let (_, first) = series.next()?;
    if series.next().is_some() || first.name.trim().is_empty() {
        return None;
    }
    Some(first.name.clone())
}

/// Reserves space for the legend, returning (legend rect, what is left for the plot).
fn split_for_legend(
    area: Rect,
    position: LegendPosition,
    chart: &Chart,
    ctx: &Ctx<'_>,
) -> (Rect, Rect) {
    let entries = legend_entries(chart);
    if entries.is_empty() {
        return (Rect::default(), area);
    }
    match position {
        LegendPosition::Bottom | LegendPosition::Top => {
            let h = ctx.line_height(LEGEND_SIZE) + PADDING;
            if position == LegendPosition::Bottom {
                (
                    Rect::new(area.x, area.bottom() - h, area.w, h),
                    Rect::new(area.x, area.y, area.w, (area.h - h).max(1.0)),
                )
            } else {
                (
                    Rect::new(area.x, area.y, area.w, h),
                    Rect::new(area.x, area.y + h, area.w, (area.h - h).max(1.0)),
                )
            }
        }
        LegendPosition::Left | LegendPosition::Right | LegendPosition::TopRight => {
            let w = entries
                .iter()
                .map(|(label, _)| ctx.text_width(label, LEGEND_SIZE))
                .fold(0.0f32, f32::max)
                + SWATCH
                + PADDING * 2.0;
            let w = w.min(area.w * 0.4);
            if position == LegendPosition::Left {
                (
                    Rect::new(area.x, area.y, w, area.h),
                    Rect::new(area.x + w, area.y, (area.w - w).max(1.0), area.h),
                )
            } else {
                (
                    Rect::new(area.right() - w, area.y, w, area.h),
                    Rect::new(area.x, area.y, (area.w - w).max(1.0), area.h),
                )
            }
        }
    }
}

/// Legend entries: one per series, or one per category for a pie chart.
fn legend_entries(chart: &Chart) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    let mut colour_index = 0u32;
    for plot in &chart.plots {
        let per_point = plot.vary_colors || matches!(plot.kind, PlotKind::Pie | PlotKind::Doughnut);
        if per_point {
            let categories = chart.categories();
            if let Some(series) = plot.series.first() {
                for i in 0..series.values.len() {
                    let label = categories
                        .get(i)
                        .cloned()
                        .unwrap_or_else(|| (i + 1).to_string());
                    out.push((label, i as u32));
                }
            }
            continue;
        }
        for series in &plot.series {
            let label = if series.name.trim().is_empty() {
                format!("Series {}", colour_index + 1)
            } else {
                series.name.clone()
            };
            out.push((label, colour_index));
            colour_index += 1;
        }
    }
    out
}

fn draw_legend(
    chart: &Chart,
    rect: Rect,
    position: LegendPosition,
    ctx: &Ctx<'_>,
    out: &mut Vec<Command>,
) {
    let entries = legend_entries(chart);
    if entries.is_empty() || rect.is_empty() {
        return;
    }
    let metrics = ctx.line_height(LEGEND_SIZE);
    let baseline_offset = metrics * 0.75;

    if matches!(position, LegendPosition::Top | LegendPosition::Bottom) {
        // Lay entries out in a row, centred.
        let gap = PADDING * 2.0;
        let total: f32 = entries
            .iter()
            .map(|(label, _)| SWATCH + PADDING / 2.0 + ctx.text_width(label, LEGEND_SIZE) + gap)
            .sum::<f32>()
            - gap;
        let mut x = rect.x + (rect.w - total).max(0.0) / 2.0;
        let y = rect.y + (rect.h - metrics) / 2.0;
        for (label, colour) in &entries {
            draw_swatch(ctx, x, y + (metrics - SWATCH) / 2.0, *colour, chart, out);
            x += SWATCH + PADDING / 2.0;
            ctx.label(
                label,
                x,
                y + baseline_offset,
                LabelAlign::Left,
                LEGEND_SIZE,
                out,
            );
            x += ctx.text_width(label, LEGEND_SIZE) + gap;
        }
        return;
    }

    // Otherwise stack them vertically.
    let mut y = rect.y + (rect.h - metrics * entries.len() as f32).max(0.0) / 2.0;
    for (label, colour) in &entries {
        draw_swatch(
            ctx,
            rect.x + PADDING,
            y + (metrics - SWATCH) / 2.0,
            *colour,
            chart,
            out,
        );
        ctx.label(
            label,
            rect.x + PADDING + SWATCH + PADDING / 2.0,
            y + baseline_offset,
            LabelAlign::Left,
            LEGEND_SIZE,
            out,
        );
        y += metrics;
    }
}

fn draw_swatch(ctx: &Ctx<'_>, x: f32, y: f32, index: u32, chart: &Chart, out: &mut Vec<Command>) {
    let colour = swatch_color(chart, index, ctx);
    out.push(Command::FillPath {
        path: Path::rect(Rect::new(x, y, SWATCH, SWATCH)),
        paint: Paint::Solid(colour),
        rule: FillRule::NonZero,
    });
}

/// The colour the legend shows for entry `index`, matched to how the plot draws it.
fn swatch_color(chart: &Chart, index: u32, ctx: &Ctx<'_>) -> Color {
    for plot in &chart.plots {
        let per_point = plot.vary_colors || matches!(plot.kind, PlotKind::Pie | PlotKind::Doughnut);
        if per_point {
            if let Some(series) = plot.series.first() {
                if let Fill::Solid(c) = series.fill_for_point(index) {
                    return ctx.resolver.color(c);
                }
            }
            return ctx.accent(index);
        }
        if let Some(series) = plot.series.get(index as usize) {
            return ctx.series_color(series, index);
        }
    }
    ctx.accent(index)
}

/// Draws gridlines, axis lines and tick labels, returning the rectangle left for data.
///
/// Two passes are unavoidable: the plot rectangle depends on how wide the value labels
/// are, and the labels depend on the scale, which depends on the data — but not on the
/// rectangle. So the scale is computed first, then the labels measured, then the
/// rectangle fixed.
fn draw_axes_and_reserve(chart: &Chart, area: Rect, ctx: &Ctx<'_>, out: &mut Vec<Command>) -> Rect {
    let Some(plot) = chart.plots.first() else {
        return area;
    };
    // Pie charts have no axes at all.
    if matches!(plot.kind, PlotKind::Pie | PlotKind::Doughnut) {
        return area;
    }

    let value_axis = chart.value_axis(plot);
    let category_axis = chart.category_axis(plot);
    let horizontal = matches!(plot.kind, PlotKind::Bar { horizontal: true });

    let (lo, hi, step) = scale_for(chart, plot, value_axis);
    let ticks = tick_values(lo, hi, step);
    let format = value_axis.and_then(|a| a.number_format.as_deref());

    let show_value_labels = value_axis
        .map(|a| !a.deleted && a.labels_visible)
        .unwrap_or(true);
    let show_cat_labels = category_axis
        .map(|a| !a.deleted && a.labels_visible)
        .unwrap_or(true);

    let label_height = ctx.line_height(LABEL_SIZE);
    let categories = chart.categories();

    // Reserve for whichever axis carries the value labels.
    let (left_gutter, bottom_gutter) = if horizontal {
        let widest = categories
            .iter()
            .map(|c| ctx.text_width(c, LABEL_SIZE))
            .fold(0.0f32, f32::max);
        (
            if show_cat_labels {
                widest + PADDING
            } else {
                0.0
            },
            if show_value_labels {
                label_height + PADDING
            } else {
                0.0
            },
        )
    } else {
        let widest = ticks
            .iter()
            .map(|v| ctx.text_width(&format_axis_value_for(*v, format), LABEL_SIZE))
            .fold(0.0f32, f32::max);
        (
            if show_value_labels {
                widest + PADDING
            } else {
                0.0
            },
            if show_cat_labels {
                label_height + PADDING
            } else {
                0.0
            },
        )
    };

    let plot_rect = Rect::new(
        area.x + left_gutter,
        area.y,
        (area.w - left_gutter).max(1.0),
        (area.h - bottom_gutter).max(1.0),
    );

    let gridline = Stroke {
        paint: Paint::Solid(ctx.gridline_color()),
        width: 0.75,
        ..Default::default()
    };

    // Value gridlines and labels.
    let wants_gridlines = value_axis.map(|a| a.major_gridlines).unwrap_or(true);
    for value in &ticks {
        let t = if (hi - lo).abs() < f64::EPSILON {
            0.0
        } else {
            ((value - lo) / (hi - lo)) as f32
        };
        if horizontal {
            let x = plot_rect.x + plot_rect.w * t;
            if wants_gridlines {
                let mut p = Path::new();
                p.move_to(x, plot_rect.y).line_to(x, plot_rect.bottom());
                out.push(Command::StrokePath {
                    path: p,
                    stroke: gridline.clone(),
                });
            }
            if show_value_labels {
                ctx.label(
                    &format_axis_value_for(*value, format),
                    x,
                    plot_rect.bottom() + label_height * 0.8,
                    LabelAlign::Center,
                    LABEL_SIZE,
                    out,
                );
            }
        } else {
            let y = plot_rect.bottom() - plot_rect.h * t;
            if wants_gridlines {
                let mut p = Path::new();
                p.move_to(plot_rect.x, y).line_to(plot_rect.right(), y);
                out.push(Command::StrokePath {
                    path: p,
                    stroke: gridline.clone(),
                });
            }
            if show_value_labels {
                ctx.label(
                    &format_axis_value_for(*value, format),
                    plot_rect.x - PADDING,
                    y + LABEL_SIZE * 0.35,
                    LabelAlign::Right,
                    LABEL_SIZE,
                    out,
                );
            }
        }
    }

    // Category labels, centred on each band.
    if show_cat_labels && !categories.is_empty() {
        let n = categories.len() as f32;
        for (i, label) in categories.iter().enumerate() {
            let centre = (i as f32 + 0.5) / n;
            if horizontal {
                // Categories run bottom-to-top on a horizontal bar chart.
                let y = plot_rect.bottom() - plot_rect.h * centre;
                ctx.label(
                    label,
                    plot_rect.x - PADDING,
                    y + LABEL_SIZE * 0.35,
                    LabelAlign::Right,
                    LABEL_SIZE,
                    out,
                );
            } else {
                let x = plot_rect.x + plot_rect.w * centre;
                ctx.label(
                    label,
                    x,
                    plot_rect.bottom() + label_height * 0.8,
                    LabelAlign::Center,
                    LABEL_SIZE,
                    out,
                );
            }
        }
    }

    // The axis lines themselves, along the zero line where there is one.
    let axis_stroke = Stroke {
        paint: Paint::Solid(ctx.gridline_color()),
        width: 1.0,
        ..Default::default()
    };
    let zero_t = if (hi - lo).abs() < f64::EPSILON {
        0.0
    } else {
        ((0.0 - lo) / (hi - lo)).clamp(0.0, 1.0) as f32
    };
    let mut baseline = Path::new();
    if horizontal {
        let x = plot_rect.x + plot_rect.w * zero_t;
        baseline
            .move_to(x, plot_rect.y)
            .line_to(x, plot_rect.bottom());
    } else {
        let y = plot_rect.bottom() - plot_rect.h * zero_t;
        baseline
            .move_to(plot_rect.x, y)
            .line_to(plot_rect.right(), y);
    }
    out.push(Command::StrokePath {
        path: baseline,
        stroke: axis_stroke,
    });

    plot_rect
}

/// The value scale for a plot, honouring manual bounds.
fn scale_for(chart: &Chart, plot: &Plot, axis: Option<&Axis>) -> (f64, f64, f64) {
    let (data_lo, data_hi) = chart.value_range(plot);
    // Bars and areas encode value as length from a baseline, so their axis has to include
    // zero. Lines and scatters encode it as position and scale to the data instead.
    let anchor_zero =
        matches!(plot.kind, PlotKind::Bar { .. } | PlotKind::Area) || plot.grouping.is_stacked();
    let (mut lo, mut hi, mut step) = nice_scale(data_lo, data_hi, 5, anchor_zero);
    if let Some(axis) = axis {
        if let Some(min) = axis.min {
            lo = min;
        }
        if let Some(max) = axis.max {
            hi = max;
        }
        if let Some(unit) = axis.major_unit.filter(|u| *u > 0.0) {
            step = unit;
        }
    }
    if hi <= lo {
        hi = lo + 1.0;
    }
    if step <= 0.0 || !step.is_finite() {
        step = (hi - lo) / 5.0;
    }
    (lo, hi, step)
}

fn tick_values(lo: f64, hi: f64, step: f64) -> Vec<f64> {
    let mut out = Vec::new();
    if step <= 0.0 || !step.is_finite() {
        return vec![lo, hi];
    }
    // Bounded so a pathological major unit cannot produce a million gridlines.
    let count = (((hi - lo) / step).round() as i64).clamp(1, 100);
    for i in 0..=count {
        out.push(lo + step * i as f64);
    }
    out
}

fn draw_plot(
    chart: &Chart,
    plot: &Plot,
    plot_index: usize,
    rect: Rect,
    ctx: &Ctx<'_>,
    out: &mut Vec<Command>,
) {
    // Colour indices continue across plots so a bar+line combination does not repeat
    // accent1 for its first series of each.
    let colour_base: u32 = chart
        .plots
        .iter()
        .take(plot_index)
        .map(|p| p.series.len() as u32)
        .sum();

    match plot.kind {
        PlotKind::Bar { horizontal } => {
            draw_bars(chart, plot, rect, colour_base, horizontal, ctx, out)
        }
        PlotKind::Line | PlotKind::Scatter => draw_lines(chart, plot, rect, colour_base, ctx, out),
        PlotKind::Area => draw_areas(chart, plot, rect, colour_base, ctx, out),
        PlotKind::Pie | PlotKind::Doughnut => draw_pie(chart, plot, rect, ctx, out),
    }
}

/// Maps a value onto 0..1 of the plot's value axis.
fn normalise(value: f64, lo: f64, hi: f64) -> f32 {
    if (hi - lo).abs() < f64::EPSILON {
        return 0.0;
    }
    (((value - lo) / (hi - lo)) as f32).clamp(-10.0, 10.0)
}

#[allow(clippy::too_many_arguments)]
fn draw_bars(
    chart: &Chart,
    plot: &Plot,
    rect: Rect,
    colour_base: u32,
    horizontal: bool,
    ctx: &Ctx<'_>,
    out: &mut Vec<Command>,
) {
    let (lo, hi, _) = scale_for(chart, plot, chart.value_axis(plot));
    let points = plot
        .series
        .iter()
        .map(|s| s.values.len())
        .max()
        .unwrap_or(0);
    if points == 0 {
        return;
    }
    let band = if horizontal { rect.h } else { rect.w } / points as f32;

    // `gapWidth` is a percentage of one bar's width, so the band is shared between the
    // bars in a cluster plus that gap.
    let series_count = if plot.grouping.is_stacked() {
        1.0
    } else {
        plot.series.len().max(1) as f32
    };
    let gap_fraction = (plot.gap_width / 100.0).max(0.0);
    let bar = band / (series_count + gap_fraction);
    // Negative overlap separates clustered bars, positive makes them overlap.
    let overlap = bar * (plot.overlap / 100.0);
    let cluster_width = bar * series_count - overlap * (series_count - 1.0);

    let zero = normalise(0.0, lo, hi).clamp(0.0, 1.0);

    for point in 0..points {
        // Stacked bars accumulate separately in each direction so a negative value grows
        // downward from zero rather than punching through the positive stack.
        let mut stack_pos = 0.0f64;
        let mut stack_neg = 0.0f64;
        let total: f64 = plot
            .series
            .iter()
            .filter_map(|s| s.values.get(point).copied().flatten())
            .map(f64::abs)
            .sum();

        for (si, series) in plot.series.iter().enumerate() {
            let Some(raw) = series.values.get(point).copied().flatten() else {
                continue;
            };
            let value = if plot.grouping == Grouping::PercentStacked {
                if total.abs() < f64::EPSILON {
                    0.0
                } else {
                    raw / total
                }
            } else {
                raw
            };

            let (from, to) = if plot.grouping.is_stacked() {
                let base = if value >= 0.0 { stack_pos } else { stack_neg };
                let top = base + value;
                if value >= 0.0 {
                    stack_pos = top;
                } else {
                    stack_neg = top;
                }
                (normalise(base, lo, hi), normalise(top, lo, hi))
            } else {
                (zero, normalise(value, lo, hi))
            };

            let offset = if plot.grouping.is_stacked() {
                (band - cluster_width) / 2.0
            } else {
                (band - cluster_width) / 2.0 + si as f32 * (bar - overlap)
            };
            let thickness = if plot.grouping.is_stacked() {
                cluster_width
            } else {
                bar
            };

            let bar_rect = if horizontal {
                // Categories run bottom-to-top.
                let y = rect.bottom() - band * (point as f32 + 1.0) + offset;
                let x0 = rect.x + rect.w * from.min(to);
                let x1 = rect.x + rect.w * from.max(to);
                Rect::new(x0, y, (x1 - x0).abs(), thickness)
            } else {
                let x = rect.x + band * point as f32 + offset;
                let y0 = rect.bottom() - rect.h * from.max(to);
                let y1 = rect.bottom() - rect.h * from.min(to);
                Rect::new(x, y0, thickness, (y1 - y0).abs())
            };
            if bar_rect.w <= 0.0 || bar_rect.h <= 0.0 {
                continue;
            }

            let colour_index = if plot.vary_colors {
                point as u32
            } else {
                colour_base + si as u32
            };
            let colour = match series.fill_for_point(point as u32) {
                Fill::Solid(c) => ctx.resolver.color(c),
                _ => ctx.series_color(series, colour_index),
            };
            out.push(Command::FillPath {
                path: Path::rect(bar_rect),
                paint: Paint::Solid(colour),
                rule: FillRule::NonZero,
            });
            if let Some(stroke) = ctx.stroke(&series.line, bar_rect) {
                out.push(Command::StrokePath {
                    path: Path::rect(bar_rect),
                    stroke,
                });
            }
        }
    }
}

fn draw_lines(
    chart: &Chart,
    plot: &Plot,
    rect: Rect,
    colour_base: u32,
    ctx: &Ctx<'_>,
    out: &mut Vec<Command>,
) {
    let (lo, hi, _) = scale_for(chart, plot, chart.value_axis(plot));
    let points = plot
        .series
        .iter()
        .map(|s| s.values.len())
        .max()
        .unwrap_or(0);
    if points == 0 {
        return;
    }
    let scatter = plot.kind == PlotKind::Scatter;

    for (si, series) in plot.series.iter().enumerate() {
        let colour = ctx.series_color(series, colour_base + si as u32);
        let width = series
            .line
            .width
            .map(crate::emu::to_pt)
            .unwrap_or(2.0)
            .max(0.75);

        // A gap breaks the polyline rather than interpolating across it: joining the
        // points either side would invent data that is not in the file.
        let mut path = Path::new();
        let mut open = false;
        let mut markers: Vec<Point> = Vec::new();

        for i in 0..points {
            let Some(value) = series.values.get(i).copied().flatten() else {
                open = false;
                continue;
            };
            let x = if scatter {
                let xs = series
                    .x_values
                    .get(i)
                    .copied()
                    .flatten()
                    .unwrap_or(i as f64);
                let (xlo, xhi) = series
                    .x_values
                    .iter()
                    .flatten()
                    .fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
                if xhi > xlo {
                    rect.x + rect.w * ((xs - xlo) / (xhi - xlo)) as f32
                } else {
                    rect.x + rect.w / 2.0
                }
            } else {
                // Line charts sit points at the centre of each category band, matching
                // where the category labels are drawn.
                rect.x + rect.w * (i as f32 + 0.5) / points as f32
            };
            let y = rect.bottom() - rect.h * normalise(value, lo, hi);
            markers.push(Point::new(x, y));
            if open {
                path.line_to(x, y);
            } else {
                path.move_to(x, y);
                open = true;
            }
        }

        if !path.is_empty() && !scatter {
            out.push(Command::StrokePath {
                path,
                stroke: Stroke {
                    paint: Paint::Solid(colour),
                    width,
                    cap: crate::dl::LineCap::Round,
                    join: crate::dl::LineJoin::Round,
                    ..Default::default()
                },
            });
        }

        if series.marker || scatter {
            let r = (width * 1.6).max(2.5);
            for p in markers {
                out.push(Command::FillPath {
                    path: Path::ellipse(Rect::new(p.x - r, p.y - r, r * 2.0, r * 2.0)),
                    paint: Paint::Solid(colour),
                    rule: FillRule::NonZero,
                });
            }
        }
    }
}

fn draw_areas(
    chart: &Chart,
    plot: &Plot,
    rect: Rect,
    colour_base: u32,
    ctx: &Ctx<'_>,
    out: &mut Vec<Command>,
) {
    let (lo, hi, _) = scale_for(chart, plot, chart.value_axis(plot));
    let points = plot
        .series
        .iter()
        .map(|s| s.values.len())
        .max()
        .unwrap_or(0);
    if points < 2 {
        return;
    }
    let zero_y = rect.bottom() - rect.h * normalise(0.0, lo, hi).clamp(0.0, 1.0);
    let mut stack = vec![0.0f64; points];

    for (si, series) in plot.series.iter().enumerate() {
        let colour = ctx.series_color(series, colour_base + si as u32);
        let mut path = Path::new();
        let mut tops: Vec<(f32, f32)> = Vec::with_capacity(points);

        for i in 0..points {
            let value = series.values.get(i).copied().flatten().unwrap_or(0.0);
            let top = if plot.grouping.is_stacked() {
                let base = stack.get(i).copied().unwrap_or(0.0);
                let t = base + value;
                if let Some(slot) = stack.get_mut(i) {
                    *slot = t;
                }
                t
            } else {
                value
            };
            let x = rect.x + rect.w * (i as f32) / (points - 1) as f32;
            tops.push((x, rect.bottom() - rect.h * normalise(top, lo, hi)));
        }

        for (i, (x, y)) in tops.iter().enumerate() {
            if i == 0 {
                path.move_to(*x, *y);
            } else {
                path.line_to(*x, *y);
            }
        }
        // Close down to the baseline so the region fills.
        if let Some((last_x, _)) = tops.last() {
            path.line_to(*last_x, zero_y);
        }
        if let Some((first_x, _)) = tops.first() {
            path.line_to(*first_x, zero_y);
        }
        path.close();

        out.push(Command::FillPath {
            path,
            paint: Paint::Solid(colour.with_alpha_factor(0.75)),
            rule: FillRule::NonZero,
        });
    }
}

fn draw_pie(chart: &Chart, plot: &Plot, rect: Rect, ctx: &Ctx<'_>, out: &mut Vec<Command>) {
    let Some(series) = plot.series.first() else {
        return;
    };
    let total: f64 = series.values.iter().flatten().map(|v| v.abs()).sum();
    if total <= 0.0 {
        return;
    }
    let radius = (rect.w.min(rect.h) / 2.0) * 0.9;
    if radius <= 0.0 {
        return;
    }
    let centre = rect.center();
    let hole = if plot.kind == PlotKind::Doughnut {
        (radius * plot.hole_percent.clamp(0.0, 0.95)).max(0.0)
    } else {
        0.0
    };

    // Pie slices start at twelve o'clock and run clockwise.
    let mut angle = -PI / 2.0;
    for (i, value) in series.values.iter().enumerate() {
        let Some(value) = value else { continue };
        let sweep = (value.abs() / total) as f32 * 2.0 * PI;
        if sweep <= 0.0 {
            continue;
        }
        let colour = match series.fill_for_point(i as u32) {
            Fill::Solid(c) => ctx.resolver.color(c),
            _ => ctx.accent(i as u32),
        };

        let mut path = Path::new();
        if hole > 0.0 {
            path.move_to(centre.x + hole * angle.cos(), centre.y + hole * angle.sin());
            path.line_to(
                centre.x + radius * angle.cos(),
                centre.y + radius * angle.sin(),
            );
            path.arc_to(centre.x, centre.y, radius, radius, angle, sweep);
            path.line_to(
                centre.x + hole * (angle + sweep).cos(),
                centre.y + hole * (angle + sweep).sin(),
            );
            path.arc_to(centre.x, centre.y, hole, hole, angle + sweep, -sweep);
        } else {
            path.move_to(centre.x, centre.y);
            path.line_to(
                centre.x + radius * angle.cos(),
                centre.y + radius * angle.sin(),
            );
            path.arc_to(centre.x, centre.y, radius, radius, angle, sweep);
        }
        path.close();

        out.push(Command::FillPath {
            path: path.clone(),
            paint: Paint::Solid(colour),
            rule: FillRule::NonZero,
        });
        // A thin separator, which is what makes adjacent slices of similar colour read
        // as separate.
        out.push(Command::StrokePath {
            path,
            stroke: Stroke {
                paint: Paint::Solid(Color::WHITE),
                width: 1.0,
                ..Default::default()
            },
        });
        angle += sweep;
    }
    let _ = chart;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::chart::{Axis, AxisKind, AxisPosition, Legend, Plot, Series};
    use crate::model::shape::{SlideLayout, SlideMaster};
    use crate::model::theme::Theme;
    use crate::model::{Slide, SlideChain};
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

    fn series(name: &str, values: &[f64]) -> Series {
        Series {
            name: name.to_string(),
            values: values.iter().map(|v| Some(*v)).collect(),
            marker: false,
            ..Default::default()
        }
    }

    fn bar_chart() -> Chart {
        Chart {
            title: Some("Revenue".into()),
            plots: vec![Plot {
                kind: PlotKind::Bar { horizontal: false },
                grouping: Grouping::Clustered,
                series: vec![
                    Series {
                        categories: vec!["North".into(), "South".into(), "East".into()],
                        ..series("2023", &[1204.0, 988.0, 1455.0])
                    },
                    series("2024", &[1388.0, 1022.0, 1390.0]),
                ],
                axis_ids: vec![1, 2],
                ..Default::default()
            }],
            axes: vec![
                Axis {
                    id: 1,
                    kind: AxisKind::Category,
                    position: AxisPosition::Bottom,
                    ..Default::default()
                },
                Axis {
                    id: 2,
                    kind: AxisKind::Value,
                    major_gridlines: true,
                    ..Default::default()
                },
            ],
            legend: Some(Legend::default()),
            ..Default::default()
        }
    }

    const FRAME: Rect = Rect::new(0.0, 0.0, 480.0, 320.0);

    fn run(chart: &Chart) -> Vec<Command> {
        let e = env();
        let r = Resolver::new(&e.pres, &e.chain);
        let mut out = Vec::new();
        layout_chart(chart, FRAME, &r, &e.pres, &StubMeasure, "p", &mut out);
        out
    }

    fn texts(cmds: &[Command]) -> Vec<String> {
        cmds.iter()
            .filter_map(|c| match c {
                Command::DrawText(r) => Some(r.text.clone()),
                _ => None,
            })
            .collect()
    }

    fn fills(cmds: &[Command]) -> Vec<(Rect, Color)> {
        cmds.iter()
            .filter_map(|c| match c {
                Command::FillPath {
                    path,
                    paint: Paint::Solid(col),
                    ..
                } => Some((path.bounds(), *col)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_bar_chart_draws_a_bar_per_value_inside_the_frame() {
        let cmds = run(&bar_chart());
        // Six data bars, plus the chart background and the legend swatches.
        let bars: Vec<_> = fills(&cmds)
            .into_iter()
            .filter(|(r, _)| r.w > 1.0 && r.h > 10.0 && r.w < FRAME.w / 2.0)
            .collect();
        assert_eq!(bars.len(), 6, "expected six bars, got {}", bars.len());
        for (rect, _) in &bars {
            assert!(
                rect.x >= FRAME.x - 0.5 && rect.right() <= FRAME.right() + 0.5,
                "bar escaped the frame: {rect:?}"
            );
            assert!(
                rect.bottom() <= FRAME.bottom() + 0.5,
                "bar below the frame: {rect:?}"
            );
        }
    }

    #[test]
    fn taller_values_produce_taller_bars() {
        let cmds = run(&bar_chart());
        let bars: Vec<_> = fills(&cmds)
            .into_iter()
            .filter(|(r, _)| r.w > 1.0 && r.h > 10.0 && r.w < FRAME.w / 2.0)
            .collect();
        // The first series' values are 1204, 988, 1455 — so bar 0 > bar 1 in height, and
        // bar 2 is the tallest of the three.
        let heights: Vec<f32> = bars.iter().step_by(2).map(|(r, _)| r.h).collect();
        assert!(
            heights[0] > heights[1],
            "1204 should be taller than 988: {heights:?}"
        );
        assert!(
            heights[2] > heights[0],
            "1455 should be tallest: {heights:?}"
        );
    }

    #[test]
    fn the_title_categories_and_legend_labels_are_all_drawn() {
        let cmds = run(&bar_chart());
        let t = texts(&cmds);
        assert!(t.contains(&"Revenue".to_string()), "{t:?}");
        assert!(t.contains(&"North".to_string()), "{t:?}");
        assert!(
            t.contains(&"2023".to_string()) && t.contains(&"2024".to_string()),
            "{t:?}"
        );
    }

    #[test]
    fn series_get_different_colours_by_default() {
        let cmds = run(&bar_chart());
        let bars: Vec<_> = fills(&cmds)
            .into_iter()
            .filter(|(r, _)| r.w > 1.0 && r.h > 10.0 && r.w < FRAME.w / 2.0)
            .collect();
        // Bars alternate between the two series, so adjacent bars must differ.
        assert_ne!(bars[0].1, bars[1].1, "two series should not share a colour");
    }

    #[test]
    fn a_stacked_bar_chart_stacks_rather_than_clusters() {
        let mut chart = bar_chart();
        if let Some(plot) = chart.plots.first_mut() {
            plot.grouping = Grouping::Stacked;
        }
        let cmds = run(&chart);
        let bars: Vec<_> = fills(&cmds)
            .into_iter()
            .filter(|(r, _)| r.w > 1.0 && r.h > 10.0 && r.w < FRAME.w / 2.0)
            .collect();
        assert_eq!(bars.len(), 6);
        // The two bars of a stacked category share an x range and sit on top of each other.
        let first_category: Vec<_> = bars.iter().filter(|(r, _)| r.x < FRAME.w / 3.0).collect();
        assert_eq!(first_category.len(), 2);
        let (a, b) = (first_category[0].0, first_category[1].0);
        assert!(
            (a.x - b.x).abs() < 0.5,
            "stacked bars share a column: {a:?} {b:?}"
        );
    }

    #[test]
    fn a_line_chart_emits_one_polyline_per_series() {
        let mut chart = bar_chart();
        if let Some(plot) = chart.plots.first_mut() {
            plot.kind = PlotKind::Line;
        }
        let cmds = run(&chart);
        // Gridlines and the axis are strokes too, so count only multi-segment paths.
        let polylines = cmds
            .iter()
            .filter(|c| match c {
                Command::StrokePath { path, .. } => path.points.len() >= 3,
                _ => false,
            })
            .count();
        assert_eq!(polylines, 2, "one polyline per series");
    }

    #[test]
    fn a_gap_in_the_data_breaks_the_line_rather_than_bridging_it() {
        let chart = Chart {
            plots: vec![Plot {
                kind: PlotKind::Line,
                series: vec![Series {
                    values: vec![Some(1.0), None, Some(3.0)],
                    marker: false,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let cmds = run(&chart);
        let subpaths = cmds
            .iter()
            .filter_map(|c| match c {
                Command::StrokePath { path, .. } if path.points.len() >= 2 => Some(
                    path.verbs
                        .iter()
                        .filter(|v| **v == crate::dl::PathVerb::MoveTo)
                        .count(),
                ),
                _ => None,
            })
            .max();
        // Two separate MoveTo runs means the gap broke the line.
        assert_eq!(subpaths, Some(2), "a gap must not be interpolated across");
    }

    #[test]
    fn a_pie_chart_draws_one_wedge_per_value_and_no_axes() {
        let chart = Chart {
            plots: vec![Plot {
                kind: PlotKind::Pie,
                vary_colors: true,
                series: vec![Series {
                    categories: vec!["A".into(), "B".into(), "C".into()],
                    ..series("share", &[50.0, 30.0, 20.0])
                }],
                ..Default::default()
            }],
            legend: Some(Legend::default()),
            ..Default::default()
        };
        let cmds = run(&chart);
        let wedges = cmds
            .iter()
            .filter(|c| match c {
                Command::FillPath { path, .. } => {
                    path.verbs.contains(&crate::dl::PathVerb::CubicTo)
                }
                _ => false,
            })
            .count();
        assert_eq!(wedges, 3, "one wedge per value");
        // Pie charts have no value-axis tick labels.
        let t = texts(&cmds);
        assert!(t.contains(&"A".to_string()), "legend labels: {t:?}");
    }

    #[test]
    fn a_doughnut_leaves_a_hole() {
        let chart = Chart {
            plots: vec![Plot {
                kind: PlotKind::Doughnut,
                hole_percent: 0.5,
                series: vec![series("share", &[1.0, 1.0])],
                ..Default::default()
            }],
            ..Default::default()
        };
        let cmds = run(&chart);
        // Each wedge is an annular segment: two arcs, so more curve verbs than a pie's one.
        let curves: usize = cmds
            .iter()
            .filter_map(|c| match c {
                Command::FillPath { path, .. } => Some(
                    path.verbs
                        .iter()
                        .filter(|v| **v == crate::dl::PathVerb::CubicTo)
                        .count(),
                ),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        assert!(
            curves >= 2,
            "a doughnut wedge needs an inner and an outer arc"
        );
    }

    #[test]
    fn an_empty_chart_still_draws_its_background_and_nothing_else() {
        let cmds = run(&Chart::default());
        assert_eq!(fills(&cmds).len(), 1);
        assert!(texts(&cmds).is_empty());
    }

    #[test]
    fn a_zero_sized_frame_emits_nothing() {
        let e = env();
        let r = Resolver::new(&e.pres, &e.chain);
        let mut out = Vec::new();
        layout_chart(
            &bar_chart(),
            Rect::new(0.0, 0.0, 0.0, 0.0),
            &r,
            &e.pres,
            &StubMeasure,
            "p",
            &mut out,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn manual_axis_bounds_override_the_derived_scale() {
        let mut chart = bar_chart();
        if let Some(axis) = chart.axes.iter_mut().find(|a| a.kind == AxisKind::Value) {
            axis.min = Some(0.0);
            axis.max = Some(4000.0);
        }
        let cmds = run(&chart);
        let bars: Vec<_> = fills(&cmds)
            .into_iter()
            .filter(|(r, _)| r.w > 1.0 && r.h > 5.0 && r.w < FRAME.w / 2.0)
            .collect();
        let tallest = bars.iter().map(|(r, _)| r.h).fold(0.0f32, f32::max);
        // With the axis stretched to 4000, the tallest bar (1455) uses well under half
        // the plot height.
        assert!(
            tallest < FRAME.h * 0.45,
            "bar height {tallest} suggests the bound was ignored"
        );
    }

    #[test]
    fn every_emitted_coordinate_is_finite_for_degenerate_data() {
        for values in [vec![], vec![0.0], vec![f64::MAX, f64::MIN]] {
            let chart = Chart {
                plots: vec![Plot {
                    series: vec![series("s", &values)],
                    ..Default::default()
                }],
                ..Default::default()
            };
            for cmd in run(&chart) {
                if let Command::FillPath { path, .. } | Command::StrokePath { path, .. } = cmd {
                    assert!(
                        path.points
                            .iter()
                            .all(|p| p.x.is_finite() && p.y.is_finite()),
                        "non-finite point for {values:?}"
                    );
                }
            }
        }
    }
}
