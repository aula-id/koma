//! Transient state for the first-run setup WIZARD (KeyInput mode).
//!
//! A 2-step, provider-agnostic flow that captures a real provider connection and
//! a Main model, then writes them to `config.json` (see `Action::SaveCreds`):
//!
//! - **Step 0 — connection:** `Endpoint` (any OpenAI-compatible base URL, not
//!   just OpenRouter) and `API key`.
//! - **Step 1 — model:** `Model` id. For any non-empty endpoint this is a LIVE
//!   omnisearch over the on-demand model catalogue (`query` + `result_sel`,
//!   filtered via [`crate::app::mode::filter_models`]); for a blank endpoint it
//!   stays a plain text box. The branch is keyed on
//!   [`KeyInputForm::is_omnisearchable`].
//!
//! Constructed via [`KeyInputForm::new`] (clean first-run defaults) or
//! [`KeyInputForm::prefilled`] (seeded from remembered creds for the
//! `/settings`-style re-entry and `--resume` picker paths). Fields are edited in
//! place via `push_char` / `backspace` against whichever field is active; the
//! controller reads `step` + `field` to know what is focused, and drives
//! `advance_step` / `back_step` for the step transitions.

use crate::config::DEFAULT_BASE_URL;

/// In-progress state of the first-run setup wizard.
///
/// `step` selects the wizard page (0 = connection, 1 = model) and `field`
/// selects the active input within that page. The string fields hold the live
/// values; `first_run` / `from_picker` only steer Esc behaviour (see
/// [`controller::input::handle_key_input`]).
#[derive(Debug, Clone)]
pub struct KeyInputForm {
    /// Wizard page: `0` = connection (endpoint + key), `1` = model.
    pub step: usize,
    /// Active field within the current step. Step 0: `0` = endpoint, `1` = key.
    /// Step 1: `0` = model.
    pub field: usize,
    /// Provider base URL (any OpenAI-compatible endpoint). Defaults to
    /// [`DEFAULT_BASE_URL`].
    pub endpoint: String,
    /// API key for the provider connection.
    pub api_key: String,
    /// Main model id. Empty until the user picks one on step 1 (catalogue or
    /// typed). On step 1 with a non-empty endpoint this is the PICKED id (set from
    /// the highlighted catalogue result or the raw `query` fallback); otherwise it
    /// is typed in directly.
    pub model: String,
    /// Step-1 omnisearch query (the live search box value). Unused for a blank
    /// endpoint (the Model field is a plain text box there).
    pub query: String,
    /// Highlighted row in the step-1 omnisearch results list. Indexes into the
    /// `filter_models` result vector; clamped to it. Fetchable-endpoint step-1 only.
    pub result_sel: usize,
    /// `true` when no prior session / configured client exists.
    /// Controls Esc behaviour: if true, Esc must quit (there is no Chat view
    /// to return to).
    pub first_run: bool,
    /// `true` when this form was entered from the `--resume` session picker.
    /// Esc returns to the picker instead of Quit / Chat.
    pub from_picker: bool,
}

impl Default for KeyInputForm {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyInputForm {
    /// Fresh first-run wizard: step 0 / field 0, endpoint = [`DEFAULT_BASE_URL`],
    /// empty key, **empty model** (user picks from the catalogue on step 1 — do
    /// not prefill [`DEFAULT_MODEL`] / koma-free here; this path is for a keyed
    /// OpenAI-compatible provider). Esc quits (`first_run = true`).
    pub fn new() -> Self {
        Self {
            step: 0,
            field: 0,
            endpoint: DEFAULT_BASE_URL.to_string(),
            api_key: String::new(),
            model: String::new(),
            query: String::new(),
            result_sel: 0,
            first_run: true,
            from_picker: false,
        }
    }

