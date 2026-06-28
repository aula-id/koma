//! Browse detail rows, list sidebar, and footer hint logic.

use ratatui::{
    layout::{Margin, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::mode::agents::source_label;
use crate::app::mode::{AgentEditField, AgentSubMode, AgentsState};
use crate::model::app_config::AppConfig;
use crate::model::settings::Settings;
use crate::view::theme::Palette;

use super::{model_display, truncate};

/// Render the LIST pane: one row per agent (`name` + source tag), RIGHT border.
pub(super) fn draw_list(
    frame: &mut Frame,
    st: &AgentsState,
    palette: &Palette,
    area: Rect,
) {
    let block = Block::new()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(palette.dim));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let content = inner.inner(Margin { horizontal: 1, vertical: 1 });
    // Focus lives in the LIST only while Browsing and not in the detail pane.
    let list_focused = st.mode == AgentSubMode::Browse && !st.in_detail;

    let lines: Vec<Line> = if st.agents.is_empty() {
        vec![Line::from(Span::styled(
            "(no agents)",
            Style::default().fg(palette.dim),
        ))]
    } else {
        let name_w = (content.width as usize).saturating_sub(12).max(4);
        st.agents
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let selected = i == st.list_sel;
                let name = truncate(&a.name, name_w);
                if selected && list_focused {
                    // Focused selection: accent-block highlight matching the @ / /
                    // command-palette convention (sel_fg on sel_bg across the row).
                    let hl = Style::default().fg(palette.sel_fg).bg(palette.sel_bg);
                    Line::from(vec![
                        Span::styled("› ", hl),
                        Span::styled(format!("{name:<width$}", width = name_w), hl),
                        Span::styled(" ", Style::default()),
                        Span::styled(source_label(a.source), Style::default().fg(palette.dim)),
                    ])
                } else if selected {
                    // Selected but focus is in the detail pane: keep accent text so
                    // the selection stays visible without the full highlight block.
                    let accent = Style::default().fg(palette.accent);
                    Line::from(vec![
                        Span::styled("› ", accent),
                        Span::styled(format!("{name:<width$}", width = name_w), accent),
                        Span::styled(" ", Style::default()),
                        Span::styled(source_label(a.source), Style::default().fg(palette.dim)),
                    ])
                } else {
                    Line::from(vec![
                        Span::styled("  ", Style::default().fg(palette.dim)),
                        Span::styled(format!("{name:<width$}", width = name_w), Style::default().fg(palette.dim)),
                        Span::styled(" ", Style::default()),
                        Span::styled(source_label(a.source), Style::default().fg(palette.dim)),
                    ])
                }
            })
            .collect()
    };
    frame.render_widget(Paragraph::new(lines), content);
}

/// Render the DETAIL pane based on the active sub-mode.
pub(super) fn draw_detail(
    frame: &mut Frame,
    st: &AgentsState,
    config: &AppConfig,
    settings: Option<&Settings>,
    palette: &Palette,
    area: Rect,
) {
    use super::editor::{delete_lines, editor_lines};

    let inner = area.inner(Margin { horizontal: 2, vertical: 1 });
    let lines = match st.mode {
        AgentSubMode::Browse => browse_lines(st, config, settings, palette, inner.width as usize),
        AgentSubMode::Edit | AgentSubMode::Create => {
            editor_lines(st, config, settings, palette, inner.width as usize)
        }
        AgentSubMode::DeleteConfirm => delete_lines(st, palette),
    };
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Detail rows for Browse: the selected agent's metadata + a body preview.
pub(super) fn browse_lines<'a>(
    st: &'a AgentsState,
    config: &AppConfig,
    settings: Option<&Settings>,
    palette: &Palette,
    width: usize,
) -> Vec<Line<'a>> {
    let Some(a) = st.current_agent() else {
        return vec![Line::from(Span::styled(
            "no agent selected",
            Style::default().fg(palette.dim),
        ))];
    };
    let value_w = width.saturating_sub(14).max(4);
    let mut lines = Vec::new();

    let row = |label: &str, value: String, color: ratatui::style::Color| -> Line<'static> {
        Line::from(vec![
            Span::styled(format!("{label:<14}"), Style::default().fg(palette.dim)),
            Span::styled(value, Style::default().fg(color)),
        ])
    };

    lines.push(row("name", a.name.clone(), palette.accent));
    lines.push(row("source", source_label(a.source).to_string(), palette.fg));
    lines.push(row(
        "description",
        truncate(&a.description, value_w),
        palette.fg,
    ));
    // Conditions (when to delegate) — only shown when set; it's what the roster
    // injects into the system prompt.
    if !a.conditions.trim().is_empty() {
        lines.push(row("conditions", truncate(&a.conditions, value_w), palette.fg));
    }
    // Model is the chosen REGISTERED model (resolved to `name @ provider`); None =
    // inherit the Main role, shown dim (with a legacy slug hint for old files).
    let (model_text, model_chosen) = model_display(config, settings, &a.model_uuid, &a.model);
    lines.push(row(
        "model",
        truncate(&model_text, value_w),
        if model_chosen { palette.fg } else { palette.dim },
    ));
    let tools = if a.tools.is_empty() {
        "(read-only default)".to_string()
    } else {
        truncate(&a.tools.join(", "), value_w)
    };
    lines.push(row(
        "tools",
        tools,
        if a.tools.is_empty() { palette.dim } else { palette.fg },
    ));

    // Body preview: a label row, then the first few prompt lines, dim.
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "prompt",
        Style::default().fg(palette.dim),
    )));
    let preview_w = width.saturating_sub(2).max(4);
    for raw in a.prompt.lines().take(8) {
        lines.push(Line::from(Span::styled(
            format!("  {}", truncate(raw, preview_w)),
            Style::default().fg(palette.fg),
        )));
    }
    lines
}

/// Context-sensitive footer hint for the active sub-mode.
pub(super) fn footer_hint(st: &AgentsState) -> &'static str {
    // Model picker owns input while open (deepest modal).
    if st.model_picker.is_some() {
        return "↑↓ select · enter ok · esc cancel";
    }

    match st.mode {
        AgentSubMode::DeleteConfirm => "y delete · n/Esc cancel",
        AgentSubMode::Create | AgentSubMode::Edit => {
            if st.editing {
                "type to edit · Ctrl+J newline (prompt) · Enter/Esc done"
            } else if st.field == AgentEditField::Model {
                "enter pick model · ↑/↓ field · esc cancel"
            } else if st.mode == AgentSubMode::Create {
                // Keep the scope-toggle affordance for Create's field nav.
                "↑/↓ field · ←/→ scope · Enter edit · s create · Esc cancel"
            } else {
                "↑/↓ field · Enter edit · s save · Esc cancel"
            }
        }
        AgentSubMode::Browse => "↑/↓ pick · →/Enter edit · n new · d delete · Esc close",
    }
}
