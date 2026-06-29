//! View – in-app MCP server management dashboard (Mcp mode).
//!
//! A simpler clone of the `/agents` dashboard: a narrow sidebar LISTs every
//! configured MCP server (enabled dot + transport tag); the detail pane on the
//! right shows the selected server read-only (Browse), an editable field form
//! (Edit/Create), or a confirm prompt (DeleteConfirm). A context-sensitive footer
//! shows key hints. There are no pickers and no full-screen body editor.
//!
//! Border convention (strict, matches project rules + `/agents`):
//! - Header: `Borders::BOTTOM` only.
//! - List/detail divider: `Borders::RIGHT` on the list pane.
//! - Footer: full-width inverse status bar (same as `/agents`).
//!
//! ```text
//!  mcp servers
//! ─────────────────────────────────────────────────────────
//! │ context7   ● stdio │  name       context7
//! │ github     ● http  │  enabled    yes
//! │ local      ○ stdio │  transport  stdio
//!                      │  status     ● 12 tools
//!                      │  command    npx
//!
//!  ↑/↓ pick · →/Enter edit · n new · d delete · Esc close
//! ```
//!
//! All draft mutation lives in [`crate::app::mode::McpState`]; key handling lives
//! in [`crate::controller::input::mcp::handle_mcp`].

mod browse;

use std::collections::HashMap;

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::mode::McpState;
use crate::view::theme::Palette;

use browse::{draw_detail, draw_list, footer_hint};

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

/// Render the MCP dashboard for `st` using the given colour `palette`.
///
/// `status` is the live per-server tool count from the
/// [`McpManager`](crate::app::mcp::McpManager) snapshot (server uuid -> tool
/// count), or `None` when no manager exists. It feeds the LIST + detail status
/// display (`● N tools` / `○ —`).
///
/// All colours flow through `palette` — no hardcoded `Color::` values.
pub fn draw(
    frame: &mut Frame,
    st: &McpState,
    status: Option<&HashMap<String, usize>>,
    palette: &Palette,
) {
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
        Paragraph::new(Span::styled(
            "mcp servers",
            Style::default().fg(palette.dim),
        )),
        header_inner.inner(Margin {
            horizontal: 2,
            vertical: 0,
        }),
    );

    // --- Body: list sidebar + detail pane ---
    let body_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(SIDEBAR_W), // list with RIGHT border as divider
            Constraint::Min(0),            // detail pane
        ])
        .split(outer[1]);

    draw_list(frame, st, status, palette, body_cols[0]);
    draw_detail(frame, st, status, palette, body_cols[1]);

    // --- Footer: full-width inverse status bar (matches /agents). ---
    let footer_rect = outer[2];
    if footer_rect.width > 0 {
        let hint = footer_hint(st);
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
