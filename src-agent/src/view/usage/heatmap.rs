//! Heatmap builders for the usage dashboard.

use std::collections::HashMap;

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::app::mode::{UsageMetric, UsageNavState, UsageRange};
use crate::model::usage::SpendBucket;
use crate::view::theme::Palette;

use super::format::{now_secs, fmt_cost, fmt_tokens_u64, METRIC_LABEL_W};

pub(crate) const HEAT_EMPTY: Color = Color::Rgb(35, 35, 35);
pub(crate) const HEAT_1: Color = Color::Rgb(0, 120, 60);
pub(crate) const HEAT_2: Color = Color::Rgb(100, 160, 50);
pub(crate) const HEAT_3: Color = Color::Rgb(200, 140, 0);
pub(crate) const HEAT_4: Color = Color::Rgb(220, 50, 50);
pub(crate) const CELL: &str = "\u{2588}";
pub(crate) const RULE: &str = "\u{2500}";
pub(crate) const BAR_CHARS: [char; 9] = [
    ' ',
    '\u{258F}', '\u{258E}', '\u{258D}', '\u{258C}',
    '\u{258B}', '\u{258A}', '\u{2589}', '\u{2588}',
];
pub(crate) const BAR_MAX_WIDTH: usize = 20;
pub(crate) const COL_GAP: u16 = 2;

pub(crate) fn heatmap_title(nav: &UsageNavState) -> String {
    let metric_label = match nav.metric {
        UsageMetric::Cost => "COST",
        UsageMetric::Tokens => "TOKEN USAGE",
    };
    match nav.range {
        UsageRange::Today => format!("{metric_label} (HOURLY)"),
        UsageRange::Week => format!("{metric_label} (DAILY)"),
        UsageRange::Year => "HEATMAP (YEARLY)".to_string(),
    }
}

pub(crate) fn heatmap_content_height(nav: &UsageNavState) -> usize {
    match nav.range {
        UsageRange::Today => 25,
        UsageRange::Week => 8,
        UsageRange::Year => 9,
    }
}

pub(crate) fn build_heatmap(
    nav: &UsageNavState,
    buckets: &[SpendBucket],
    max_width: usize,
    palette: &Palette,
) -> Vec<Line<'static>> {
    match nav.range {
        UsageRange::Today => build_hourly_horizontal_chart(buckets, nav.metric, max_width, palette),
        UsageRange::Week => build_day_horizontal_chart(buckets, nav.metric, max_width, palette),
        UsageRange::Year => build_heatmap_yearly(buckets, nav.metric, palette),
    }
}

fn build_hourly_horizontal_chart(
    buckets: &[SpendBucket],
    metric: UsageMetric,
    max_width: usize,
    palette: &Palette,
) -> Vec<Line<'static>> {
    let map: HashMap<i64, SpendBucket> = buckets.iter().cloned().map(|b| (b.bucket_epoch, b)).collect();
    let now = now_secs();
    let tz = crate::model::usage::local_utc_offset_secs();
    let local_now = now + tz;
    let today = local_now - local_now % 86400 - tz;
    let current_hour = ((local_now % 86400) / 3600) as usize;
    let epochs: Vec<i64> = (0..24).map(|i| today + i * 3600).collect();
    let values: Vec<f64> = epochs
        .iter()
        .map(|ep| map.get(ep).map(|b| metric_val(b, metric)).unwrap_or(0.0))
        .collect();
    let nonzero: Vec<f64> = values.iter().copied().filter(|&v| v > 0.0).collect();
    let (p33, p66, p90) = percentile_thresholds(&nonzero);
    let max_val = values.iter().cloned().fold(0.0_f64, f64::max);
    let label_w = 3usize;
    let bar_w = max_width.saturating_sub(label_w + METRIC_LABEL_W + 1).max(1);
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(25);

    for (&v, &h) in values.iter().zip(epochs.iter()) {
        let col = heat_color(v, p33, p66, p90, false);
        let fill = if max_val <= 0.0 { 0usize } else { ((v / max_val) * bar_w as f64).round() as usize };
        let hour = (((h + tz) % 86400) / 3600) as usize;
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(bar_w + 2);
        let label_style = if hour == current_hour {
            Style::default().fg(palette.accent).bg(HEAT_EMPTY).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.dim).bg(HEAT_EMPTY)
        };
        spans.push(Span::styled(format!("{hour:02}"), label_style));
        for j in 0..bar_w {
            if j < fill {
                spans.push(Span::styled(CELL, Style::default().fg(col).bg(HEAT_EMPTY)));
            } else {
                spans.push(Span::styled(CELL, Style::default().fg(HEAT_EMPTY).bg(HEAT_EMPTY)));
            }
        }
        let val_str = bar_metric_label(v, metric);
        spans.push(Span::styled(
            format!(" {val_str:>width$}", width = METRIC_LABEL_W - 1),
            Style::default().fg(palette.dim).bg(HEAT_EMPTY),
        ));
        lines.push(Line::from(spans));
    }

    lines.push(heat_legend(palette));
    lines
}

