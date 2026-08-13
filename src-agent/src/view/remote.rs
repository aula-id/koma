//! Remote host manager view — compact overlay, fullscreen detail, and editor forms.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::mode::remote::{ConnectionState, HostEditField, RemoteState, RemoteSub};
use crate::view::theme::Palette;

/// Draw the remote host manager mode.
///
/// `input_rect` is the composer area; `transcript_rect` is the scrollable
/// transcript above it. Compact mode renders as a popup overlay anchored
/// above the composer (same pattern as `/skill`, `/bash`, `/todo`).
/// Fullscreen modes take the full frame area.
pub fn draw(
    frame: &mut ratatui::Frame,
    m: &RemoteState,
    input_rect: Rect,
    transcript_rect: Rect,
    palette: &Palette,
) {
    match m.sub {
        RemoteSub::Compact => render_compact(frame, m, input_rect, transcript_rect, palette),
        RemoteSub::Fullscreen => render_fullscreen(frame, m, frame.area(), palette),
        RemoteSub::CreateHost | RemoteSub::EditHost => {
            render_editor(frame, m, frame.area(), palette)
        }
    }
}

/// Compact overlay above the composer (popup style).
fn render_compact(
    frame: &mut ratatui::Frame,
    m: &RemoteState,
    input_rect: Rect,
    transcript_rect: Rect,
    palette: &Palette,
) {
    let row_count = m.filtered.len().min(10) as u16;
    let height = row_count + 4; // search + borders + hint

    // Anchor above the composer, extending upward into the transcript area.
    let h = height.min(transcript_rect.height);
    let y = input_rect.y.saturating_sub(h);
    let popup = Rect {
        x: input_rect.x,
        y,
        width: input_rect.width,
        height: h,
    };

    crate::view::clear_and_fill(frame, popup, palette.bg);

    let block = Block::bordered()
        .title(Span::styled(
            " remote hosts ",
            Style::default().fg(palette.dim),
        ))
        .border_style(Style::default().fg(palette.dim))
        .padding(ratatui::widgets::Padding::horizontal(1));

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    // Search line.
    let search_line = Line::from(vec![
        Span::styled(" > ", Style::default().fg(palette.accent)),
        Span::styled(&m.query, Style::default().fg(palette.fg)),
        Span::styled("_", Style::default().fg(palette.accent)),
    ]);
    let search_widget = Paragraph::new(search_line);
    if inner.height > 0 {
        frame.render_widget(
            search_widget,
            Rect {
                x: inner.x,
                y: inner.y,
                width: inner.width,
                height: 1,
            },
        );
    }

    // Host list.
    let list_area = Rect {
        x: inner.x,
        y: inner.y + 1,
        width: inner.width,
        height: inner.height.saturating_sub(2), // minus search + hint
    };
    for (row, &host_idx) in m
        .filtered
        .iter()
        .enumerate()
        .take(list_area.height as usize)
    {
        let host = &m.hosts[host_idx];
        let is_selected = row == m.selected;
        let is_pending_delete = m.pending_delete.as_deref() == Some(&host.id);

        let row_style = if is_selected {
            Style::default().fg(palette.sel_fg).bg(palette.sel_bg)
        } else {
            Style::default().fg(palette.fg).bg(palette.bg)
        };

        let content = format!(
            " {} {}  {}@{}{}",
            if host.last_connected.is_some() {
                "●"
            } else {
                "○"
            },
            host.name,
            host.user,
            host.host,
            if is_pending_delete { " [confirm?]" } else { "" },
        );
        let width = list_area.width as usize;
        let padded = format!("{content:<width$}");
        let line = Line::from(Span::styled(padded, row_style));

        let y = list_area.y + row as u16;
        if y < list_area.y + list_area.height {
            frame.render_widget(
                Paragraph::new(line),
                Rect {
                    x: list_area.x,
                    y,
                    width: list_area.width,
                    height: 1,
                },
            );
        }
    }

    // Empty state.
    if m.filtered.is_empty() && list_area.height > 0 {
        let empty = Line::from(Span::styled(
            "no hosts. Ctrl+A to add.",
            Style::default().fg(palette.dim),
        ));
        frame.render_widget(
            Paragraph::new(empty),
            Rect {
                x: inner.x,
                y: inner.y + 1,
                width: inner.width,
                height: 1,
            },
        );
    }

    // Hint bar — full-width inverse bar matching other overlays.
    if inner.height > 0 {
        let hint_y = inner.y + inner.height - 1;
        let hint = "enter detail · ctrl+a add · esc close";
        let bar_style = Style::default()
            .fg(palette.sel_fg)
            .bg(palette.sel_bg)
            .add_modifier(Modifier::BOLD);
        let padded = format!(
            " {:<width$}",
            hint,
            width = inner.width.saturating_sub(1) as usize
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::raw(padded))).style(bar_style),
            Rect {
                x: inner.x,
                y: hint_y,
                width: inner.width,
                height: 1,
            },
        );
    }
}

