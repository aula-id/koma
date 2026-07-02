//! The `/todo` command: open the task-panel overlay.

use anyhow::Result;

use crate::app::mode::{parse_todo_file, Mode, TodoState};
use crate::app::state::AppState;

/// Handle the `/todo` command: open the read-only task-panel overlay.
///
/// Reads the current todo list from the foreground session's memory/TODO.md.
/// The panel re-reads on every key press.
pub(super) fn handle_todo(state: &mut AppState) -> Result<()> {
    let items = load_todos_from_session(state);
    let st = TodoState::new(items);
    *state.mode_mut() = Mode::Todo(Box::new(st));
    Ok(())
}

/// Load todo items from the session's `memory/TODO.md` file.
fn load_todos_from_session(state: &AppState) -> Vec<crate::app::mode::todo::TodoItem> {
    let Some(session) = state.rest.fg().session.as_ref() else {
        return Vec::new();
    };
    let memory_dir = crate::model::store::memory_dir(&session.pwd_hash)
        .unwrap_or_default();
    let path = memory_dir.join("TODO.md");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    parse_todo_file(&content)
}
