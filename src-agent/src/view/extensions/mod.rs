//! View — in-app `/extension` installed-extension manager (`Mode::Extensions`).
//!
//! A read-only top-down dashboard (no side-by-side detail pane, unlike `/mcp`): Browse is a
//! full-width list of installed extensions; Detail shows one extension's full info +
//! selectable TUI-screen rows; UninstallConfirm is a `y`/`n` prompt naming what the nuke
//! deletes. Minimalist border convention (project rule): a `Borders::BOTTOM` header rule + a
//! full-width inverse footer, no full boxes.
//!
//! ```text
//!  extensions
//! ─────────────────────────────────────────────────────────
//!  › workflow            0.1.1   daemon  free   ● running
//!    echo-tool           0.0.1   daemon  free   ○ stopped  (disabled)
//!
//!  ↑/↓ pick · →/Enter detail · Esc close
//! ```

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::mode::{ExtSubMode, ExtensionsState};
use crate::view::theme::Palette;

/// Truncate `s` to at most `max` chars, appending `…` if cut.
fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        let cut = max.saturating_sub(1);
        chars[..cut].iter().collect::<String>() + "…"
    }
}

/// Split `s` into chunks of at most `width` chars (char-boundary safe). Empty → one empty.
fn wrap_chars(s: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    if s.is_empty() {
        return vec![String::new()];
    }
    let chars: Vec<char> = s.chars().collect();
    chars.chunks(width).map(|c| c.iter().collect()).collect()
}

/// Render the `/extension` dashboard for `st`.
pub fn draw(frame: &mut Frame, st: &ExtensionsState, palette: &Palette) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // header text + BOTTOM border
            Constraint::Min(0),    // body
            Constraint::Length(1), // footer key hints
        ])
        .split(frame.area());

    // --- Header ---
    let header_block = Block::new()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(palette.dim));
    let header_inner = header_block.inner(outer[0]);
    frame.render_widget(header_block, outer[0]);
    let header_text = match st.sub_mode {
        ExtSubMode::Browse => "extensions".to_string(),
        ExtSubMode::Detail | ExtSubMode::UninstallConfirm => match st.current() {
            Some(r) => format!("extensions / {}", r.name),
            None => "extensions".to_string(),
        },
    };
    frame.render_widget(
        Paragraph::new(Span::styled(header_text, Style::default().fg(palette.dim))),
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
    let lines = match st.sub_mode {
        ExtSubMode::Browse => browse_lines(st, palette, body.width as usize),
        ExtSubMode::Detail => detail_lines(st, palette, body.width as usize),
        ExtSubMode::UninstallConfirm => uninstall_lines(st, palette),
    };
    frame.render_widget(Paragraph::new(lines), body);

    // --- Footer: full-width inverse status bar (matches /mcp). ---
    let footer_rect = outer[2];
    if footer_rect.width > 0 {
        let hint = footer_hint(st);
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

/// The running dot span: `● running` (success) when live, `○ stopped` (dim) otherwise.
fn running_span(running: bool, palette: &Palette) -> Span<'static> {
    if running {
        Span::styled("● running", Style::default().fg(palette.success))
    } else {
        Span::styled("○ stopped", Style::default().fg(palette.dim))
    }
}

/// Browse: one row per installed extension.
fn browse_lines<'a>(st: &'a ExtensionsState, palette: &Palette, width: usize) -> Vec<Line<'a>> {
    if st.rows.is_empty() {
        return vec![Line::from(Span::styled(
            "(no extensions installed)",
            Style::default().fg(palette.dim),
        ))];
    }
    // Reserve room on the right for version/kind/tier/status metadata.
    let name_w = width.saturating_sub(40).clamp(8, 40);
    st.rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let selected = i == st.list_sel;
            let name = truncate(&row.name, name_w);
            let (marker, name_style): (&str, Style) = if selected {
                ("› ", Style::default().fg(palette.sel_fg).bg(palette.sel_bg))
            } else {
                ("  ", Style::default().fg(palette.fg))
            };
            let mut spans = vec![
                Span::styled(marker, Style::default().fg(palette.accent)),
                Span::styled(format!("{name:<name_w$}"), name_style),
                Span::styled(
                    format!("  {:<8} {:<8} {:<5} ", row.version, row.kind, row.tier),
                    Style::default().fg(palette.dim),
                ),
                running_span(row.running, palette),
            ];
            if !row.enabled {
                spans.push(Span::styled(
                    "  (disabled)",
                    Style::default().fg(palette.warn),
                ));
            }
            Line::from(spans)
        })
        .collect()
}

