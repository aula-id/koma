//! View — `/usage` cost and token usage dashboard.
//!
//! Two views toggled with Tab:
//! - **View A (Global)**: KPI list, range-adaptive heatmap, top-models table,
//!   per-model token bars, role split.
//! - **View B (Session)**: models-used table, hourly heatmap, session KPI totals.
//!
//! All DB queries are non-fatal (return empty/zero on missing ledger).

mod format;
mod heatmap;

use format::*;
use heatmap::*;

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::mode::{UsageNavState, UsageRange, UsageView};
use crate::app::state::AppStateRest;
use crate::model::usage::{BucketSize, UsageData};
use crate::view::theme::Palette;

/// Collect the `/usage` dashboard's ledger projection.
pub fn collect_usage_data(nav: &UsageNavState, rest: &AppStateRest) -> UsageData {
    let session_view = nav.view == UsageView::Session;
    let since = nav.range.since_secs();
    let (heat_bucket, heat_n) = match nav.range {
        UsageRange::Today => (BucketSize::Hour, 24),
        UsageRange::Week => (BucketSize::Day, 7),
        UsageRange::Year => (BucketSize::Day, 371),
    };
    let uuid = rest
        .fg()
        .session
        .as_ref()
        .map(|s| s.id.clone())
        .unwrap_or_default();
    UsageData::collect(session_view, since, heat_bucket, heat_n, &uuid)
}

pub fn draw(
    frame: &mut Frame,
    rest: &AppStateRest,
    nav: &UsageNavState,
    data: &UsageData,
    palette: &Palette,
) {
    let area = frame.area();

    if area.width < 20 || area.height < 6 {
        frame.render_widget(
            Paragraph::new(Span::styled("terminal too small", Style::default().fg(palette.accent))),
            area,
        );
        return;
    }

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    draw_header(frame, nav, palette, outer[0]);
    draw_footer(frame, palette, outer[2]);

    match nav.view {
        UsageView::Global  => draw_global(frame, nav, data, palette, outer[1]),
        UsageView::Session => draw_session(frame, rest, nav, data, palette, outer[1]),
    }
}

fn draw_header(frame: &mut Frame, nav: &UsageNavState, palette: &Palette, area: Rect) {
    let view_label = match nav.view {
        UsageView::Global  => "global",
        UsageView::Session => "session",
    };

    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::styled(
        "koma / usage  ",
        Style::default().fg(palette.accent).add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(
        format!("[tab: {view_label}]  "),
        Style::default().fg(palette.dim),
    ));

    if nav.view == UsageView::Global {
        let ranges: &[(UsageRange, &str)] = &[
            (UsageRange::Today, "1:today"),
            (UsageRange::Week,  "2:week"),
            (UsageRange::Year,  "3:year"),
        ];
        for (r, label) in ranges {
            if *r == nav.range {
                spans.push(Span::styled(
                    format!(" {label} "),
                    Style::default()
                        .fg(palette.sel_fg)
                        .bg(palette.sel_bg)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::styled(
                    format!(" {label} "),
                    Style::default().fg(palette.dim),
                ));
            }
            spans.push(Span::raw("  "));
        }
        let metric_label = match nav.metric {
            crate::app::mode::UsageMetric::Cost   => "[m: cost]",
            crate::app::mode::UsageMetric::Tokens => "[m: tokens]",
        };
        spans.push(Span::styled(metric_label, Style::default().fg(palette.dim)));
    }

    let header_block = Block::new()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(palette.dim));
    let inner = header_block.inner(area);
    frame.render_widget(header_block, area);
    let margin = inner.inner(Margin { horizontal: 1, vertical: 0 });
    frame.render_widget(Paragraph::new(Line::from(spans)), margin);
}

fn draw_footer(frame: &mut Frame, palette: &Palette, area: Rect) {
    let margin = area.inner(Margin { horizontal: 1, vertical: 0 });
    frame.render_widget(
        Paragraph::new(Span::styled(
            "[Tab] view  [1-3] range  [m] metric  [Esc] exit",
            Style::default().fg(palette.dim),
        )),
        margin,
    );
}

