//! Security daemon control panel view.
//!
//! A full-screen status panel in the house border style:
//! - Header: label "security" with BOTTOM border only.
//! - Control block: navigable `[x] Daemon running` checkbox, status strip
//!   (installed / runtime / lane / refused), YOLO checkbox.
//! - Body (cycled by `h`): TOOLS | DEPENDENCIES | REFUSED panes.
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

    // Catalog summary — tools load dynamically via sec_catalog / sec_run.
    lines.push(Line::from(Span::styled("catalog (dynamic)", dim_style)));
    if st.status.tools.is_empty() {
        lines.push(Line::from(Span::styled(
            if st.status.running {
                "  (no toolkit rows yet)"
            } else {
                "  start daemon to load smoke pack inventory"
            },
            dim_style,
        )));
    } else {
        use std::collections::BTreeMap;
        let mut by_domain: BTreeMap<&str, usize> = BTreeMap::new();
        for tool in &st.status.tools {
            let d = if tool.domain.is_empty() {
                "other"
            } else {
                tool.domain.as_str()
            };
            *by_domain.entry(d).or_insert(0) += 1;
        }
        let parts: Vec<String> = by_domain
            .into_iter()
            .map(|(d, n)| format!("{d}:{n}"))
            .collect();
        lines.push(Line::from(Span::styled(
            format!(
                "  {} tools · {} · agent uses sec_catalog → sec_run",
                st.status.tools.len(),
                parts.join(" · ")
            ),
            if active { tool_style } else { dim_style },
        )));
        lines.push(Line::from(Span::styled(
            "  boundary: sec_runtime status|up|down|net|rules · files: sec_fs · plan: catalog only",
            dim_style,
        )));
        lines.push(Line::from(Span::styled(
            "  working set: pins survive compact · rediscover via sec_catalog",
            dim_style,
        )));
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
