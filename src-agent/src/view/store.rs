//! View — in-app `/store` marketplace browser (`Mode::ExtStore`).
//!
//! A network-backed sibling of `view::extensions`: Browse is a loading/error/list of
//! fetched catalogue rows; Detail shows one extension's plain-text description +
//! contribution counts + requires/versions (an ALREADY-INSTALLED extension offers
//! nothing else here — hint "/extension to manage"); InstallConfirm is a `y`/`n` prompt,
//! or a "connect koma.run" notice when no bearer is on file. Minimalist border
//! convention (project rule): a `Borders::BOTTOM` header rule + a full-width inverse
//! footer, no full boxes.
//!
//! ```text
//!  store
//! ─────────────────────────────────────────────────────────
//!  › koma Gateway        paid   0.3.1   Premium koma models, one endpoint.   koma
//!    Workflow            free   0.1.1   PRD→research→TRD→CRD pipeline       koma
//!
//!  ↑/↓ pick · Enter detail · Esc close
//! ```

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::mode::{ExtStoreState, StoreSubMode};
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

/// Render the `/store` marketplace browser for `st`.
pub fn draw(frame: &mut Frame, st: &ExtStoreState, palette: &Palette) {
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
        StoreSubMode::Browse => "store".to_string(),
        StoreSubMode::Detail | StoreSubMode::InstallConfirm => match st.current() {
            Some(r) => format!("store / {}", r.name),
            None => "store".to_string(),
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
        StoreSubMode::Browse => browse_lines(st, palette, body.width as usize),
        StoreSubMode::Detail => detail_lines(st, palette, body.width as usize),
        StoreSubMode::InstallConfirm => install_confirm_lines(st, palette),
    };
    frame.render_widget(Paragraph::new(lines), body);

    // --- Footer: full-width inverse status bar (matches /extension). ---
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

/// Browse: loading spinner line / error + retry hint / empty notice / one row per
/// fetched catalogue item.
fn browse_lines<'a>(st: &'a ExtStoreState, palette: &Palette, width: usize) -> Vec<Line<'a>> {
    if st.loading {
        return vec![Line::from(Span::styled(
            "loading catalogue…",
            Style::default().fg(palette.dim),
        ))];
    }
    if let Some(err) = &st.error {
        return vec![
            Line::from(Span::styled(
                err.clone(),
                Style::default().fg(palette.error),
            )),
            Line::from(Span::styled(
                "press r to retry",
                Style::default().fg(palette.dim),
            )),
        ];
    }
    if st.rows.is_empty() {
        return vec![Line::from(Span::styled(
            "(no extensions found)",
            Style::default().fg(palette.dim),
        ))];
    }
    // Reserve room on the right for tier/version/author metadata.
    let name_w = width.saturating_sub(44).clamp(8, 32);
    let tagline_w = width.saturating_sub(name_w + 34).clamp(8, 60);
    st.rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let selected = i == st.list_sel;
            let name = truncate(&row.name, name_w);
            let (marker, name_style) = if selected {
                ("› ", Style::default().fg(palette.sel_fg).bg(palette.sel_bg))
            } else {
                ("  ", Style::default().fg(palette.fg))
            };
            let mut spans = vec![
                Span::styled(marker, Style::default().fg(palette.accent)),
                Span::styled(format!("{name:<name_w$}"), name_style),
                Span::styled(
                    format!("  {:<5} {:<8} ", row.tier, row.latest_version),
                    Style::default().fg(palette.dim),
                ),
                Span::styled(
                    format!("{:<tagline_w$}", truncate(&row.tagline, tagline_w)),
                    Style::default().fg(palette.fg),
                ),
                Span::styled(
                    format!("  {}", row.author),
                    Style::default().fg(palette.dim),
                ),
            ];
            if row.installed {
                spans.push(Span::styled(
                    "  INSTALLED",
                    Style::default().fg(palette.success),
                ));
            }
            Line::from(spans)
        })
        .collect()
}