fn section(frame: &mut Frame, title: &str, palette: &Palette, area: Rect) -> Rect {
    if area.width == 0 || area.height == 0 {
        return Rect { x: area.x, y: area.y, width: area.width, height: 0 };
    }

    let w = area.width as usize;
    let label_w = title.chars().count().min(w);
    let rule_len = w.saturating_sub(label_w + 1);
    let rule: String = RULE.repeat(rule_len);

    let line = Line::from(vec![
        Span::styled(
            title.chars().take(label_w).collect::<String>(),
            Style::default().fg(palette.accent).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(rule, Style::default().fg(palette.dim)),
    ]);

    let label_row = Rect { x: area.x, y: area.y, width: area.width, height: 1 };
    frame.render_widget(Paragraph::new(line), label_row);

    Rect {
        x: area.x,
        y: area.y.saturating_add(1),
        width: area.width,
        height: area.height.saturating_sub(1),
    }
}

fn draw_global(frame: &mut Frame, nav: &UsageNavState, data: &UsageData, palette: &Palette, area: Rect) {
    if area.height < 3 {
        return;
    }

    let totals  = &data.totals;
    let models  = data.top_models.as_slice();
    let rsplit  = &data.role_split;

    let mid_w        = area.width.saturating_sub(COL_GAP) as usize;
    let left_w       = mid_w * 45 / 100;
    let right_w      = mid_w.saturating_sub(left_w);
    let heatmap_rows = heatmap_content_height(nav);
    let model_rows   = if models.is_empty() { 2 } else { 1 + models.len() * 2 };
    let mid_content  = heatmap_rows.max(model_rows).max(1);
    let mid_total    = (mid_content + 1) as u16;

    let role_total = 3u16;
    let kpi_total = 7u16;

    let blank = Constraint::Length(1);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(kpi_total),
            blank,
            Constraint::Length(mid_total),
            blank,
            Constraint::Length(role_total),
            Constraint::Min(0),
        ])
        .split(area);

    {
        let inner = section(frame, "KPI", palette, rows[0]);
        draw_kpi_strip(frame, totals, palette, inner);
    }

    {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(left_w as u16),
                Constraint::Length(COL_GAP),
                Constraint::Min(0),
            ])
            .split(rows[2]);

        let heat_inner = section(frame, &heatmap_title(nav), palette, cols[0]);
        draw_heatmap(frame, nav, &data.heatmap_buckets, heat_inner, palette);

        let models_inner = section(frame, "TOP MODELS", palette, cols[2]);
        draw_models(frame, models, totals, nav, palette, models_inner, right_w);
    }

    {
        let inner = section(frame, "ROLE SPLIT", palette, rows[4]);
        draw_role_split(frame, rsplit, palette, inner);
    }
}

fn draw_kpi_strip(
    frame: &mut Frame,
    totals: &crate::model::usage::RangeTotals,
    palette: &Palette,
    area: Rect,
) {
    if area.height == 0 || area.width < 10 {
        return;
    }

    let avg = if totals.calls > 0 { totals.cost / totals.calls as f64 } else { 0.0 };

    let metrics: &[(&str, String)] = &[
        ("total",    fmt_cost(totals.cost)),
        ("in",       fmt_tokens_i64(totals.tokens_in)),
        ("cached",   fmt_tokens_i64(totals.tokens_cached)),
        ("out",      fmt_tokens_i64(totals.tokens_out)),
        ("calls",    totals.calls.to_string()),
        ("avg/call", fmt_cost(avg)),
    ];

    let lines: Vec<Line<'static>> = metrics
        .iter()
        .map(|(label, value)| {
            Line::from(vec![
                Span::styled(
                    format!("{:<KPI_LABEL_W$}", label),
                    Style::default().fg(palette.dim),
                ),
                Span::styled(value.clone(), Style::default().fg(palette.fg)),
            ])
        })
        .collect();

    let visible: Vec<Line<'static>> = lines.into_iter().take(area.height as usize).collect();
    frame.render_widget(Paragraph::new(visible), area);
}

fn draw_heatmap(frame: &mut Frame, nav: &UsageNavState, buckets: &[crate::model::usage::SpendBucket], area: Rect, palette: &Palette) {
    if area.width < 8 || area.height == 0 {
        return;
    }

    let lines = build_heatmap(nav, buckets, area.width as usize, palette);
    let visible: Vec<Line<'static>> = lines.into_iter().take(area.height as usize).collect();
    frame.render_widget(Paragraph::new(visible), area);
}

