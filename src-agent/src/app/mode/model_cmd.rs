//! State for the `/model` session model switcher overlay.
//!
//! One [`Mode::Model`] variant with internal submodes (`[`ModelCmdSub`]`).
//! Shared list UI: `(id_or_none, label)`. Meaning depends on submode:
//!   - `RolePick` / `AgentPick` → model uuid | None = inherit
//!   - `AgentList` → agent name | never None except empty
//!   - `Help` → unused

use crate::model::app_config::ModelRole;

/// Internal submodes of the `/model` command overlay.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelCmdSub {
    /// Show help with current role bindings.
    Help {
        /// Help text lines to display.
        lines: Vec<String>,
    },
    /// Pick a model for a session role.
    RolePick {
        /// Which role is being assigned.
        role: ModelRole,
    },
    /// List non-extension agents for model reassignment.
    AgentList,
    /// Pick a model for a named agent.
    AgentPick {
        /// The agent being reassigned.
        agent_name: String,
        /// Whether the agent has an existing model override.
        current_model: Option<String>,
    },
}

/// State for the `/model` session model switcher overlay.
pub struct ModelCmdState {
    /// Current submode.
    pub sub: ModelCmdSub,
    /// Shared list UI: `(id_or_none, label)`. Meaning depends on submode.
    pub options: Vec<(Option<String>, String)>,
    /// Cursor within `options`.
    pub cursor: usize,
    /// Help / error footnote under the list.
    pub note: String,
}

impl ModelCmdState {
    /// Move the cursor up one row (clamps at 0).
    pub fn up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Move the cursor down one row (clamps at the last option).
    pub fn down(&mut self) {
        if self.cursor + 1 < self.options.len() {
            self.cursor += 1;
        }
    }

    /// The currently highlighted option, if any.
    pub fn selected(&self) -> Option<&(Option<String>, String)> {
        self.options.get(self.cursor)
    }

    /// The uuid of the currently highlighted model (None for inherit).
    pub fn selected_uuid(&self) -> Option<String> {
        self.selected()
            .and_then(|(uuid, _)| uuid.clone())
    }
}

#[cfg(test)]
#[path = "model_cmd_test.rs"]
mod tests;