/// Detail: loading spinner line / error / the fetched description + contribution
/// counts + requires/versions, with an INSTALLED marker + hint when applicable.
fn detail_lines<'a>(st: &'a ExtStoreState, palette: &Palette, width: usize) -> Vec<Line<'a>> {
    let Some(row) = st.current() else {
        return vec![Line::from(Span::styled(
            "no extension selected",
            Style::default().fg(palette.dim),
        ))];
    };
    if st.detail_loading {
        return vec![Line::from(Span::styled(
            "loading detail…",
            Style::default().fg(palette.dim),
        ))];
    }
    if let Some(err) = &st.detail_error {
        return vec![Line::from(Span::styled(
            err.clone(),
            Style::default().fg(palette.error),
        ))];
    }
    let Some(detail) = &st.detail else {
        return vec![Line::from(Span::styled(
            "(no detail)",
            Style::default().fg(palette.dim),
        ))];
    };

    let label_w = 14usize;
    let value_w = width.saturating_sub(label_w).max(8);
    let mut lines: Vec<Line> = Vec::new();

    let kv = |label: &str, value: String| -> Line<'static> {
        Line::from(vec![
            Span::styled(
                format!("{label:<label_w$}"),
                Style::default().fg(palette.dim),
            ),
            Span::styled(value, Style::default().fg(palette.fg)),
        ])
    };

    lines.push(kv("id", row.id.clone()));
    lines.push(kv("tier", row.tier.clone()));
    lines.push(kv("kind", row.kind.clone()));
    lines.push(kv("author", row.author.clone()));
    if row.installed {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:<label_w$}", "status"),
                Style::default().fg(palette.dim),
            ),
            Span::styled("INSTALLED", Style::default().fg(palette.success)),
            Span::styled("  — /extension to manage", Style::default().fg(palette.dim)),
        ]));
    }
    lines.push(kv(
        "contributes",
        format!(
            "{} models · {} panels · {} tools · {} sub-agents",
            detail.contributes_models,
            detail.contributes_panels,
            detail.contributes_tools,
            detail.contributes_sub_agents
        ),
    ));
    if detail.requires.is_empty() {
        lines.push(kv("requires", "(none)".to_string()));
    } else {
        lines.push(kv("requires", detail.requires.join(", ")));
    }
    if !detail.versions.is_empty() {
        lines.push(kv("versions", detail.versions.join(", ")));
    }
    if let Some(err) = &st.install_error {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            err.clone(),
            Style::default().fg(palette.error),
        )));
    }

    lines.push(Line::from(""));
    // `detail.description` carries real paragraph breaks (blank lines after each
    // stripped markdown heading — see `strip_markdown_headers`), so split on them
    // FIRST and wrap each resulting line independently. Feeding the whole multi-line
    // string straight into `wrap_chars` would char-chunk across the embedded `\n`s
    // (they're just ordinary chars to it), gluing a heading straight onto the
    // following prose instead of showing it on its own line.
    for para_line in detail.description.split('\n') {
        if para_line.is_empty() {
            lines.push(Line::from(""));
        } else {
            for chunk in wrap_chars(para_line, value_w.max(20)) {
                lines.push(Line::from(Span::styled(
                    chunk,
                    Style::default().fg(palette.fg),
                )));
            }
        }
    }

    lines
}

/// InstallConfirm: either the `y`/`n` install prompt (when a koma.run bearer is on
/// file), or a notice to connect one first (naming the setup path).
fn install_confirm_lines<'a>(st: &'a ExtStoreState, palette: &Palette) -> Vec<Line<'a>> {
    let name = st.current().map(|r| r.name.as_str()).unwrap_or("?");
    if !st.komarun_connected {
        return vec![
            Line::from(""),
            Line::from(Span::styled(
                "connect koma.run in /settings → OAuth first",
                Style::default().fg(palette.warn),
            )),
        ];
    }
    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("install ", Style::default().fg(palette.fg)),
            Span::styled(format!("'{name}'"), Style::default().fg(palette.accent)),
            Span::styled("?", Style::default().fg(palette.fg)),
        ]),
    ];
    if st.installing {
        lines.push(Line::from(Span::styled(
            "installing…",
            Style::default().fg(palette.dim),
        )));
    }
    if let Some(err) = &st.install_error {
        lines.push(Line::from(Span::styled(
            err.clone(),
            Style::default().fg(palette.error),
        )));
    }
    lines
}

/// Context-sensitive footer hint for the active sub-mode.
fn footer_hint(st: &ExtStoreState) -> &'static str {
    match st.sub_mode {
        StoreSubMode::InstallConfirm => {
            if st.komarun_connected {
                "y install · n/Esc cancel"
            } else {
                "Esc back"
            }
        }
        StoreSubMode::Detail => {
            if st.current().map(|r| r.installed).unwrap_or(true) {
                "Esc back"
            } else {
                "i install · Esc back"
            }
        }
        StoreSubMode::Browse => {
            if st.error.is_some() {
                "↑/↓ pick · Enter detail · r retry · Esc close"
            } else {
                "↑/↓ pick · Enter detail · Esc close"
            }
        }
    }
}