/// Fullscreen view: host detail + sessions pane.
fn render_fullscreen(frame: &mut ratatui::Frame, m: &RemoteState, area: Rect, palette: &Palette) {
    // Header (2 rows).
    let header_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 2,
    };
    let body_area = Rect {
        x: area.x,
        y: area.y + 2,
        width: area.width,
        height: area.height.saturating_sub(3),
    };
    let footer_area = Rect {
        x: area.x,
        y: area.y + area.height - 1,
        width: area.width,
        height: 1,
    };

    // Header title depends on connection_state.
    let title = match m.connection_state {
        Some(ConnectionState::AuthRequired { .. }) => "remote > password required",
        Some(
            ConnectionState::Resolving
            | ConnectionState::Authenticating
            | ConnectionState::Bootstrapping
            | ConnectionState::Connecting,
        ) => "remote > connecting...",
        Some(ConnectionState::Error { .. }) => "remote > error",
        Some(ConnectionState::Connected { .. }) => "remote > connected",
        Some(ConnectionState::Disconnected) => "remote > disconnected",
        None => "remote",
    };
    let header = Paragraph::new(Line::from(Span::styled(
        format!(" {title} "),
        Style::default()
            .fg(palette.fg)
            .bg(palette.bg)
            .add_modifier(Modifier::BOLD),
    )));
    frame.render_widget(header, header_area);
    // Separator line.
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

    // Body: two panes (left = host detail, right = sessions).
    let halves = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(body_area);

    render_host_detail(frame, m, halves[0], palette);
    render_sessions_pane(frame, m, halves[1], palette);

    // Footer hint depends on connection_state.
    let hint = match &m.connection_state {
        Some(ConnectionState::AuthRequired { .. }) => {
            " Type password, Enter to submit, Esc to cancel ".into()
        }
        Some(ConnectionState::Resolving) => " resolving... (Esc to cancel) ".into(),
        Some(ConnectionState::Authenticating) => " authenticating... (Esc to cancel) ".into(),
        Some(ConnectionState::Bootstrapping) => " bootstrapping... (Esc to cancel) ".into(),
        Some(ConnectionState::Connecting) => " connecting... (Esc to cancel) ".into(),
        Some(ConnectionState::Error { message }) => {
            format!(" error: {} — Esc to dismiss ", message)
        }
        Some(ConnectionState::Connected { .. }) => {
            " Connected — Disconnect (d) or Esc back ".into()
        }
        Some(ConnectionState::Disconnected) => " c connect  e edit  Del delete  Esc back ".into(),
        None => " c connect  e edit  Del delete  Esc back ".into(),
    };
    let hint_line = Line::from(Span::styled(
        hint,
        Style::default().fg(palette.sel_fg).bg(palette.accent),
    ));
    frame.render_widget(Paragraph::new(hint_line), footer_area);
}

/// Render the host detail pane (left half).
fn render_host_detail(frame: &mut ratatui::Frame, m: &RemoteState, area: Rect, palette: &Palette) {
    let block = Block::default()
        .title(" host detail ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.dim));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(host) = m.selected_host() else {
        let msg = Paragraph::new(Span::styled(
            "no host selected",
            Style::default().fg(palette.dim),
        ));
        frame.render_widget(msg, inner);
        return;
    };

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("  name: ", Style::default().fg(palette.dim)),
        Span::styled(&host.name, Style::default().fg(palette.fg)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  addr: ", Style::default().fg(palette.dim)),
        Span::styled(host.address(), Style::default().fg(palette.accent)),
    ]));
    if let Some(ref key) = host.key_path {
        lines.push(Line::from(vec![
            Span::styled("  key:  ", Style::default().fg(palette.dim)),
            Span::styled(key.as_str(), Style::default().fg(palette.fg)),
        ]));
    }
    let status_span = if host.last_connected.is_some() {
        Span::styled("● connected", Style::default().fg(palette.success))
    } else {
        Span::styled("○ not connected", Style::default().fg(palette.dim))
    };
    lines.push(Line::from(vec![
        Span::styled("  status: ", Style::default().fg(palette.dim)),
        status_span,
    ]));
    if !host.tags.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("  tags: ", Style::default().fg(palette.dim)),
            Span::styled(host.tags.join(", "), Style::default().fg(palette.info)),
        ]));
    }

    let widget = Paragraph::new(lines).wrap(Wrap { trim: true });
    frame.render_widget(widget, inner);
}

/// Render the sessions pane (right half).
fn render_sessions_pane(
    frame: &mut ratatui::Frame,
    m: &RemoteState,
    area: Rect,
    palette: &Palette,
) {
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
fn render_editor(frame: &mut ratatui::Frame, m: &RemoteState, area: Rect, palette: &Palette) {
    let Some(editor) = &m.editor else { return };

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
    let title = match m.sub {
        RemoteSub::EditHost => "remote > edit host",
        _ => "remote > add host",
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
