//! View – `--resume` session picker (SessionPicker mode).
//!
//! Displays the list of saved sessions filtered in real time as the user
//! types.  Layout (top to bottom):
//!
//! 1. Top+bottom rule search bar — title ` search ` on the TOP rule, cursor appended.
//! 2. Flat (borderless) session list — columns aligned, scrollable.
//!    The selected row is highlighted with `palette.sel_fg` on `palette.sel_bg`.
//!    The list scrolls to keep the selection visible.
//! 3. One-line keybinding hint with session count.
//!
//! Filtering and selection state live in [`app::mode::PickerState`].
//! Keystroke handling lives in [`controller::input::handle_picker`].

use std::time::SystemTime;
use ratatui::{
    layout::{Constraint, Direction, Layout, Margin},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph},
    Frame,
};
use crate::app::mode::PickerState;
use crate::view::theme::Palette;

/// Format a `SystemTime` as a human-readable relative age string.
///
/// Returns strings like `"5s ago"`, `"3m ago"`, `"2h ago"`, `"4d ago"`.
/// Falls back to `"?"` if the system clock is behind `modified` (unlikely
/// but possible with clock skew or file-system metadata from the future).
fn fmt_modified(modified: SystemTime) -> String {
    match SystemTime::now().duration_since(modified) {
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
        // SystemTime::duration_since returns Err when `modified` > `now`.
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

/// Render the session picker for `picker` using the given colour `palette`.
pub fn draw(frame: &mut Frame, picker: &PickerState, palette: &Palette) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // search: top+bottom rules
            Constraint::Min(1),    // flat session list
            Constraint::Length(1), // keybinding hint line
        ])
        .split(frame.area());

    // --- Search bar ---
    // Top+bottom rules only — title " search " sits on the TOP rule, dim style.
    let search_block = Block::new()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(Style::default().fg(palette.dim))
        .title(Span::styled(" search ", Style::default().fg(palette.dim)))
        .padding(Padding::horizontal(1));

    let search_inner = search_block.inner(chunks[0]);

    let search_line = Line::from(vec![
        Span::styled(picker.query.as_str(), Style::default().fg(palette.fg)),
        Span::styled("█", Style::default().fg(palette.accent)),
    ]);
    frame.render_widget(search_block, chunks[0]);
    frame.render_widget(Paragraph::new(search_line), search_inner);

    // --- Session list (flat, no borders) ---
    // Render rows directly into the inset area (1 char horizontal margin).
    let inner = chunks[1].inner(Margin { horizontal: 1, vertical: 0 });

    // Build one styled Line per filtered entry with aligned columns.
    //
    // Each row: `{name:<name_w$}  {count:>3} msgs   {age:>8}`
    //
    // `name_w` is derived from the inner width so the right columns always
    // land at the same horizontal offset regardless of name length.
    let inner_w = inner.width as usize;
    let mut lines: Vec<Line> = Vec::new();

    for (i, &j) in picker.filtered_idx.iter().enumerate() {
        let meta = &picker.all[j];
        // Monochrome lock marker (NO emoji — house rule). A locked session is one
        // open in a live instance (including ours); it reads as "[locked]" and
        // can't be entered. Kept fixed-width so the right column stays aligned
        // across locked and unlocked rows.
        let lock = if meta.locked { "[locked]" } else { "        " };
        let right = format!(
            "{:>3} msgs   {:>8}  {lock}",
            meta.message_count,
            fmt_modified(meta.modified)
        );
        // Width available for the name: total inner width minus right column
        // minus two separator spaces, clamped to at least 4 chars.
        let name_w = inner_w
            .saturating_sub(right.chars().count() + 2)
            .max(4);
        let name = truncate(&meta.name, name_w);
        let row = format!("{name:<name_w$}  {right}");

        // Locked rows are dimmed so they visibly read as non-selectable; the
        // selection highlight still applies to selectable (unlocked) rows.
        let style = if meta.locked {
            Style::default().fg(palette.dim)
        } else if i == picker.selected {
            Style::default().fg(palette.sel_fg).bg(palette.sel_bg)
        } else {
            Style::default().fg(palette.fg)
        };
        lines.push(Line::styled(row, style));
    }

    // Scroll so the selected row stays visible within the inner height.
    let list_height = inner.height as usize;
    let scroll = picker
        .selected
        .saturating_sub(list_height.saturating_sub(1)) as u16;

    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), inner);

    // --- Keybinding hint (flat, with session count prepended) ---
    let hint = format!(
        "{} sessions · ↑↓ select · type to filter · Enter open · /new spawn · Esc/Ctrl+C quit",
        picker.filtered_idx.len()
    );
    let instructions = Paragraph::new(hint).style(Style::default().fg(palette.dim));
    frame.render_widget(instructions, chunks[2]);
}
