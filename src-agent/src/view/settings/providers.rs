use ratatui::{
    layout::{Constraint, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table},
    Frame,
};
use crate::app::mode::SettingsState;
use crate::view::theme::Palette;
use super::utils::truncate;

/// Render the API Providers interactive screen inside `area`.
///
/// Shows a borderless table (header + one row per provider) and a `[+ add]`
/// button below it. The selected real row is inverse-highlighted; the selected
/// add-button row is also inverse-highlighted. Armed-for-delete rows are
/// prefixed with "DEL? " to signal the pending confirm.
pub(super) fn draw_providers(
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
    let col_key_w  = 8u16;
    let col_ep_w   = area.width.saturating_sub(col_name_w + col_type_w + col_key_w + 3);

    // Header row.
    let header = Row::new(vec![
        Cell::from(Span::styled("Name",     Style::default().fg(palette.dim))),
        Cell::from(Span::styled("Endpoint", Style::default().fg(palette.dim))),
        Cell::from(Span::styled("Type",     Style::default().fg(palette.dim))),
        Cell::from(Span::styled("Key",      Style::default().fg(palette.dim))),
    ]);

    // Data rows.
    let rows: Vec<Row> = st.providers.iter().enumerate().map(|(i, p)| {
        let selected = st.in_detail && i == st.prov_sel && !st.prov_on_add_button();
        let armed    = selected && st.prov_delete_armed;

        let name_str = if armed {
            format!("DEL? {}", if p.name.is_empty() { "\u{2014}" } else { &p.name })
        } else if p.name.is_empty() {
            "\u{2014}".to_string()
        } else {
            p.name.clone()
        };
        let name_str = truncate(&name_str, col_name_w as usize);
        let ep_str   = truncate(&p.endpoint, col_ep_w as usize);
        let type_str = p.api_type.short_label().to_string();
        let key_str  = if p.api_key.is_empty() { "\u{2014}".to_string() } else { "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}".to_string() };

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
        ]).style(row_style)
    }).collect();

    let widths = [
        Constraint::Length(col_name_w),
        Constraint::Min(col_ep_w.max(10)),
        Constraint::Length(col_type_w),
        Constraint::Length(col_key_w),
    ];

    // Height for the table: header (1) + rows; leave 1 row for the add button.
    let table_h = area.height.saturating_sub(1).max(1);
    let table_area = Rect { x: area.x, y: area.y, width: area.width, height: table_h };
    let btn_area   = Rect { x: area.x, y: area.y + table_h, width: area.width, height: 1 };

    let table = Table::new(rows, widths)
        .header(header);
    frame.render_widget(table, table_area);

    // Add-button row.
    let on_btn = st.in_detail && st.prov_on_add_button();
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

