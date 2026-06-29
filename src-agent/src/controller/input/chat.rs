//! Key handler for the normal Chat mode.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crate::app::state::AppStateRest;
use crate::controller::command;
use super::{is_ctrl, Action};

/// If the input's current (last, whitespace-delimited) token is a file
/// reference (`@...`), return the partial path after the `@`. The file palette
/// is shown while this is `Some`.
pub fn file_ref_partial(input: &str) -> Option<&str> {
    let start = input.rfind([' ', '\n']).map(|i| i + 1).unwrap_or(0);
    let token = &input[start..];
    token.strip_prefix('@')
}

/// Replace the current `@token` in `rest.input` with the selected entry.
/// Completing a FILE appends a trailing space (closes the palette).
/// Completing a FOLDER (trailing `/`) does NOT append a space so the palette
/// stays open and the user can browse into the subfolder.
///
/// When the selected file has a recognised image extension AND the session has
/// an images directory, the pick is routed through the ingest core instead:
/// the `@token` is erased, an `[Image #N]` marker is inserted at the caret,
/// and a trailing space closes the palette — exactly like path-paste.  Non-
/// image picks and folder picks behave exactly as before.
fn complete_file_ref(rest: &mut AppStateRest, matches: &[String]) {
    let sel = rest.palette_sel.min(matches.len().saturating_sub(1));
    let entry = &matches[sel];

    // Folders: never attach; keep the palette open at the new depth.
    if entry.ends_with('/') {
        let start = rest.input.rfind([' ', '\n']).map(|i| i + 1).unwrap_or(0);
        rest.input.truncate(start);
        rest.input.push('@');
        rest.input.push_str(entry);
        rest.palette_sel = 0;
        rest.cursor_end();
        return;
    }

    // Strip a leading workspace prefix `[N]` to get the bare relative path.
    // Single-workspace entries have no prefix (bare relative path).
    let (ws_idx, bare) = if let Some(rest_after_bracket) = entry.strip_prefix('[') {
        if let Some(end) = rest_after_bracket.find(']') {
            let idx = rest_after_bracket[..end].parse::<usize>().unwrap_or(0);
            (idx, &rest_after_bracket[end + 1..])
        } else {
            (0usize, entry.as_str())
        }
    } else {
        (0usize, entry.as_str())
    };

    // Image file: ingest through the same core as path-paste.
    if crate::model::attachment::has_image_extension(bare) {
        // Resolve the absolute path for this workspace entry.
        let abs_path: Option<std::path::PathBuf> = rest
            .fg()
            .session
            .as_ref()
            .map(|s| {
                let dirs = s.workdirs();
                let root = dirs.get(ws_idx).or_else(|| dirs.first()).cloned()
                    .unwrap_or_else(|| std::path::PathBuf::from("."));
                root.join(bare)
            });

        if let Some(abs) = abs_path {
            // Erase the `@token` from input and park the caret at that position
            // BEFORE calling insert_marker (which inserts at the current cursor).
            let start = rest.input.rfind([' ', '\n']).map(|i| i + 1).unwrap_or(0);
            rest.input.truncate(start);
            rest.cursor = rest.input.chars().count(); // char index after truncation
            rest.palette_sel = 0;
            rest.hist_idx = None;

            if rest.try_attach_image_path(&abs.to_string_lossy()) {
                // Marker was inserted; add a trailing space to close the palette.
                rest.push_char(' ');
                rest.cursor_end();
                return;
            }
            // Ingest failed (file missing / not an image / write error): fall
            // through to the normal `@entry ` insertion below.
        }
    }

    // Default path: insert `@entry ` (file) into the input.
    let start = rest.input.rfind([' ', '\n']).map(|i| i + 1).unwrap_or(0);
    rest.input.truncate(start);
    rest.input.push('@');
    rest.input.push_str(entry);
    rest.input.push(' '); // a FILE completion always closes the palette
    rest.palette_sel = 0;
    // The input was rewritten wholesale; park the caret at the end.
    rest.cursor_end();
}

