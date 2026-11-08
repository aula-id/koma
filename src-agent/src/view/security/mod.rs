//! `/security` control panel view.
//!
//! A full-screen status panel in the house border style:
//! - Header: label "security" with BOTTOM border only.
//! - Body: Dashboard | Images | Refused (cycled by `h`, jumped by 1/2/3).
//! - Dashboard is Paragraph text: gates, YOLO, guest map, catalog window.
//! - Footer: full-width inverse hint bar (per active pane).

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::mode::security::{is_real_composition, map_column_ids, map_domains};
use crate::app::mode::{SecPane, SecurityState};
use crate::view::theme::Palette;

const CATALOG_WINDOW: usize = 24;

/// Render the `/security` control panel for `st` using the given colour `palette`.
pub fn draw(frame: &mut Frame, st: &SecurityState, palette: &Palette) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // header text + BOTTOM border
            Constraint::Min(0),    // status + inventory
            Constraint::Length(1), // footer key hints
        ])
        .split(frame.area());

    let header_block = Block::new()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(palette.dim));
    let header_inner = header_block.inner(outer[0]);
    frame.render_widget(header_block, outer[0]);
    frame.render_widget(
        Paragraph::new(Span::styled("security", Style::default().fg(palette.dim))),
        header_inner.inner(Margin {
            horizontal: 2,
            vertical: 0,
        }),
    );

    let body = outer[1].inner(Margin {
        horizontal: 2,
        vertical: 1,
    });

    let mut lines: Vec<Line> = Vec::new();
    push_tabs(&mut lines, st, palette);
    lines.push(Line::from(""));

    match st.pane {
        SecPane::Tools => render_dashboard(&mut lines, st, palette),
        SecPane::Images => render_images(&mut lines, st, palette),
        SecPane::Refused => render_refused(&mut lines, st, palette),
    }

    frame.render_widget(Paragraph::new(lines), body);

    let footer_rect = outer[2];
    if footer_rect.width > 0 {
        let hint = match st.pane {
            SecPane::Tools => {
                "↑↓ tools · ←→ map · f filter · / search · s start/stop · c runtime · u refresh · o arm · e yolo · l lane · h/123 · Esc"
            }
            SecPane::Images => {
                "↑↓ · p pull · enter/a activate · x remove · r refresh · h/123 panes · Esc"
            }
            SecPane::Refused => "↑↓ · h/123 panes · Esc",
        };
        let bar_style = Style::default()
            .fg(palette.sel_fg)
            .bg(palette.sel_bg)
            .add_modifier(Modifier::BOLD);
        let padded = format!(
            " {:<width$}",
            hint,
            width = footer_rect.width.saturating_sub(1) as usize
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::raw(padded))).style(bar_style),
            footer_rect,
        );
    }
}

fn push_tabs(lines: &mut Vec<Line>, st: &SecurityState, palette: &Palette) {
    let tabs = [
        (SecPane::Tools, "Dashboard (1)"),
        (SecPane::Images, "Images (2)"),
        (SecPane::Refused, "Refused (3)"),
    ];
    let mut spans = Vec::new();
    for (i, (pane, label)) in tabs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        let on = st.pane == *pane;
        let text = if on {
            format!("▸ {label}")
        } else {
            format!("  {label}")
        };
        spans.push(Span::styled(
            text,
            if on {
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.dim)
            },
        ));
    }
    lines.push(Line::from(spans));
}

fn gate_span(label: &str, up: bool, palette: &Palette) -> Vec<Span<'static>> {
    let (mark, style) = if up {
        (
            "●",
            Style::default()
                .fg(palette.success)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        ("○", Style::default().fg(palette.warn))
    };
    vec![
        Span::styled(format!("{label} "), Style::default().fg(palette.dim)),
        Span::styled(mark.to_string(), style),
        Span::raw("  "),
    ]
}

