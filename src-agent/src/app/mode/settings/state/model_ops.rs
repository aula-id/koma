//! Models Select screen helpers for [`SettingsState`]: open/save/delete and role
//! picker operations.

use super::super::{ModelDraft, ModelModal, RolePickerState};
use super::SettingsState;

impl SettingsState {
    // --- Models Select screen helpers ---

    /// `true` when the selected category is "Models Select".
    pub fn is_models_category(&self) -> bool {
        super::super::SETTING_CATEGORIES[self.cat].name == "Models Select"
    }

    /// Indices into `self.models` that pass the current filter, in order.
    pub fn visible_model_indices(&self) -> Vec<usize> {
        self.models
            .iter()
            .enumerate()
            .filter(|(_, m)| self.model_filter.matches(m.session_only))
            .map(|(i, _)| i)
            .collect()
    }

    /// Number of model rows visible under the current filter.
    pub fn visible_model_count(&self) -> usize {
        self.visible_model_indices().len()
    }

    /// `true` when the `[+add global]` button is highlighted.
    pub fn model_on_add_global(&self) -> bool {
        self.model_sel == self.visible_model_count()
    }

    /// `true` when the `[+add local]` button is highlighted.
    pub fn model_on_add_local(&self) -> bool {
        self.model_sel == self.visible_model_count() + 1
    }

    /// `true` when either add button is highlighted.
    pub fn model_on_add_button(&self) -> bool {
        self.model_on_add_global() || self.model_on_add_local()
    }

    /// Real index into `self.models` for the currently selected visible row, or
    /// `None` when a button row is selected.
    pub fn selected_model_index(&self) -> Option<usize> {
        let vis = self.visible_model_indices();
        vis.get(self.model_sel).copied()
    }

    /// Clamp `model_sel` to the valid range `0 ..= visible_model_count() + 1`.
    pub fn clamp_model_sel(&mut self) {
        let max = self.visible_model_count() + 1;
        if self.model_sel > max {
            self.model_sel = max;
        }
    }

    /// Move selection up in the models list; clears the delete-armed flag.
    pub fn model_up(&mut self) {
        self.model_sel = self.model_sel.saturating_sub(1);
        self.model_delete_armed = false;
    }

    /// Move selection down in the models list (max index = visible_count + 1
    /// which is the [+add local] button row); clears the delete-armed flag.
    pub fn model_down(&mut self) {
        let max = self.visible_model_count() + 1;
        self.model_sel = (self.model_sel + 1).min(max);
        self.model_delete_armed = false;
    }

    /// First Ctrl+X arms the delete; second Ctrl+X confirms it. No effect when
    /// an add-button row is selected. Operates on the real model index.
    pub fn model_arm_or_delete(&mut self) {
        if self.model_on_add_button() {
            return;
        }
        if self.model_delete_armed {
            // Compute the real index before mutating state.
            let real_idx = self.selected_model_index();
            if let Some(idx) = real_idx {
                self.models.remove(idx);
            }
            self.model_delete_armed = false;
            // Clamp to new visible range.
            self.clamp_model_sel();
        } else {
            self.model_delete_armed = true;
        }
    }

    /// Cancel the armed-delete state (any key other than Ctrl+X).
    pub fn model_disarm(&mut self) {
        self.model_delete_armed = false;
    }

    /// Open the add-model modal with a blank draft.
    ///
    /// `session_only`: `false` = global scope (`[+add global]`),
    /// `true` = session-local scope (`[+add local]`).
    /// The save button matching the chosen scope is pre-focused so the user
    /// can confirm without navigating (still able to switch via Left/Right).
    pub fn open_model_modal_add(&mut self, session_only: bool) {
        self.model_modal = Some(ModelModal::new_add(0, session_only));
        self.mm_preselect_save();
    }

    /// Open the edit-model modal, prefilled from `models[idx]`.
    ///
    /// `idx` must be a REAL index into `self.models` (not a visible position).
    /// The draft's `session_only` is carried into the modal so the save path
    /// knows the original scope, and the matching save button is pre-focused.
    pub fn open_model_modal_edit(&mut self, idx: usize) {
        if let Some(m) = self.models.get(idx) {
            self.model_modal = Some(ModelModal {
                editing_idx: Some(idx),
                uuid: m.uuid.clone(),
                name: m.name.clone(),
                provider_idx: m.provider_idx,
                model_id: m.model_id.clone(),
                field: 0,
                roles: m.roles.clone(),
                role_picker: None,
                query: String::new(),
                result_sel: 0,
                // Carry the stored route forward. `route_sel` starts at 0 and is
                // re-synced when endpoints load / the user navigates the list.
                route: m.route.clone(),
                route_sel: 0,
                endpoints: None,
                endpoints_loading: false,
                endpoints_for: None,
                // Inherit the original scope so Save/SaveSession default to it.
                session_only: m.session_only,
            });
        }
        self.mm_preselect_save();
    }

    /// Cycle the model filter forward (All → Local → Global → All) and clamp selection.
    pub fn model_filter_next(&mut self) {
        self.model_filter = self.model_filter.next();
        self.clamp_model_sel();
    }

    /// Cycle the model filter backward (All → Global → Local → All) and clamp selection.
    pub fn model_filter_prev(&mut self) {
        self.model_filter = self.model_filter.prev();
        self.clamp_model_sel();
    }

