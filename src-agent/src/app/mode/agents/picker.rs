//! Picker overlay states for the `/agents` dashboard editor:
//! the multi-select tool picker and the single-select model picker.

use crate::model::app_config::{AppConfig, ModelEntry};
use crate::model::settings::Settings;
use crate::tool::agent_selectable_tools;

/// State for the tool multi-select picker overlay.
///
/// Opened from the Edit/Create form when the user presses Enter on the Tools
/// field. Closed by Enter (confirm) or Esc (cancel). All mutations happen
/// through the `AgentsState` helper methods so the cursor always stays within
/// filtered bounds.
#[derive(Debug, Clone)]
pub struct ToolPickerState {
    /// Full selectable tool name list (filtered copy of `all_tools()`, minus
    /// the excluded internal tools).
    pub options: Vec<String>,
    /// Parallel to `options`; `true` = this tool is currently checked.
    pub checked: Vec<bool>,
    /// Index into the FILTERED view (see `filtered_indices`).
    pub cursor: usize,
    /// Live search string; filters `options` by substring match.
    pub filter: String,
}

impl ToolPickerState {
    /// Build from the current `draft_tools` comma-joined string.
    ///
    /// The selectable list is [`crate::tool::agent_selectable_tools`] — the shared
    /// source of truth (every built-in tool minus the internal / infra ones), also used
    /// by the GUI /agents dashboard, so the two never drift. An option is pre-checked if
    /// its name appears in `draft_tools` (case-insensitive, split on comma, trimmed).
    pub(super) fn from_draft(draft_tools: &str) -> Self {
        let options: Vec<String> = agent_selectable_tools();

        let selected: Vec<String> = draft_tools
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();

        let checked: Vec<bool> = options
            .iter()
            .map(|n| selected.contains(&n.to_lowercase()))
            .collect();

        Self {
            options,
            checked,
            cursor: 0,
            filter: String::new(),
        }
    }

    /// Indices into `options` that match the current `filter`.
    ///
    /// If `filter` is empty, all indices are returned in order.
    pub fn filtered_indices(&self) -> Vec<usize> {
        if self.filter.is_empty() {
            (0..self.options.len()).collect()
        } else {
            let q = self.filter.to_lowercase();
            self.options
                .iter()
                .enumerate()
                .filter(|(_, n)| n.to_lowercase().contains(&q))
                .map(|(i, _)| i)
                .collect()
        }
    }

    /// Move the cursor up within the filtered list.
    pub fn up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Move the cursor down within the filtered list.
    pub fn down(&mut self) {
        let len = self.filtered_indices().len();
        if len == 0 {
            return;
        }
        if self.cursor + 1 < len {
            self.cursor += 1;
        }
    }

    /// Toggle the checked state for the option at the current filtered cursor.
    ///
    /// No-op when the filtered list is empty.
    pub fn toggle(&mut self) {
        let indices = self.filtered_indices();
        if indices.is_empty() {
            return;
        }
        let real = indices[self.cursor.min(indices.len() - 1)];
        self.checked[real] = !self.checked[real];
    }

    /// Append a character to the filter and clamp the cursor.
    pub fn push_filter(&mut self, c: char) {
        self.filter.push(c);
        self.clamp_cursor();
    }

    /// Remove the last character from the filter and clamp the cursor.
    pub fn backspace_filter(&mut self) {
        self.filter.pop();
        self.clamp_cursor();
    }

    /// Clamp `cursor` so it stays within the current filtered bounds.
    fn clamp_cursor(&mut self) {
        let len = self.filtered_indices().len();
        if len == 0 {
            self.cursor = 0;
        } else if self.cursor >= len {
            self.cursor = len - 1;
        }
    }

    /// The checked tool names, in `options` order.
    pub fn selected(&self) -> Vec<String> {
        self.options
            .iter()
            .zip(self.checked.iter())
            .filter(|(_, &c)| c)
            .map(|(n, _)| n.clone())
            .collect()
    }
}

