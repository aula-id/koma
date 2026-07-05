//! View — the `/settings` OAuth submenu: the connections list plus the
//! connect-flow overlays (provider picker, Codex browser wait, Codex paste,
//! Kilo Code device wait, failure).

use ratatui::{
    layout::{Constraint, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table, Wrap},
    Frame,
};

use crate::app::mode::settings::OAuthFlowState;
use crate::app::mode::SettingsState;
use crate::view::theme::Palette;
use super::utils::truncate;

/// Braille spinner frames, matching the `/security` panel's in-flight probe glyph.
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Pull the parenthesized identity out of an `OAuthDraft::label` like
/// `"codex (foo@bar.com)"` → `"foo@bar.com"`. Falls back to the whole label if it
/// doesn't match that shape.
fn account_of(label: &str) -> &str {
    match label.find('(') {
        Some(start) => {
            let inner = &label[start + 1..];
            inner.strip_suffix(')').unwrap_or(inner)
        }
        None => label,
    }
}

/// Render the `/settings` OAuth submenu inside `area`.
pub(super) fn draw_oauth(
    frame: &mut Frame,
    st: &SettingsState,
    palette: &Palette,
    area: Rect,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    match &st.oauth_flow {
        OAuthFlowState::Idle => draw_list(frame, st, palette, area),
        OAuthFlowState::Starting => draw_message(
            frame,
            palette,
            area,
            &format!("{} starting login…", SPINNER[0]),
            None,
            false,
        ),
        OAuthFlowState::Pick(cursor) => draw_picker(frame, *cursor, palette, area),
        OAuthFlowState::CodexWait { url, frame: f, copied } => draw_message(
            frame,
            palette,
            area,
            &format!(
                "{} waiting for browser · listening on localhost:{}",
                SPINNER[(*f as usize) % SPINNER.len()],
                crate::service::oauth::registry::CODEX_PORT,
            ),
            Some(url),
            *copied,
        ),
        OAuthFlowState::CodexPaste { input } => draw_paste(frame, input, palette, area),
        OAuthFlowState::KiloWait {
            user_code,
            verification_url,
            frame: f,
            copied,
        } => draw_message(
            frame,
            palette,
            area,
            &format!(
                "{} approve in browser · code: {}",
                SPINNER[(*f as usize) % SPINNER.len()],
                user_code,
            ),
            Some(verification_url),
            *copied,
        ),
        OAuthFlowState::Failed(msg) => draw_failed(frame, msg, palette, area),
    }
}

/// Idle screen: the connections table + `[+connect]` button, mirroring
/// `draw_providers`'s shape (borderless table, inverse-highlighted selection,
/// `DEL? ` prefix on the armed row).
fn draw_list(frame: &mut Frame, st: &SettingsState, palette: &Palette, area: Rect) {
    let col_prov_w = 12u16;
    let col_status_w = 16u16;
    let col_acct_w = area.width.saturating_sub(col_prov_w + col_status_w + 2);

    let header = Row::new(vec![
        Cell::from(Span::styled("Provider", Style::default().fg(palette.dim))),
        Cell::from(Span::styled("Account", Style::default().fg(palette.dim))),
        Cell::from(Span::styled("Status", Style::default().fg(palette.dim))),
    ]);

    let rows: Vec<Row> = st
        .oauth_drafts
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let selected = st.in_detail && i == st.oauth_sel;
            let armed = selected && st.oauth_armed == Some(i);

            let prov_str = d.provider.label();
            let acct_raw = account_of(&d.label);
            let acct_str = if armed {
                format!("DEL? {acct_raw}")
            } else {
                acct_raw.to_string()
            };
            let acct_str = truncate(&acct_str, col_acct_w as usize);
            let status_str = truncate(&d.status, col_status_w as usize);

            let row_style = if selected {
                Style::default().fg(palette.sel_fg).bg(palette.sel_bg)
            } else {
                Style::default().fg(palette.fg)
            };

            Row::new(vec![
                Cell::from(prov_str),
                Cell::from(acct_str),
                Cell::from(status_str),
            ])
            .style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Length(col_prov_w),
        Constraint::Min(col_acct_w.max(10)),
        Constraint::Length(col_status_w),
    ];

    let table_h = area.height.saturating_sub(1).max(1);
    let table_area = Rect { x: area.x, y: area.y, width: area.width, height: table_h };
    let btn_area = Rect { x: area.x, y: area.y + table_h, width: area.width, height: 1 };

    let table = Table::new(rows, widths).header(header);
    frame.render_widget(table, table_area);

    let on_btn = st.in_detail && st.oauth_on_add_button();
    let btn_style = if on_btn {
        Style::default().fg(palette.sel_fg).bg(palette.sel_bg)
    } else {
        Style::default().fg(palette.accent)
    };
    frame.render_widget(
        Paragraph::new(Span::styled("[ + connect ]", btn_style)),
        btn_area,
    );
}

/// Provider picker overlay: an inline 3-option list, cursor-highlighted.
fn draw_picker(frame: &mut Frame, cursor: usize, palette: &Palette, area: Rect) {
    const OPTIONS: [&str; 3] = ["Codex", "Kilo Code", "Codex (paste token)"];
    let lines: Vec<Line> = OPTIONS
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let (marker, style) = if i == cursor {
                ("› ", Style::default().fg(palette.sel_fg).bg(palette.sel_bg))
            } else {
                ("  ", Style::default().fg(palette.fg))
            };
            Line::from(vec![
                Span::styled(marker, Style::default().fg(palette.accent)),
                Span::styled(*label, style),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// A one-line status/spinner message plus an optional URL/code line beneath it —
/// shared shape for `Starting`/`CodexWait`/`KiloWait`. `copied` shows a dim
/// confirmation line under the URL after a successful `c` (copy-url) press.
fn draw_message(
    frame: &mut Frame,
    palette: &Palette,
    area: Rect,
    headline: &str,
    url: Option<&str>,
    copied: bool,
) {
    let mut lines = vec![Line::from(Span::styled(
        headline.to_string(),
        Style::default().fg(palette.accent),
    ))];
    if let Some(u) = url {
        if !u.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                u.to_string(),
                Style::default().fg(palette.dim),
            )));
            if copied {
                lines.push(Line::from(Span::styled(
                    "url copied to clipboard",
                    Style::default().fg(palette.dim),
                )));
            }
        }
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

/// The manual paste-token screen: a single input line with a trailing cursor.
fn draw_paste(frame: &mut Frame, input: &str, palette: &Palette, area: Rect) {
    let line = Line::from(vec![
        Span::styled("token: ", Style::default().fg(palette.dim)),
        Span::styled(input.to_string(), Style::default().fg(palette.fg)),
        Span::styled("█", Style::default().fg(palette.accent)),
    ]);
    frame.render_widget(Paragraph::new(vec![line]).wrap(Wrap { trim: false }), area);
}

/// Failure screen: the error line in red plus a dismiss hint.
fn draw_failed(frame: &mut Frame, msg: &str, palette: &Palette, area: Rect) {
    let lines = vec![
        Line::from(Span::styled(msg.to_string(), Style::default().fg(Color::Red))),
        Line::from(""),
        Line::from(Span::styled(
            "enter/esc dismiss",
            Style::default().fg(palette.dim),
        )),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}
