//! The `/todo` command: open the task-panel overlay.

use anyhow::Result;

use crate::app::mode::{Mode, TodoState};
use crate::app::state::AppState;

/// Handle the `/todo` command: open the read-only task-panel overlay.
///
/// Reads the current todo list from the foreground session's runtime (or
/// loads from `memory/TODO.md` if the runtime doesn't have an in-memory
/// list yet). The panel re-reads on every key press.
pub(super) fn handle_todo(state: &mut AppState) -> Result<()> {
    // For now, load from the session's memory dir if available.
    // The todowrite tool writes to memory/TODO.md; we parse it here.
    let items = load_todos_from_session(state);
    let st = TodoState::new(items);
    *state.mode_mut() = Mode::Todo(Box::new(st));
    Ok(())
}

/// Load todo items from the session's `memory/TODO.md` file.
///
/// Expected format (markdown checkbox list):
/// ```markdown
/// - [ ] pending item (high)
/// - [~] in-progress item (medium)
/// - [x] completed item (low)
/// - [-] cancelled item (medium)
/// ```
fn load_todos_from_session(state: &AppState) -> Vec<crate::app::mode::todo::TodoItem> {
    use crate::app::mode::todo::{TodoItem, TodoPriority, TodoStatus};

    let Some(session) = state.rest.fg().session.as_ref() else {
        return Vec::new();
    };
    let path = session.path.join("memory").join("TODO.md");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };

    let mut items = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        // Match: - [ ] text (priority)  OR  - [x] text  etc.
        let Some(inner) = line.strip_prefix("- [") else {
            continue;
        };
        let (status_char, rest) = match inner.split_once(']') {
            Some((s, r)) => (s.trim(), r.trim()),
            None => continue,
        };
        if rest.is_empty() {
            continue;
        }

        let status = match status_char {
            " " | "" => TodoStatus::Pending,
            "~" => TodoStatus::InProgress,
            "x" => TodoStatus::Completed,
            "-" => TodoStatus::Cancelled,
            _ => TodoStatus::Pending,
        };

        // Extract priority from trailing "(high|medium|low)" if present.
        let (content, priority) = if let Some(idx) = rest.rfind('(') {
            if rest.ends_with(')') {
                let p = &rest[idx + 1..rest.len() - 1];
                (rest[..idx].trim().to_string(), TodoPriority::from_str(p))
            } else {
                (rest.to_string(), TodoPriority::Medium)
            }
        } else {
            (rest.to_string(), TodoPriority::Medium)
        };

        items.push(TodoItem { content, status, priority });
    }
    items
}
