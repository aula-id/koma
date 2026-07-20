//! View – in-app agents management dashboard (Agents mode).
//!
//! Two-pane layout, mirroring `/settings`: a narrow sidebar LISTs every agent
//! (with a source tag); the detail pane on the right shows the selected agent
//! read-only (Browse), an editable field form (Edit/Create), or a confirm
//! prompt (DeleteConfirm). A context-sensitive footer shows key hints.
//!
//! Border convention (strict, matches project rules + `/settings`):
//! - Header: `Borders::BOTTOM` only.
//! - List/detail divider: `Borders::RIGHT` on the list pane.
//! - Footer: plain dim line (no full box anywhere).
//!
//! ```text
//!  agents
//! ─────────────────────────────────────────────────────────
//! │ explore  built-in │  name         my-agent
//! │ general  built-in │  description  Does the thing
//! │ my-agent session  │  model        (inherit main)
//!                     │  prompt       You are a focused subagent…
//!
//!  ↑/↓ pick · →/Enter edit · n new · d delete · Esc close
//! ```
//!
//! All draft mutation lives in [`crate::app::mode::AgentsState`]; key handling
//! lives in [`crate::controller::input::handle_agents`].

mod browse;
mod editor;
mod pickers;

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::mode::AgentsState;
use crate::app::state::AppStateRest;
use crate::model::app_config::AppConfig;
use crate::model::settings::Settings;
use crate::view::theme::Palette;

use browse::{draw_detail, draw_list, footer_hint};
use editor::draw_field_editor;
use pickers::{draw_model_picker, draw_tool_picker};

/// List (sidebar) column width in terminal columns (includes the RIGHT border).
const SIDEBAR_W: u16 = 26;

/// Truncate `s` to at most `max` chars, appending `…` if cut.
pub(crate) fn truncate(s: &str, max: usize) -> String {
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

/// The display label for the agent's chosen registered model in a detail/browse
/// row: the entry's `name @ provider` when `model_uuid` resolves to a registered
/// model, an `(unknown model)` note when the uuid dangles, or `(inherit main)`
/// (with a legacy hint appended when an old file still carries a free-text
/// `model` slug). Returns `(text, is_chosen)` so the caller can dim "inherit".
pub(crate) fn model_display(
    config: &AppConfig,
    settings: Option<&Settings>,
    model_uuid: &Option<String>,
    legacy: &Option<String>,
) -> (String, bool) {
    match model_uuid {
        Some(uuid) => {
            let entry = settings
                .and_then(|s| s.session_models.iter().find(|e| &e.uuid == uuid))
                .or_else(|| config.models.iter().find(|e| &e.uuid == uuid));
            match entry {
                Some(e) => {
                    // Mirror runtime resolution: static providers first, then
                    // oauth_conns. Looking only at providers made every OAuth-
                    // backed model render as "name @ ?" in the agents UI.
                    let provider = if let Some(p) =
                        config.providers.iter().find(|p| p.uuid == e.provider_uuid)
                    {
                        if !p.name.trim().is_empty() {
                            p.name.clone()
                        } else if !p.endpoint.trim().is_empty() {
                            p.endpoint.clone()
                        } else {
                            "?".to_string()
                        }
                    } else if let Some(c) =
                        config.oauth_conns.iter().find(|c| c.uuid == e.provider_uuid)
                    {
                        if !c.name.trim().is_empty() {
                            c.name.clone()
                        } else {
                            let short: String = c.uuid.chars().take(8).collect();
                            format!("{} ({short})", c.provider.label())
                        }
                    } else {
                        "?".to_string()
                    };
                    (format!("{} @ {}", e.name, provider), true)
                }
                None => ("(unknown model)".to_string(), true),
            }
        }
        None => match legacy {
            Some(m) if !m.trim().is_empty() => {
                (format!("(inherit main)  was: {m}"), false)
            }
            _ => ("(inherit main)".to_string(), false),
        },
    }
}

/// Render the agents dashboard for `st` using the given colour `palette`.
///
/// `config` supplies the registered-model catalogue + API providers (to resolve a
/// chosen model entry to its `name @ provider` label), and `settings` the active
/// session's settings (whose `session_models` are the per-session registered
/// models, listed first in the model picker). Both are threaded down to the
/// detail/editor rows and the model picker.
///
/// All colours flow through `palette` — no hardcoded `Color::` values.
pub fn draw(
    frame: &mut Frame,
    rest: &AppStateRest,
    st: &AgentsState,
    config: &AppConfig,
    settings: Option<&Settings>,
    palette: &Palette,
) {
    // The full-screen field editor takes over the WHOLE frame when open: render it
    // instead of the normal list/detail dashboard and bail (it owns all input). The
    // title is the active field's label (prompt / description / conditions).
    if let Some((field, ed)) = &st.editor {
        draw_field_editor(frame, ed, field.label(), st.editor_clear_confirm, palette);
        return;
    }

    // Outer vertical zones: header | body | footer.
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // header text + BOTTOM border
            Constraint::Min(0),    // list + detail
            Constraint::Length(1), // footer key hints
        ])
        .split(frame.area());

    // --- Header ---
    let header_block = Block::new()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(palette.dim));
    let header_inner = header_block.inner(outer[0]);
    frame.render_widget(header_block, outer[0]);
    frame.render_widget(
        Paragraph::new(Span::styled("agents", Style::default().fg(palette.dim))),
        header_inner.inner(Margin { horizontal: 2, vertical: 0 }),
    );

    // --- Body: list sidebar + detail pane ---
    let body_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(SIDEBAR_W), // list with RIGHT border as divider
            Constraint::Min(0),            // detail pane
        ])
        .split(outer[1]);

    draw_list(frame, st, palette, body_cols[0]);
    draw_detail(frame, st, config, settings, palette, body_cols[1]);

    // --- Footer ---
    // Full-width inverse status bar: background fills the entire footer line
    // edge to edge; text is left-padded by 1 space so it doesn't touch the edge.
    let footer_rect = outer[2];
    if footer_rect.width > 0 {
        let hint = footer_hint(st);
        let bar_style = Style::default()
            .fg(palette.sel_fg)
            .bg(palette.sel_bg)
            .add_modifier(Modifier::BOLD);
        // Pad the hint with a leading space, then right-pad to the full width so
        // the Paragraph's base style (bar_style) paints the background edge to edge.
        let padded = format!(" {:<width$}", hint, width = footer_rect.width.saturating_sub(1) as usize);
        frame.render_widget(
            Paragraph::new(Line::from(Span::raw(padded))).style(bar_style),
            footer_rect,
        );
    }

    // --- Tool picker overlay (rendered on top of everything else) ---
    if let Some(picker) = &st.tool_picker {
        draw_tool_picker(frame, rest, picker, palette, frame.area());
    }

    // --- Model picker overlay (rendered last; only one modal open at a time) ---
    if let Some(picker) = &st.model_picker {
        draw_model_picker(frame, rest, picker, palette, frame.area());
    }
}
