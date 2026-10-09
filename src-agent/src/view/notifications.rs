use crate::{app::mode::notifications::NotificationsState, view::theme::Palette};
use ratatui::{
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};
pub fn draw(frame: &mut Frame, s: &NotificationsState, p: &Palette) {
    let mut lines = vec![Line::from(format!(
        "{} history · Search: {}",
        if s.app { "App" } else { "Session" },
        s.query
    ))];
    if let Some(error) = &s.error {
        lines.push(Line::from(error.clone()));
    }
    if s.pending.is_some() {
        lines.push(Line::from("Loading…"));
    }
    let filtered = s.filtered();
    if filtered.is_empty() && s.pending.is_none() {
        lines.push(Line::from("No notifications."));
    }
    for (i, e) in filtered.iter().enumerate().skip(s.cursor.saturating_sub(5)) {
        lines.push(Line::from(Span::styled(
            format!(
                "{} {} [{}] {}",
                if i == s.cursor { ">" } else { " " },
                if e.read { " " } else { "●" },
                e.severity,
                e.message.lines().next().unwrap_or_default()
            ),
            Style::default().fg(if i == s.cursor { p.accent } else { p.fg }),
        )));
        if s.expanded.as_ref() == Some(&e.id) {
            lines.push(Line::from(format!(
                "{} · {} · {}",
                e.timestamp, e.source, e.id
            )));
            lines.extend(e.message.lines().map(|line| Line::from(line.to_string())));
        }
    }
    let footer = if s.confirm_clear {
        "Clear this scope permanently? y = confirm; any other key = cancel"
    } else {
        "↑↓ Select · Enter Details/read · Tab Session/App · Ctrl+R Mark all read · Ctrl+X Clear · Esc Back"
    };
    let area = frame.area();
    let chunks = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Min(0),
        ratatui::layout::Constraint::Length(1),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .title("Notifications")
                .borders(Borders::ALL),
        ),
        chunks[0],
    );
    frame.render_widget(Paragraph::new(footer), chunks[1]);
}
