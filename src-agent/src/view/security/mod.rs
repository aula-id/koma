//! View — `/security` daemon control panel.
//!
//! A full-screen status panel in the house border style:
//! - Header: label "security" with BOTTOM border only.
//! - Status block: daemon running / installed / mode (enabled/disabled) flags.
//! - Body (toggled by `h`): one of TWO panes —
//!   - TOOLS (default): tools grouped by domain, each row `name  [compute]  risk?`,
//!     greyed when the daemon is not running or not installed.
//!   - DEPENDENCIES: install-health grouped by tier (`── tier N (label) ──`), each row
//!     `name  ok|missing  method` with optional dim `needs:`/`enables:` hints; `i`
//!     installs the selected dependency.
//! - Footer: full-width inverse hint bar (per active pane).
//!
//! No sidebar — this is a single-pane status view with no sub-modes.
//!
//! ```text
//!  security
//! ─────────────────────────────────────────────────────────
//!  daemon: running  ·  installed: yes  ·  mode: enabled
//!
//!  [web]
//!    sec_http        [light]   risky
//!    sec_scrape      [heavy]
//!
//!  [network]
//!    sec_portscan    [heavy]   risky
//!
//!  t toggle · s start · x stop · r restart · ↑/↓ · Esc close
//! ```

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::mode::SecurityState;
use crate::view::theme::Palette;

