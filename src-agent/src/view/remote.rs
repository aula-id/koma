//! Two-pane remote host management dashboard (mirrors `/agents` layout).
//!
//! ```text
//! ┌──────────────────┬──────────────────────────────┐
//! │ remote hosts     │ host detail                  │
//! │                  │                              │
//! │ › prod-server    │  name    prod-server         │
//! │   staging-server │  addr    root@10.0.0.1       │
//! │   dev-box        │  key     ~/.ssh/id_ed25519   │
//! │                  │  status  ● connected          │
//! │                  │                              │
//! │                  │  sessions                    │
//! │                  │  ● session-a (current)       │
//! │                  │  ○ session-b                 │
//! │                  │                              │
//! └──────────────────┴──────────────────────────────┘
//!   ↑↓ pick · Enter connect · n new · d delete · e edit · Esc close
//! ```
//!
//! Edit/Create/DeleteConfirm share the same two-pane layout as Browse — the
//! detail pane shows the editor form fields or delete prompt instead of the
//! host metadata. Only `SessionHub` takes over the full frame.

use ratatui::layout::Rect;
use ratatui::layout::{Constraint, Direction, Layout, Margin};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::mode::remote::{HostEditField, RemoteIntent, RemoteState, RemoteView};
use crate::view::theme::Palette;

/// List sidebar column width (includes RIGHT border).
const SIDEBAR_W: u16 = 26;

/// Draw the remote workflow.
///
/// Matches the `/agents` design signature: Browse/Edit/DeleteConfirm all share
/// the same two-pane layout (sidebar + detail). Only `SessionHub` takes over
/// the full frame.
pub fn draw(frame: &mut Frame, m: &RemoteState, palette: &Palette) {
    match m.view {
        RemoteView::SessionHub => {
            draw_session_hub(frame, m, frame.area(), palette);
        }
        RemoteView::Browse | RemoteView::DeleteConfirm | RemoteView::Edit => {
            draw_browse(frame, m, palette);
        }
    }
}

/// Two-pane browse layout: header | body (list + detail) | footer.
fn draw_browse(frame: &mut Frame, m: &RemoteState, palette: &Palette) {
    // Fill background so the chat underneath doesn't bleed through.
    crate::view::clear_and_fill(frame, frame.area(), palette.bg);

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // header
            Constraint::Min(0),   // body
            Constraint::Length(1), // footer
        ])
        .split(frame.area());

    // Header.
    let header_block = Block::new()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(palette.dim));
    let header_inner = header_block.inner(outer[0]);
    frame.render_widget(header_block, outer[0]);
    let title = match m.intent {
        RemoteIntent::Manage => "remote",
        RemoteIntent::Resume => "remote — resume",
        RemoteIntent::New => "remote — new session",
    };
    frame.render_widget(
        Paragraph::new(Span::styled(title, Style::default().fg(palette.dim))),
        header_inner.inner(Margin {
            horizontal: 2,
            vertical: 0,
        }),
    );

    // Body: list sidebar + detail pane.
    let body_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(SIDEBAR_W), Constraint::Min(0)])
        .split(outer[1]);

    draw_host_list(frame, m, palette, body_cols[0]);
    draw_detail(frame, m, palette, body_cols[1]);

    // Footer.
    let footer_rect = outer[2];
    if footer_rect.width > 0 {
        let hint = footer_hint(m);
        let bar_style = Style::default()
            .fg(palette.sel_fg)
            .bg(palette.sel_bg)
            .add_modifier(Modifier::BOLD);
        let padded = format!(
            " {:<width$}",
            hint,
            width = footer_rect.width.saturating_sub(1) as usize
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::raw(padded))).style(bar_style),
            footer_rect,
        );
    }
}

