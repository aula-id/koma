//! API Providers screen helpers for [`SettingsState`].

use super::super::ProviderDraft;
use super::super::ProviderModal;
use super::SettingsState;

impl SettingsState {
    // --- API Providers screen helpers ---

    /// Move selection up in the providers list; clears the delete-armed flag.
    pub fn prov_up(&mut self) {
        self.prov_sel = self.prov_sel.saturating_sub(1);
        self.prov_delete_armed = false;
        self.prov_msg = None;
    }

    /// Move selection down in the providers list (max index = providers.len()
    /// which is the add-button row); clears the delete-armed flag.
    pub fn prov_down(&mut self) {
        self.prov_sel = (self.prov_sel + 1).min(self.providers.len());
        self.prov_delete_armed = false;
        self.prov_msg = None;
    }

    /// `true` when the `[+ add]` button row is highlighted.
    pub fn prov_on_add_button(&self) -> bool {
        self.prov_sel == self.providers.len()
    }

    /// First Ctrl+X arms the delete; second Ctrl+X confirms it.
    /// Has no effect when the add-button row is selected.
    ///
    /// W12b HOST-ENFORCED GUARD: an EXTENSION-managed provider (`ProviderDraft::ext_id`
    /// set) can never be deleted here — it is refused with a footer message directing the
    /// user to uninstall the owning extension instead. This mirrors the daemon-side reject
    /// (`delete_provider` / `apply_global_config_req`) so no editing surface can orphan an
    /// extension's key-backed gateway; the settings save also re-attaches any ext provider a
    /// draft omitted, so even a stale draft can't drop one.
    pub fn prov_arm_or_delete(&mut self) {
        if self.prov_on_add_button() {
            return;
        }
        // Refuse deleting an extension-managed provider (only uninstall removes it).
        if let Some(ext) = self
            .providers
            .get(self.prov_sel)
            .and_then(|p| p.ext_id.as_deref())
        {
            self.prov_msg = Some(format!("managed by extension {ext} — uninstall to remove"));
            self.prov_delete_armed = false;
            return;
        }
        if self.prov_delete_armed {
            // Confirm: remove the entry.
            if self.prov_sel < self.providers.len() {
                self.providers.remove(self.prov_sel);
            }
            self.prov_delete_armed = false;
            // Clamp selection to the new length.
            let max = self.providers.len(); // add-button index
            if self.prov_sel > max {
                self.prov_sel = max;
            }
        } else {
            self.prov_delete_armed = true;
        }
    }

    /// Cancel the armed-delete state (any key other than Ctrl+X).
    pub fn prov_disarm(&mut self) {
        self.prov_delete_armed = false;
        self.prov_msg = None;
    }

    /// Open the add-provider modal with a blank draft.
    pub fn open_provider_modal(&mut self) {
        self.prov_modal = Some(ProviderModal::new());
    }

    /// Close the add-provider modal without saving.
    pub fn close_provider_modal(&mut self) {
        self.prov_modal = None;
    }

    /// Save the modal draft as a new provider and close the modal. Mints a fresh
    /// uuid for the new entry (the add-provider modal only ever creates).
    pub fn save_provider_modal(&mut self) {
        if let Some(m) = self.prov_modal.take() {
            let name     = m.name.trim().to_string();
            let endpoint = m.endpoint.trim().to_string();
            let api_key  = m.api_key.trim().to_string();
            self.providers.push(ProviderDraft {
                uuid: super::super::new_uuid(),
                name,
                endpoint,
                api_type: m.api_type,
                api_key,
                // A user-created provider is never extension-managed.
                ext_id: None,
            });
            // Select the newly added entry (last real index).
            self.prov_sel = self.providers.len().saturating_sub(1);
        }
    }

    /// Move focus up in the modal (clamp 0..=4).
    pub fn modal_up(&mut self) {
        if let Some(m) = self.prov_modal.as_mut() {
            m.field = m.field.saturating_sub(1);
        }
    }

    /// Move focus down in the modal (clamp 0..=4).
    pub fn modal_down(&mut self) {
        if let Some(m) = self.prov_modal.as_mut() {
            m.field = (m.field + 1).min(4);
        }
    }

    /// Move focus left in the modal: moves Cancel→Save on field 4.
    pub fn modal_left(&mut self) {
        if let Some(m) = self.prov_modal.as_mut() {
            if m.field == 4 {
                m.field = 3;
            }
        }
    }

    /// Move focus right in the modal: moves Save→Cancel on field 3.
    pub fn modal_right(&mut self) {
        if let Some(m) = self.prov_modal.as_mut() {
            if m.field == 3 {
                m.field = 4;
            }
        }
    }

    /// Append `c` to the active text field in the modal (field 0=name, 1=endpoint, 2=api_key).
    pub fn modal_push_char(&mut self, c: char) {
        if let Some(m) = self.prov_modal.as_mut() {
            match m.field {
                0 => m.name.push(c),
                1 => m.endpoint.push(c),
                2 => m.api_key.push(c),
                _ => {}
            }
        }
    }

    /// Delete the last character of the active text field in the modal.
    pub fn modal_backspace(&mut self) {
        if let Some(m) = self.prov_modal.as_mut() {
            match m.field {
                0 => { m.name.pop(); }
                1 => { m.endpoint.pop(); }
                2 => { m.api_key.pop(); }
                _ => {}
            }
        }
    }
}
