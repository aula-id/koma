//! MCP dashboard: list sidebar, detail/editor rows, delete prompt, footer hint.

use std::collections::HashMap;

use ratatui::{
    layout::{Margin, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::mode::mcp::transport_label;
use crate::app::mode::{McpEditField, McpState, McpSubMode};
use crate::view::theme::Palette;

use super::truncate;

/// Split `s` into chunks of at most `width` chars (char-boundary safe, handles
/// multibyte). If `s` is empty returns a single empty string so callers always
/// get at least one element.  `width` is clamped to at least 1.
fn wrap_chars(s: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    if s.is_empty() {
        return vec![String::new()];
    }
    let chars: Vec<char> = s.chars().collect();
    chars
        .chunks(width)
        .map(|c| c.iter().collect())
        .collect()
}

/// Push wrapped label+value lines into `lines`.
///
/// - First line:        `label` (left-padded to `label_w`) + chunk[0], label in `dim`, value in `color`
/// - Continuation lines: `label_w` spaces + chunk[n], value in `color`
fn push_wrapped(
    lines: &mut Vec<Line<'static>>,
    label: &str,
    value: String,
    label_w: usize,
    width: usize,
    dim: ratatui::style::Color,
    color: ratatui::style::Color,
) {
    let chunks = wrap_chars(&value, width);
    for (i, chunk) in chunks.into_iter().enumerate() {
        if i == 0 {
            lines.push(Line::from(vec![
                Span::styled(format!("{label:<label_w$}"), Style::default().fg(dim)),
                Span::styled(chunk, Style::default().fg(color)),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled(
                    " ".repeat(label_w),
                    Style::default().fg(dim),
                ),
                Span::styled(chunk, Style::default().fg(color)),
            ]));
        }
    }
}

/// Render the LIST pane: one row per server (`name` + enabled marker + transport
/// + live status), with a RIGHT border as the divider.
///
/// `status` maps server uuid -> discovered tool count (from the live
/// [`McpManager`](crate::app::mcp::McpManager) snapshot). A present key = the
/// server is connected (`● N tools`); absent = not connected (`○ —`). `None`
/// (no manager) shows no status column at all.
pub(super) fn draw_list(
    frame: &mut Frame,
    st: &McpState,
    status: Option<&HashMap<String, usize>>,
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
    let list_focused = st.mode == McpSubMode::Browse && !st.in_detail;

    let lines: Vec<Line> = if st.servers.is_empty() {
        vec![Line::from(Span::styled(
            "(no servers)",
            Style::default().fg(palette.dim),
        ))]
    } else {
        // Reserve a few columns at the right for the enabled dot + transport tag.
        let name_w = (content.width as usize).saturating_sub(10).max(4);
        st.servers
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let selected = i == st.list_sel;
                let name = truncate(&s.name, name_w);
                // Enabled dot (● on / ○ off) then the transport tag, both dim.
                let dot = if s.enabled { "●" } else { "○" };
                let tag = transport_label(s.transport);
                if selected && list_focused {
                    let hl = Style::default().fg(palette.sel_fg).bg(palette.sel_bg);
                    Line::from(vec![
                        Span::styled("› ", hl),
                        Span::styled(format!("{name:<width$}", width = name_w), hl),
                        Span::styled(format!(" {dot} {tag}"), Style::default().fg(palette.dim)),
                    ])
                } else if selected {
                    let accent = Style::default().fg(palette.accent);
                    Line::from(vec![
                        Span::styled("› ", accent),
                        Span::styled(format!("{name:<width$}", width = name_w), accent),
                        Span::styled(format!(" {dot} {tag}"), Style::default().fg(palette.dim)),
                    ])
                } else {
                    Line::from(vec![
                        Span::styled("  ", Style::default().fg(palette.dim)),
                        Span::styled(
                            format!("{name:<width$}", width = name_w),
                            Style::default().fg(palette.dim),
                        ),
                        Span::styled(format!(" {dot} {tag}"), Style::default().fg(palette.dim)),
                    ])
                }
            })
            .collect()
    };

    // Suppress the unused-arg warning path: status is consumed in the detail pane.
    let _ = status;
    frame.render_widget(Paragraph::new(lines), content);
}