/// Render the `/security` control panel for `st` using the given colour `palette`.
pub fn draw(frame: &mut Frame, st: &SecurityState, palette: &Palette) {
    // Outer vertical zones: header | body | footer.
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // header text + BOTTOM border
            Constraint::Min(0),    // status + tool inventory
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
        Paragraph::new(Span::styled(
            "security",
            Style::default().fg(palette.dim),
        )),
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

    // Status line: daemon running/stopped, installed yes/no, mode enabled/disabled.
    let running_label = if st.status.running { "running" } else { "stopped" };
    let installed_label = if st.status.installed { "yes" } else { "no" };
    let mode_label = if st.status.running { "enabled" } else { "disabled" };

    let running_style = if st.status.running {
        Style::default().fg(palette.accent)
    } else {
        Style::default().fg(palette.dim)
    };
    let installed_style = if st.status.installed {
        Style::default().fg(palette.fg)
    } else {
        Style::default().fg(palette.dim)
    };

    lines.push(Line::from(vec![
        Span::styled("daemon: ", Style::default().fg(palette.dim)),
        Span::styled(running_label, running_style),
        Span::styled("  ·  installed: ", Style::default().fg(palette.dim)),
        Span::styled(installed_label, installed_style),
        Span::styled("  ·  mode: ", Style::default().fg(palette.dim)),
        Span::styled(mode_label, Style::default().fg(palette.dim)),
    ]));
    lines.push(Line::from(""));

    let active = st.status.running && st.status.installed;
    let dim_style = Style::default().fg(palette.dim);

    if !st.health_view {
        // ── TOOLS PANE (default) — tool inventory, greyed when daemon not running. ──
        let tool_style = if active {
            Style::default().fg(palette.fg)
        } else {
            Style::default().fg(palette.dim)
        };

        if st.status.tools.is_empty() {
            lines.push(Line::from(Span::styled(
                "no tools — daemon stopped",
                dim_style,
            )));
        } else {
            // Group tools by domain, preserving insertion order.
            let mut domains: Vec<String> = Vec::new();
            for t in &st.status.tools {
                if !domains.contains(&t.domain) {
                    domains.push(t.domain.clone());
                }
            }

            for domain in &domains {
                // Domain header.
                let domain_label = if domain.is_empty() { "other" } else { domain.as_str() };
                lines.push(Line::from(Span::styled(
                    format!("[{domain_label}]"),
                    dim_style,
                )));

                // Tools in this domain.
                for (idx, t) in st.status.tools.iter().enumerate() {
                    if &t.domain != domain {
                        continue;
                    }

                    let is_selected = idx == st.selected;
                    // A tool the user disabled in this panel renders dim with an "  off"
                    // suffix; active tools render as before. The selection highlight still
                    // applies on top so the cursor stays visible over a disabled row.
                    let is_inactive = st.inactive.contains(&t.name);
                    let name_style = if is_selected && active {
                        Style::default()
                            .fg(palette.sel_fg)
                            .bg(palette.sel_bg)
                            .add_modifier(Modifier::BOLD)
                    } else if is_inactive {
                        dim_style
                    } else {
                        tool_style
                    };

                    let compute_span = if t.compute.is_empty() {
                        Span::raw("")
                    } else {
                        Span::styled(
                            format!("  [{}]", t.compute),
                            dim_style,
                        )
                    };

                    let risk_span = if t.risk {
                        Span::styled("  risky", Style::default().fg(palette.dim).add_modifier(Modifier::ITALIC))
                    } else {
                        Span::raw("")
                    };

                    // Inactive marker — minimalist dim suffix, no box (house style).
                    let off_span = if is_inactive {
                        Span::styled("  off", dim_style)
                    } else {
                        Span::raw("")
                    };

                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(format!("{:<20}", t.name), name_style),
                        compute_span,
                        risk_span,
                        off_span,
                    ]));
                }

                lines.push(Line::from(""));
            }
        }
    } else {
        // ── DEPENDENCIES PANE — install-health grouped by tier, house top-down style. ──
        render_deps(&mut lines, st, palette);
    }

    frame.render_widget(
        Paragraph::new(lines),
        body,
    );

    // --- Footer: full-width inverse hint bar (per active pane). ---
    let footer_rect = outer[2];
    if footer_rect.width > 0 {
        let hint = if st.health_view {
            "i install · h tools · ↑/↓ · Esc close"
        } else {
            "Enter toggle · d domain · t toggle · s start · x stop · r restart · ↑/↓ · h deps · Esc close"
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

/// Fixed parenthetical label for a tier header (the install *strategy* for that tier),
/// matching the daemon's tiering. Unknown tiers fall back to a generic label so the
/// pane never panics on an unexpected value.
fn tier_label(tier: u8) -> &'static str {
    match tier {
        1 => "pip",
        2 => "auto-download",
        3 => "manual",
        _ => "other",
    }
}

/// Render the dependency install-health pane into `lines`, grouped by tier in the house
/// minimalist top-down style (`── tier N (label) ──` rules, NO full boxes).
///
/// Each row: `name` (left-padded) · a present/missing marker · the install `method`,
/// with optional dim secondary lines (`needs:` hint, `enables:` backing tools). Present
/// rows render in the accent colour with an `ok` marker; missing rows render dim with a
/// `missing` marker. The row at `health_selected` is highlighted (sel_fg/sel_bg bold).
fn render_deps(lines: &mut Vec<Line>, st: &SecurityState, palette: &Palette) {
    let dim_style = Style::default().fg(palette.dim);

    if st.install_health.is_empty() {
        lines.push(Line::from(Span::styled(
            "no health data (daemon stopped)",
            dim_style,
        )));
        return;
    }

    // Distinct tiers in ascending order (preserves any unexpected tier values too).
    let mut tiers: Vec<u8> = Vec::new();
    for e in &st.install_health {
        if !tiers.contains(&e.tier) {
            tiers.push(e.tier);
        }
    }
    tiers.sort_unstable();

    for tier in &tiers {
        // Tier header — a top-down rule, house style (no box).
        lines.push(Line::from(Span::styled(
            format!("── tier {} ({}) ──", tier, tier_label(*tier)),
            dim_style,
        )));

        for (idx, e) in st.install_health.iter().enumerate() {
            if e.tier != *tier {
                continue;
            }

            let is_selected = idx == st.health_selected;

            // Name style: selection wins; otherwise present → accent, missing → dim.
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

            // Present/missing marker — coloured to match the row's state.
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

            // Secondary, dim hint lines (optional). `needs:` only when there is a hint;
            // `enables:` only when this dep backs at least one tool.
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
}