    /// Close the add/edit-model modal without saving.
    pub fn close_model_modal(&mut self) {
        self.model_modal = None;
    }

    /// Save the modal draft as a model (replace when editing, else append) and
    /// close the modal; selects the affected row.
    ///
    /// `session_only` is `true` when the caller chose "Save session" (scoped to
    /// this session only) and `false` for plain "Save" (global scope). Persistence
    /// is stubbed — the flag is stored in-memory only.
    ///
    /// Role-steal (per role): a model may hold several roles, but each role is
    /// globally unique. For EACH role the target now holds, that role is removed
    /// from every OTHER model before the target's full role set is written, so no
    /// two models ever share a role.
    pub fn save_model_modal(&mut self, session_only: bool) {
        if let Some(m) = self.model_modal.take() {
            let mut draft = ModelDraft {
                // Preserve the carried identity (edit) or the freshly minted one
                // (add); mint as a last resort if it somehow arrived empty.
                uuid: if m.uuid.is_empty() {
                    super::super::new_uuid()
                } else {
                    m.uuid.clone()
                },
                name: m.name.trim().to_string(),
                model_id: m.model_id.trim().to_string(),
                provider_idx: m.provider_idx,
                roles: m.roles.clone(),
                route: m.route.clone(),
                session_only,
            };

            // Determine the operation and target index:
            //   - Same scope (edit in place): replace the existing entry.
            //   - Scope changed (e.g. global entry saved as session): add a NEW
            //     copy with a fresh uuid; leave the original entry untouched so
            //     it keeps its scope and role in the global catalogue.
            //   - No editing_idx (new entry): push as usual.
            let editing = m.editing_idx.filter(|&i| i < self.models.len());
            let target_idx = match editing {
                Some(i) if self.models[i].session_only == session_only => {
                    // Same scope — replace in place (keep carried uuid).
                    i
                }
                Some(_) => {
                    // Scope changed — mint a fresh uuid so the original entry
                    // keeps its own identity; the new copy is appended.
                    draft.uuid = super::super::new_uuid();
                    self.models.len() // will be the pushed index
                }
                None => self.models.len(), // new entry — push
            };

            // Per-role steal: drop each claimed role from every OTHER model of
            // THE SAME SCOPE only. A session model must not strip roles from
            // global models, and vice versa, so both a global main and a session
            // main can coexist (with session winning via resolve_role).
            for (i, other) in self.models.iter_mut().enumerate() {
                if i != target_idx && other.session_only == draft.session_only {
                    other.roles.retain(|r| !draft.roles.contains(r));
                }
            }

            if target_idx < self.models.len() {
                // Replace in place (same-scope edit).
                self.models[target_idx] = draft;
            } else {
                // Append (new entry or cross-scope copy).
                self.models.push(draft);
            }
            // After mutating self.models, find the visible position of the
            // affected real index and move the selection there; clamp to valid
            // range if the saved entry is filtered out by the current filter.
            let saved_real = if target_idx < self.models.len() {
                target_idx
            } else {
                self.models.len().saturating_sub(1)
            };
            let vis = self.visible_model_indices();
            self.model_sel = vis.iter().position(|&r| r == saved_real)
                .unwrap_or_else(|| vis.len().min(self.model_sel));
        }
    }

    // --- Role checkbox picker overlay (EDIT-mode Role field) ---

    /// `true` when the Role checkbox picker overlay is open over the model modal.
    /// Lets the input layer route keys to the picker first (its own deepest
    /// nesting level) without borrowing the modal mutably to check.
    pub fn mm_role_picker_open(&self) -> bool {
        self.model_modal
            .as_ref()
            .map(|m| m.role_picker.is_some())
            .unwrap_or(false)
    }

    /// Open the Role checkbox picker, seeding its checked state from the modal's
    /// current `roles`. No-op when no modal is open.
    pub fn open_role_picker(&mut self) {
        if let Some(m) = self.model_modal.as_mut() {
            m.role_picker = Some(RolePickerState::from_roles(&m.roles));
        }
    }

    /// Confirm the Role picker: commit its selection into the modal's `roles`
    /// (the global per-role steal still runs later, on save) and close the
    /// overlay. No-op when no modal/picker is open.
    pub fn confirm_role_picker(&mut self) {
        if let Some(m) = self.model_modal.as_mut() {
            if let Some(p) = m.role_picker.take() {
                m.roles = p.selected_roles();
            }
        }
    }

    /// Cancel the Role picker without modifying `roles` (discard the selection).
    pub fn cancel_role_picker(&mut self) {
        if let Some(m) = self.model_modal.as_mut() {
            m.role_picker = None;
        }
    }

    /// Move the Role picker cursor up. No-op when the picker is closed.
    pub fn mm_role_picker_up(&mut self) {
        if let Some(p) = self.model_modal.as_mut().and_then(|m| m.role_picker.as_mut()) {
            p.up();
        }
    }

    /// Move the Role picker cursor down. No-op when the picker is closed.
    pub fn mm_role_picker_down(&mut self) {
        if let Some(p) = self.model_modal.as_mut().and_then(|m| m.role_picker.as_mut()) {
            p.down();
        }
    }

    /// Toggle the checkbox under the Role picker cursor. No-op when closed.
    pub fn mm_role_picker_toggle(&mut self) {
        if let Some(p) = self.model_modal.as_mut().and_then(|m| m.role_picker.as_mut()) {
            p.toggle();
        }
    }
}
