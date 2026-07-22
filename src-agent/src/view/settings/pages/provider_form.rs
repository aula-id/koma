use super::super::utils::truncate;
use crate::app::mode::settings::ProviderModal;
use crate::view::theme::Palette;
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

pub(crate) fn draw_provider_form(
    frame: &mut Frame,
    modal: &ProviderModal,
    palette: &Palette,
    area: Rect,
) {
    let inner = area;
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let label_w = 14usize;
    let val_w = (inner.width as usize).saturating_sub(label_w + 1).max(4);
    let mut lines: Vec<Line> = Vec::new();

    // Blank padding top
    for _ in 0..(inner.height as usize / 3).min(6) {
        lines.push(Line::from(""));
    }

    // Name
    {
        let active = modal.field == 0;
        let lc = if active { palette.accent } else { palette.dim };
        let label = Span::styled(
            format!("{:<width$}", "Name", width = label_w),
            Style::default().fg(lc),
        );
        let mut val = truncate(&modal.name, val_w.saturating_sub(1));
        if active {
            val.push('\u{2588}');
        }
        let vc = if active { palette.fg } else { palette.dim };
        lines.push(Line::from(vec![
            Span::raw("  "),
            label,
            Span::styled(val, Style::default().fg(vc)),
        ]));
    }
    // Endpoint
    {
        let active = modal.field == 1;
        let lc = if active { palette.accent } else { palette.dim };
        let label = Span::styled(
            format!("{:<width$}", "Endpoint", width = label_w),
            Style::default().fg(lc),
        );
        let mut val = truncate(&modal.endpoint, val_w.saturating_sub(1));
        if active {
            val.push('\u{2588}');
        }
        let vc = if active { palette.fg } else { palette.dim };
        lines.push(Line::from(vec![
            Span::raw("  "),
            label,
            Span::styled(val, Style::default().fg(vc)),
        ]));
    }
    // API key
    {
        let active = modal.field == 2;
        let lc = if active { palette.accent } else { palette.dim };
        let label = Span::styled(
            format!("{:<width$}", "API key", width = label_w),
            Style::default().fg(lc),
        );
        let mut val = truncate(&modal.api_key, val_w.saturating_sub(1));
        if active {
            val.push('\u{2588}');
        }
        let vc = if active { palette.fg } else { palette.dim };
        lines.push(Line::from(vec![
            Span::raw("  "),
            label,
            Span::styled(val, Style::default().fg(vc)),
        ]));
    }
    // Blank
    lines.push(Line::from(""));
    // Buttons
    let save_text = "[ Save ]";
    let cancel_text = "[ Cancel ]";
    let gap = "   ";
    let group_len = save_text.len() + gap.len() + cancel_text.len();
    let inner_w = inner.width as usize;
    let pad_left = inner_w.saturating_sub(group_len) / 2;
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
    ]));

    frame.render_widget(Paragraph::new(lines), inner);
}