fn render_dashboard(lines: &mut Vec<Line>, st: &SecurityState, palette: &Palette) {
    let dim = Style::default().fg(palette.dim);
    let guest_up = st.images.iter().any(|r| r.active);
    let mut gates = Vec::new();
    gates.extend(gate_span("runtime", st.status.runtime_ok, palette));
    gates.extend(gate_span("daemon", st.status.running, palette));
    gates.extend(gate_span("guest", guest_up, palette));
    gates.extend(gate_span("boundary", st.status.boundary_up, palette));
    gates.push(Span::styled(
        format!("lane {} (l)", st.lane_label),
        dim,
    ));
    lines.push(Line::from(gates));

    let start_stop = if st.status.running {
        "Stop (s)"
    } else {
        "Start (s)"
    };
    lines.push(Line::from(vec![
        Span::styled(start_stop, Style::default().fg(palette.accent)),
        Span::styled("   Runtime (c)   Refresh (u)", dim),
    ]));
    lines.push(Line::from(""));

    let armed = st.yolo_armed;
    let in_yolo = st.agent_mode_label == "yolo";
    let enter_label = if in_yolo { "Leave (e)" } else { "Enter (e)" };
    let arm_label = if armed { "Disarm (o)" } else { "Arm (o)" };
    let enter_style = if st.status.running && armed {
        Style::default().fg(palette.accent)
    } else {
        dim
    };
    lines.push(Line::from(vec![
        Span::styled(
            format!(
                "YOLO {} · mode {}",
                if armed { "armed" } else { "locked" },
                st.agent_mode_label
            ),
            if armed {
                Style::default().fg(palette.warn)
            } else {
                dim
            },
        ),
        Span::raw("   "),
        Span::styled(arm_label, Style::default().fg(palette.accent)),
        Span::raw("   "),
        Span::styled(enter_label, enter_style),
    ]));
    lines.push(Line::from(""));

    render_guest_map(lines, st, palette);
    lines.push(Line::from(""));

    let filtered = st.filtered_catalog();
    let shown = filtered.len();
    let ready = filtered
        .iter()
        .filter(|idx| st.catalog.get(**idx).map(|r| r.ready_mark == "ready").unwrap_or(false))
        .count();
    let active = st
        .images
        .iter()
        .find(|r| r.active)
        .map(|r| r.id.as_str())
        .unwrap_or("none");
    let mut counts = format!("{shown} shown · {ready} ready on {active}");
    if st.catalog_filter != crate::app::mode::CatalogFilter::All {
        counts.push_str(match st.catalog_filter {
            crate::app::mode::CatalogFilter::ReadyHere => " · ready here",
            crate::app::mode::CatalogFilter::NeedsImage => " · needs image",
            crate::app::mode::CatalogFilter::All => "",
        });
    }
    if st.catalog_searching || !st.catalog_query.is_empty() {
        counts.push_str(&format!(" · /{}", st.catalog_query));
    }
    lines.push(Line::from(Span::styled(counts, dim)));

    if let Some(row) = st.selected_catalog() {
        lines.push(render_selected_strip(row, palette));
    }
    lines.push(Line::from(""));

    if filtered.is_empty() {
        lines.push(Line::from(Span::styled(
            if st.catalog.is_empty() {
                "no catalog rows"
            } else {
                "no tools match this filter"
            },
            dim,
        )));
        return;
    }

    let start = window_start(st.selected, shown, CATALOG_WINDOW);
    let end = (start + CATALOG_WINDOW).min(shown);
    for pos in start..end {
        let Some(row) = st.catalog.get(filtered[pos]) else {
            continue;
        };
        let on = pos == st.selected;
        let mark = mark_glyph(&row.ready_mark);
        let label = format!(
            "{prefix}{mark} {:<18} {:<12} {}",
            clip(&row.id, 18),
            clip(&row.image, 12),
            clip(&row.blurb, 42),
            prefix = if on { ">" } else { " " },
        );
        let style = if on {
            Style::default()
                .fg(palette.sel_fg)
                .bg(palette.sel_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.fg)
        };
        lines.push(Line::from(Span::styled(label, style)));
    }
}

fn render_selected_strip(row: &crate::app::mode::CatalogRow, palette: &Palette) -> Line<'static> {
    let dim = Style::default().fg(palette.dim);
    let (glyph, reason, hint) = match row.ready_mark.as_str() {
        "ready" => ("●", "image active", ""),
        "local" => ("▒", "pulled, not active", "  Activate (a)"),
        _ => ("·", "not pulled", "  Pull (p)"),
    };
    let hint = if is_real_composition(&row.image) {
        hint
    } else {
        ""
    };
    let glyph_style = match row.ready_mark.as_str() {
        "ready" => Style::default().fg(palette.success),
        "local" => Style::default().fg(palette.accent),
        _ => dim,
    };
    Line::from(vec![
        Span::styled(glyph.to_string(), glyph_style),
        Span::styled(
            format!(" {}  image {}  exec {}  {}", row.id, row.image, row.exec, reason),
            Style::default().fg(palette.fg),
        ),
        Span::styled(hint.to_string(), Style::default().fg(palette.accent)),
    ])
}

