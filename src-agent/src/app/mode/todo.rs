//! Todo mode state: the working state for the `/todo` task-panel overlay.
//!
//! A master/detail panel (modelled on `Mode::Bash`):
//! the LEFT pane lists every todo item in the current session; the
//! RIGHT pane shows the selected item's full content + status + priority.
//! The user can navigate the list and press Enter to reset an item to
//! pending (signalling the model to redo it); the model reads/writes
//! todos via the `todowrite` tool (writes to `memory/TODO.md`).

use std::time::Instant;

use serde::{Deserialize, Serialize};

/// Minimum interval between disk re-reads when the overlay is open.
const REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

/// Status of a single todo item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

impl TodoStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
        }
    }

    /// Human-readable display label for the UI (not the wire format).
    pub fn display(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in progress",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "in_progress" => Self::InProgress,
            "completed" => Self::Completed,
            "cancelled" => Self::Cancelled,
            _ => Self::Pending,
        }
    }

    /// Single-char shorthand for the markdown checkbox format.
    pub fn checkbox_char(&self) -> &'static str {
        match self {
            Self::Pending => " ",
            Self::InProgress => "~",
            Self::Completed => "x",
            Self::Cancelled => "-",
        }
    }
}

/// Priority of a single todo item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoPriority {
    High,
    Medium,
    Low,
}

impl TodoPriority {
    pub fn label(&self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "high" => Self::High,
            "low" => Self::Low,
            _ => Self::Medium,
        }
    }
}

/// A single todo item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub content: String,
    pub status: TodoStatus,
    pub priority: TodoPriority,
    /// Immutable/pinned marker (used by the plan-mode rail items). Inert for
    /// the regular working TODO.md; defaults to `false` for back-compat.
    #[serde(default)]
    pub locked: bool,
}

impl TodoItem {
    /// Serialize back to the markdown checkbox line format:
    /// `- [x] content (high)`, or `- [x] content (high) [locked]` when pinned.
    pub fn to_line(&self) -> String {
        if self.locked {
            format!(
                "- [{}] {} ({}) [locked]",
                self.status.checkbox_char(),
                self.content,
                self.priority.label(),
            )
        } else {
            format!(
                "- [{}] {} ({})",
                self.status.checkbox_char(),
                self.content,
                self.priority.label(),
            )
        }
    }
}

/// The two immutable rail items pinned to the tail of every plan todo list.
pub const PLAN_RAIL_SERVE: &str = "serve plan to user";
pub const PLAN_RAIL_SAVE: &str = "save plan to file & prompt approval";

/// Parse the contents of a `TODO.md` file into a list of [`TodoItem`]s.
///
/// Expected format (markdown checkbox list):
/// ```markdown
/// - [ ] pending item (high)
/// - [~] in-progress item (medium)
/// - [x] completed item (low)
/// - [-] cancelled item (medium)
/// ```
pub fn parse_todo_file(content: &str) -> Vec<TodoItem> {
    let mut items = Vec::new();
    for line in content.lines() {
        let line = line.trim();
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

        // Detect + strip a trailing " [locked]" marker before parsing
        // priority/content, so back-compat lines (without the marker) parse
        // identically to before.
        let (rest, locked) = match rest.strip_suffix("[locked]") {
            Some(stripped) => (stripped.trim_end(), true),
            None => (rest, false),
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

        items.push(TodoItem { content, status, priority, locked });
    }
    items
}

/// Load a plan-todo list from an explicit path (the session-scoped
/// `plan_todos.md`, distinct from the per-directory working `TODO.md`).
/// Returns an empty `Vec` if the file is absent or unreadable.
pub fn load_todos_from(path: &std::path::Path) -> Vec<TodoItem> {
    std::fs::read_to_string(path)
        .map(|c| parse_todo_file(&c))
        .unwrap_or_default()
}

/// Load the CURRENT todo list for a session: the session-scoped
/// `plan_todos.md` (the model's plan checklist + the two locked rails) while
/// `in_plan`, else the per-directory `memory/TODO.md` (the regular working
/// list `todowrite` writes to outside Plan mode) — the exact backing-file
/// selection `/todo`'s own overlay uses (see
/// `app::runtime::commands::todo::load_todos_with_pwd`).
///
/// Shared so every mirror of "the session's current todo list" — the GUI
/// Explore "PLAN" section's `SessionRuntime::plan_todos`, refreshed at session
/// load, mode transitions, and after every tool round — follows the SAME
/// source of truth as the TUI overlay, in every mode, not just Plan. Empty
/// when the relevant file doesn't exist yet.
pub fn load_current_todos(session: &crate::model::session::Session, in_plan: bool) -> Vec<TodoItem> {
    if in_plan {
        load_todos_from(&session.plan_todos_path())
    } else {
        crate::model::store::memory_dir(&session.pwd_hash)
            .ok()
            .map(|dir| load_todos_from(&dir.join("TODO.md")))
            .unwrap_or_default()
    }
}

/// Write a plan-todo list to an explicit path, atomically (temp file +
/// rename) so a crash mid-write never leaves a truncated file. Serializes
/// via [`TodoItem::to_line`].
pub fn save_todos_to(path: &std::path::Path, items: &[TodoItem]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content: String = items
        .iter()
        .map(|item| item.to_line())
        .collect::<Vec<_>>()
        .join("\n");
    crate::model::memory::atomic_write(path, format!("{content}\n").as_bytes())
}

/// Working state for the `/todo` task-panel.
///
/// Holds the todo item list + the LIST cursor. No drafts, no sub-modes —
/// read-only for the user; the model writes via the `todowrite` tool.
#[derive(Debug, Clone)]
pub struct TodoState {
    /// Snapshot of the todo items (one row per item).
    pub items: Vec<TodoItem>,
    /// Selected index into `items` (the LIST cursor).
    pub selected: usize,
    /// Session pwd_hash for disk reads (used by periodic refresh).
    pub pwd_hash: String,
    /// When the overlay was opened while the session was in plan mode, the
    /// session-scoped `plan_todos.md` path to read/write instead of the
    /// per-directory `memory/TODO.md`. `None` for the normal working list.
    /// Daemon-only concern: the client never needs the path itself, only the
    /// resulting items (which already project via `TodoItemSnapshot`), so this
    /// is NOT threaded through the snapshot/shadow projection.
    pub plan_path: Option<std::path::PathBuf>,
    /// Timestamp of the last disk refresh.
    pub last_refresh: Instant,
}

impl Default for TodoState {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            selected: 0,
            pwd_hash: String::new(),
            plan_path: None,
            last_refresh: Instant::now(),
        }
    }
}

