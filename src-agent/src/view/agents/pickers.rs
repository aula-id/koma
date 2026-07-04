//! Overlay picker modals: tool multi-select and model single-select.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
    Frame,
};

use crate::app::mode::agents::{ModelPickerState, ToolPickerState};
use crate::app::state::AppStateRest;
use crate::view::theme::Palette;

use super::truncate;

/// Compute a centered overlay `Rect` with the given width and height,
/// clamped to the available area.
pub(super) fn centered_rect(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect { x, y, width: w, height: h }
}

/// Render the tool multi-select picker overlay as a proper bordered modal.
///
/// Visual structure (Borders::ALL box, backdrop dimmed):
///
/// ```text
/// ┌─ tools (N selected) ────────────┐
/// │ type to filter                  │
/// │ [x] read                        │
/// │ [ ] grep                        │
/// │ …                               │
/// │ space toggle · enter ok · esc   │
/// └─────────────────────────────────┘
/// ```
pub(super) fn draw_tool_picker(
    frame: &mut Frame,
    rest: &AppStateRest,
    picker: &ToolPickerState,
    palette: &Palette,
    area: Rect,
) {
    let filtered = picker.filtered_indices();
    // Content rows: filter line (1) + options (min 1, max 10) + hint (2 lines, split for narrow modals).
    let opt_rows = filtered.len().clamp(1, 10) as u16;
    let content_h = 1 + opt_rows + 2; // filter + options + hint (2 lines)
    // Total height includes top and bottom borders.
    let total_h = content_h + 2;
    // Width: content is "[x] toolname" with 1-space left pad + padding.
    // 36 inner chars + 2 borders = 38 total, clamped to frame.
    let popup_w = 38_u16.min(area.width.saturating_sub(2));
    let popup = centered_rect(area, popup_w, total_h);

    // --- Dim the backdrop (everything outside the modal rect). ---
    // We mutate the frame buffer directly: for each cell not inside the modal,
    // set its foreground to palette.dim so the background recedes.
    {
        let buf = frame.buffer_mut();
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                // Skip cells that are inside (or on the border of) the modal.
                if x >= popup.x && x < popup.right() && y >= popup.y && y < popup.bottom() {
                    continue;
                }
                buf[(x, y)].set_fg(palette.dim);
            }
        }
    }

    // --- Modal box: Clear → Block (Borders::ALL) → inner content. ---
    let n_checked = picker.checked.iter().filter(|&&c| c).count();
    let title = if n_checked > 0 {
        format!(" tools ({n_checked} selected) ")
    } else {
        " tools ".to_string()
    };
    let modal_block = Block::bordered()
        .border_style(Style::default().fg(palette.dim))
        .title(Span::styled(title, Style::default().fg(palette.accent)));
    let inner = modal_block.inner(popup);

    frame.render_widget(Clear, popup);
    frame.render_widget(modal_block, popup);

    // Bail out if the inner area is too small to render content.
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let body_w = inner.width;
    let body_x = inner.x;

    // Filter line (top row of inner area).
    let filter_text = if picker.filter.is_empty() {
        format!("{:<width$}", "type to filter", width = body_w as usize)
    } else {
        let shown = format!("{}█", picker.filter);
        format!("{:<width$}", shown, width = body_w as usize)
    };
    let filter_color = if picker.filter.is_empty() { palette.dim } else { palette.fg };
    frame.render_widget(
        Paragraph::new(Span::styled(filter_text, Style::default().fg(filter_color))),
        Rect { x: body_x, y: inner.y, width: body_w, height: 1 },
    );

    // Option rows.
    let cursor = picker.cursor.min(filtered.len().saturating_sub(1));
    let opt_area_y = inner.y + 1;
    // Scrolloff window (persisted offset on rest — ToolPickerState is rebuilt
    // per client frame). `opt_rows` is the viewport height; `lines` holds one
    // row per filtered option, so window start == the Paragraph scroll offset.
    let (scroll, _) = crate::view::scroll::scroll_window(
        &rest.agents_tool_picker_offset,
        cursor,
        filtered.len(),
        opt_rows as usize,
    );

    let mut lines: Vec<Line> = Vec::new();
    if filtered.is_empty() {
        lines.push(Line::from(Span::styled(
            "(no matches)",
            Style::default().fg(palette.dim),
        )));
    } else {
        for (fi, &oi) in filtered.iter().enumerate() {
            let mark = if picker.checked[oi] { "[x]" } else { "[ ]" };
            let label = format!("{} {}", mark, picker.options[oi]);
            if fi == cursor {
                lines.push(Line::from(Span::styled(
                    format!("{:<width$}", label, width = body_w as usize),
                    Style::default().fg(palette.sel_fg).bg(palette.sel_bg),
                )));
            } else {
                lines.push(Line::from(Span::styled(
                    label,
                    Style::default().fg(palette.accent),
                )));
            }
        }
    }

    frame.render_widget(
        Paragraph::new(lines).scroll((scroll as u16, 0)),
        Rect { x: body_x, y: opt_area_y, width: body_w, height: opt_rows },
    );

    // Hint lines (last two rows of inner area): split for narrow modals.
    let hint_y = opt_area_y + opt_rows;
    frame.render_widget(
        Paragraph::new(Span::styled("space toggle", Style::default().fg(palette.dim))),
        Rect { x: body_x, y: hint_y, width: body_w, height: 1 },
    );
    frame.render_widget(
        Paragraph::new(Span::styled("enter ok \u{b7} esc cancel", Style::default().fg(palette.dim))),
        Rect { x: body_x, y: hint_y + 1, width: body_w, height: 1 },
    );
}

