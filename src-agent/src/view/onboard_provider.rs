//! View – guided PROVIDER onboarding wizard (`Mode::OnboardProvider`).
//!
//! Minimalist, top-down, no full box — matching [`crate::view::onboard`] and the
//! border-style convention. The Login step REUSES the `/settings` OAuth connect-flow
//! sub-renderers (picker / wait / paste / failed); the ModelSelect step renders a
//! type-to-filter model list. The filtered candidate ids are recomputed here from the
//! on-demand catalogue (`candidate_model_ids`), exactly as the `/settings` model
//! omnisearch does — so both the local TUI (live cache) and the thin client (projected
//! `models_cache` + compiled-in Codex static list) render identically.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::app::mode::onboard_provider::{
    candidate_model_ids, OnboardProviderState, OnboardProviderStep,
};
use crate::app::mode::settings::OAuthFlowState;
use crate::dto::openrouter::ModelInfo;
use crate::view::settings::oauth::{draw_failed, draw_message, draw_paste, draw_picker};
use crate::view::theme::Palette;

/// Total width (chars) of the content block. Clamped to the available area.
const BLOCK_W: u16 = 64;

/// Braille spinner frames (matches the `/settings` OAuth wait screens).
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Render the wizard for `state`.
pub fn draw(
    frame: &mut Frame,
    state: &OnboardProviderState,
    cache: &[ModelInfo],
    cache_endpoint: Option<&str>,
    palette: &Palette,
) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(20), Constraint::Min(1)])
        .split(area);

    let block_w = BLOCK_W.min(area.width);
    let bx = area.width.saturating_sub(block_w) / 2;
    let body = Rect {
        x: bx,
        y: chunks[1].y,
        width: block_w,
        height: chunks[1].height,
    };
    if body.height == 0 || body.width == 0 {
        return;
    }

    // Title.
    put_line(
        frame,
        body,
        0,
        Line::from(Span::styled("koma", Style::default().fg(palette.accent))),
    );

    match state.step {
        OnboardProviderStep::Login => {
            put_line(frame, body, 2, dim_line("sign in to a provider account", palette));
            // Content region for the reused sub-renderer (rows 4 .. footer).
            let content = sub_rect(body, 4, 1);
            draw_login(frame, &state.oauth_flow, palette, content);
            put_line(
                frame,
                body,
                body.height.saturating_sub(1),
                dim_line(login_hint(&state.oauth_flow), palette),
            );
        }
        OnboardProviderStep::ModelSelect => {
            put_line(frame, body, 2, dim_line("select a model", palette));
            // Omnisearch query line with a trailing cursor.
            put_line(
                frame,
                body,
                4,
                Line::from(vec![
                    Span::styled("search: ", Style::default().fg(palette.dim)),
                    Span::styled(state.query.clone(), Style::default().fg(palette.fg)),
                    Span::styled("█", Style::default().fg(palette.accent)),
                ]),
            );
            let ids = candidate_model_ids(state.provider, &state.query, cache, cache_endpoint);
            draw_model_list(frame, body, 6, &ids, state.result_sel, palette);
            put_line(
                frame,
                body,
                body.height.saturating_sub(1),
                dim_line("up/down move · type to filter · enter select · esc back", palette),
            );
        }
    }
}

/// Login step content: dispatch the reused OAuth sub-renderers off `flow`.
fn draw_login(frame: &mut Frame, flow: &OAuthFlowState, palette: &Palette, area: Rect) {
    if area.height == 0 {
        return;
    }
    match flow {
        // No idle "connections list" in the wizard — Idle falls back to the picker.
        OAuthFlowState::Idle => draw_picker(frame, 0, palette, area),
        OAuthFlowState::Pick(cursor) => draw_picker(frame, *cursor, palette, area),
        OAuthFlowState::Starting => draw_message(
            frame,
            palette,
            area,
            &format!("{} starting login…", SPINNER[0]),
            None,
            false,
        ),
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

/// Footer key hint for the current Login sub-state.
fn login_hint(flow: &OAuthFlowState) -> &'static str {
    match flow {
        OAuthFlowState::Pick(_) | OAuthFlowState::Idle => "up/down move · enter select · esc back",
        OAuthFlowState::CodexPaste { .. } => "enter save · esc back",
        OAuthFlowState::Failed(_) => "enter/esc dismiss",
        _ => "esc cancel · c copy url · o open in browser",
    }
}

/// Render the filtered model list with a windowed, highlighted selection. Reserves
/// the last body row for the footer; scrolls so the selection stays on screen.
fn draw_model_list(
    frame: &mut Frame,
    body: Rect,
    start_row: u16,
    ids: &[String],
    sel: usize,
    palette: &Palette,
) {
    let avail = body.height.saturating_sub(start_row).saturating_sub(1);
    if avail == 0 {
        return;
    }
    if ids.is_empty() {
        put_line(
            frame,
            body,
            start_row,
            dim_line("no models — type to search, or press enter to use the typed id", palette),
        );
        return;
    }
    let max_rows = avail as usize;
    let sel = sel.min(ids.len() - 1);
    // Scroll so the selection stays visible (bottom-anchored window).
    let start = if sel >= max_rows { sel + 1 - max_rows } else { 0 };
    for (row, i) in (start..ids.len()).take(max_rows).enumerate() {
        let (marker, style) = if i == sel {
            ("› ", Style::default().fg(palette.sel_fg).bg(palette.sel_bg))
        } else {
            ("  ", Style::default().fg(palette.fg))
        };
        let line = Line::from(vec![
            Span::styled(marker, Style::default().fg(palette.accent)),
            Span::styled(ids[i].clone(), style),
        ]);
        put_line(frame, body, start_row + row as u16, line);
    }
}

/// Render one line at row `r` within `body` (no-op when `r` is out of range).
fn put_line(frame: &mut Frame, body: Rect, r: u16, line: Line) {
    if r >= body.height {
        return;
    }
    let rect = Rect {
        x: body.x,
        y: body.y + r,
        width: body.width,
        height: 1,
    };
    frame.render_widget(Paragraph::new(line), rect);
}

/// A dim single-line span (owned, so it outlives the borrow of `text`).
fn dim_line(text: &str, palette: &Palette) -> Line<'static> {
    Line::from(Span::styled(text.to_string(), Style::default().fg(palette.dim)))
}

/// A sub-rect of `body` starting `top` rows down, reserving `bottom` rows at the end.
fn sub_rect(body: Rect, top: u16, bottom: u16) -> Rect {
    let y = body.y + top.min(body.height);
    let used = top.saturating_add(bottom);
    Rect {
        x: body.x,
        y,
        width: body.width,
        height: body.height.saturating_sub(used),
    }
}