fn draw_models(
    frame: &mut Frame,
    models: &[crate::model::usage::ModelCostRange],
    totals: &crate::model::usage::RangeTotals,
    nav: &UsageNavState,
    palette: &Palette,
    area: Rect,
    width_hint: usize,
) {
    use crate::app::mode::UsageMetric;
    let w = (area.width as usize).max(width_hint.min(area.width as usize));
    if w < 20 || area.height == 0 {
        return;
    }

    let total_cost = totals.cost;
    let max_tokens: i64 = models.iter().map(|m| m.tokens_in + m.tokens_out).max().unwrap_or(1).max(1);

    let fixed_cols = 34usize;
    let col_model = w.saturating_sub(fixed_cols).clamp(8, 24);

    let mut lines: Vec<Line<'static>> = Vec::new();

    lines.push(Line::from(Span::styled(
        format!(
            "{:<col_model$}  {:>9}  {:>9}  {:>6}  {:>5}",
            "model", "cost", "tokens", "calls", "%"
        ),
        Style::default().fg(palette.dim),
    )));

    for m in models {
        let id  = truncate(&m.model_id, col_model);
        let pct = if total_cost > 0.0 { (m.total_cost / total_cost * 100.0).round() as u64 } else { 0 };
        let total_tok = m.tokens_in + m.tokens_out;

        lines.push(Line::from(vec![
            Span::styled(format!("{:<col_model$}", id),             Style::default().fg(palette.fg)),
            Span::styled(format!("  {:>9}", fmt_cost(m.total_cost)),Style::default().fg(palette.fg)),
            Span::styled(format!("  {:>9}", fmt_tokens_i64(total_tok)), Style::default().fg(palette.dim)),
            Span::styled(format!("  {:>6}", m.call_count),          Style::default().fg(palette.dim)),
            Span::styled(format!("  {:>4}%", pct),                  Style::default().fg(palette.dim)),
        ]));

        let bar_w = w.saturating_sub(col_model + 3).min(BAR_MAX_WIDTH);
        let (bar_val, bar_max) = match nav.metric {
            UsageMetric::Tokens => (total_tok, max_tokens),
            UsageMetric::Cost   => {
                let scale = 1_000_000i64;
                ((m.total_cost * scale as f64) as i64, (total_cost * scale as f64).max(1.0) as i64)
            }
        };
        let bar = build_bar(bar_val, bar_max.max(1), bar_w);
        lines.push(Line::from(vec![
            Span::raw(format!("{:<col_model$}  ", "")),
            Span::styled(bar, Style::default().fg(palette.accent)),
        ]));
    }

    if models.is_empty() {
        lines.push(Line::from(Span::styled("no data for range", Style::default().fg(palette.dim))));
    }

    let visible: Vec<Line<'static>> = lines.into_iter().take(area.height as usize).collect();
    frame.render_widget(Paragraph::new(visible), area);
}

fn draw_role_split(frame: &mut Frame, split: &crate::model::usage::RoleSplit, palette: &Palette, area: Rect) {
    if area.height == 0 || area.width < 12 {
        return;
    }

    let total = (split.main_cost + split.sub_cost).max(1e-12);
    let main_pct = (split.main_cost / total * 100.0).round() as u64;
    let sub_pct  = (split.sub_cost  / total * 100.0).round() as u64;

    let bar_w = (area.width as usize).saturating_sub(22).min(BAR_MAX_WIDTH);
    let total_i = (total * 1_000_000.0) as i64;
    let main_bar = build_bar((split.main_cost * 1_000_000.0) as i64, total_i, bar_w);
    let sub_bar  = build_bar((split.sub_cost  * 1_000_000.0) as i64, total_i, bar_w);

    let lines = vec![
        Line::from(vec![
            Span::styled("main ", Style::default().fg(palette.dim)),
            Span::styled(format!("{:>8}  ", fmt_cost(split.main_cost)), Style::default().fg(palette.fg)),
            Span::styled(main_bar, Style::default().fg(HEAT_1)),
            Span::styled(format!("  {:>3}%  {:>3}c", main_pct, split.main_calls), Style::default().fg(palette.dim)),
        ]),
        Line::from(vec![
            Span::styled("sub  ", Style::default().fg(palette.dim)),
            Span::styled(format!("{:>8}  ", fmt_cost(split.sub_cost)), Style::default().fg(palette.fg)),
            Span::styled(sub_bar, Style::default().fg(HEAT_3)),
            Span::styled(format!("  {:>3}%  {:>3}c", sub_pct, split.sub_calls), Style::default().fg(palette.dim)),
        ]),
    ];

    let visible: Vec<Line<'static>> = lines.into_iter().take(area.height as usize).collect();
    frame.render_widget(Paragraph::new(visible), area);
}