/// Render the DETAIL pane based on the active sub-mode.
pub(super) fn draw_detail(
    frame: &mut Frame,
    st: &McpState,
    status: Option<&HashMap<String, usize>>,
    palette: &Palette,
    area: Rect,
) {
    let inner = area.inner(Margin { horizontal: 2, vertical: 1 });
    let lines = match st.mode {
        McpSubMode::Browse => browse_lines(st, status, palette, inner.width as usize),
        McpSubMode::Edit | McpSubMode::Create => editor_lines(st, palette, inner.width as usize),
        McpSubMode::DeleteConfirm => delete_lines(st, palette),
    };
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The live-status span for the server with `uuid`: `● N tools` when connected
/// (present in the map), else `○ —`. Returns `None` when there's no manager at
/// all (the caller then omits the status row).
fn status_span(
    uuid: &str,
    status: Option<&HashMap<String, usize>>,
    palette: &Palette,
) -> Option<(String, ratatui::style::Color)> {
    let map = status?;
    match map.get(uuid) {
        Some(n) => Some((format!("● {n} tools"), palette.accent)),
        None => Some(("○ —".to_string(), palette.dim)),
    }
}

/// Detail rows for Browse: the selected server's fields + live status.
fn browse_lines<'a>(
    st: &'a McpState,
    status: Option<&HashMap<String, usize>>,
    palette: &Palette,
    width: usize,
) -> Vec<Line<'a>> {
    use crate::model::app_config::McpTransport;

    let Some(s) = st.current() else {
        return vec![Line::from(Span::styled(
            "no server selected",
            Style::default().fg(palette.dim),
        ))];
    };
    let value_w = width.saturating_sub(14).max(4);
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Short single-line rows (fixed-width values that never need wrapping).
    let row = |label: &str, value: String, color: ratatui::style::Color| -> Line<'static> {
        Line::from(vec![
            Span::styled(format!("{label:<14}"), Style::default().fg(palette.dim)),
            Span::styled(value, Style::default().fg(color)),
        ])
    };

    // name — may be long, wrap it.
    push_wrapped(&mut lines, "name", s.name.clone(), 14, value_w, palette.dim, palette.accent);

    // enabled / transport — always short, keep as single rows.
    lines.push(row(
        "enabled",
        if s.enabled { "yes".into() } else { "no".into() },
        if s.enabled { palette.fg } else { palette.dim },
    ));
    lines.push(row("transport", transport_label(s.transport).to_string(), palette.fg));

    // Live status (best-effort): only when a manager exists.
    if let Some((text, color)) = status_span(&s.uuid, status, palette) {
        lines.push(row("status", text, color));
    }

    match s.transport {
        McpTransport::Stdio => {
            if s.command.trim().is_empty() {
                lines.push(row("command", "(none)".to_string(), palette.dim));
            } else {
                push_wrapped(&mut lines, "command", s.command.clone(), 14, value_w, palette.dim, palette.fg);
            }
            if s.args.is_empty() {
                lines.push(row("args", "(none)".to_string(), palette.dim));
            } else {
                let joined = s.args.join(" ");
                push_wrapped(&mut lines, "args", joined, 14, value_w, palette.dim, palette.fg);
            }
            if s.env.is_empty() {
                lines.push(row("env", "(none)".to_string(), palette.dim));
            } else {
                let joined = s
                    .env
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                push_wrapped(&mut lines, "env", joined, 14, value_w, palette.dim, palette.fg);
            }
        }
        McpTransport::Http => {
            if s.url.trim().is_empty() {
                lines.push(row("url", "(none)".to_string(), palette.dim));
            } else {
                push_wrapped(&mut lines, "url", s.url.clone(), 14, value_w, palette.dim, palette.fg);
            }
        }
    }

    lines
}