fn build_day_horizontal_chart(
    buckets: &[SpendBucket],
    metric: UsageMetric,
    max_width: usize,
    palette: &Palette,
) -> Vec<Line<'static>> {
    const DOW: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let map: HashMap<i64, SpendBucket> = buckets.iter().cloned().map(|b| (b.bucket_epoch, b)).collect();
    let now = now_secs();
    let tz = crate::model::usage::local_utc_offset_secs();
    let local_now = now + tz;
    let snap = local_now - local_now % 86400 - tz;
    let epochs: Vec<i64> = (0..7).map(|i| snap - (6 - i) * 86400).collect();
    let values: Vec<f64> = epochs
        .iter()
        .map(|ep| map.get(ep).map(|b| metric_val(b, metric)).unwrap_or(0.0))
        .collect();
    let labels: Vec<&str> = epochs
        .iter()
        .map(|ep| {
            let local = ep + tz;
            DOW[((local / 86400 + 3) % 7) as usize]
        })
        .collect();
    let nonzero: Vec<f64> = values.iter().copied().filter(|&v| v > 0.0).collect();
    let (p33, p66, p90) = percentile_thresholds(&nonzero);
    let max_val = values.iter().cloned().fold(0.0_f64, f64::max);
    let label_w = 4usize;
    let bar_w = max_width.saturating_sub(label_w + METRIC_LABEL_W + 1).max(1);
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(8);

    for (i, (&v, label)) in values.iter().zip(labels.iter()).enumerate() {
        let col = heat_color(v, p33, p66, p90, false);
        let fill = if max_val <= 0.0 { 0usize } else { ((v / max_val) * bar_w as f64).round() as usize };
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(bar_w + 2);
        let label_style = if i == 6 {
            Style::default().fg(palette.accent).bg(HEAT_EMPTY).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.dim).bg(HEAT_EMPTY)
        };
        spans.push(Span::styled(format!("{label} "), label_style));
        for j in 0..bar_w {
            if j < fill {
                spans.push(Span::styled(CELL, Style::default().fg(col).bg(HEAT_EMPTY)));
            } else {
                spans.push(Span::styled(CELL, Style::default().fg(HEAT_EMPTY).bg(HEAT_EMPTY)));
            }
        }
        let val_str = bar_metric_label(v, metric);
        spans.push(Span::styled(
            format!(" {val_str:>width$}", width = METRIC_LABEL_W - 1),
            Style::default().fg(palette.dim).bg(HEAT_EMPTY),
        ));
        lines.push(Line::from(spans));
    }

    lines.push(heat_legend(palette));
    lines
}

fn build_heatmap_yearly(
    buckets: &[SpendBucket],
    metric: UsageMetric,
    palette: &Palette,
) -> Vec<Line<'static>> {
    let map: HashMap<i64, SpendBucket> = buckets.iter().cloned().map(|b| (b.bucket_epoch, b)).collect();
    let now = now_secs();
    let tz = crate::model::usage::local_utc_offset_secs();
    let local_now = now + tz;
    let today = local_now - local_now % 86400 - tz;
    let today_dow = ((local_now / 86400 + 3) % 7) as usize;
    const COLS: usize = 53;
    const ROWS: usize = 7;
    let grid_start = today - (today_dow as i64 + (ROWS * (COLS - 1)) as i64) * 86400;
    let nonzero: Vec<f64> = map.values().map(|b| metric_val(b, metric)).filter(|&v| v > 0.0).collect();
    let (p33, p66, p90) = percentile_thresholds(&nonzero);
    let row_labels = ["   ", "Mon", "   ", "Wed", "   ", "Fri", "   "];
    let mut result: Vec<Line<'static>> = Vec::with_capacity(ROWS + 2);

    for (row, &label) in row_labels.iter().enumerate() {
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(COLS + 1);
        spans.push(Span::styled(format!("{label} "), Style::default().fg(palette.dim)));
        for col in 0..COLS {
            let day = grid_start + (col as i64 * ROWS as i64 + row as i64) * 86400;
            let future = day > today;
            let v = if future { -1.0 } else { map.get(&day).map(|b| metric_val(b, metric)).unwrap_or(0.0) };
            spans.push(Span::styled(CELL, Style::default().fg(heat_color(v, p33, p66, p90, future))));
        }
        result.push(Line::from(spans));
    }

    result.push(Line::default());
    result.push(heat_legend(palette));
    result
}

