use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph},
    Frame,
};
use crate::app::mode::settings::SettingsPage;
use crate::view::theme::Palette;

const MENU_ITEMS: &[(u8, &str, SettingsPage)] = &[
    (1, "Appearance", SettingsPage::Appearance),
    (2, "General",    SettingsPage::General),
    (3, "Providers",  SettingsPage::Providers),
    (4, "OAuth",      SettingsPage::OAuth),
    (5, "Models",     SettingsPage::Models),
];

pub(crate) fn draw_menu(
    frame: &mut Frame,
    menu_sel: usize,
    palette: &Palette,
    area: Rect,
) {
    let content_w = area.width.saturating_sub(4);
    let content_h = area.height.saturating_sub(4);
    let box_w = 38u16.min(content_w);
    let box_h = (MENU_ITEMS.len() as u16 * 2 + 2).min(content_h);
    let x = area.x + (area.width.saturating_sub(box_w)) / 2;
    let y = area.y + (area.height.saturating_sub(box_h)) / 2;
    let popup = Rect { x, y, width: box_w, height: box_h };

    let block = Block::bordered()
        .border_style(Style::default().fg(palette.accent))
        .padding(Padding::horizontal(1));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let mut lines: Vec<Line> = Vec::new();
    // Top padding (1 blank line)
    lines.push(Line::from(""));
    for (i, (num, label, _page)) in MENU_ITEMS.iter().enumerate() {
        let is_selected = i == menu_sel;
        let style = if is_selected {
            Style::default().fg(palette.sel_fg).bg(palette.sel_bg).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.fg)
        };
        let chip_s = format!("[{num}]");
        let label_s = format!("  {label}");
        // row length: " " + chip + text + " "
        let row_len = 1 + chip_s.len() + label_s.len() + 1;
        let pad = (inner.width as usize).saturating_sub(row_len);
        let chip = Span::styled(chip_s, style);
        let text = Span::styled(label_s, style);
        let mut spans = vec![Span::raw(" "), chip, text];
        if pad > 0 {
            spans.push(Span::raw(" ".repeat(pad)));
        }
        lines.push(Line::from(spans));
    }
    // Bottom padding (1 blank line)
    lines.push(Line::from(""));
    frame.render_widget(Paragraph::new(lines), inner);
}
