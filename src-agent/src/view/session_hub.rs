//! View – unified session hub (`/resume`, `Mode::SessionHub`).
//!
//! Splits the screen into TWO horizontal halves, each independently scrollable,
//! matching the flat/boxless border convention of the other pickers:
//!
//! - TOP half — `cooking (N)` header, then the LIVE sessions: each row a
//!   `● working` / `○ ready` marker + name, with the foreground tagged
//!   `(current)`.
//! - BOTTOM half — `history (N)` header, then the on-disk sessions: each row a
//!   name + a relative last-active time. Shows `no past sessions` when empty.
//!
//! The FOCUSED pane's header rule is accented and the selected row in it uses
//! `palette.sel_*`; the unfocused pane is dim. A one-line keybinding hint sits at
//! the bottom.
//!
//! Selection/scroll state lives in [`crate::app::mode::SessionHub`]; keystroke
//! handling lives in [`crate::controller::input::handle_session_hub`].

use std::time::SystemTime;
use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph},
    Frame,
};
use crate::app::mode::{HubPane, SessionHub, SessionKind};
use crate::view::theme::Palette;

/// Format a `SystemTime` as a human-readable relative age string.
///
/// Returns strings like `"5s ago"`, `"3m ago"`, `"2h ago"`, `"4d ago"`. Falls
/// back to `"?"` if the system clock is behind `t` (clock skew / future mtime).
fn fmt_age(t: SystemTime) -> String {
    match SystemTime::now().duration_since(t) {
        Ok(dur) => {
            let secs = dur.as_secs();
            if secs < 60 {
                format!("{secs}s ago")
            } else if secs < 3600 {
                format!("{}m ago", secs / 60)
            } else if secs < 86400 {
                format!("{}h ago", secs / 3600)
            } else {
                format!("{}d ago", secs / 86400)
            }
        }
        Err(_) => "?".to_string(),
    }
}

/// Truncate `s` to at most `max` Unicode scalar values, appending `…` if cut.
fn truncate(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else if max == 0 {
        String::new()
    } else {
        // Reserve one char for the ellipsis.
        let mut out: String = chars[..max.saturating_sub(1)].iter().collect();
        out.push('…');
        out
    }
}

/// Render the session hub for `hub` using the given colour `palette`.
pub fn draw(frame: &mut Frame, hub: &SessionHub, palette: &Palette) {
    // Split: cooking half | history half | one-line hint. The two halves share the
    // remaining height evenly (each is independently scrollable inside its slot).
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(50), // cooking half
            Constraint::Min(3),         // history half (takes the rest)
            Constraint::Length(1),      // keybinding hint line
        ])
        .split(frame.area());

    draw_cooking(frame, chunks[0], hub, palette);
    draw_history(frame, chunks[1], hub, palette);

    // --- Keybinding hint ---
    let hint = "Tab switch pane · ↑↓ select · Enter open · N new · Esc close";
    let instructions = Paragraph::new(hint).style(Style::default().fg(palette.dim));
    frame.render_widget(instructions, chunks[2]);
}

/// Render the TOP "cooking" pane (the live sessions) into `area`.
fn draw_cooking(frame: &mut Frame, area: Rect, hub: &SessionHub, palette: &Palette) {
    let focused = hub.focus == HubPane::Cooking;

    let real_sessions = hub.cooking.iter().filter(|e| e.kind == SessionKind::Session).count();
    let inner = pane_inner(frame, area, &format!("cooking ({real_sessions})"), focused, palette);
    let inner_w = inner.width as usize;

    let mut lines: Vec<Line> = Vec::new();
    for (i, entry) in hub.cooking.iter().enumerate() {
        let style = if focused && i == hub.cooking_selected {
            Style::default().fg(palette.sel_fg).bg(palette.sel_bg)
        } else if entry.kind == SessionKind::NewSession {
            Style::default().fg(palette.accent)
        } else if focused {
            Style::default().fg(palette.fg)
        } else {
            Style::default().fg(palette.dim)
        };

        let row = if entry.kind == SessionKind::NewSession {
            entry.name.clone()
        } else {
            let state_marker = if entry.working { "● working" } else { "○ ready  " };
            let current = if entry.is_foreground { "  (current)" } else { "" };
            let right = format!("{state_marker}{current}");
            let name_w = inner_w.saturating_sub(right.chars().count() + 2).max(4);
            let name = truncate(&entry.name, name_w);
            format!("{name:<name_w$}  {right}")
        };

        lines.push(Line::styled(row, style));
    }

    // Scroll so the selected row stays visible within this pane's height (only the
    // focused pane scrolls to its cursor; the unfocused pane sits at the top).
    let list_height = inner.height as usize;
    let scroll = if focused {
        hub.cooking_selected
            .saturating_sub(list_height.saturating_sub(1)) as u16
    } else {
        0
    };
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), inner);
}

/// Render the BOTTOM "history" pane (the on-disk sessions) into `area`.
fn draw_history(frame: &mut Frame, area: Rect, hub: &SessionHub, palette: &Palette) {
    let focused = hub.focus == HubPane::History;

    let inner = pane_inner(frame, area, &format!("history ({})", hub.history.len()), focused, palette);
    let inner_w = inner.width as usize;

    let mut lines: Vec<Line> = Vec::new();
    if hub.history.is_empty() {
        // Empty history is normal (cooking always has >= 1 live session).
        lines.push(Line::styled(
            "no past sessions",
            Style::default().fg(palette.dim),
        ));
    } else {
        for (i, entry) in hub.history.iter().enumerate() {
            let age = fmt_age(entry.last_active);
            let name_w = inner_w.saturating_sub(age.chars().count() + 2).max(4);
            let name = truncate(&entry.name, name_w);
            let row = format!("{name:<name_w$}  {age:>}");

            let style = if focused && i == hub.history_selected {
                Style::default().fg(palette.sel_fg).bg(palette.sel_bg)
            } else if focused {
                Style::default().fg(palette.fg)
            } else {
                Style::default().fg(palette.dim)
            };
            lines.push(Line::styled(row, style));
        }
    }

    let list_height = inner.height as usize;
    let scroll = if focused {
        hub.history_selected
            .saturating_sub(list_height.saturating_sub(1)) as u16
    } else {
        0
    };
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), inner);
}

/// Draw a pane's header rule (`title` on the TOP rule) into `area` and return the
/// inset content area below it. The focused pane's rule is accented; an unfocused
/// pane's rule is dim. Mirrors the single-rule header used by the other pickers.
fn pane_inner(frame: &mut Frame, area: Rect, title: &str, focused: bool, palette: &Palette) -> Rect {
    let rule_style = if focused {
        Style::default().fg(palette.accent)
    } else {
        Style::default().fg(palette.dim)
    };
    let header = Block::new()
        .borders(Borders::TOP)
        .border_style(rule_style)
        .title(Span::styled(format!(" {title} "), rule_style))
        .padding(Padding::horizontal(1));
    let inner = header.inner(area);
    frame.render_widget(header, area);
    // One char horizontal margin so rows align with the picker style.
    inner.inner(Margin { horizontal: 1, vertical: 0 })
}
