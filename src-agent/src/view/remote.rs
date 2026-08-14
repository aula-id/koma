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

use ratatui::layout::Rect;
use ratatui::layout::{Constraint, Direction, Layout, Margin};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::mode::remote::{RemoteIntent, RemoteState, RemoteView};
use crate::view::theme::Palette;

/// List sidebar column width (includes RIGHT border).
const SIDEBAR_W: u16 = 26;

/// Draw the remote workflow.
pub fn draw(frame: &mut Frame, m: &RemoteState, palette: &Palette) {
    match m.view {
        RemoteView::Edit => {
            if m.editor.is_some() {
                draw_editor(frame, m, frame.area(), palette);
            }
        }
        RemoteView::SessionHub => {
            draw_session_hub(frame, m, frame.area(), palette);
        }
        RemoteView::Browse => {
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

    // Delete confirm overlay (on top of everything).
    if m.pending_delete.is_some() {
        draw_delete_confirm(frame, m, frame.area(), palette);
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
            "●"
        } else {
            "○"
        };

        let name_w = (inner.width as usize).saturating_sub(3).max(2);
        let name = truncate_str(&host.name, name_w);

        if is_selected {
            let hl = Style::default().fg(palette.sel_fg).bg(palette.sel_bg);
            let content = format!("{dot} {name:<width$}", width = name_w);
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(content, hl))),
                Rect {
                    x: inner.x,
                    y: list_start_y + row as u16,
                    width: inner.width,
                    height: 1,
                },
            );
        } else {
            let content = format!(" {dot} {name:<width$}", width = name_w);
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    content,
                    Style::default().fg(palette.fg),
                ))),
                Rect {
                    x: inner.x,
                    y: list_start_y + row as u16,
                    width: inner.width,
                    height: 1,
                },
            );
        }
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

/// Render the detail pane (right side of browse mode).
fn draw_detail(frame: &mut Frame, m: &RemoteState, palette: &Palette, area: Rect) {
    let inner = area.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });

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
    lines.push(row(
        "addr",
        host.address(),
        palette.fg,
    ));
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
        for (i, session) in m.sessions.iter().take(8).enumerate() {
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
            let _ = i; // suppress unused warning
        }
    }

    let widget = Paragraph::new(lines).wrap(Wrap { trim: true });
    frame.render_widget(widget, inner);
}

/// Render a delete confirm popup overlaid on the center of the screen.
fn draw_delete_confirm(frame: &mut Frame, m: &RemoteState, area: Rect, palette: &Palette) {
    let host_name = m
        .pending_delete
        .as_deref()
        .and_then(|id| m.hosts.iter().find(|h| h.id == id))
        .map(|h| h.name.as_str())
        .unwrap_or("host");

    let msg = format!(" Delete {host_name}? (y/n) ");
    let popup_w = msg.len() as u16 + 4;
    let popup_h: u16 = 3;
    let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup = Rect {
        x,
        y,
        width: popup_w.min(area.width),
        height: popup_h.min(area.height),
    };

    let block = Block::bordered()
        .title(Span::styled(
            " confirm ",
            Style::default().fg(palette.fg).add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(palette.accent));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("Delete {host_name}?"),
            Style::default().fg(palette.fg),
        ))),
        inner,
    );
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

/// Render the host editor form (create or edit).
///
/// Full-screen layout:
///   Header: "remote > add host" or "remote > edit host"
///   5 rows for fields: name, user, host, port, key path
///   Error message (if any)
///   Footer hint bar
fn draw_editor(frame: &mut Frame, m: &RemoteState, area: Rect, palette: &Palette) {
    let Some(editor) = &m.editor else { return };

    // Fill background so the chat underneath doesn't bleed through.
    crate::view::clear_and_fill(frame, area, palette.bg);

    let header_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 2,
    };
    let footer_area = Rect {
        x: area.x,
        y: area.y + area.height - 1,
        width: area.width,
        height: 1,
    };

    // Header.
    let title = if editor.edit_id.is_some() {
        "remote > edit host"
    } else {
        "remote > add host"
    };
    let header = Paragraph::new(Line::from(Span::styled(
        format!(" {title} "),
        Style::default()
            .fg(palette.fg)
            .bg(palette.bg)
            .add_modifier(Modifier::BOLD),
    )));
    frame.render_widget(header, header_area);
    // Separator.
    let sep = Paragraph::new(Line::from(Span::styled(
        "─".repeat(area.width as usize),
        Style::default().fg(palette.dim),
    )));
    frame.render_widget(
        sep,
        Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: 1,
        },
    );

    // Body area (between header and footer).
    let body_area = Rect {
        x: area.x,
        y: area.y + 2,
        width: area.width,
        height: area.height.saturating_sub(3),
    };

    // Render each field row inside a bordered block.
    let block = Block::default()
        .title(" fields ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.dim));
    let inner = block.inner(body_area);
    frame.render_widget(block, body_area);

    use crate::app::mode::remote::HostEditField;
    let fields: [(HostEditField, &str); 5] = [
        (HostEditField::Name, &editor.name),
        (HostEditField::User, &editor.user),
        (HostEditField::Host, &editor.host),
        (HostEditField::Port, &editor.port),
        (HostEditField::KeyPath, &editor.key_path),
    ];

    for (row, &(field, value)) in fields.iter().enumerate() {
        if row as u16 >= inner.height {
            break;
        }
        let y = inner.y + row as u16;
        let is_focused = editor.focused == field;

        // Label.
        let label_style = if is_focused {
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.dim)
        };

        let cursor = if is_focused && m.editing_field {
            "█"
        } else if is_focused {
            "▌"
        } else {
            " "
        };

        let value_display = if field == HostEditField::KeyPath && value.is_empty() {
            "(none)".to_string()
        } else {
            value.to_string()
        };

        let value_style = if is_focused {
            Style::default().fg(palette.fg).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.fg)
        };

        let line = Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{:>9}: ", field.label()), label_style),
            Span::styled(&value_display, value_style),
            Span::styled(cursor, Style::default().fg(palette.accent)),
        ]);

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

    // Error message (below fields, inside the block).
    if let Some(ref error) = editor.error {
        let err_row = fields.len() as u16 + 1;
        if err_row < inner.height {
            let err_line = Line::from(Span::styled(
                format!("  ✗ {error}"),
                Style::default().fg(palette.error),
            ));
            frame.render_widget(
                Paragraph::new(err_line),
                Rect {
                    x: inner.x,
                    y: inner.y + err_row,
                    width: inner.width,
                    height: 1,
                },
            );
        }
    }

    // Footer hint.
    let hint = if m.editing_field {
        " Enter confirm field  Esc cancel edit "
    } else {
        " Enter edit field  s save  Esc back  ↑↓ navigate "
    };
    let hint_line = Line::from(Span::styled(
        hint,
        Style::default().fg(palette.sel_fg).bg(palette.accent),
    ));
    frame.render_widget(Paragraph::new(hint_line), footer_area);
}

/// Context-sensitive footer hint for the active view.
fn footer_hint(m: &RemoteState) -> &'static str {
    match m.intent {
        RemoteIntent::Manage => "↑↓ pick · Enter connect · n new · d delete · e edit · i import · Esc close",
        RemoteIntent::Resume => "↑↓ pick · Enter connect · Esc close",
        RemoteIntent::New => "↑↓ pick · Enter connect · Esc close",
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