impl TodoState {
    /// Build the panel from an initial item list, cursor at the top.
    pub fn new(items: Vec<TodoItem>, pwd_hash: String) -> Self {
        Self { items, selected: 0, pwd_hash, plan_path: None, last_refresh: Instant::now() }
    }

    /// Move the LIST cursor up (saturating at 0).
    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Move the LIST cursor down, clamped to the last item.
    pub fn move_down(&mut self) {
        if self.selected + 1 < self.items.len() {
            self.selected += 1;
        }
    }

    /// Replace the item list and re-clamp the cursor.
    pub fn refresh(&mut self, items: Vec<TodoItem>) {
        self.items = items;
        if self.selected >= self.items.len() {
            self.selected = self.items.len().saturating_sub(1);
        }
    }

    /// Re-read the backing file from disk and refresh the item list in-place.
    /// When `plan_path` is set (the overlay was opened in plan mode) this reads
    /// the session-scoped `plan_todos.md`; otherwise the per-directory
    /// `memory/TODO.md`. The cursor stays on the same row index (clamped if the
    /// list shrank).
    pub fn refresh_from_disk(&mut self) {
        let items = if let Some(path) = &self.plan_path {
            load_todos_from(path)
        } else {
            crate::model::store::memory_dir(&self.pwd_hash)
                .ok()
                .and_then(|dir| std::fs::read_to_string(dir.join("TODO.md")).ok())
                .map(|c| parse_todo_file(&c))
                .unwrap_or_default()
        };
        self.refresh(items);
        self.last_refresh = Instant::now();
    }

    /// Periodic refresh: re-read from disk only if enough time has elapsed.
    /// Returns `true` if the item list actually changed (caller should mark dirty).
    pub fn maybe_refresh(&mut self) -> bool {
        if self.last_refresh.elapsed() >= REFRESH_INTERVAL {
            let old_hash: Vec<_> = self.items.iter()
                .map(|i| (i.content.clone(), i.status.clone(), i.priority.clone()))
                .collect();
            self.refresh_from_disk();
            let new_hash: Vec<_> = self.items.iter()
                .map(|i| (i.content.clone(), i.status.clone(), i.priority.clone()))
                .collect();
            old_hash != new_hash
        } else {
            false
        }
    }

    /// The currently-selected item, if any.
    pub fn current(&self) -> Option<&TodoItem> {
        self.items.get(self.selected)
    }

    /// Reset the selected item's status to `Pending`, write the updated list
    /// back to disk (`plan_todos.md` when `plan_path` is set, else the
    /// per-directory `memory/TODO.md`), and re-read from disk so the overlay
    /// reflects the change. Only the user can do this — it signals the model
    /// to redo the todo. Locked items (the plan-mode rails) are guarded by the
    /// caller ([`crate::controller::input::todo::handle_todo`]) — this is a
    /// no-op if called on one anyway, as a defense-in-depth backstop.
    pub fn reset_to_pending(&mut self) {
        // Re-read from disk first so we don't clobber writes the model made via
        // todowrite since our last refresh.
        self.refresh_from_disk();
        if let Some(item) = self.items.get_mut(self.selected) {
            if item.locked || item.status == TodoStatus::Pending {
                return; // Locked, or already pending — nothing to do.
            }
            item.status = TodoStatus::Pending;
        } else {
            return;
        }
        // Write the full list back to disk atomically (temp + rename) so a
        // crash mid-write never leaves a truncated file.
        if let Some(path) = self.plan_path.clone() {
            let _ = save_todos_to(&path, &self.items);
        } else {
            let Ok(memory_dir) = crate::model::store::memory_dir(&self.pwd_hash) else {
                return;
            };
            let path = memory_dir.join("TODO.md");
            let content: String = self
                .items
                .iter()
                .map(|item| item.to_line())
                .collect::<Vec<_>>()
                .join("\n");
            let _ = std::fs::create_dir_all(&memory_dir);
            let tmp = path.with_extension("md.tmp");
            if std::fs::write(&tmp, format!("{content}\n")).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
        self.refresh_from_disk();
    }

    /// Count completed items (for the title display).
    pub fn completed_count(&self) -> usize {
        self.items.iter().filter(|i| i.status == TodoStatus::Completed).count()
    }
}