pub(crate) fn build_session_hourly_heatmap(
    hourly: &[SpendBucket],
    palette: &Palette,
    max_width: usize,
) -> Vec<Line<'static>> {
    if hourly.is_empty() {
        return vec![Line::from(Span::styled("no data yet", Style::default().fg(palette.dim)))];
    }
    let map: std::collections::HashMap<i64, &SpendBucket> =
        hourly.iter().map(|b| (b.bucket_epoch, b)).collect();
    let nonzero: Vec<f64> = hourly.iter().map(|b| b.cost).filter(|&v| v > 0.0).collect();
    let (p33, p66, p90) = percentile_thresholds(&nonzero);
    let max_val = hourly.iter().map(|b| b.cost).fold(0.0_f64, f64::max);
    let first = hourly.first().map(|b| b.bucket_epoch).unwrap_or(0);
    let last = hourly.last().map(|b| b.bucket_epoch).unwrap_or(first);
    let n_hours = (((last - first) / 3600) + 1).clamp(1, 24) as usize;
    let label_w = 3usize;
    let bar_w = max_width.saturating_sub(label_w).max(1);
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(n_hours + 1);

    for i in 0..n_hours {
        let epoch = first + i as i64 * 3600;
        let v = map.get(&epoch).map(|b| b.cost).unwrap_or(0.0);
        let col = heat_color(v, p33, p66, p90, false);
        let fill = if max_val <= 0.0 { 0usize } else { ((v / max_val) * bar_w as f64).round() as usize };
        let tz = crate::model::usage::local_utc_offset_secs();
        let hour = (((epoch + tz) % 86400) / 3600) as usize;
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(bar_w + 1);
        spans.push(Span::styled(format!("{hour:02}"), Style::default().fg(palette.dim).bg(HEAT_EMPTY)));
        for j in 0..bar_w {
            if j < fill {
                spans.push(Span::styled(CELL, Style::default().fg(col).bg(HEAT_EMPTY)));
            } else {
                spans.push(Span::styled(CELL, Style::default().fg(HEAT_EMPTY).bg(HEAT_EMPTY)));
            }
        }
        lines.push(Line::from(spans));
    }

    lines.push(heat_legend(palette));
    lines
}

fn heat_legend(palette: &Palette) -> Line<'static> {
    Line::from(vec![
        Span::styled("     cheap ", Style::default().fg(palette.dim)),
        Span::styled(CELL, Style::default().fg(HEAT_EMPTY)),
        Span::styled(CELL, Style::default().fg(HEAT_1)),
        Span::styled(CELL, Style::default().fg(HEAT_2)),
        Span::styled(CELL, Style::default().fg(HEAT_3)),
        Span::styled(CELL, Style::default().fg(HEAT_4)),
        Span::styled(" expensive", Style::default().fg(palette.dim)),
    ])
}

/// Format the per-bar metric value for display at the right edge.
pub(crate) fn bar_metric_label(v: f64, metric: UsageMetric) -> String {
    match metric {
        UsageMetric::Cost => fmt_cost(v),
        UsageMetric::Tokens => fmt_tokens_u64(v as u64),
    }
}

fn heat_color(v: f64, p33: f64, p66: f64, p90: f64, future: bool) -> Color {
    if future || v < 0.0 || v == 0.0 { return HEAT_EMPTY; }
    if p33 >= p90 { return HEAT_2; }
    if v <= p33 { HEAT_1 }
    else if v <= p66 { HEAT_2 }
    else if v <= p90 { HEAT_3 }
    else { HEAT_4 }
}

fn metric_val(b: &SpendBucket, metric: UsageMetric) -> f64 {
    match metric {
        UsageMetric::Cost => b.cost,
        UsageMetric::Tokens => b.tokens as f64,
    }
}

fn percentile_thresholds(nonzero: &[f64]) -> (f64, f64, f64) {
    if nonzero.is_empty() { return (0.0, 0.0, 0.0); }
    let mut s = nonzero.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    (percentile(&s, 33), percentile(&s, 66), percentile(&s, 90))
}

fn percentile(sorted: &[f64], pct: usize) -> f64 {
    if sorted.is_empty() { return 0.0; }
    sorted[((sorted.len() - 1) * pct) / 100]
}

pub(crate) fn build_bar(value: i64, max_val: i64, max_width: usize) -> String {
    if max_width == 0 || max_val <= 0 { return " ".repeat(max_width); }
    let v = value.max(0) as usize;
    let total_units = max_width * 8;
    let units = ((v as f64 / max_val as f64) * total_units as f64).round() as usize;
    let units = units.min(total_units);
    let full = units / 8;
    let rem = units % 8;
    let mut s = String::with_capacity(max_width);
    for _ in 0..full { s.push(BAR_CHARS[8]); }
    if full < max_width {
        if rem > 0 {
            s.push(BAR_CHARS[rem]);
            for _ in 0..(max_width - full - 1) { s.push(' '); }
        } else {
            for _ in 0..(max_width - full) { s.push(' '); }
        }
    }
    s
}
