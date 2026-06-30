//! View — `/security` daemon control panel.
//!
//! A full-screen status panel in the house border style:
//! - Header: label "security" with BOTTOM border only.
//! - Control block: a navigable `[x] Daemon running` checkbox (Space starts/stops the
//!   daemon), a compact dim `installed: yes · N tools` info line, then the YOLO checkbox
//!   (gated — locked until the daemon is running).
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
//!  [x] Daemon running
//!  installed: yes · 12 tools
//!
//!  [ ] Enable YOLO mode
//!  YOLO mode disabled — harness active
//!
//!  [web]
//!    sec_http        [light]   risky
//!    sec_scrape      [heavy]
//!
//!  [network]
//!    sec_portscan    [heavy]   risky
//!
//!  ↑↓ move · Space toggle · d domain · h deps · r restart · Esc
//! ```

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::mode::{SecSel, SecurityState};
use crate::app::sec::InstallHealthEntry;
use crate::view::theme::Palette;

/// LOUD red used for the armed/enabled YOLO state (checkbox + warning). Bright enough to
/// read as a real danger marker against the panel, matching the house "no box, loud text"
/// convention for warnings.
const YOLO_RED: Color = Color::Rgb(255, 60, 60);
/// GREEN used for the ARMED/active YOLO checkbox indicator — distinct from the red warning line.
const YOLO_GREEN: Color = Color::Rgb(0, 200, 83);

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

    // The daemon running-state is now the navigable `SecSel::Daemon` checkbox at the top of
    // the tools-pane item loop (rendered below), not a standalone status line. The compact
    // `installed: N tools` info line is emitted right under that checkbox, inside the loop.
    let active = st.status.running && st.status.installed;
    let dim_style = Style::default().fg(palette.dim);

    if !st.health_view {
        // ── TOOLS PANE (default): the YOLO checkbox, then tool inventory by domain. ──
        //
        // Walk `tool_items()` — the SAME ordered list the cursor navigates — and highlight
        // purely on `pos == st.selected`. There is no separate flat-index path any more, so
        // the highlight can never land on a different on-screen row than the one below the
        // cursor (the bug this rewrite fixes). `pos` is the index into `tool_items()`.
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
        // Track the last-rendered tool's domain so we emit a `[domain]` header (and the
        // blank-line spacing) only when the domain changes between consecutive tool rows.
        let mut last_domain: Option<&str> = None;
        for (pos, item) in items.iter().enumerate() {
            let is_selected = pos == st.selected;
            match item {
                SecSel::Daemon => {
                    // Daemon start/stop checkbox — checked = running. Toggling it (Space)
                    // starts the daemon when stopped, stops it when running. Selection
                    // highlight is NOT gated on daemon state: it must stay navigable while
                    // stopped so the user can start it from here.
                    let (box_label, base_style) = if st.status.running {
                        // Running indicator reuses the active-green (same green as armed YOLO).
                        ("[x] Daemon running", Style::default().fg(YOLO_GREEN).add_modifier(Modifier::BOLD))
                    } else {
                        ("[ ] Daemon stopped", Style::default().fg(palette.fg))
                    };
                    let row_style = if is_selected { sel_style } else { base_style };
                    lines.push(Line::from(Span::styled(box_label, row_style)));

                    // Compact dim info line under the checkbox: installed flag + tool count,
                    // with ONE inline trailing segment so the loading spinner / missing-dep
                    // legend reads as part of this line rather than a separate checkbox-like
                    // row. (Replaces the old `daemon: … · mode: …` status block; the `mode`
                    // fragment is gone — the checkbox above is the running state.)
                    let installed_label = if st.status.installed { "yes" } else { "no" };
                    let mut info_spans = vec![Span::styled(
                        format!("installed: {} · {} tools", installed_label, st.status.tools.len()),
                        dim_style,
                    )];
                    if st.health_fetching {
                        // Health probe in flight — animated braille spinner from the frame
                        // counter `service_global` advances each tick.
                        const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
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
                        // At least one tool has a missing dep — inline [!!] legend (no
                        // separate row, so it stops looking like another checkbox).
                        info_spans.push(Span::styled(" · ", dim_style));
                        info_spans.push(Span::styled(
                            "[!!]",
                            Style::default()
                                .fg(Color::Rgb(255, 200, 0))
                                .add_modifier(Modifier::BOLD),
                        ));
                        info_spans.push(Span::styled(" dependency not installed", dim_style));
                    }
                    lines.push(Line::from(info_spans));

                    lines.push(Line::from(""));
                }
                SecSel::Yolo => {
                    // YOLO arm checkbox — GATED on the daemon running, because YOLO bypasses
                    // the harness and that only means anything with a live daemon.
                    if !st.status.running {
                        // LOCKED: render dim with a "(start daemon first)" hint and no
                        // armed/enabled state (it can't be armed). Still navigable so the
                        // cursor can rest here, but toggling it is refused in the handler.
                        let row_style = if is_selected { sel_style } else { dim_style };
                        lines.push(Line::from(vec![
                            Span::styled("[ ] Enable YOLO mode", row_style),
                            Span::styled("   (start daemon first)", dim_style),
                        ]));
                    } else {
                        // Daemon is running: original behaviour. Checked = armed. Selection
                        // highlight is NOT gated on `active`.
                        let (box_label, base_style) = if st.yolo_armed {
                            ("[x] Enable YOLO mode", Style::default().fg(YOLO_GREEN).add_modifier(Modifier::BOLD))
                        } else {
                            ("[ ] Enable YOLO mode", Style::default().fg(palette.fg))
                        };
                        let row_style = if is_selected { sel_style } else { base_style };
                        lines.push(Line::from(Span::styled(box_label, row_style)));

                        // Non-navigable warning directly below the checkbox. LOUD red when
                        // enabled (harness bypassable), dim reassurance when disabled.
                        if st.yolo_armed {
                            lines.push(Line::from(Span::styled(
                                "! YOLO MODE ENABLED",
                                Style::default().fg(YOLO_RED).add_modifier(Modifier::BOLD),
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
                SecSel::Tool(i) => {
                    let t = &st.status.tools[*i];

                    // Domain header on every domain change (including the first tool).
                    if last_domain != Some(t.domain.as_str()) {
                        // Blank-line spacing before a NEW group, but not before the very
                        // first group (the YOLO block already emitted a trailing blank).
                        if last_domain.is_some() {
                            lines.push(Line::from(""));
                        }
                        let domain_label = if t.domain.is_empty() { "other" } else { t.domain.as_str() };
                        lines.push(Line::from(Span::styled(
                            format!("[{domain_label}]"),
                            dim_style,
                        )));
                        last_domain = Some(t.domain.as_str());
                    }

                    // A tool the user disabled in this panel renders dim with an "  off"
                    // suffix; active tools render as before. The selection highlight still
                    // applies on top so the cursor stays visible over a disabled row.
                    let is_inactive = st.inactive.contains(&t.name);
                    let name_style = if is_selected && active {
                        sel_style
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

                    // Not-installed marker — yellow bold [!!] when ANY dependency backing
                    // this tool is absent. Visible even on dimmed/inactive rows.
                    let missing_dep = missing_dep_for(t.name.as_str(), &st.install_health);
                    let mark_span = if missing_dep {
                        Span::styled(
                            "  [!!]",
                            Style::default()
                                .fg(Color::Rgb(255, 200, 0))
                                .add_modifier(Modifier::BOLD),
                        )
                    } else {
                        Span::raw("")
                    };

                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(format!("{:<20}", t.name), name_style),
                        compute_span,
                        risk_span,
                        off_span,
                        mark_span,
                    ]));
                }
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
            "↑↓ move · i install · h tools · r restart · Esc"
        } else {
            "↑↓ move · Space toggle · d domain · h deps · r restart · Esc"
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

/// Returns `true` if ANY dependency entry whose `tools` list contains `tool_name` is absent
/// (`present == false`). Used by both the per-row [!!] marker and the header legend.
fn missing_dep_for(tool_name: &str, health: &[InstallHealthEntry]) -> bool {
    health.iter().any(|d| d.tools.iter().any(|t| t == tool_name) && !d.present)
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
/// `missing` marker. The selected row is highlighted (sel_fg/sel_bg bold).
///
/// Like the tools pane, this walks `health_items()` — the SAME tier-grouped order the
/// cursor navigates — and highlights on `pos == st.health_selected`, where `pos` is the
/// index into that list. Tier headers are emitted on tier change.
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
    // Track the last-rendered entry's tier so we emit a tier header (and the blank-line
    // spacing) only when the tier changes between consecutive rows.
    let mut last_tier: Option<u8> = None;
    for (pos, idx) in items.iter().enumerate() {
        let e = &st.install_health[*idx];

        // Tier header on every tier change (including the first entry).
        if last_tier != Some(e.tier) {
            // Blank-line spacing before a NEW tier group, but not before the first.
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
    // Trailing blank to match the prior layout's per-group spacing tail.
    lines.push(Line::from(""));
}