/// Render the single-select MODEL picker overlay as a bordered modal.
///
/// Mirrors [`draw_tool_picker`] (dimmed backdrop + `Clear` + accent bordered
/// box + footer hint) but it is a PICK-ONE list, so there are no checkboxes and
/// no filter line: each option is a plain row, the cursor row carries the
/// inverse highlight, and a `›` accent marker flags the cursor for clarity.
/// Row 0 is `(inherit main)`; the rest are registered models labelled
/// `name — model_id @ provider`.
///
/// ```text
/// ┌─ model ───────────────────────────────────┐
/// │ › (inherit main)                          │
/// │   fast — openai/gpt-4o-mini @ OpenRouter   │
/// │   local — llama3 @ Local llama            │
/// │ ↑↓ select · enter ok · esc cancel         │
/// └───────────────────────────────────────────┘
/// ```
pub(super) fn draw_model_picker(
    frame: &mut Frame,
    rest: &AppStateRest,
    picker: &ModelPickerState,
    palette: &Palette,
    area: Rect,
) {
    // Content rows: options (min 1, max 10) + two hint lines (split for narrow modals). Borders add 2.
    let opt_rows = picker.options.len().clamp(1, 10) as u16;
    let content_h = opt_rows + 2;
    let total_h = content_h + 2;
    // Model labels are longer than provider names; give the modal more room.
    let popup_w = 56_u16.min(area.width.saturating_sub(2));
    let popup = centered_rect(area, popup_w, total_h);

    // Dim the backdrop (fg dim + bg reset, like the settings modals so a stacked
    // layer still recedes).
    {
        let buf = frame.buffer_mut();
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if x >= popup.x && x < popup.right() && y >= popup.y && y < popup.bottom() {
                    continue;
                }
                buf[(x, y)].set_fg(palette.dim).set_bg(Color::Reset);
            }
        }
    }

    let modal_block = Block::bordered()
        .border_style(Style::default().fg(palette.accent))
        .title(Span::styled(" model ", Style::default().fg(palette.accent)));
    let inner = modal_block.inner(popup);

    frame.render_widget(Clear, popup);
    frame.render_widget(modal_block, popup);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let body_w = inner.width;
    let body_x = inner.x;
    let cursor = picker.cursor.min(picker.options.len().saturating_sub(1));
    // Scrolloff window (persisted offset on rest — ModelPickerState is rebuilt
    // per client frame). `opt_rows` is the viewport height; `lines` holds one
    // row per option.
    let (scroll, _) = crate::view::scroll::scroll_window(
        &rest.agents_model_picker_offset,
        cursor,
        picker.options.len(),
        opt_rows as usize,
    );

    let lines: Vec<Line> = picker
        .options
        .iter()
        .enumerate()
        .map(|(i, (_, name))| {
            let marker = if i == cursor { "›" } else { " " };
            if i == cursor {
                let label = truncate(&format!("{marker} {name}"), body_w as usize);
                Line::from(Span::styled(
                    format!("{:<width$}", label, width = body_w as usize),
                    Style::default().fg(palette.sel_fg).bg(palette.sel_bg),
                ))
            } else {
                Line::from(Span::styled(
                    truncate(&format!("{marker} {name}"), body_w as usize),
                    Style::default().fg(palette.fg),
                ))
            }
        })
        .collect();

    frame.render_widget(
        Paragraph::new(lines).scroll((scroll as u16, 0)),
        Rect { x: body_x, y: inner.y, width: body_w, height: opt_rows },
    );

    // Hint lines (last two inner rows): split for narrow modals.
    let hint_y = inner.y + opt_rows;
    frame.render_widget(
        Paragraph::new(Span::styled(
            "\u{2191}\u{2193} select",
            Style::default().fg(palette.dim),
        )),
        Rect { x: body_x, y: hint_y, width: body_w, height: 1 },
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            "enter ok \u{b7} esc cancel",
            Style::default().fg(palette.dim),
        )),
        Rect { x: body_x, y: hint_y + 1, width: body_w, height: 1 },
    );
}
