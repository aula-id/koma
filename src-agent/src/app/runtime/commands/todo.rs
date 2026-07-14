//! The `/todo` command: open the task-panel overlay.

use anyhow::Result;

use crate::app::mode::{parse_todo_file, Mode, TodoState};
use crate::app::state::{AgentMode, AppState};

/// Handle the `/todo` command: open the task-panel overlay.
///
/// While the session is in plan mode, reads the session-scoped
/// `plan_todos.md` (the model's plan checklist + the two locked rails)
/// instead of the per-directory `memory/TODO.md` — same source the
/// `checklist` interception + `plan_ready` write to. Outside plan mode,
/// behaviour is unchanged. The panel re-reads on every key press.
pub(super) fn handle_todo(state: &mut AppState) -> Result<()> {
    let in_plan = state.rest.agent_mode == AgentMode::Plan;
    let (items, pwd_hash, plan_path) = load_todos_with_pwd(state, in_plan);
    let mut st = TodoState::new(items, pwd_hash);
    st.plan_path = plan_path;
    *state.mode_mut() = Mode::Todo(Box::new(st));
    Ok(())
}

/// Load todo items from the session's backing file: the session-scoped
/// `plan_todos.md` when `in_plan` is true, else the per-directory
/// `memory/TODO.md`. Returns the items, the session's pwd_hash (for later
/// disk refresh), and — when loaded from the plan file — its path (so the
/// overlay keeps reading the SAME file on every periodic/keypress refresh).
fn load_todos_with_pwd(
    state: &AppState,
    in_plan: bool,
) -> (Vec<crate::app::mode::todo::TodoItem>, String, Option<std::path::PathBuf>) {
    let Some(session) = state.rest.fg().session.as_ref() else {
        return (Vec::new(), String::new(), None);
    };
    let pwd_hash = session.pwd_hash.clone();
    if in_plan {
        let path = session.plan_todos_path();
        let items = crate::app::mode::todo::load_todos_from(&path);
        return (items, pwd_hash, Some(path));
    }
    let memory_dir = crate::model::store::memory_dir(&session.pwd_hash)
        .unwrap_or_default();
    let path = memory_dir.join("TODO.md");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return (Vec::new(), pwd_hash, None);
    };
    (parse_todo_file(&content), pwd_hash, None)
}
