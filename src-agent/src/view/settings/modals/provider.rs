use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
    Frame,
};
use crate::app::mode::settings::ProviderModal;
use crate::view::theme::Palette;
use super::super::utils::truncate;

/// Render the add-provider modal overlay with a dimmed backdrop.
///
/// Mirrors the `draw_tool_picker` approach from `view/agents.rs`:
/// walk `frame.buffer_mut()` to dim every cell outside the modal rect,
/// then `Clear` + `Block::bordered()` + inner content.
pub(in crate::view::settings) fn draw_provider_modal(
    frame: &mut Frame,
    modal: &ProviderModal,
    palette: &Palette,
    area: Rect,
) {
    // Modal dimensions: ~50 wide, 9 tall (header + 4 rows + blank + save + 2 borders).
    const MODAL_W: u16 = 52;
    const MODAL_H: u16 = 9;
    let w = MODAL_W.min(area.width.saturating_sub(2));
    let h = MODAL_H.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect { x, y, width: w, height: h };

    // Dim everything outside the modal.
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

    // Modal box.
    let modal_block = Block::bordered()
        .border_style(Style::default().fg(palette.accent))
        .title(Span::styled(" Add API provider ", Style::default().fg(palette.accent)));
    let inner = modal_block.inner(popup);

    frame.render_widget(Clear, popup);
    frame.render_widget(modal_block, popup);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let label_w = 10usize;
    let val_w   = (inner.width as usize).saturating_sub(label_w + 1).max(4);
    let mut lines: Vec<Line> = Vec::new();

    // Row 0: Name
    {
        let active = modal.field == 0;
        let lc = if active { palette.accent } else { palette.dim };
        let label = Span::styled(format!("{:<width$}", "Name", width = label_w), Style::default().fg(lc));
        let mut val = truncate(&modal.name, val_w.saturating_sub(1));
        if active { val.push('\u{2588}'); }
        let vc = if active { palette.fg } else { palette.dim };
        lines.push(Line::from(vec![label, Span::styled(val, Style::default().fg(vc))]));
    }

    // Row 1: Endpoint
    {
        let active = modal.field == 1;
        let lc = if active { palette.accent } else { palette.dim };
        let label = Span::styled(format!("{:<width$}", "Endpoint", width = label_w), Style::default().fg(lc));
        let mut val = truncate(&modal.endpoint, val_w.saturating_sub(1));
        if active { val.push('\u{2588}'); }
        let vc = if active { palette.fg } else { palette.dim };
        lines.push(Line::from(vec![label, Span::styled(val, Style::default().fg(vc))]));
    }

    // Row 2: API key
    {
        let active = modal.field == 2;
        let lc = if active { palette.accent } else { palette.dim };
        let label = Span::styled(format!("{:<width$}", "API key", width = label_w), Style::default().fg(lc));
        let mut val = truncate(&modal.api_key, val_w.saturating_sub(1));
        if active { val.push('\u{2588}'); }
        let vc = if active { palette.fg } else { palette.dim };
        lines.push(Line::from(vec![label, Span::styled(val, Style::default().fg(vc))]));
    }

    // Blank line.
    lines.push(Line::from(""));

    // Button row: `[ Save ]   [ Cancel ]` centered together.
    // Only the chip text carries the highlight bg; padding uses DEFAULT style so
    // the bg does not bleed across the full modal width.
    let save_text   = "[ Save ]";
    let cancel_text = "[ Cancel ]";
    let gap         = "   ";
    let group_len   = save_text.len() + gap.len() + cancel_text.len();
    let inner_w     = inner.width as usize;
    let pad_left    = inner_w.saturating_sub(group_len) / 2;
    let pad_right   = inner_w.saturating_sub(group_len).saturating_sub(pad_left);
    let save_style = if modal.field == 3 {
        Style::default().fg(palette.sel_fg).bg(palette.sel_bg)
    } else {
        Style::default().fg(palette.accent)
    };
    let cancel_style = if modal.field == 4 {
        Style::default().fg(palette.sel_fg).bg(palette.sel_bg)
    } else {
        Style::default().fg(palette.accent)
    };
    lines.push(Line::from(vec![
        Span::raw(" ".repeat(pad_left)),
        Span::styled(save_text, save_style),
        Span::raw(gap),
        Span::styled(cancel_text, cancel_style),
        Span::raw(" ".repeat(pad_right)),
    ]));

    frame.render_widget(Paragraph::new(lines), inner);
}