/// This session's sent user messages, oldest-first (for bash-style recall).
fn user_messages(rest: &AppStateRest) -> Vec<String> {
    rest.fg().session
        .as_ref()
        .map(|s| {
            s.conversation
                .messages()
                .iter()
                .filter(|m| m.role == crate::dto::chat::Role::User)
                .map(|m| m.content.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// Handle a key press while the app is in Chat mode.
///
/// Ctrl+C and Esc both interrupt an in-flight request when `waiting` is true;
/// when idle they quit the app.  Ctrl+R re-sends the last message (idle only).
pub fn handle_chat(rest: &mut AppStateRest, key: KeyEvent) -> Action {
    // The full-screen sub-agent VIEWER is the most modal surface: while it's open
    // every key routes to it. Up/Down/PgUp scroll; Esc closes it back to the still-
    // open `$` panel; everything else is swallowed so nothing leaks underneath.
    if rest.agent_viewer.is_some() {
        match key.code {
            KeyCode::Up => rest.agent_viewer_scroll_up(1),
            KeyCode::Down => rest.agent_viewer_scroll_down(1),
            KeyCode::PageUp => rest.agent_viewer_scroll_up(10),
            KeyCode::PageDown => rest.agent_viewer_scroll_down(10),
            KeyCode::Esc => rest.agent_viewer = None, // back to the `$` panel
            _ => {}
        }
        return Action::None;
    }

    // The sub-agents panel is modal: Up/Down move the selection, Enter opens the
    // full-screen viewer for a spawned row, Ctrl+X kills the selected one (abrupt
    // abort), Esc or any other key closes it. Mirrors the help-overlay modal
    // handling above.
    if rest.subagents_open {
        let count = rest.fg().subagents.len();
        if is_ctrl(&key, 'x') {
            let sel = rest.subagent_sel;
            if let Some(sa) = rest.fg_mut().subagents.get_mut(sel) {
                sa.abort.abort();
                sa.status = crate::app::subagent::SubAgentStatus::Killed;
            }
            return Action::None;
        }
        match key.code {
            KeyCode::Up => {
                rest.subagent_sel = rest.subagent_sel.saturating_sub(1);
            }
            KeyCode::Down => {
                if count > 0 {
                    rest.subagent_sel = (rest.subagent_sel + 1).min(count - 1);
                }
            }
            // Enter opens the full-screen viewer for the selected SPAWNED sub-agent
            // (any of running/done/killed/error — it has structured `messages`).
            // The `$` panel only ever selects spawned rows (`subagent_sel` indexes
            // `subagents`); queued/PENDING delegations aren't selectable here, so a
            // valid selection is always a spawned agent. An empty list (nothing
            // spawned yet, only pending) just shows a status note.
            KeyCode::Enter => {
                if rest.subagent_sel < count {
                    rest.agent_viewer = Some(rest.subagent_sel);
                    rest.agent_viewer_scroll = 0;
                    rest.agent_viewer_follow = true; // open pinned to the bottom
                } else {
                    rest.status = "sub-agent queued — not started yet".into();
                }
            }
            // Esc or any non-nav key closes the panel.
            _ => {
                rest.subagents_open = false;
            }
        }
        return Action::None;
    }

    // Tool-approval modal: while a risky call is paused, only y/n/Esc matter.
    // `y` approves (run it), `n`/Esc deny (feed "denied by user"); every other
    // key is swallowed so the prompt stays up and input can't leak underneath.
    if rest.fg().awaiting_approval {
        return match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => Action::ApproveTool,
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Action::DenyTool,
            _ => Action::None,
        };
    }

    // Ctrl+C: interrupt if waiting OR a compaction animation is in flight
    // (the animation keeps `compact_anim_start` set while the deferred apply
    // is pending, and `waiting` may have already cleared if the model replied
    // fast). Never quit mid-animation — that would leave the spinner stuck.
    if is_ctrl(&key, 'c') {
        return if rest.fg().waiting || rest.compact_anim_start.is_some() {
            Action::Interrupt
        } else {
            Action::None
        };
    }
    // Ctrl+R: resend (only when idle).
    if is_ctrl(&key, 'r') {
        return if rest.fg().waiting {
            Action::None
        } else {
            Action::Resend
        };
    }
    // Ctrl+J: insert a newline (reliable multiline trigger; unlike Shift+Enter
    // this works on every terminal since Ctrl+J is literally the LF control code).
    if is_ctrl(&key, 'j') {
        rest.push_char('\n');
        return Action::None;
    }
    // Ctrl+V: paste image from the OS clipboard (Wayland / X11). Shells out to
    // `wl-paste --type image/png` or `xclip` on a background thread (non-blocking);
    // the result lands in `rest.clipboard_rx` and is drained by the event loop on
    // the next tick. No-op when a fetch is already in flight or when waiting for a
    // model response (to avoid attaching images mid-turn).
    if is_ctrl(&key, 'v') {
        if !rest.fg().waiting {
            super::request_clipboard_image(rest);
            rest.set_toast_info("reading image from clipboard…".to_string());
        }
        return Action::None;
    }
    // Ctrl+E: toggle internet mode (Simple <-> Full), persist, and set status.
    // Session is directly on AppStateRest so this can be done inline, matching
    // the BackTab agent_mode toggle pattern.
    if is_ctrl(&key, 'e') {
        if let Some(sess) = rest.fg_mut().session.as_mut() {
            let new_mode = sess.settings.internet_mode.toggled();
            sess.settings.internet_mode = new_mode;
            // Refresh the system-prompt roster so any mode-gated agents stay in
            // sync on this mid-session flip (rebuild reads in-memory settings).
            sess.rebuild_system();
            match sess.save() {
                Ok(()) => {
                    let (status, toast) =
                        crate::app::runtime::commands::internet::internet_feedback(new_mode);
                    rest.status = status;
                    if let Some(t) = toast {
                        rest.set_toast_info(t);
                    }
                }
                Err(e) => {
                    rest.set_toast(format!("error saving settings: {e}"));
                }
            }
        } else {
            rest.set_toast("no active session".to_string());
        }
        return Action::None;
    }

    // Max visible entries in the `@` file-reference palette (shared across all
    // key handlers in this function and kept in sync with the view constant).
    const FILE_PAL_MAX: usize = 10;

    match key.code {
        KeyCode::Esc => {
            // Interrupt if waiting OR a compaction animation is still running
            // (compact_anim_start remains set during the deferred-apply window).
            if rest.fg().waiting || rest.compact_anim_start.is_some() {
                // A live Esc cancels the in-flight turn; clear any pending
                // idle-Esc so it can't pair with a later one across this turn.
                rest.last_esc = None;
                return Action::Interrupt;
            }
            // Idle Esc: double-Esc within ~400ms opens the message-rewind
            // picker ("edit a previous message"); a lone Esc is otherwise a
            // no-op (only /quit exits the app). Pair the two presses by
            // recording the first's instant and comparing against it.
            const DOUBLE_ESC_MS: u128 = 400;
            let now = std::time::Instant::now();
            match rest.last_esc.take() {
                Some(prev) if now.duration_since(prev).as_millis() <= DOUBLE_ESC_MS => {
                    // Second Esc in time: open the picker. The runtime builds
                    // the RewindState and refuses to enter with no user message.
                    Action::OpenRewind
                }
                _ => {
                    // First Esc (or the previous one was too long ago): arm the
                    // timer and wait for a possible second press.
                    rest.last_esc = Some(now);
                    Action::None
                }
            }
        }
        KeyCode::Enter => {
            // Shift+Enter inserts a newline instead of submitting — but only when
            // the terminal actually reports the SHIFT modifier on Enter (many do
            // not). Ctrl+J above is the always-works fallback. Plain Enter falls
            // through to the palette/slash/submit logic unchanged.
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                rest.push_char('\n');
                return Action::None;
            }
            let cmd_matches = command::palette_matches(&rest.input);
            if !cmd_matches.is_empty() {
                // Command palette open: run the highlighted command, not the raw text.
                let sel = rest.palette_sel.min(cmd_matches.len() - 1);
                let name = cmd_matches[sel].0;
                rest.take_input();
                // Slash command (not submit): discard staged attachments so
                // they can't leak into the next message.
                rest.pending_attachments.clear();
                Action::Slash(command::parse(name))
            } else {
                // File palette: complete instead of submitting when a file match is selected.
                let fmatches: Vec<String> = file_ref_partial(&rest.input)
                    .map(|p| rest.fg().dir_cache.read().map(|c| c.search(p, FILE_PAL_MAX)).unwrap_or_default())
                    .unwrap_or_default();
                if !fmatches.is_empty() {
                    complete_file_ref(rest, &fmatches);
                    Action::None
                } else if rest.input.trim().starts_with('/') {
                    let line = rest.take_input();
                    // Slash command (not submit): discard staged attachments.
                    rest.pending_attachments.clear();
                    Action::Slash(command::parse(&line))
                } else if rest.input.trim_start().starts_with('!') {
                    // `!` user-shell shortcut: run the rest of the line directly in
                    // the session cwd (no model round-trip). Strip the leading `!` +
                    // trim. An empty command (a lone `!`) is a no-op. Discard staged
                    // attachments — they belong to a message, not a shell run.
                    //
                    // Busy guard (mirrors the Submit arm below): if a turn is
                    // streaming OR a previous `!` is still draining off-thread, do
                    // NOT route the shell — running it now would push a shell entry
                    // mid-turn, interleaving it between the pending question and the
                    // not-yet-committed reply ([user, shell, assistant] on the next
                    // send). Leave the input intact (no `take_input`) so the typed
                    // command is preserved, and consume the keystroke as a no-op.
                    if rest.fg().waiting || rest.fg().awaiting_shell {
                        Action::None
                    } else {
                        let line = rest.take_input();
                        rest.pending_attachments.clear();
                        let cmd = line.trim_start().strip_prefix('!').unwrap_or("").trim();
                        if cmd.is_empty() {
                            Action::None
                        } else {
                            Action::Shell(cmd.to_string())
                        }
                    }
                } else if !rest.input.trim().is_empty() && !rest.fg().waiting && !rest.fg().awaiting_shell {
                    Action::Submit(rest.take_input())
                } else {
                    Action::None
                }
            }
        }
        KeyCode::Backspace => {
            rest.backspace();
            Action::None
        }
        // Caret movement within the input line (mid-text editing). Left/Right
        // step one char; Home jumps to the start. End is handled below (it also
        // doubles as "scroll to bottom" when the input is empty).
        KeyCode::Left => {
            rest.cursor_left();
            Action::None
        }
        KeyCode::Right => {
            rest.cursor_right();
            Action::None
        }
        KeyCode::Home => {
            rest.cursor_home();
            Action::None
        }
        KeyCode::Up => {
            // Command palette takes precedence; then file palette; then within-input
            // line movement; finally history recall (only when already on line 0).
            if !command::palette_matches(&rest.input).is_empty() {
                rest.palette_sel = rest.palette_sel.saturating_sub(1);
            } else {
                let fmatches: Vec<String> = file_ref_partial(&rest.input)
                    .map(|p| rest.fg().dir_cache.read().map(|c| c.search(p, FILE_PAL_MAX)).unwrap_or_default())
                    .unwrap_or_default();
                if !fmatches.is_empty() {
                    rest.palette_sel = rest.palette_sel.saturating_sub(1);
                } else if !rest.cursor_up() {
                    let users = user_messages(rest);
                    rest.history_prev(&users);
                }
            }
            Action::None
        }
        KeyCode::Down => {
            let n = command::palette_matches(&rest.input).len();
            if n > 0 {
                rest.palette_sel = (rest.palette_sel + 1).min(n - 1);
            } else {
                let fmatches: Vec<String> = file_ref_partial(&rest.input)
                    .map(|p| rest.fg().dir_cache.read().map(|c| c.search(p, FILE_PAL_MAX)).unwrap_or_default())
                    .unwrap_or_default();
                if !fmatches.is_empty() {
                    rest.palette_sel = (rest.palette_sel + 1).min(fmatches.len() - 1);
                } else if !rest.cursor_down() {
                    let users = user_messages(rest);
                    rest.history_next(&users);
                }
            }
            Action::None
        }
        KeyCode::Tab => {
            let cmd_matches = command::palette_matches(&rest.input);
            if !cmd_matches.is_empty() {
                let sel = rest.palette_sel.min(cmd_matches.len() - 1);
                rest.input = format!("{} ", cmd_matches[sel].0);
                rest.palette_sel = 0;
                rest.cursor_end(); // input replaced wholesale → caret to the end
            } else {
                let fmatches: Vec<String> = file_ref_partial(&rest.input)
                    .map(|p| rest.fg().dir_cache.read().map(|c| c.search(p, FILE_PAL_MAX)).unwrap_or_default())
                    .unwrap_or_default();
                if !fmatches.is_empty() {
                    complete_file_ref(rest, &fmatches);
                }
            }
            Action::None
        }
        KeyCode::PageUp => {
            for _ in 0..10 {
                rest.scroll_up();
            }
            Action::None
        }
        KeyCode::PageDown => {
            for _ in 0..10 {
                rest.scroll_down();
            }
            Action::None
        }
        // End: with input present, move the caret to the end of the line (text
        // editing). With an EMPTY input it keeps its old meaning — jump the
        // transcript to the bottom and resume following.
        KeyCode::End => {
            if rest.input.is_empty() {
                rest.reset_scroll();
            } else {
                rest.cursor_end();
            }
            Action::None
        }
        // Shift+Tab toggles the tool-approval mode (Auto <-> Normal). Crossterm
        // reports Shift+Tab as BackTab, so it never collides with plain Tab.
        KeyCode::BackTab => {
            rest.agent_mode = rest.agent_mode.toggled();
            rest.status = format!("mode: {}", rest.agent_mode.label());
            Action::None
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            rest.push_char(c);
            Action::None
        }
        _ => Action::None,
    }
}
