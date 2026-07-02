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
    #[allow(dead_code)]
    pub fn refresh(&mut self, items: Vec<TodoItem>) {
        self.items = items;
        if self.selected >= self.items.len() {
            self.selected = self.items.len().saturating_sub(1);
        }
    }

    /// The currently-selected item, if any.
    #[allow(dead_code)]
    pub fn current(&self) -> Option<&TodoItem> {
        self.items.get(self.selected)
    }
}
