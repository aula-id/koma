//! The `/todo` command: open the task-panel overlay.

use anyhow::Result;

use crate::app::mode::{parse_todo_file, Mode, TodoState};
use crate::app::state::{AgentMode, AppState};

/// Handle the `/todo` command: open the task-panel overlay.
///
/// Plan → `plan_todos.md`. SDLC → L2 graph projection (mission contract tasks).
/// Else → `memory/TODO.md`. The panel re-reads on every key press.
pub(super) fn handle_todo(state: &mut AppState) -> Result<()> {
    let mode = state.rest.agent_mode();
    let (items, pwd_hash, plan_path, sdlc_graph) = load_todos_with_pwd(state, mode);
    let mut st = TodoState::new(items, pwd_hash);
    st.plan_path = plan_path;
    st.sdlc_graph = sdlc_graph;
    *state.mode_mut() = Mode::Todo(Box::new(st));
    Ok(())
}

/// Load todo items for the overlay from the mode-appropriate source.
fn load_todos_with_pwd(
    state: &AppState,
    mode: AgentMode,
) -> (
    Vec<crate::app::mode::todo::TodoItem>,
    String,
    Option<std::path::PathBuf>,
    bool,
) {
    let Some(session) = state.rest.fg().session.as_ref() else {
        return (Vec::new(), String::new(), None, false);
    };
    let pwd_hash = session.pwd_hash.clone();
    match mode {
        AgentMode::Plan => {
            let path = session.plan_todos_path();
            let items = crate::app::mode::todo::load_todos_from(&path);
            (items, pwd_hash, Some(path), false)
        }
        AgentMode::Sdlc => {
            let items = crate::model::sdlc::graph::load_sdlc_todo_items(&session.path);
            (items, pwd_hash, Some(session.path.clone()), true)
        }
        _ => {
            let memory_dir =
                crate::model::store::memory_dir(&session.pwd_hash).unwrap_or_default();
            let path = memory_dir.join("TODO.md");
            let Ok(content) = std::fs::read_to_string(&path) else {
                return (Vec::new(), pwd_hash, None, false);
            };
            (
                parse_todo_file(&content),
                pwd_hash,
                None,
                false,
            )
        }
    }
}