/// Render the host list sidebar (left pane) with search + selection.
fn draw_host_list(frame: &mut Frame, m: &RemoteState, palette: &Palette, area: Rect) {
    let block = Block::new()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(palette.dim));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 {
        return;
    }

    // Host rows.
    let list_start_y = inner.y;
    let list_height = inner.height;
    for (row, &host_idx) in m
        .filtered
        .iter()
        .enumerate()
        .take(list_height as usize)
    {
        let host = &m.hosts[host_idx];
        let is_selected = row == m.selected;

        let dot = if host.last_connected.is_some() {
            Span::styled("●", Style::default().fg(palette.success))
        } else {
            Span::styled("○", Style::default().fg(palette.dim))
        };

        let name_w = (inner.width as usize).saturating_sub(3).max(2);
        let name = truncate_str(&host.name, name_w);

        let line = if is_selected {
            let hl = Style::default().fg(palette.sel_fg).bg(palette.sel_bg);
            Line::from(vec![
                Span::styled("› ", hl),
                Span::styled(format!("{name:<width$}", width = name_w), hl),
                Span::styled(" ", Style::default()),
                dot,
            ])
        } else {
            Line::from(vec![
                Span::styled("  ", Style::default().fg(palette.dim)),
                Span::styled(
                    format!("{name:<width$}", width = name_w),
                    Style::default().fg(palette.dim),
                ),
                Span::styled(" ", Style::default()),
                dot,
            ])
        };

        frame.render_widget(
            Paragraph::new(line),
            Rect {
                x: inner.x,
                y: list_start_y + row as u16,
                width: inner.width,
                height: 1,
            },
        );
    }

    // Empty state.
    if m.filtered.is_empty() && list_height > 0 {
        let empty_text = if m.intent == RemoteIntent::Manage {
            "no hosts. Press n to add."
        } else {
            "no saved hosts. Use /remote to add one."
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                empty_text,
                Style::default().fg(palette.dim),
            ))),
            Rect {
                x: inner.x,
                y: list_start_y,
                width: inner.width,
                height: 1,
            },
        );
    }
}

/// Render the detail pane (right side) based on the active view.
fn draw_detail(frame: &mut Frame, m: &RemoteState, palette: &Palette, area: Rect) {
    let inner = area.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });

    match m.view {
        RemoteView::Edit => {
            draw_editor_detail(frame, m, palette, inner);
        }
        RemoteView::DeleteConfirm => {
            draw_delete_detail(frame, m, palette, inner);
        }
        RemoteView::Browse | RemoteView::SessionHub => {
            draw_host_detail(frame, m, palette, inner);
        }
    }
}

/// Detail rows for Browse: the selected host's metadata.
fn draw_host_detail(frame: &mut Frame, m: &RemoteState, palette: &Palette, inner: Rect) {
    let Some(host) = m.selected_host() else {
        let msg = Paragraph::new(Span::styled(
            "no host selected",
            Style::default().fg(palette.dim),
        ));
        frame.render_widget(msg, inner);
        return;
    };

    let value_w = (inner.width as usize).saturating_sub(14).max(4);
    let mut lines: Vec<Line> = Vec::new();

    let row = |label: &str, value: String, color: ratatui::style::Color| -> Line<'static> {
        Line::from(vec![
            Span::styled(format!("{label:<14}"), Style::default().fg(palette.dim)),
            Span::styled(value, Style::default().fg(color)),
        ])
    };

    lines.push(row("name", host.name.clone(), palette.accent));
    lines.push(row("addr", host.address(), palette.fg));
    if let Some(ref key) = host.key_path {
        lines.push(row("key", truncate_str(key, value_w), palette.fg));
    }

    let status_span = if host.last_connected.is_some() {
        Span::styled("● connected", Style::default().fg(palette.success))
    } else {
        Span::styled("○ not connected", Style::default().fg(palette.dim))
    };
    lines.push(Line::from(vec![
        Span::styled("status         ", Style::default().fg(palette.dim)),
        status_span,
    ]));

    if !host.tags.is_empty() {
        lines.push(row(
            "tags",
            truncate_str(&host.tags.join(", "), value_w),
            palette.info,
        ));
    }

    // Connection state (if active).
    if let Some(ref cs) = m.connection_state {
        use crate::app::mode::remote::ConnectionState;
        let cs_text = match cs {
            ConnectionState::Disconnected => "disconnected".to_string(),
            ConnectionState::Resolving => "resolving…".to_string(),
            ConnectionState::Authenticating => "authenticating…".to_string(),
            ConnectionState::AuthRequired { .. } => "auth required".to_string(),
            ConnectionState::Bootstrapping => "bootstrapping…".to_string(),
            ConnectionState::Connecting => "connecting…".to_string(),
            ConnectionState::Connected { session_id } => {
                format!("connected ({})", truncate_str(session_id, 12))
            }
            ConnectionState::Error { message } => format!("error: {}", truncate_str(message, 30)),
        };
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("connection     ", Style::default().fg(palette.dim)),
            Span::styled(cs_text, Style::default().fg(palette.accent)),
        ]));
    }

    // Sessions (if any discovered).
    if !m.sessions.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "sessions",
            Style::default().fg(palette.dim),
        )));
        for session in m.sessions.iter().take(8) {
            let dot = if session.working {
                Span::styled("●", Style::default().fg(palette.accent))
            } else {
                Span::styled("○", Style::default().fg(palette.dim))
            };
            let fg_label = if session.is_foreground {
                " (current)"
            } else {
                ""
            };
            lines.push(Line::from(vec![
                Span::raw("  "),
                dot,
                Span::raw(" "),
                Span::styled(&session.name, Style::default().fg(palette.fg)),
                Span::styled(fg_label, Style::default().fg(palette.dim)),
            ]));
        }
    }

    let widget = Paragraph::new(lines).wrap(Wrap { trim: true });
    frame.render_widget(widget, inner);
}

