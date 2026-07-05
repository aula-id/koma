use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
    Frame,
};
use ratatui::layout::Rect;
use crate::view::theme::Palette;

/// Render the Role checkbox picker overlay (model EDIT modal) as a bordered
/// modal over a dimmed backdrop.
///
/// Mirrors the `/agents` tool picker (`view/agents.rs::draw_tool_picker`) but
/// SIMPLER — the option set is the fixed [`ModelRole::ALL`] (5 entries), so there
/// is no "type to filter" line. Each role is a `[ ] label` / `[x] label` row; the
/// cursor row carries the inverse highlight. A footer line shows the key hints.
///
/// ```text
/// ┌─ roles ─────────────────┐
/// │ [x] main                │
/// │ [ ] awareness           │
/// │ [ ] safeguard           │
/// │ [ ] compactor           │
/// │ space toggle · enter ok…│
/// └─────────────────────────┘
/// ```
pub(super) fn draw_role_picker(
    frame: &mut Frame,
    picker: &crate::app::mode::settings::RolePickerState,
    palette: &Palette,
    area: Rect,
) {
    use crate::app::mode::settings::ModelRole;

    let n = ModelRole::ALL.len();
    // Content rows: one per role + two hint lines (split for narrow modals). Borders add 2.
    let content_h = n as u16 + 2;
    let total_h = content_h + 2;
    // Width: "[x] awareness" is short; a 28-col inner is comfortable. Clamp to frame.
    let popup_w = 30_u16.min(area.width.saturating_sub(2));
    let w = popup_w;
    let h = total_h.min(area.height.saturating_sub(2)).max(3);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect { x, y, width: w, height: h };

    // Dim everything outside the modal (fg dim + bg reset — same as the other
    // settings modals, so a stacked overlay still recedes the layer beneath it).
    {
        let buf = frame.buffer_mut();
        for cy in area.top()..area.bottom() {
            for cx in area.left()..area.right() {
                if cx >= popup.x && cx < popup.right() && cy >= popup.y && cy < popup.bottom() {
                    continue;
                }
                buf[(cx, cy)].set_fg(palette.dim).set_bg(Color::Reset);
            }
        }
    }

    let modal_block = Block::bordered()
        .border_style(Style::default().fg(palette.accent))
        .title(Span::styled(" roles ", Style::default().fg(palette.accent)));
    let inner = modal_block.inner(popup);

    frame.render_widget(Clear, popup);
    frame.render_widget(modal_block, popup);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let body_w = inner.width as usize;
    let cursor = picker.cursor.min(n.saturating_sub(1));
    let mut lines: Vec<Line> = Vec::new();
    for (i, role) in ModelRole::ALL.iter().enumerate() {
        let checked = picker.checked.get(i).copied().unwrap_or(false);
        let mark = if checked { "[x]" } else { "[ ]" };
        if i == cursor {
            // Cursor row: full-width inverse highlight.
            let text = format!("{} {}", mark, role.label());
            lines.push(Line::from(Span::styled(
                format!("{:<width$}", text, width = body_w),
                Style::default().fg(palette.sel_fg).bg(palette.sel_bg),
            )));
        } else {
            // Checkbox accent when checked, dim when not; label follows the box.
            let box_color = if checked { palette.accent } else { palette.dim };
            lines.push(Line::from(vec![
                Span::styled(mark, Style::default().fg(box_color)),
                Span::styled(
                    format!(" {}", role.label()),
                    Style::default().fg(if checked { palette.fg } else { palette.dim }),
                ),
            ]));
        }
    }

    // Footer hint: two lines so narrow modals don't truncate.
    lines.push(Line::from(Span::styled(
        "space toggle",
        Style::default().fg(palette.dim),
    )));
    lines.push(Line::from(Span::styled(
        "enter ok \u{b7} esc cancel",
        Style::default().fg(palette.dim),
    )));

    frame.render_widget(Paragraph::new(lines), inner);
}