/// Render the Models Select interactive screen inside `area`.
///
/// Layout (top to bottom):
///   Line 0: `Model List   [+add global]  [+add local]`
///   Line 1: `[X]all [ ]local [ ]global`  (radio; cursor + active filter independently shown)
///   Line 2+: model table (header + visible data rows)
///
/// The five control slots (add global=0, add local=1, filter all=2, filter
/// local=3, filter global=4) share the same `model_sel` index as the data rows.
/// A data row at visible position `p` is highlighted when `model_sel == 5 + p`.
/// This mirrors [`crate::app::mode::settings::state::model_ops::MODEL_CTRL_SLOTS`].
///
/// An armed-for-delete data row is prefixed with "DEL? ". A scope-glyph prefix
/// (`* ` dim = global, two spaces = local) is rendered at the left of each name cell.
///
/// Columns: Name (12 = glyph 2 + name 10), Role (11), Model (flexible), Provider (12).
pub(super) fn draw_models(
    frame: &mut Frame,
    st: &SettingsState,
    palette: &Palette,
    area: Rect,
) {
    use crate::app::mode::settings::{ModelFilterMode, ModelRole, MODEL_CTRL_SLOTS};

    if area.height == 0 || area.width == 0 {
        return;
    }

    let focused = st.in_detail;
    let filter  = st.model_filter;

    // ---- Line 0: title + inline add buttons -----------------------------------
    // Format: `Model List   [+add global]  [+add local]`
    {
        let on_global = focused && st.model_sel == 0;
        let on_local  = focused && st.model_sel == 1;

        let btn_g_style = if on_global {
            Style::default().fg(palette.sel_fg).bg(palette.sel_bg)
        } else {
            Style::default().fg(palette.accent)
        };
        let btn_l_style = if on_local {
            Style::default().fg(palette.sel_fg).bg(palette.sel_bg)
        } else {
            Style::default().fg(palette.accent)
        };

        let title_line = Line::from(vec![
            Span::styled("Model List   ", Style::default().fg(palette.dim)),
            Span::styled("[+add global]", btn_g_style),
            Span::raw("  "),
            Span::styled("[+add local]", btn_l_style),
        ]);
        let title_area = Rect { x: area.x, y: area.y, width: area.width, height: 1 };
        frame.render_widget(Paragraph::new(title_line), title_area);
    }

    if area.height < 2 {
        return;
    }

    // ---- Line 1: filter radio bar ---------------------------------------------
    // Active filter shown with `[X]`; cursor highlight independently from `model_sel`.
    // A filter option can simultaneously be the active one AND the cursor target.
    {
        // Helper: radio chip text — `[X]` when the active filter, `[ ]` otherwise.
        let mk_radio = |mode: ModelFilterMode, label: &str| -> String {
            if filter == mode { format!("[X]{}", label) } else { format!("[ ]{}", label) }
        };
        // Cursor highlight: sel==2 → All, sel==3 → Local, sel==4 → Global.
        let cursor_style = |slot: usize| -> Style {
            if focused && st.model_sel == slot {
                Style::default().fg(palette.sel_fg).bg(palette.sel_bg)
            } else {
                Style::default().fg(palette.dim)
            }
        };

        let radio_line = Line::from(vec![
            Span::styled(mk_radio(ModelFilterMode::All,    "all"),    cursor_style(2)),
            Span::raw(" "),
            Span::styled(mk_radio(ModelFilterMode::Local,  "local"),  cursor_style(3)),
            Span::raw(" "),
            Span::styled(mk_radio(ModelFilterMode::Global, "global"), cursor_style(4)),
        ]);
        let radio_area = Rect { x: area.x, y: area.y + 1, width: area.width, height: 1 };
        frame.render_widget(Paragraph::new(radio_line), radio_area);
    }

    if area.height < 3 {
        return;
    }

    // ---- Lines 2+: model table -----------------------------------------------
    let table_y = area.y + 2;
    let table_h = area.height.saturating_sub(2);

    // Column widths: Name (12 total = 2 glyph + 10 name text), Role (11),
    // Model (flexible), Provider (12).
    let col_name_w  = 12u16;
    let col_role_w  = 11u16;
    let col_prov_w  = 12u16;
    let col_model_w = area.width.saturating_sub(col_name_w + col_role_w + col_prov_w + 3);
    // Name text budget after the 2-char glyph prefix.
    let name_text_w = col_name_w.saturating_sub(2) as usize;

    // Header row.
    let header = Row::new(vec![
        Cell::from(Span::styled("Name",     Style::default().fg(palette.dim))),
        Cell::from(Span::styled("Role",     Style::default().fg(palette.dim))),
        Cell::from(Span::styled("Model",    Style::default().fg(palette.dim))),
        Cell::from(Span::styled("Provider", Style::default().fg(palette.dim))),
    ]);

    // Collect visible indices once so visible position == enumerate index.
    let vis_indices = st.visible_model_indices();

    // Data rows — iterate visible positions only.
    // A data row at visible position `p` is highlighted when model_sel == MODEL_CTRL_SLOTS + p.
    let rows: Vec<Row> = vis_indices.iter().enumerate().map(|(vis_pos, &real_idx)| {
        let m = &st.models[real_idx];
        let selected = focused && st.model_sel == MODEL_CTRL_SLOTS + vis_pos;
        let armed    = selected && st.model_delete_armed;

        // Name cell: dim glyph prefix + styled name text.
        // The glyph is always rendered with palette.dim regardless of selection.
        let glyph = if m.session_only { "  " } else { "* " };
        let name_text = if armed {
            format!("DEL? {}", if m.name.is_empty() { "\u{2014}" } else { &m.name })
        } else if m.name.is_empty() {
            "\u{2014}".to_string()
        } else {
            m.name.clone()
        };
        let name_text = truncate(&name_text, name_text_w);

        let row_style = if selected {
            Style::default().fg(palette.sel_fg).bg(palette.sel_bg)
        } else {
            Style::default().fg(palette.fg)
        };

        // Build a multi-span Line for the name cell so the glyph stays dim and
        // does NOT inherit the selection background (only the text span does).
        let name_line = Line::from(vec![
            Span::styled(glyph,     Style::default().fg(palette.dim)),
            Span::styled(name_text, row_style),
        ]);

        // A model may hold several roles → comma-join their labels (truncated to
        // the column width); an em-dash when it holds none.
        let role_str = if m.roles.is_empty() {
            "\u{2014}".to_string()
        } else {
            m.roles
                .iter()
                .map(|r: &ModelRole| r.label())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let role_str  = truncate(&role_str, col_role_w as usize);
        let model_str = if m.model_id.is_empty() {
            "\u{2014}".to_string()
        } else {
            truncate(&m.model_id, col_model_w as usize)
        };
        let prov_str = st
            .providers
            .get(m.provider_idx)
            .map(|p| p.name.as_str())
            .filter(|n| !n.is_empty())
            .unwrap_or("\u{2014}");
        let prov_str = truncate(prov_str, col_prov_w as usize);

        Row::new(vec![
            Cell::from(name_line),
            Cell::from(role_str).style(row_style),
            Cell::from(model_str).style(row_style),
            Cell::from(prov_str).style(row_style),
        ])
    }).collect();

    let widths = [
        Constraint::Length(col_name_w),
        Constraint::Length(col_role_w),
        Constraint::Min(col_model_w.max(10)),
        Constraint::Length(col_prov_w),
    ];

    let table_area = Rect { x: area.x, y: table_y, width: area.width, height: table_h };
    let table = Table::new(rows, widths).header(header);
    frame.render_widget(table, table_area);
}