fn render_guest_map(lines: &mut Vec<Line>, st: &SecurityState, palette: &Palette) {
    let dim = Style::default().fg(palette.dim);
    let sel = Style::default()
        .fg(palette.sel_fg)
        .bg(palette.sel_bg)
        .add_modifier(Modifier::BOLD);
    let header = if st.map_filter.is_some() {
        format!("filter {}", st.map_filter.as_deref().unwrap_or(""))
    } else {
        "map all".into()
    };
    let header_style = if st.map_filter.is_none() { sel } else { dim };
    lines.push(Line::from(vec![
        Span::styled(header, header_style),
        Span::styled("    base  -2  ext  src", dim),
    ]));

    for domain in map_domains() {
        let cols = map_column_ids(domain, &st.images);
        if cols.is_empty() {
            continue;
        }
        let mut spans = vec![Span::styled(format!("{domain:<10}"), dim)];
        for kind in ["", "-2", "-ext", "-src"] {
            let id = if kind.is_empty() {
                (*domain).to_string()
            } else {
                format!("{domain}{kind}")
            };
            if !cols.iter().any(|c| c == &id) {
                spans.push(Span::raw("    "));
                continue;
            }
            let row = st.images.iter().find(|r| r.id == id);
            let glyph = match row {
                Some(r) if r.active => "██",
                Some(r) if r.local => "▒▒",
                Some(_) => "··",
                None => "  ",
            };
            let on = st.map_filter.as_deref() == Some(id.as_str());
            spans.push(Span::styled(
                format!("{glyph}  "),
                if on { sel } else { Style::default().fg(palette.fg) },
            ));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(Span::styled(
        "· remote  ▒ local  █ active",
        dim,
    )));
}

fn mark_glyph(mark: &str) -> &'static str {
    match mark {
        "ready" => "●",
        "local" => "▒",
        _ => "·",
    }
}

fn window_start(selected: usize, len: usize, window: usize) -> usize {
    if len <= window {
        return 0;
    }
    let half = window / 2;
    selected.saturating_sub(half).min(len - window)
}

fn clip(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count <= n {
        return s.to_string();
    }
    let mut t: String = s.chars().take(n.saturating_sub(1)).collect();
    t.push('…');
    t
}

fn render_images(lines: &mut Vec<Line>, st: &SecurityState, palette: &Palette) {
    let dim_style = Style::default().fg(palette.dim);
    let sel_style = Style::default()
        .fg(palette.sel_fg)
        .bg(palette.sel_bg)
        .add_modifier(Modifier::BOLD);

    lines.push(Line::from(Span::styled(
        "guest compositions — pull domain images; activate pins image-ref",
        dim_style,
    )));
    if st.images_busy {
        lines.push(Line::from(Span::styled(
            "working…",
            Style::default().fg(palette.accent),
        )));
    }
    lines.push(Line::from(""));

    if st.images.is_empty() {
        lines.push(Line::from(Span::styled(
            "no compositions catalogued",
            dim_style,
        )));
        return;
    }

    for (pos, row) in st.images.iter().enumerate() {
        let is_selected = pos == st.images_selected;
        let marker = if row.active { "*" } else { " " };
        let where_label = if row.active {
            "active"
        } else if row.local {
            "local "
        } else {
            "remote"
        };
        let name_style = if is_selected {
            sel_style
        } else if row.active {
            Style::default()
                .fg(palette.success)
                .add_modifier(Modifier::BOLD)
        } else if row.local {
            Style::default().fg(palette.accent)
        } else {
            Style::default().fg(palette.fg)
        };
        let where_style = if row.active {
            Style::default()
                .fg(palette.success)
                .add_modifier(Modifier::BOLD)
        } else if row.local {
            Style::default().fg(palette.accent)
        } else {
            dim_style
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{marker} "), name_style),
            Span::styled(format!("{:<14}", row.id), name_style),
            Span::raw(" "),
            Span::styled(format!("{:>5}", row.size_label), dim_style),
            Span::raw("  "),
            Span::styled(where_label, where_style),
            Span::raw("  "),
            Span::styled(row.blurb.clone(), dim_style),
        ]));
    }
}

fn render_refused(lines: &mut Vec<Line>, st: &SecurityState, palette: &Palette) {
    let dim_style = Style::default().fg(palette.dim);
    let sel_style = Style::default()
        .fg(palette.sel_fg)
        .bg(palette.sel_bg)
        .add_modifier(Modifier::BOLD);

    lines.push(Line::from(Span::styled(
        "handshake refusals — tools the parent rejected at connect",
        dim_style,
    )));
    lines.push(Line::from(""));

    if st.status.refused.is_empty() {
        lines.push(Line::from(Span::styled(
            if st.status.running {
                "none — every descriptor passed"
            } else {
                "no data (daemon stopped)"
            },
            dim_style,
        )));
        return;
    }

    for (pos, entry) in st.status.refused.iter().enumerate() {
        let is_selected = pos == st.refused_selected;
        let name_style = if is_selected {
            sel_style
        } else {
            Style::default()
                .fg(palette.error)
                .add_modifier(Modifier::BOLD)
        };
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{:<20}", entry.name), name_style),
        ]));
        lines.push(Line::from(Span::styled(
            format!("      {}", entry.reason),
            dim_style,
        )));
    }
}
