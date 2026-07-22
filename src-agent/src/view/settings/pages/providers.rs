use super::super::utils::truncate;
use crate::app::mode::SettingsState;
use crate::view::theme::Palette;
use ratatui::{
    layout::{Constraint, Rect},
    style::Style,
    text::Span,
    widgets::{Cell, Paragraph, Row, Table},
    Frame,
};

/// Render the API Providers interactive screen inside `area`.
///
/// Shows a borderless table (header + one row per provider) and a `[+ add]`
/// button below it. The selected real row is inverse-highlighted; the selected
/// add-button row is also inverse-highlighted. Armed-for-delete rows are
/// prefixed with "DEL? " to signal the pending confirm.
pub(crate) fn draw_providers_page(
    frame: &mut Frame,
    st: &SettingsState,
    palette: &Palette,
    area: Rect,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    // Column widths: Name (14), Endpoint (flexible), Type (11), Key (8).
    let col_name_w = 14u16;
    let col_type_w = 11u16;
    let col_key_w = 8u16;
    let col_ep_w = area
        .width
        .saturating_sub(col_name_w + col_type_w + col_key_w + 3);

    // Header row.
    let header = Row::new(vec![
        Cell::from(Span::styled("Name", Style::default().fg(palette.dim))),
        Cell::from(Span::styled("Endpoint", Style::default().fg(palette.dim))),
        Cell::from(Span::styled("Type", Style::default().fg(palette.dim))),
        Cell::from(Span::styled("Key", Style::default().fg(palette.dim))),
    ]);

    // Data rows.
    let rows: Vec<Row> = st
        .providers
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let selected = i == st.prov_sel && !st.prov_on_add_button();
            let armed = selected && st.prov_delete_armed;

            let name_str = if armed {
                format!(
                    "DEL? {}",
                    if p.name.is_empty() {
                        "\u{2014}"
                    } else {
                        &p.name
                    }
                )
            } else if p.name.is_empty() {
                "\u{2014}".to_string()
            } else {
                p.name.clone()
            };
            let name_str = truncate(&name_str, col_name_w as usize);
            let ep_str = truncate(&p.endpoint, col_ep_w as usize);
            let type_str = p.api_type.short_label().to_string();
            let key_str = if p.api_key.is_empty() {
                "\u{2014}".to_string()
            } else {
                "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}".to_string()
            };

            let row_style = if selected {
                Style::default().fg(palette.sel_fg).bg(palette.sel_bg)
            } else {
                Style::default().fg(palette.fg)
            };

            Row::new(vec![
                Cell::from(name_str),
                Cell::from(ep_str),
                Cell::from(type_str),
                Cell::from(key_str),
            ])
            .style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Length(col_name_w),
        Constraint::Min(col_ep_w.max(10)),
        Constraint::Length(col_type_w),
        Constraint::Length(col_key_w),
    ];

    // Height for the table: header (1) + rows; leave 1 row for the add button.
    let table_h = area.height.saturating_sub(1).max(1);
    let table_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: table_h,
    };
    let btn_area = Rect {
        x: area.x,
        y: area.y + table_h,
        width: area.width,
        height: 1,
    };

    let table = Table::new(rows, widths).header(header);
    frame.render_widget(table, table_area);

    // Add-button row.
    let on_btn = st.prov_on_add_button();
    let btn_style = if on_btn {
        Style::default().fg(palette.sel_fg).bg(palette.sel_bg)
    } else {
        Style::default().fg(palette.accent)
    };
    frame.render_widget(
        Paragraph::new(Span::styled("[ + add provider ]", btn_style)),
        btn_area,
    );
}
