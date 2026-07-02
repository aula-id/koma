//! Todo mode state: the working state for the `/todo` task-panel overlay.
//!
//! A READ-ONLY master/detail panel (modelled on `Mode::Bash`):
//! the LEFT pane lists every todo item in the current session; the
//! RIGHT pane shows the selected item's full content + status + priority.
//! The user can navigate the list; the model reads/writes todos via the
//! `todowrite` tool (writes to `memory/TODO.md`).

use serde::{Deserialize, Serialize};

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

    pub fn from_str(s: &str) -> Self {
        match s {
            "in_progress" => Self::InProgress,
            "completed" => Self::Completed,
            "cancelled" => Self::Cancelled,
            _ => Self::Pending,
        }
    }

    /// Cycle to the next status: Pending → InProgress → Completed → Cancelled → Pending.
    pub fn cycle(&self) -> Self {
        match self {
            Self::Pending => Self::InProgress,
            Self::InProgress => Self::Completed,
            Self::Completed => Self::Cancelled,
            Self::Cancelled => Self::Pending,
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
}

impl TodoItem {
    /// Serialize back to the markdown checkbox line format:
    /// `- [x] content (high)`
    pub fn to_line(&self) -> String {
        format!(
            "- [{}] {} ({})",
            self.status.checkbox_char(),
            self.content,
            self.priority.label(),
        )
    }
}

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

/// Working state for the `/todo` task-panel.
///
/// Holds the todo item list + the LIST cursor. No drafts, no sub-modes —
/// read-only for the user; the model writes via the `todowrite` tool.
#[derive(Debug, Clone, Default)]
pub struct TodoState {
    /// Snapshot of the todo items (one row per item).
    pub items: Vec<TodoItem>,
    /// Selected index into `items` (the LIST cursor).
    pub selected: usize,
}

impl TodoState {
    /// Build the panel from an initial item list, cursor at the top.
    pub fn new(items: Vec<TodoItem>) -> Self {
        Self { items, selected: 0 }
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

    /// Re-read `memory/TODO.md` from disk and refresh the item list in-place.
    /// The cursor stays on the same row index (clamped if the list shrank).
    pub fn refresh_from_disk(&mut self, pwd_hash: &str) {
        let items = crate::model::store::memory_dir(pwd_hash)
            .ok()
            .and_then(|dir| std::fs::read_to_string(dir.join("TODO.md")).ok())
            .map(|c| parse_todo_file(&c))
            .unwrap_or_default();
        self.refresh(items);
    }

    /// The currently-selected item, if any.
    #[allow(dead_code)] // public API; consumers index directly or use toggle_selected
    pub fn current(&self) -> Option<&TodoItem> {
        self.items.get(self.selected)
    }

    /// Toggle the selected item's status (cycle: pending → in_progress → completed
    /// → cancelled → pending), write the updated list back to `memory/TODO.md`,
    /// and re-read from disk so the overlay reflects the change.
    pub fn toggle_selected(&mut self, pwd_hash: &str) {
        if let Some(item) = self.items.get_mut(self.selected) {
            item.status = item.status.cycle();
        }
        // Write the full list back to disk.
        let Ok(memory_dir) = crate::model::store::memory_dir(pwd_hash) else {
            return;
        };
        let path = memory_dir.join("TODO.md");
        let content: String = self
            .items
            .iter()
            .map(|item| item.to_line())
            .collect::<Vec<_>>()
            .join("\n");
        // Ensure the directory exists (the model's todowrite tool creates it,
        // but the user might toggle before any write has happened).
        let _ = std::fs::create_dir_all(&memory_dir);
        let _ = std::fs::write(&path, format!("{content}\n"));
        // Re-read to stay in sync with what's actually on disk.
        self.refresh_from_disk(pwd_hash);
    }
}
