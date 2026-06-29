//! View — `/security` daemon control panel.
//!
//! A full-screen status panel in the house border style:
//! - Header: label "security" with BOTTOM border only.
//! - Status block: daemon running / installed / mode (enabled/disabled) flags.
//! - Tool inventory: tools grouped by domain, each row `name  [compute]  risk?`,
//!   greyed when the daemon is not running or not installed.
//! - Footer: full-width inverse hint bar.
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

    // Tool inventory — greyed when daemon is not running/installed.
    let active = st.status.running && st.status.installed;
    let tool_style = if active {
        Style::default().fg(palette.fg)
    } else {
        Style::default().fg(palette.dim)
    };
    let dim_style = Style::default().fg(palette.dim);

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

    frame.render_widget(
        Paragraph::new(lines),
        body,
    );

    // --- Footer: full-width inverse hint bar. ---
    let footer_rect = outer[2];
    if footer_rect.width > 0 {
        let hint = "Enter toggle · d domain · t toggle · s start · x stop · r restart · ↑/↓ · Esc close";
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
