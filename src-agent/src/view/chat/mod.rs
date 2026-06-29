//! Chat screen renderer: the read-only view of [`AppStateRest`].
//!
//! Last stage of the keystroke -> Action -> state -> render flow. Pure
//! function of state: it borrows the session transcript, the live streaming
//! buffer, the input, and the status line into a structured layout:
//!
//! ```text
//! simple-coder · {name} [{model}]          ← header (1 line)
//! ─────────────────────────────────────────  ← dim bottom border
//! [messages / transcript area]             ← scrollable, fills space,
//!                                             auto-follows the bottom
//! ─────────────────────────────────────────  ← dim top border (input block)
//! › {input buffer}                         ← input line (1 line)
//! ─────────────────────────────────────────  ← dim bottom border (input block)
//! {status}                                 ← status line (1 line)
//! ```
//!
//! The header has a dim bottom border and horizontal padding of 2. The input
//! has dim top + bottom borders and horizontal padding of 2. The transcript
//! itself is flat (no borders); structure comes from spacing and colour.
//!
//! Each message is rendered as a block with a coloured bullet on the first
//! line and continuation lines hanging-indented (2 cols) under the text.
//! A blank line separates consecutive blocks. The transcript auto-follows the
//! bottom as responses stream; scrolling up pauses following, reaching the
//! bottom resumes it.
//!
//! Assistant replies render as **Markdown** (headings, lists, bold, inline
//! `code`, boxed and syntax-highlighted fenced code blocks, and aligned GFM
//! tables) via the in-tree [`crate::view::markdown`] renderer, built on
//! `pulldown-cmark` and `syntect`. It returns FINAL visual lines (already
//! wrapped/boxed/aligned) while the bullet and hanging indent use the palette.
//! User messages and the live streaming buffer stay plain (the buffer renders
//! plain for perf and partial-fence safety). Everything is pre-wrapped — the
//! markdown included — so the emitted line count equals the exact visual line
//! count, which the follow-scroll math depends on.
//!
//! - User messages   → `★` in `palette.accent`, plain text
//! - AI messages     → `●` in `palette.fg`, markdown body
//! - Streaming buffer → `●` in `palette.fg`, plain text
//! - System / status → `palette.dim`
//!
//! When the user types `/` followed by a command name with no whitespace, a
//! bordered popup palette floats above the input row listing the matching
//! commands filtered in real time. Up/Down navigate the list; Tab completes.

mod header;
mod helpers;
mod input;
mod overlays;
mod status;
mod subagents;
mod transcript;

use ratatui::{
    layout::{Constraint, Direction, Layout},
    Frame,
};
use crate::app::state::AppStateRest;
use crate::view::theme::Palette;

/// Render the chat screen from `rest` using the given colour `palette`.
///
/// `resolved_model` is the concrete model id that will actually be sent for
/// chat requests (already resolved through session overrides and the global
/// catalogue by `view::draw`). It is used in the header so the displayed
/// model always matches what the request layer will use.
///
/// Borrows throughout — no per-frame clones of the transcript or streaming
/// buffer. The header has a dim bottom border + padding; the input has dim
/// top + bottom borders + padding; the transcript is flat.
pub fn draw(frame: &mut Frame, rest: &AppStateRest, resolved_model: &str, palette: &Palette) {
    // --- Full-screen sub-agent viewer ---
    // When the `$`-panel viewer is open, it OWNS the whole frame (like the nano
    // prompt editor): short-circuit the normal chat draw and render only the
    // selected sub-agent's conversation, view-only.
    if let Some(idx) = rest.agent_viewer {
        subagents::render_agent_viewer(frame, rest, idx, palette);
        return;
    }

    // --- Input height ---
    // The input box grows to fit its wrapped content (capped). Compute the row
    // count BEFORE the layout split so the layout can reserve the right height.
    let input_rows = input::input_row_count(rest, frame.area().width, frame.area().height);
    let input_h = (input_rows as u16) + 2; // + top & bottom borders

    // Layout: header (text + bottom rule) | transcript | model name row |
    // input (top+bottom rules) | status. Header/input get thin dim borders so
    // the screen reads as structured, not boxed; the transcript stays flat.
    // The model-name row is a single dim right-aligned line sitting directly
    // above the input's top border (no extra gap — it reads as a label for it).
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),       // header line + bottom border
            Constraint::Min(1),          // transcript
            Constraint::Length(1),       // model name (right-aligned, dim)
            Constraint::Length(input_h), // top border + input row(s) + bottom border
            Constraint::Length(1),       // status bar
        ])
        .split(frame.area());

    // --- Header ---
    header::render_header(frame, chunks[0], rest, palette);

    // --- Model name row ---
    header::render_model_row(frame, chunks[2], rest, resolved_model, palette);

    // --- Transcript ---
    transcript::render_transcript(frame, chunks[1], rest, palette);

    // --- Input box / compaction animation ---
    input::render_input(frame, chunks[3], rest, palette);

    // --- Status bar ---
    status::render_status(frame, chunks[4], rest, palette);

    // --- Slash command palette ---
    let cmd_palette_active = overlays::render_command_palette(
        frame, chunks[3], chunks[1], rest, palette,
    );

    // --- File reference palette --- only when the command palette is NOT active.
    if !cmd_palette_active {
        overlays::render_file_palette(frame, chunks[3], chunks[1], rest, palette);
    }

    // --- Sub-agents panel ---
    if rest.subagents_open {
        subagents::render_subagents_panel(frame, chunks[3], chunks[1], rest, palette);
    }

    // --- Toast ---
    overlays::render_toast(frame, chunks[1], rest, palette);

    // --- Tool-approval prompt ---
    if rest.fg().awaiting_approval {
        overlays::render_tool_approval(frame, chunks[3], chunks[1], rest, palette);
    }
}