/// Resolve a [`ModelEntry`]'s provider connection to a human-readable name for
/// the model picker / browse rows. Checks `config.providers` first (static-key
/// connections), then `config.oauth_conns` (OAuth-backed models like xAI /
/// Codex / koma.run). Falls back to `"?"` when neither catalogue holds the uuid.
fn entry_provider_name(config: &AppConfig, entry: &ModelEntry) -> String {
    if let Some(p) = config.providers.iter().find(|p| p.uuid == entry.provider_uuid) {
        return if !p.name.trim().is_empty() {
            p.name.clone()
        } else if !p.endpoint.trim().is_empty() {
            p.endpoint.clone()
        } else {
            "?".to_string()
        };
    }
    if let Some(c) = config.oauth_conns.iter().find(|c| c.uuid == entry.provider_uuid) {
        return if !c.name.trim().is_empty() {
            c.name.clone()
        } else {
            c.provider.label().to_string()
        };
    }
    "?".to_string()
}

/// One-line label for a registered model in the picker / browse row:
/// `"name — model_id @ <provider name>"`.
fn entry_label(config: &AppConfig, entry: &ModelEntry) -> String {
    format!(
        "{} — {} @ {}",
        entry.name,
        entry.model_id,
        entry_provider_name(config, entry)
    )
}

/// State for the single-select MODEL picker overlay.
///
/// Opened from the Edit/Create form when the user presses Enter on the Model
/// field. It is a pick-ONE list over the REGISTERED models (the same entries
/// edited in `/settings` → Models): row 0 is the `None` "(inherit main)"
/// sentinel, then every [`ModelEntry`] from `settings.session_models` followed
/// by the global `config.models`. The cursor row is the chosen value. Closed by
/// Enter (confirm → write the cursor's uuid into `draft_model_uuid`) or Esc
/// (cancel → discard). Mirrors the tool picker's modal flow.
#[derive(Debug, Clone)]
pub struct ModelPickerState {
    /// One row per choice: `(model_uuid_or_none, display_label)`. Row 0 is always
    /// the `None` "(inherit main)" sentinel; the rest are registered model entries
    /// (session overrides first, then the global catalogue) in order.
    pub options: Vec<(Option<String>, String)>,
    /// Highlighted row, in `0..options.len()`.
    pub cursor: usize,
}

impl ModelPickerState {
    /// Build the option list from the registered models, placing the cursor on the
    /// row matching `current` (the agent's current `model_uuid`).
    ///
    /// Row 0 is the `None` "(inherit main)" sentinel; the remaining rows are the
    /// session model overrides (`settings.session_models`) followed by the global
    /// catalogue (`config.models`), each labelled `"name — model_id @ provider"`.
    /// The cursor lands on the row whose uuid equals `current` (or row 0 when
    /// `current` is `None` or no longer matches a registered entry).
    pub fn from_models(config: &AppConfig, settings: &Settings, current: &Option<String>) -> Self {
        let mut options: Vec<(Option<String>, String)> =
            vec![(None, "(inherit main)".to_string())];
        for entry in settings
            .session_models
            .iter()
            .chain(config.models.iter())
        {
            options.push((Some(entry.uuid.clone()), entry_label(config, entry)));
        }
        let cursor = match current {
            Some(uuid) => options
                .iter()
                .position(|(u, _)| u.as_deref() == Some(uuid.as_str()))
                .unwrap_or(0),
            None => 0,
        };
        Self { options, cursor }
    }

    /// Move the cursor up (clamps at 0).
    pub fn up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Move the cursor down (clamps at the last option).
    pub fn down(&mut self) {
        if self.cursor + 1 < self.options.len() {
            self.cursor += 1;
        }
    }

    /// The model uuid at the cursor: `None` for the "(inherit main)" row, else the
    /// chosen registered model's uuid.
    pub fn selected(&self) -> Option<String> {
        self.options
            .get(self.cursor)
            .and_then(|(u, _)| u.clone())
    }
}
