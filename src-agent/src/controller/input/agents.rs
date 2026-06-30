//! Key handler for the `/agents` management dashboard (`Mode::Agents`).

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crate::app::mode::AgentsState;
use crate::app::state::AppStateRest;
use super::{is_ctrl, Action};

/// Handle a key press inside the `/agents` management dashboard.
///
/// Context-sensitive dispatch keyed on the sub-mode + editing flag (deepest
/// focus first):
///
/// 0. **DeleteConfirm** – modal y/n; `y` deletes (`Action::DeleteAgent`),
///    `n`/Esc cancels back to Browse.
///
/// 1. **editing** (Edit/Create, typing a field) – Char/Backspace mutate the
///    draft; Ctrl+J / Shift+Enter add a newline in the body; Enter or Esc
///    commit the field and drop back to field navigation (Esc in Create only
///    leaves the field, it does NOT cancel the whole flow).
///
/// 2. **Edit/Create** (navigating fields, not editing) – ↑/↓ move the field
///    cursor; Enter starts editing the field (Name/Description/… ) or, for the
///    scope row in Create, toggles scope; `s` saves/creates; Esc cancels the
///    whole flow back to Browse.
///
/// 3. **Browse** – ↑/↓ move the LIST cursor; →/Enter open the selected agent
///    for editing (built-ins are read-only → status note, no transition);
///    `n` starts Create; `d` deletes the selected file-backed agent; Esc closes
///    the dashboard (`Action::CloseAgents`).
pub fn handle_agents(s: &mut AgentsState, rest: &mut AppStateRest, key: KeyEvent) -> Action {
    use crate::app::mode::{AgentEditField, AgentSubMode};
    use crate::model::agent_def::AgentSource;

    if is_ctrl(&key, 'c') {
        return Action::Quit;
    }

    // --- Full-screen field editor (TOP priority: intercepts ALL keys) ---
    // A nano-style 2D editor over the active field's draft (Body / Description /
    // Conditions — tagged inside `s.editor`). While open it owns every key:
    // printable chars insert, arrows/Home/End move the cursor, Enter splits the
    // line, Backspace/Delete remove, and Esc COMMITS the text back into the matching
    // draft and closes (returning to the field list). Everything else is swallowed.
    if s.editor.is_some() {
        // Ctrl+X is a two-step "clear the whole field" guard. The pending flag
        // lives on `s` (not behind the `ed` borrow), so it is read/set BEFORE
        // borrowing the buffer mutably — that keeps the borrow checker happy and
        // lets the confirm resolve without `ed` for the cancel path.
        if s.editor_clear_confirm {
            // A confirmation is armed: `y`/`Y` wipes the buffer, ANY other key
            // cancels. Either way the flag clears and the keystroke is consumed.
            if let KeyCode::Char('y') | KeyCode::Char('Y') = key.code {
                if let Some((_field, ed)) = s.editor.as_mut() {
                    ed.lines = vec![String::new()];
                    ed.row = 0;
                    ed.col = 0;
                    ed.scroll = 0;
                }
            }
            s.editor_clear_confirm = false;
            return Action::None;
        }
        if is_ctrl(&key, 'x') {
            // First press: arm the confirmation; don't touch the text yet.
            s.editor_clear_confirm = true;
            return Action::None;
        }

        // Normal editing: borrow the buffer and apply the keystroke.
        if let Some((_field, ed)) = s.editor.as_mut() {
            match key.code {
                KeyCode::Esc => {
                    // Commit: write the edited text back into the tagged draft, close.
                    s.commit_editor();
                }
                KeyCode::Enter => ed.newline(),
                KeyCode::Backspace => ed.backspace(),
                KeyCode::Delete => ed.delete(),
                KeyCode::Left => ed.move_left(),
                KeyCode::Right => ed.move_right(),
                KeyCode::Up => ed.move_up(),
                KeyCode::Down => ed.move_down(),
                KeyCode::Home => ed.home(),
                KeyCode::End => ed.end(),
                // Printable char with no Ctrl modifier → insert it.
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    ed.insert_char(c);
                }
                // Swallow anything else (other Ctrl combos, Tab, function keys, …).
                _ => {}
            }
        }
        return Action::None;
    }

    // --- Model picker overlay (DEEPEST priority: intercepts ALL keys) ---
    // Single-select pick-one list over the registered models: ↑/↓ navigate, Enter
    // commits the cursor's model uuid into the draft, Esc discards. Sits above the
    // tool picker (both are mutually exclusive — only one is ever open at a time).
    if s.model_picker.is_some() {
        match key.code {
            KeyCode::Up => {
                if let Some(p) = s.model_picker.as_mut() {
                    p.up();
                }
            }
            KeyCode::Down => {
                if let Some(p) = s.model_picker.as_mut() {
                    p.down();
                }
            }
            KeyCode::Enter => {
                s.confirm_model_picker();
            }
            KeyCode::Esc => {
                s.cancel_model_picker();
            }
            _ => {}
        }
        return Action::None;
    }

    // --- Tool picker overlay (intercepts ALL keys) ---
    if s.tool_picker.is_some() {
        match key.code {
            KeyCode::Up => {
                if let Some(p) = s.tool_picker.as_mut() {
                    p.up();
                }
            }
            KeyCode::Down => {
                if let Some(p) = s.tool_picker.as_mut() {
                    p.down();
                }
            }
            KeyCode::Char(' ') => {
                if let Some(p) = s.tool_picker.as_mut() {
                    p.toggle();
                }
            }
            KeyCode::Enter => {
                s.confirm_tool_picker();
            }
            KeyCode::Esc => {
                s.cancel_tool_picker();
            }
            KeyCode::Backspace => {
                if let Some(p) = s.tool_picker.as_mut() {
                    p.backspace_filter();
                }
            }
            KeyCode::Char(c)
                if !key.modifiers.contains(KeyModifiers::CONTROL) && c != ' ' =>
            {
                if let Some(p) = s.tool_picker.as_mut() {
                    p.push_filter(c);
                }
            }
            _ => {}
        }
        return Action::None;
    }

    match s.mode {
        // --- DeleteConfirm: modal y/n ---
        AgentSubMode::DeleteConfirm => match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => Action::DeleteAgent,
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                s.mode = AgentSubMode::Browse;
                Action::None
            }
            _ => Action::None,
        },

        // --- Edit / Create ---
        AgentSubMode::Edit | AgentSubMode::Create => {
            if s.editing {
                // Typing into the highlighted draft field (plain text fields).
                // Ctrl+J always inserts a body newline (reliable multiline key).
                if is_ctrl(&key, 'j') {
                    s.newline();
                    return Action::None;
                }
                match key.code {
                    // Commit the field; stay in the editor.
                    KeyCode::Esc => {
                        s.editing = false;
                        Action::None
                    }
                    KeyCode::Enter => {
                        // Shift+Enter (when reported) inserts a body newline;
                        // plain Enter commits the field.
                        if s.field == AgentEditField::Body
                            && key.modifiers.contains(KeyModifiers::SHIFT)
                        {
                            s.newline();
                        } else {
                            s.editing = false;
                        }
                        Action::None
                    }
                    KeyCode::Backspace => {
                        s.backspace();
                        Action::None
                    }
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        s.push_char(c);
                        Action::None
                    }
                    _ => Action::None,
                }
            } else {
                // Navigating the field list.
                match key.code {
                    KeyCode::Esc => {
                        s.cancel();
                        Action::None
                    }
                    KeyCode::Up => {
                        s.field_up();
                        Action::None
                    }
                    KeyCode::Down | KeyCode::Tab => {
                        s.field_down();
                        Action::None
                    }
                    KeyCode::Enter => {
                        match s.field {
                            // Tools field uses the multi-select picker overlay.
                            AgentEditField::Tools => {
                                s.open_tool_picker();
                            }
                            // Model field uses the single-select picker over the
                            // registered models (inherit-or-registered); it falls
                            // back to a no-op when there are no settings to read the
                            // session models from.
                            AgentEditField::Model => {
                                if let Some(settings) =
                                    rest.fg().session.as_ref().map(|sess| sess.settings.clone())
                                {
                                    s.open_model_picker(&rest.config, &settings);
                                }
                            }
                            // Body, Description, and Conditions all open the
                            // full-screen nano-style editor (comfortable multi-line
                            // editing) instead of the cramped inline path.
                            AgentEditField::Body => {
                                s.open_editor(AgentEditField::Body);
                            }
                            AgentEditField::Description => {
                                s.open_editor(AgentEditField::Description);
                            }
                            AgentEditField::Conditions => {
                                s.open_editor(AgentEditField::Conditions);
                            }
                            // Every other field (Name in Create) is a plain text box.
                            _ => {
                                s.editing = true;
                            }
                        }
                        Action::None
                    }
                    // Scope toggle is only meaningful in Create; bind it to ←/→
                    // so it never collides with field text entry.
                    KeyCode::Left | KeyCode::Right if s.mode == AgentSubMode::Create => {
                        s.toggle_scope();
                        Action::None
                    }
                    // Save (Edit) / create (Create).
                    KeyCode::Char('s') | KeyCode::Char('S') => {
                        if s.mode == AgentSubMode::Create {
                            if s.draft_name.trim().is_empty() {
                                rest.fg_mut().status = "name required".into();
                                Action::None
                            } else if s.draft_description.trim().is_empty() {
                                rest.fg_mut().status = "description required".into();
                                Action::None
                            } else {
                                Action::CreateAgent
                            }
                        } else if s.draft_description.trim().is_empty() {
                            rest.fg_mut().status = "description required".into();
                            Action::None
                        } else {
                            Action::SaveAgent
                        }
                    }
                    _ => Action::None,
                }
            }
        }

        // --- Browse: navigate the LIST ---
        AgentSubMode::Browse => match key.code {
            KeyCode::Esc => Action::CloseAgents,
            KeyCode::Up => {
                s.list_up();
                Action::None
            }
            KeyCode::Down | KeyCode::Tab => {
                s.list_down();
                Action::None
            }
            KeyCode::Enter | KeyCode::Right => {
                match s.current_agent().map(|a| a.source) {
                    Some(AgentSource::Builtin) => {
                        rest.fg_mut().status = "built-in agents are read-only".into();
                        Action::None
                    }
                    Some(_) => {
                        s.enter_edit();
                        Action::None
                    }
                    None => Action::None,
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                s.enter_create();
                Action::None
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                match s.current_agent().map(|a| a.source) {
                    Some(AgentSource::Builtin) => {
                        rest.fg_mut().status = "cannot delete a built-in agent".into();
                        Action::None
                    }
                    Some(_) => {
                        s.enter_delete();
                        Action::None
                    }
                    None => Action::None,
                }
            }
            _ => Action::None,
        },
    }
}
