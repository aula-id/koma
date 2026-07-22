//! View — the EXTENSION-DRIVEN TUI screen (`Mode::ExtScreen`, TUI SCREEN PROTOCOL v1).
//!
//! Renders a `Screen` model the extension supplies — `{ title?, body:[Node], footer? }` —
//! top-down: text nodes word-wrapped, `kv` nodes as aligned `k  v` rows, `divider` as a
//! horizontal rule, `menu` items indented with a `›` cursor over the union of every menu.
//! Unknown node types are skipped (forward-compat). Minimalist border convention: a
//! `Borders::BOTTOM` header rule + a full-width inverse footer key-hint bar, no full boxes.
//! While an invoke is in flight the body shows a one-line `loading…`; an invoke error shows a
//! one-line error (Esc still works).

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::mode::ExtScreenState;
use crate::view::theme::Palette;

/// Word-wrap `s` to `width` columns, breaking on whitespace and hard-splitting any word
/// longer than `width` (char-boundary safe). Empty → one empty line.
fn word_wrap(s: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut line_len = 0usize;
    for word in s.split_whitespace() {
        let wlen = word.chars().count();
        if wlen > width {
            // Flush the current line, then hard-split the long word into width-sized chunks.
            if !line.is_empty() {
                out.push(std::mem::take(&mut line));
                line_len = 0;
            }
            let chars: Vec<char> = word.chars().collect();
            for chunk in chars.chunks(width) {
                out.push(chunk.iter().collect());
            }
            continue;
        }
        let extra = if line.is_empty() { wlen } else { wlen + 1 };
        if line_len + extra > width {
            out.push(std::mem::take(&mut line));
            line_len = 0;
        }
        if line.is_empty() {
            line.push_str(word);
            line_len = wlen;
        } else {
            line.push(' ');
            line.push_str(word);
            line_len += wlen + 1;
        }
    }
    if out.is_empty() || !line.is_empty() {
        out.push(line);
    }
    out
}

/// Render one open extension screen for `st`.
pub fn draw(frame: &mut Frame, st: &ExtScreenState, palette: &Palette) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // header text + BOTTOM border
            Constraint::Min(0),    // screen body
            Constraint::Length(1), // footer key hints
        ])
        .split(frame.area());

    // --- Header: the screen's own title, or the declared fallback. ---
    let title = st
        .screen
        .as_ref()
        .and_then(|s| s.get("title"))
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| st.screen_title.clone());
    let header_block = Block::new()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(palette.dim));
    let header_inner = header_block.inner(outer[0]);
    frame.render_widget(header_block, outer[0]);
    frame.render_widget(
        Paragraph::new(Span::styled(title, Style::default().fg(palette.accent))),
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
    let width = body.width as usize;
    let lines = body_lines(st, palette, width);
    frame.render_widget(Paragraph::new(lines), body);

    // --- Footer: full-width inverse key-hint bar (matches /extension). ---
    let footer_rect = outer[2];
    if footer_rect.width > 0 {
        let hint = if st.menu_entries().is_empty() {
            "Esc back"
        } else {
            "↑/↓ move · Enter select · Esc back"
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

/// Build the body lines: the rendered `Screen` nodes, plus the screen footer + any
/// loading/error status.
fn body_lines<'a>(st: &'a ExtScreenState, palette: &Palette, width: usize) -> Vec<Line<'a>> {
    let mut lines: Vec<Line> = Vec::new();

    // Render the current screen's body nodes (if a screen has landed).
    if let Some(screen) = &st.screen {
        if let Some(body) = screen.get("body").and_then(|b| b.as_array()) {
            // Running index over the UNION of menu items, matched against `menu_cursor`.
            let mut menu_idx = 0usize;
            for node in body {
                match node.get("t").and_then(|t| t.as_str()) {
                    Some("text") => {
                        let text = node
                            .get("text")
                            .and_then(|x| x.as_str())
                            .unwrap_or_default();
                        for chunk in word_wrap(text, width) {
                            lines.push(Line::from(Span::styled(
                                chunk,
                                Style::default().fg(palette.fg),
                            )));
                        }
                    }
                    Some("kv") => {
                        let k = node.get("k").and_then(|x| x.as_str()).unwrap_or_default();
                        let v = node.get("v").and_then(|x| x.as_str()).unwrap_or_default();
                        let label_w = 16usize;
                        lines.push(Line::from(vec![
                            Span::styled(
                                format!("{k:<label_w$}"),
                                Style::default().fg(palette.dim),
                            ),
                            Span::styled(v.to_string(), Style::default().fg(palette.fg)),
                        ]));
                    }
                    Some("divider") => {
                        lines.push(Line::from(Span::styled(
                            "─".repeat(width.max(1)),
                            Style::default().fg(palette.dim),
                        )));
                    }
                    Some("menu") => {
                        if let Some(items) = node.get("items").and_then(|i| i.as_array()) {
                            for item in items {
                                let id =
                                    item.get("id").and_then(|x| x.as_str()).unwrap_or_default();
                                if id.is_empty() {
                                    // Skipped in navigation too (see `screen_menu_entries`).
                                    continue;
                                }
                                let label =
                                    item.get("label").and_then(|x| x.as_str()).unwrap_or(id);
                                let selected = menu_idx == st.menu_cursor;
                                let marker = if selected { "› " } else { "  " };
                                let label_style = if selected {
                                    Style::default().fg(palette.sel_fg).bg(palette.sel_bg)
                                } else {
                                    Style::default().fg(palette.fg)
                                };
                                lines.push(Line::from(vec![
                                    Span::styled(marker, Style::default().fg(palette.accent)),
                                    Span::styled(label.to_string(), label_style),
                                ]));
                                menu_idx += 1;
                            }
                        }
                    }
                    // Unknown node types are skipped (forward-compat).
                    _ => {}
                }
            }
        }

        // The screen's own footer text (distinct from the key-hint bar), rendered dim.
        if let Some(footer) = screen.get("footer").and_then(|f| f.as_str()) {
            if !footer.is_empty() {
                lines.push(Line::from(""));
                for chunk in word_wrap(footer, width) {
                    lines.push(Line::from(Span::styled(
                        chunk,
                        Style::default().fg(palette.dim),
                    )));
                }
            }
        }
    }

    // One-line loading state while an invoke is in flight.
    if st.waiting {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            "loading…",
            Style::default().fg(palette.dim),
        )));
    }

    // One-line error (Esc still works). Shown even with a screen present.
    if let Some(err) = &st.error {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            err.clone(),
            Style::default().fg(palette.error),
        )));
    }

    // Nothing at all yet (no screen, not waiting, no error) — a benign placeholder.
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "(no screen)",
            Style::default().fg(palette.dim),
        )));
    }

    lines
}