/// Detail rows for Edit / Create: one labelled draft field per row, with an
/// inline block cursor on the active text field and ←/→ hints on toggles.
fn editor_lines<'a>(st: &'a McpState, palette: &Palette, width: usize) -> Vec<Line<'a>> {
    let value_w = width.saturating_sub(16).max(4);
    let mut lines = Vec::new();

    for f in st.fields() {
        let selected = f == st.field;
        let editing_here = st.editing && selected;
        let marker = Span::styled(
            if selected { "› " } else { "  " },
            Style::default().fg(palette.accent),
        );
        let label_color = if selected { palette.accent } else { palette.dim };
        let label = Span::styled(
            format!("{:<14}", f.label()),
            Style::default().fg(label_color),
        );

        // Toggle fields (Enabled / Transport) render their bool/enum value, with a
        // ←/→ hint when selected. They never enter inline text-edit.
        if f == McpEditField::Enabled {
            let val = if st.draft_enabled { "yes" } else { "no" };
            let mut row = vec![
                marker,
                label,
                Span::styled(val.to_string(), Style::default().fg(palette.fg)),
            ];
            if selected {
                row.push(Span::styled("  ←/→ toggle", Style::default().fg(palette.dim)));
            }
            lines.push(Line::from(row));
            continue;
        }
        if f == McpEditField::Transport {
            let mut row = vec![
                marker,
                label,
                Span::styled(
                    transport_label(st.draft_transport).to_string(),
                    Style::default().fg(palette.fg),
                ),
            ];
            if selected {
                row.push(Span::styled(
                    "  ←/→ stdio/http",
                    Style::default().fg(palette.dim),
                ));
            }
            lines.push(Line::from(row));
            continue;
        }

        // Single-line text fields (Name / Command / Args / Env / Url).
        let raw = st.draft(f);
        if raw.is_empty() && !editing_here {
            // Show placeholder as a single dim line.
            let ph = match f {
                McpEditField::Name => "(required)",
                McpEditField::Command => "(required — e.g. npx)",
                McpEditField::Args => "(space separated)",
                McpEditField::Env => "(KEY=VAL, KEY2=VAL2)",
                McpEditField::Url => "(required — https://…)",
                // Toggles handled above.
                McpEditField::Enabled | McpEditField::Transport => "",
            };
            lines.push(Line::from(vec![
                marker,
                label,
                Span::styled(ph.to_string(), Style::default().fg(palette.dim)),
            ]));
        } else {
            // Wrap the full value; if editing append cursor to raw first so it
            // appears at the end of the last wrapped line.
            let display_raw = if editing_here {
                let mut s = raw.to_string();
                s.push('█');
                s
            } else {
                raw.to_string()
            };
            let chunks = wrap_chars(&display_raw, value_w);
            for (i, chunk) in chunks.into_iter().enumerate() {
                if i == 0 {
                    // First line: marker + label + value chunk.
                    lines.push(Line::from(vec![
                        marker.clone(),
                        label.clone(),
                        Span::styled(chunk, Style::default().fg(palette.fg)),
                    ]));
                } else {
                    // Continuation: 16 spaces (marker 2 + label 14) + value chunk.
                    lines.push(Line::from(vec![
                        Span::styled(
                            " ".repeat(16),
                            Style::default().fg(palette.dim),
                        ),
                        Span::styled(chunk, Style::default().fg(palette.fg)),
                    ]));
                }
            }
        }
    }

    lines
}

/// Detail rows for DeleteConfirm: a one-line `y`/`n` prompt.
fn delete_lines<'a>(st: &'a McpState, palette: &Palette) -> Vec<Line<'a>> {
    let name = st.current().map(|s| s.name.as_str()).unwrap_or("?");
    vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("delete ", Style::default().fg(palette.fg)),
            Span::styled(format!("'{name}'"), Style::default().fg(palette.accent)),
            Span::styled("?", Style::default().fg(palette.fg)),
        ]),
        Line::from(Span::styled(
            "this removes the server from config.json",
            Style::default().fg(palette.dim),
        )),
    ]
}

/// Context-sensitive footer hint for the active sub-mode.
pub(super) fn footer_hint(st: &McpState) -> &'static str {
    match st.mode {
        McpSubMode::DeleteConfirm => "y delete · n/Esc cancel",
        McpSubMode::Create | McpSubMode::Edit => {
            if st.editing {
                "type to edit · Enter/Esc done"
            } else if st.field.is_toggle() {
                "←/→/Space toggle · ↑/↓ field · s save · Esc cancel"
            } else {
                "↑/↓ field · Enter edit · s save · Esc cancel"
            }
        }
        McpSubMode::Browse => "↑/↓ pick · →/Enter edit · n new · d delete · Esc close",
    }
}