fn draw_session(frame: &mut Frame, rest: &AppStateRest, _nav: &UsageNavState, data: &UsageData, palette: &Palette, area: Rect) {
    if area.height < 3 {
        return;
    }

    let sess_models = data.session_models.as_slice();
    let hourly      = data.session_hourly.as_slice();
    let db_calls    = data.session_calls;

    let mid_w     = area.width.saturating_sub(COL_GAP) as usize;
    let left_w    = mid_w * 55 / 100;
    let right_w   = mid_w.saturating_sub(left_w);
    let model_rows = if sess_models.is_empty() { 2 } else { 1 + sess_models.len() };
    let hourly_rows = if hourly.is_empty() { 1 } else { hourly.len().min(24) + 1 };
    let mid_content = model_rows.max(hourly_rows).max(1);
    let mid_total   = (mid_content + 1) as u16;

    let kpi_total = 6u16;

    let blank = Constraint::Length(1);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(kpi_total),
            blank,
            Constraint::Length(mid_total),
            Constraint::Min(0),
        ])
        .split(area);

    {
        let inner = section(frame, "SESSION TOTALS", palette, rows[0]);
        draw_session_kpi(frame, rest, db_calls, palette, inner);
    }

    {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(left_w as u16),
                Constraint::Length(COL_GAP),
                Constraint::Min(0),
            ])
            .split(rows[2]);

        let models_inner = section(frame, "MODELS USED", palette, cols[0]);
        draw_session_models(frame, sess_models, palette, models_inner, left_w);

        let hourly_inner = section(frame, "HOURLY HEATMAP", palette, cols[2]);
        draw_session_hourly(frame, hourly, palette, hourly_inner, right_w);
    }
}

fn draw_session_kpi(
    frame: &mut Frame,
    rest: &AppStateRest,
    db_calls: i64,
    palette: &Palette,
    area: Rect,
) {
    if area.height == 0 || area.width < 10 {
        return;
    }

    let fg = rest.fg();
    let metrics: &[(&str, String)] = &[
        ("in",     fmt_tokens_u64(fg.tokens_in)),
        ("cached", fmt_tokens_u64(fg.tokens_cached)),
        ("out",    fmt_tokens_u64(fg.tokens_out)),
        ("cost",   fmt_cost(fg.cost)),
        ("calls",  db_calls.to_string()),
    ];

    let lines: Vec<Line<'static>> = metrics
        .iter()
        .map(|(label, value)| {
            Line::from(vec![
                Span::styled(
                    format!("{:<KPI_LABEL_W$}", label),
                    Style::default().fg(palette.dim),
                ),
                Span::styled(value.clone(), Style::default().fg(palette.fg)),
            ])
        })
        .collect();

    let visible: Vec<Line<'static>> = lines.into_iter().take(area.height as usize).collect();
    frame.render_widget(Paragraph::new(visible), area);
}

fn draw_session_models(
    frame: &mut Frame,
    models: &[crate::model::usage::ModelCostRange],
    palette: &Palette,
    area: Rect,
    width_hint: usize,
) {
    let w = (area.width as usize).max(width_hint.min(area.width as usize));
    if area.height == 0 || w < 20 {
        return;
    }

    let fixed_cols = 30usize;
    let col_model  = w.saturating_sub(fixed_cols).clamp(8, 24);
    let bar_w      = w
        .saturating_sub(col_model + fixed_cols + 2)
        .clamp(0, 12);

    let max_tokens: i64 = models
        .iter()
        .map(|m| m.tokens_in + m.tokens_out)
        .max()
        .unwrap_or(1)
        .max(1);

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("{:<col_model$}  {:>9}  {:>9}  {:>6}", "model", "cost", "tokens", "calls"),
        Style::default().fg(palette.dim),
    )));

    for m in models {
        let id        = truncate(&m.model_id, col_model);
        let total_tok = m.tokens_in + m.tokens_out;
        let bar       = build_bar(total_tok, max_tokens, bar_w);
        lines.push(Line::from(vec![
            Span::styled(format!("{:<col_model$}", id),                  Style::default().fg(palette.fg)),
            Span::styled(format!("  {:>9}", fmt_cost(m.total_cost)),     Style::default().fg(palette.fg)),
            Span::styled(format!("  {:>9}", fmt_tokens_i64(total_tok)),  Style::default().fg(palette.dim)),
            Span::styled(format!("  {:>6}", m.call_count),               Style::default().fg(palette.dim)),
            Span::styled(format!("  {bar}"),                             Style::default().fg(palette.accent)),
        ]));
    }

    if models.is_empty() {
        lines.push(Line::from(Span::styled("no usage recorded yet", Style::default().fg(palette.dim))));
    }

    let visible: Vec<Line<'static>> = lines.into_iter().take(area.height as usize).collect();
    frame.render_widget(Paragraph::new(visible), area);
}

fn draw_session_hourly(frame: &mut Frame, hourly: &[crate::model::usage::SpendBucket], palette: &Palette, area: Rect, width_hint: usize) {
    let w = (area.width as usize).max(width_hint.min(area.width as usize));
    if area.height == 0 || w < 4 {
        return;
    }

    let lines = build_session_hourly_heatmap(hourly, palette, w);
    let h = area.height as usize;
    let skip = lines.len().saturating_sub(h);
    let visible: Vec<Line<'static>> = lines.into_iter().skip(skip).collect();
    frame.render_widget(Paragraph::new(visible), area);
}