    /// Construct a wizard pre-populated with existing credentials, starting on
    /// step 0 / field 0. The endpoint defaults to [`DEFAULT_BASE_URL`] (the
    /// legacy/remembered creds carry no endpoint — that path was always
    /// OpenRouter). An empty `model` stays empty so the user must pick (or keep)
    /// a real catalogue id — not auto-filled with the free-tier default.
    ///
    /// - `first_run = true`:   Esc quits (no usable Chat fallback).
    /// - `from_picker = true`: Esc returns to the session picker.
    pub fn prefilled(api_key: String, model: String, first_run: bool, from_picker: bool) -> Self {
        Self {
            step: 0,
            field: 0,
            endpoint: DEFAULT_BASE_URL.to_string(),
            api_key,
            model,
            query: String::new(),
            result_sel: 0,
            first_run,
            from_picker,
        }
    }

    /// Number of fields on the current step (step 0 has 2, step 1 has 1).
    fn field_count(&self) -> usize {
        if self.step == 0 {
            2
        } else {
            1
        }
    }

    /// `true` when the entered endpoint is non-empty, so its `/models` catalogue
    /// can be fetched and step 1 becomes a live omnisearch (works for ANY
    /// OpenAI-compatible endpoint, not just OpenRouter). A blank endpoint → step 1
    /// stays a plain text Model box.
    pub fn is_omnisearchable(&self) -> bool {
        !self.endpoint.trim().is_empty()
    }

    /// Advance to the next field within the current step (clamped at the last
    /// field of the step).
    pub fn next_field(&mut self) {
        if self.field + 1 < self.field_count() {
            self.field += 1;
        }
    }

    /// Move to the previous field within the current step (clamps at zero).
    pub fn prev_field(&mut self) {
        if self.field > 0 {
            self.field -= 1;
        }
    }

    /// `true` when the cursor is on the last field of the current step. Used by
    /// the controller to decide whether Enter advances the cursor or the step.
    pub fn is_last_field(&self) -> bool {
        self.field + 1 == self.field_count()
    }

    /// Append a character to whichever field is currently active.
    pub fn push_char(&mut self, c: char) {
        match (self.step, self.field) {
            (0, 0) => self.endpoint.push(c),
            (0, _) => self.api_key.push(c),
            // Step 1 only has the model field.
            (_, _) => self.model.push(c),
        }
    }

    /// Delete the last character from the active field.
    pub fn backspace(&mut self) {
        match (self.step, self.field) {
            (0, 0) => {
                self.endpoint.pop();
            }
            (0, _) => {
                self.api_key.pop();
            }
            (_, _) => {
                self.model.pop();
            }
        };
    }

    /// Move from the connection step (0) to the model step (1), resetting the
    /// field cursor to the first field of the new step. The omnisearch query +
    /// result cursor are cleared so the model step opens with a blank search
    /// (the dropdown shows the top of the catalogue). No-op past step 1.
    pub fn advance_step(&mut self) {
        if self.step == 0 {
            self.step = 1;
            self.field = 0;
            self.query.clear();
            self.result_sel = 0;
        }
    }

    /// Move back from the model step (1) to the connection step (0). Returns the
    /// field cursor to the last field of step 0 (the API key) so the user lands
    /// where they left off. The omnisearch query + result cursor are cleared so a
    /// later re-advance opens fresh. No-op at step 0.
    pub fn back_step(&mut self) {
        if self.step == 1 {
            self.step = 0;
            // Step 0's last field (API key) — where the user advanced from.
            self.field = 1;
            self.query.clear();
            self.result_sel = 0;
        }
    }

    /// Minimal completion gate: on the model step with a non-empty model AND a
    /// non-empty key entered back on step 0. The controller already blocks the
    /// step-0→1 advance on an empty key, so reaching step 1 implies a key was
    /// given; this re-checks it for a single authoritative "can finish" answer.
    pub fn can_finish(&self) -> bool {
        self.step == 1 && !self.model.trim().is_empty() && !self.api_key.trim().is_empty()
    }
}