/// Detail rows for Edit/Create: the host editor form fields.
///
/// Matches the agents `editor_lines` pattern: `›` marker on the selected field,
/// label in accent/dim, value display, and an `(editing)` hint when actively
/// typing into a field.
fn draw_editor_detail(
    frame: &mut Frame,
    m: &RemoteState,
    palette: &Palette,
    inner: Rect,
) {
    let Some(editor) = &m.editor else {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "no editor",
                Style::default().fg(palette.dim),
            )),
            inner,
        );
        return;
    };

    let value_w = (inner.width as usize).saturating_sub(16).max(4);
    let mut lines: Vec<Line> = Vec::new();

    // Title row: "add host" or "edit host".
    let title = if editor.edit_id.is_some() {
        "edit host"
    } else {
        "add host"
    };
    lines.push(Line::from(Span::styled(
        title,
        Style::default().fg(palette.dim),
    )));
    lines.push(Line::from(""));

    // Editor fields with `›` marker pattern (matching agents design).
    let fields: [(HostEditField, &str); 5] = [
        (HostEditField::Name, &editor.name),
        (HostEditField::User, &editor.user),
        (HostEditField::Host, &editor.host),
        (HostEditField::Port, &editor.port),
        (HostEditField::KeyPath, &editor.key_path),
    ];

    for &(field, value) in &fields {
        let selected = field == editor.focused;
        let editing_here = m.editing_field && selected;

        let marker = Span::styled(
            if selected { "› " } else { "  " },
            Style::default().fg(palette.accent),
        );
        let label_color = if selected {
            palette.accent
        } else {
            palette.dim
        };
        let label = Span::styled(
            format!("{:<14}", field.label()),
            Style::default().fg(label_color),
        );

        // Value: show placeholder for empty key_path, block cursor when editing.
        let (shown, color) = if field == HostEditField::KeyPath && value.is_empty() {
            ("(none)".to_string(), palette.dim)
        } else if editing_here {
            let mut s = truncate_str(value, value_w.saturating_sub(1));
            s.push('█');
            (s, palette.fg)
        } else {
            (truncate_str(value, value_w), palette.fg)
        };

        let mut row = vec![marker, label, Span::styled(shown, Style::default().fg(color))];
        if selected && !editing_here {
            row.push(Span::styled(
                "  Enter edit",
                Style::default().fg(palette.dim),
            ));
        }
        lines.push(Line::from(row));
    }

    // Error message (below fields).
    if let Some(ref error) = editor.error {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  ✗ {error}"),
            Style::default().fg(palette.error),
        )));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Detail rows for DeleteConfirm: a one-line y/n prompt (matches agents
/// `delete_lines` pattern).
fn draw_delete_detail(
    frame: &mut Frame,
    m: &RemoteState,
    palette: &Palette,
    inner: Rect,
) {
    let host_name = m
        .selected_host()
        .map(|h| h.name.as_str())
        .unwrap_or("host");
    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("delete ", Style::default().fg(palette.fg)),
            Span::styled(format!("'{host_name}'"), Style::default().fg(palette.accent)),
            Span::styled("?", Style::default().fg(palette.fg)),
        ]),
        Line::from(Span::styled(
            "this removes the saved host",
            Style::default().fg(palette.dim),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Render the dedicated remote session hub (fullscreen overlay).
fn draw_session_hub(frame: &mut Frame, m: &RemoteState, area: Rect, palette: &Palette) {
    crate::view::clear_and_fill(frame, area, palette.bg);
    let host_name = m
        .selected_host_id
        .as_deref()
        .and_then(|id| m.hosts.iter().find(|host| host.id == id))
        .map(|host| host.name.as_str())
        .unwrap_or("remote host");
    let rows = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Length(2),
        ratatui::layout::Constraint::Min(1),
        ratatui::layout::Constraint::Length(1),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" resume remote · {host_name} "),
            Style::default().fg(palette.fg).add_modifier(Modifier::BOLD),
        ))),
        rows[0],
    );
    draw_sessions_list(frame, m, rows[1], palette);
    frame.render_widget(
        Paragraph::new(" enter resume selected UUID · esc choose another host ")
            .style(Style::default().fg(palette.sel_fg).bg(palette.accent)),
        rows[2],
    );
}

