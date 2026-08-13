//! Remote host manager view — compact overlay and fullscreen detail.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::app::mode::remote::{RemoteState, RemoteSub};
use crate::view::theme::Palette;

/// Draw the remote host manager mode.
pub fn draw(
    frame: &mut ratatui::Frame,
    m: &RemoteState,
    area: Rect,
    palette: &Palette,
) {
    match m.sub {
        RemoteSub::Compact => render_compact(frame, m, area, palette),
        RemoteSub::Fullscreen | RemoteSub::Connecting | RemoteSub::PasswordInput => {
            render_fullscreen(frame, m, area, palette);
        }
    }
}

/// Compact overlay above the composer (popup style).
fn render_compact(
    frame: &mut ratatui::Frame,
    m: &RemoteState,
    area: Rect,
    palette: &Palette,
) {
    let row_count = m.filtered.len().min(12) as u16;
    let height = row_count + 4; // search + borders + hint
    let h = height.min(area.height.saturating_sub(2));

    // Anchor above the bottom (composer area).
    let y = area.y + area.height.saturating_sub(h);
    let popup = Rect {
        x: area.x + 2,
        y,
        width: area.width.saturating_sub(4).min(60),
        height: h,
    };

    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(" remote hosts ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.dim));

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
        frame.render_widget(search_widget, Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        });
    }

    // Host list.
    let list_area = Rect {
        x: inner.x,
        y: inner.y + 1,
        width: inner.width,
        height: inner.height.saturating_sub(1),
    };
    for (row, &host_idx) in m.filtered.iter().enumerate().take(list_area.height as usize) {
        let host = &m.hosts[host_idx];
        let is_selected = row == m.selected;
        let is_pending_delete = m.pending_delete.as_deref() == Some(&host.id);

        let dot = if host.last_connected.is_some() {
            Span::styled("●", Style::default().fg(palette.success))
        } else {
            Span::styled("○", Style::default().fg(palette.dim))
        };

        let name_style = if is_selected {
            Style::default().fg(palette.sel_fg).bg(palette.sel_bg)
        } else {
            Style::default().fg(palette.fg)
        };

        let delete_warn = if is_pending_delete {
            Span::styled(" [confirm?]", Style::default().fg(palette.error))
        } else {
            Span::raw("")
        };

        let line = Line::from(vec![
            Span::raw(" "),
            dot,
            Span::raw(" "),
            Span::styled(&host.name, name_style),
            Span::styled(
                format!("  {}@{}", host.user, host.host),
                Style::default().fg(palette.dim),
            ),
            delete_warn,
        ]);

        let y = list_area.y + row as u16;
        if y < list_area.y + list_area.height {
            frame.render_widget(
                Paragraph::new(line),
                Rect { x: list_area.x, y, width: list_area.width, height: 1 },
            );
        }
    }

    // Empty state.
    if m.filtered.is_empty() && inner.height > 2 {
        let empty = Line::from(Span::styled(
            "no hosts. Ctrl+A to add.",
            Style::default().fg(palette.dim),
        ));
        frame.render_widget(
            Paragraph::new(empty),
            Rect { x: inner.x, y: inner.y + 1, width: inner.width, height: 1 },
        );
    }

    // Hint bar.
    if inner.height > 0 {
        let hint_y = inner.y + inner.height - 1;
        let hint = Line::from(vec![
            Span::styled(" Enter ", Style::default().fg(palette.sel_fg).bg(palette.accent)),
            Span::styled(" detail  ", Style::default().fg(palette.dim)),
            Span::styled(" Ctrl+A ", Style::default().fg(palette.sel_fg).bg(palette.accent)),
            Span::styled(" add  ", Style::default().fg(palette.dim)),
            Span::styled(" Esc ", Style::default().fg(palette.sel_fg).bg(palette.accent)),
            Span::styled(" close", Style::default().fg(palette.dim)),
        ]);
        frame.render_widget(
            Paragraph::new(hint),
            Rect { x: inner.x, y: hint_y, width: inner.width, height: 1 },
        );
    }
}

/// Fullscreen view: host detail + sessions pane.
fn render_fullscreen(
    frame: &mut ratatui::Frame,
    m: &RemoteState,
    area: Rect,
    palette: &Palette,
) {
    // Header (2 rows).
    let header_area = Rect { x: area.x, y: area.y, width: area.width, height: 2 };
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

    // Header.
    let title = match m.sub {
        RemoteSub::Connecting => "remote > connecting...",
        RemoteSub::PasswordInput => "remote > password required",
        _ => "remote",
    };
    let header = Paragraph::new(Line::from(Span::styled(
        format!(" {title} "),
        Style::default().fg(palette.fg).bg(palette.bg).add_modifier(Modifier::BOLD),
    )));
    frame.render_widget(header, header_area);
    // Separator line.
    let sep = Paragraph::new(Line::from(Span::styled(
        "─".repeat(area.width as usize),
        Style::default().fg(palette.dim),
    )));
    frame.render_widget(
        sep,
        Rect { x: area.x, y: area.y + 1, width: area.width, height: 1 },
    );

    // Body: two panes (left = host detail, right = sessions).
    let halves = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(body_area);

    render_host_detail(frame, m, halves[0], palette);
    render_sessions_pane(frame, m, halves[1], palette);

    // Footer hint.
    let hint = match m.sub {
        RemoteSub::Connecting => {
            let stage_text = m.connection_status.as_ref()
                .map(|s| s.stage.as_str())
                .unwrap_or("connecting");
            let error_text = m.connection_status.as_ref()
                .and_then(|s| s.error.as_deref())
                .unwrap_or("");
            if !error_text.is_empty() {
                format!(" {} - error: {} ", stage_text, error_text)
            } else {
                format!(" {}... (Esc to cancel) ", stage_text)
            }
        }
        RemoteSub::PasswordInput => " Type password, Enter to submit, Esc to cancel ".into(),
        _ => " c connect  e edit  Del delete  Esc back ".into(),
    };
    let hint_line = Line::from(Span::styled(
        hint,
        Style::default().fg(palette.sel_fg).bg(palette.accent),
    ));
    frame.render_widget(Paragraph::new(hint_line), footer_area);
}

/// Render the host detail pane (left half).
fn render_host_detail(
    frame: &mut ratatui::Frame,
    m: &RemoteState,
    area: Rect,
    palette: &Palette,
) {
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
        Span::styled(
            host.address(),
            Style::default().fg(palette.accent),
        ),
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
            Span::styled(
                host.tags.join(", "),
                Style::default().fg(palette.info),
            ),
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
            Rect { x: inner.x, y, width: inner.width, height: 1 },
        );
    }
}
