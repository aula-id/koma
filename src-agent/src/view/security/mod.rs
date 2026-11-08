//! Security daemon control panel view.
//!
//! A full-screen status panel in the house border style:
//! - Header: label "security" with BOTTOM border only.
//! - Control block: navigable `[x] Daemon running` checkbox, status strip
//!   (installed / runtime / lane / refused), YOLO checkbox.
//! - Body (cycled by `h`): TOOLS | IMAGES | DEPENDENCIES | REFUSED panes.
//! - Footer: full-width inverse hint bar (per active pane).

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::mode::{SecPane, SecSel, SecurityState};
use crate::app::sec::InstallHealthEntry;
use crate::view::theme::Palette;

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

    // --- Header ---
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

    // --- Body ---
    let body = outer[1].inner(Margin {
        horizontal: 2,
        vertical: 1,
    });

    let mut lines: Vec<Line> = Vec::new();

    let active = st.status.running && st.status.installed;
    let dim_style = Style::default().fg(palette.dim);

    match st.pane {
        SecPane::Tools => {
            render_tools(&mut lines, st, palette, active, dim_style);
        }
        SecPane::Images => {
            render_images(&mut lines, st, palette);
        }
        SecPane::Deps => {
            render_deps(&mut lines, st, palette);
        }
        SecPane::Refused => {
            render_refused(&mut lines, st, palette);
        }
    }

    frame.render_widget(Paragraph::new(lines), body);

    // --- Footer ---
    let footer_rect = outer[2];
    if footer_rect.width > 0 {
        let hint = match st.pane {
            SecPane::Tools => {
                "↑↓ · Space toggle · d domain · a all · n none · u refresh · c runtime · l lane · h panes · r restart · Esc"
            }
            SecPane::Images => "↑↓ · p pull · enter activate · x remove · r refresh · h panes · Esc",
            SecPane::Deps => "↑↓ · i install · h panes · r restart · Esc",
            SecPane::Refused => "↑↓ · u refresh toolkit · h panes · r restart · Esc",
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

fn render_tools(
    lines: &mut Vec<Line>,
    st: &SecurityState,
    palette: &Palette,
    active: bool,
    dim_style: Style,
) {
    let tool_style = if active {
        Style::default().fg(palette.fg)
    } else {
        Style::default().fg(palette.dim)
    };
    let sel_style = Style::default()
        .fg(palette.sel_fg)
        .bg(palette.sel_bg)
        .add_modifier(Modifier::BOLD);

    let items = st.tool_items();
    for (pos, item) in items.iter().enumerate() {
        let is_selected = pos == st.selected;
        match item {
            SecSel::Daemon => {
                let (box_label, base_style) = if st.status.running {
                    (
                        "[x] Daemon running",
                        Style::default()
                            .fg(palette.success)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    ("[ ] Daemon stopped", Style::default().fg(palette.fg))
                };
                let row_style = if is_selected { sel_style } else { base_style };
                lines.push(Line::from(Span::styled(box_label, row_style)));

                // Status strip: installed · tools · runtime · lane · refused
                let installed_label = if st.status.installed { "yes" } else { "no" };
                let runtime_style = if st.status.runtime_ok {
                    Style::default().fg(palette.success)
                } else {
                    Style::default()
                        .fg(palette.warn)
                        .add_modifier(Modifier::BOLD)
                };
                let mut info_spans = vec![
                    Span::styled(
                        format!(
                            "installed: {} · {} tools",
                            installed_label,
                            st.status.tools.len()
                        ),
                        dim_style,
                    ),
                    Span::styled(" · runtime: ", dim_style),
                    Span::styled(
                        if st.status.runtime.is_empty() {
                            "unknown".into()
                        } else {
                            st.status.runtime.clone()
                        },
                        runtime_style,
                    ),
                    Span::styled(
                        format!(" · lane: {}", st.lane_label),
                        dim_style,
                    ),
                ];
                if !st.status.refused.is_empty() {
                    info_spans.push(Span::styled(" · ", dim_style));
                    info_spans.push(Span::styled(
                        format!("{} refused", st.status.refused.len()),
                        Style::default()
                            .fg(palette.error)
                            .add_modifier(Modifier::BOLD),
                    ));
                    info_spans.push(Span::styled(" (h → refused)", dim_style));
                }
                if st.health_fetching {
                    const SPINNER: [&str; 10] =
                        ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
                    let spinner = SPINNER[(st.health_frame % 10) as usize];
                    info_spans.push(Span::styled(" · ", dim_style));
                    info_spans.push(Span::styled(
                        format!("{spinner} checking dependencies…"),
                        Style::default().fg(palette.accent),
                    ));
                } else if st
                    .status
                    .tools
                    .iter()
                    .any(|tool| missing_dep_for(tool.name.as_str(), &st.install_health))
                {
                    info_spans.push(Span::styled(" · ", dim_style));
                    info_spans.push(Span::styled(
                        "[!!]",
                        Style::default()
                            .fg(palette.warn)
                            .add_modifier(Modifier::BOLD),
                    ));
                    info_spans.push(Span::styled(" dependency not installed", dim_style));
                }
                if !st.status.installed {
                    info_spans.push(Span::styled(
                        " · run: koma --security-install",
                        Style::default().fg(palette.warn),
                    ));
                }
                lines.push(Line::from(info_spans));
                lines.push(Line::from(""));
            }
            SecSel::Yolo => {
                if !st.status.running {
                    let row_style = if is_selected { sel_style } else { dim_style };
                    lines.push(Line::from(vec![
                        Span::styled("[ ] Enable YOLO mode", row_style),
                        Span::styled("   (start daemon first)", dim_style),
                    ]));
                } else {
                    let (box_label, base_style) = if st.yolo_armed {
                        (
                            "[x] Enable YOLO mode",
                            Style::default()
                                .fg(palette.success)
                                .add_modifier(Modifier::BOLD),
                        )
                    } else {
                        ("[ ] Enable YOLO mode", Style::default().fg(palette.fg))
                    };
                    let row_style = if is_selected { sel_style } else { base_style };
                    lines.push(Line::from(Span::styled(box_label, row_style)));
                    if st.yolo_armed {
                        lines.push(Line::from(Span::styled(
                            "! YOLO MODE ENABLED",
                            Style::default()
                                .fg(palette.error)
                                .add_modifier(Modifier::BOLD),
                        )));
                    } else {
                        lines.push(Line::from(Span::styled(
                            "YOLO mode disabled — harness active",
                            dim_style,
                        )));
                    }
                }
                lines.push(Line::from(""));
            }
            SecSel::Tool(_) => {
                // Not navigable on the dashboard (kept for IPC/compat).
            }
        }
    }

    // Catalog + boundary summary — live fields from enrich_status on panel refresh.
    lines.push(Line::from(Span::styled("catalog + boundary", dim_style)));
    if st.status.running {
        let cat = if st.status.catalog_total > 0 {
            format!(
                "  catalog ready {}/{} · handshake {}",
                st.status.catalog_ready, st.status.catalog_total, st.status.tools.len()
            )
        } else {
            format!("  handshake {} tools · agent: sec_catalog → sec_run", st.status.tools.len())
        };
        lines.push(Line::from(Span::styled(
            cat,
            if active { tool_style } else { dim_style },
        )));
        let net = if st.status.net.is_empty() {
            if st.status.boundary_up { "unknown" } else { "down" }
        } else {
            st.status.net.as_str()
        };
        let rules_n = st.status.rules.len();
        let rules_preview = if rules_n == 0 {
            "deny-all".to_string()
        } else if rules_n <= 3 {
            st.status.rules.join(", ")
        } else {
            format!("{}, … (+{})", st.status.rules[..2].join(", "), rules_n - 2)
        };
        lines.push(Line::from(Span::styled(
            format!(
                "  boundary {} · net={} · rules[{}]: {}",
                if st.status.boundary_up { "up" } else { "down" },
                net,
                rules_n,
                rules_preview
            ),
            if active { tool_style } else { dim_style },
        )));
        lines.push(Line::from(Span::styled(
            "  working set pins survive compact · plan: catalog only",
            dim_style,
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  start daemon to load catalog + boundary status",
            dim_style,
        )));
    }
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
        lines.push(Line::from(Span::styled("no compositions catalogued", dim_style)));
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

/// Returns `true` if ANY dependency entry whose `tools` list contains `tool_name` is absent.
fn missing_dep_for(tool_name: &str, health: &[InstallHealthEntry]) -> bool {
    health
        .iter()
        .any(|d| d.tools.iter().any(|t| t == tool_name) && !d.present)
}

fn tier_label(tier: u8) -> &'static str {
    match tier {
        1 => "pip",
        2 => "auto-download",
        3 => "manual",
        _ => "other",
    }
}

/// Render the dependency install-health pane into `lines`.
fn render_deps(lines: &mut Vec<Line>, st: &SecurityState, palette: &Palette) {
    let dim_style = Style::default().fg(palette.dim);

    if st.install_health.is_empty() {
        lines.push(Line::from(Span::styled(
            "no health data (daemon stopped)",
            dim_style,
        )));
        return;
    }

    let items = st.health_items();
    let mut last_tier: Option<u8> = None;
    for (pos, idx) in items.iter().enumerate() {
        let e = &st.install_health[*idx];

        if last_tier != Some(e.tier) {
            if last_tier.is_some() {
                lines.push(Line::from(""));
            }
            lines.push(Line::from(Span::styled(
                format!("── tier {} ({}) ──", e.tier, tier_label(e.tier)),
                dim_style,
            )));
            last_tier = Some(e.tier);
        }

        let is_selected = pos == st.health_selected;

        let name_style = if is_selected {
            Style::default()
                .fg(palette.sel_fg)
                .bg(palette.sel_bg)
                .add_modifier(Modifier::BOLD)
        } else if e.present {
            Style::default().fg(palette.accent)
        } else {
            dim_style
        };

        let (marker, marker_style) = if e.present {
            ("ok     ", Style::default().fg(palette.accent))
        } else {
            ("missing", dim_style)
        };

        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{:<18}", e.name), name_style),
            Span::raw("  "),
            Span::styled(marker, marker_style),
            Span::styled(format!("  {}", e.method), dim_style),
        ]));

        if !e.hint.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("      needs: {}", e.hint),
                dim_style,
            )));
        }
        if !e.tools.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("      enables: {}", e.tools.join(", ")),
                dim_style,
            )));
        }
    }
    lines.push(Line::from(""));
}