/// Render the sessions list inside the session hub.
fn draw_sessions_list(frame: &mut Frame, m: &RemoteState, area: Rect, palette: &Palette) {
    let block = Block::default()
        .title(" sessions ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.dim));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if m.sessions.is_empty() {
        let msg = Paragraph::new(Span::styled(
            "no sessions",
            Style::default().fg(palette.dim),
        ));
        frame.render_widget(msg, inner);
        return;
    }

    for (i, session) in m.sessions.iter().enumerate().take(inner.height as usize) {
        let is_selected = i == m.session_selected;
        let dot = if session.working {
            Span::styled("●", Style::default().fg(palette.accent))
        } else {
            Span::styled("○", Style::default().fg(palette.dim))
        };
        let row_style = if is_selected {
            Style::default().fg(palette.sel_fg).bg(palette.sel_bg)
        } else {
            Style::default().fg(palette.fg)
        };
        let fg_label = if session.is_foreground {
            " (current)"
        } else {
            ""
        };

        let line = Line::from(vec![
            Span::raw(" "),
            dot,
            Span::raw(" "),
            Span::styled(&session.name, row_style),
            Span::styled(fg_label, Style::default().fg(palette.dim)),
        ]);

        let y = inner.y + i as u16;
        frame.render_widget(
            Paragraph::new(line),
            Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: 1,
            },
        );
    }
}

/// Context-sensitive footer hint for the active view.
fn footer_hint(m: &RemoteState) -> &'static str {
    match m.view {
        RemoteView::Edit => {
            if m.editing_field {
                "Enter confirm field · Esc cancel edit"
            } else {
                "↑↓ navigate · Enter edit field · s save · Esc back"
            }
        }
        RemoteView::DeleteConfirm => "y delete · n/Esc cancel",
        RemoteView::SessionHub => "↑↓ pick · Enter resume · Esc back",
        RemoteView::Browse => match m.intent {
            RemoteIntent::Manage => {
                "↑↓ pick · Enter connect · n new · d delete · e edit · i import · Esc close"
            }
            RemoteIntent::Resume | RemoteIntent::New => {
                "↑↓ pick · Enter connect · Esc close"
            }
        },
    }
}

/// Truncate `s` to at most `max` chars, appending `…` if cut.
fn truncate_str(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        let cut = max.saturating_sub(1);
        chars[..cut].iter().collect::<String>() + "…"
    }
}