/// Detail: the selected extension's full info + selectable TUI-screen rows.
fn detail_lines<'a>(st: &'a ExtensionsState, palette: &Palette, width: usize) -> Vec<Line<'a>> {
    let Some(row) = st.current() else {
        return vec![Line::from(Span::styled(
            "no extension selected",
            Style::default().fg(palette.dim),
        ))];
    };
    let label_w = 14usize;
    let value_w = width.saturating_sub(label_w).max(8);
    let mut lines: Vec<Line> = Vec::new();

    let kv = |label: &str, value: String, color: Color| -> Line<'static> {
        Line::from(vec![
            Span::styled(format!("{label:<label_w$}"), Style::default().fg(palette.dim)),
            Span::styled(value, Style::default().fg(color)),
        ])
    };
    // Push a possibly-long value, wrapped with a hanging indent under the label.
    let push_wrapped = |lines: &mut Vec<Line>, label: &str, value: &str, color: Color| {
        let chunks = wrap_chars(value, value_w);
        for (i, chunk) in chunks.into_iter().enumerate() {
            if i == 0 {
                lines.push(Line::from(vec![
                    Span::styled(format!("{label:<label_w$}"), Style::default().fg(palette.dim)),
                    Span::styled(chunk, Style::default().fg(color)),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::styled(" ".repeat(label_w), Style::default().fg(palette.dim)),
                    Span::styled(chunk, Style::default().fg(color)),
                ]));
            }
        }
    };

    push_wrapped(&mut lines, "id", &row.id, palette.fg);
    if !row.description.is_empty() {
        push_wrapped(&mut lines, "description", &row.description, palette.fg);
    }
    lines.push(kv("version", row.version.clone(), palette.fg));
    lines.push(kv("tier", row.tier.clone(), palette.fg));
    lines.push(kv("kind", row.kind.clone(), palette.fg));
    lines.push(kv(
        "enabled",
        if row.enabled { "yes".into() } else { "no".into() },
        if row.enabled { palette.fg } else { palette.dim },
    ));
    lines.push(Line::from(vec![
        Span::styled(format!("{:<label_w$}", "running"), Style::default().fg(palette.dim)),
        running_span(row.running, palette),
    ]));
    lines.push(kv(
        "contributes",
        format!(
            "{} tools · {} panels · {} sub-agents · {} models",
            row.tools, row.panels, row.sub_agents, row.models
        ),
        palette.fg,
    ));
    if row.granted.is_empty() {
        lines.push(kv("granted", "(none)".into(), palette.dim));
    } else {
        push_wrapped(&mut lines, "granted", &row.granted.join(", "), palette.fg);
    }
    match row.workspace_dir.as_deref() {
        Some(ws) => push_wrapped(&mut lines, "workspace", ws, palette.fg),
        None => lines.push(kv("workspace", "(none)".into(), palette.dim)),
    }

    // TUI screens: selectable rows (only when the extension declares any).
    if !row.tui_screens.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "screens",
            Style::default().fg(palette.dim),
        )));
        for (i, ts) in row.tui_screens.iter().enumerate() {
            let selected = i == st.screen_sel;
            let marker = if selected { "› " } else { "  " };
            let title_style = if selected {
                Style::default().fg(palette.sel_fg).bg(palette.sel_bg)
            } else {
                Style::default().fg(palette.fg)
            };
            lines.push(Line::from(vec![
                Span::styled(marker, Style::default().fg(palette.accent)),
                Span::styled(ts.title.clone(), title_style),
                Span::styled(format!("  ({})", ts.id), Style::default().fg(palette.dim)),
            ]));
        }
    }

    if let Some(err) = &st.error {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            err.clone(),
            Style::default().fg(palette.error),
        )));
    }

    lines
}

/// UninstallConfirm: a `y`/`n` prompt that names what the nuke deletes.
fn uninstall_lines<'a>(st: &'a ExtensionsState, palette: &Palette) -> Vec<Line<'a>> {
    let name = st.current().map(|r| r.name.as_str()).unwrap_or("?");
    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("uninstall ", Style::default().fg(palette.fg)),
            Span::styled(format!("'{name}'"), Style::default().fg(palette.accent)),
            Span::styled("?", Style::default().fg(palette.fg)),
        ]),
        Line::from(Span::styled(
            "removes the package from disk, deregisters its tools/models, and drops its config entry",
            Style::default().fg(palette.dim),
        )),
    ];
    if let Some(ws) = st.current().and_then(|r| r.workspace_dir.as_deref()) {
        lines.push(Line::from(Span::styled(
            format!("also deletes its data directory: {ws}"),
            Style::default().fg(palette.warn),
        )));
    }
    lines
}

/// Context-sensitive footer hint for the active sub-mode.
fn footer_hint(st: &ExtensionsState) -> &'static str {
    match st.sub_mode {
        ExtSubMode::UninstallConfirm => "y uninstall · n/Esc cancel",
        ExtSubMode::Detail => {
            if st.current_tui_screens_len() > 0 {
                "↑/↓ screen · Enter open · u uninstall · Esc back"
            } else {
                "u uninstall · Esc back"
            }
        }
        ExtSubMode::Browse => "↑/↓ pick · →/Enter detail · Esc close",
    }
}
