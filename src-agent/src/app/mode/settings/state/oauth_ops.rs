//! OAuth submenu screen helpers for [`SettingsState`] — mirrors
//! `provider_ops.rs`'s shape (list nav + arm-delete) plus the connect-flow
//! transitions (`oauth_flow`).

use super::super::OAuthFlowState;
use super::SettingsState;

impl SettingsState {
    // --- OAuth screen: category predicate + list nav ---

    /// `true` when the selected category is "OAuth".
    pub fn is_oauth_category(&self) -> bool {
        super::super::SETTING_CATEGORIES[self.cat].name == "OAuth"
    }

    /// Move selection up in the connections list; clears the delete-armed flag.
    pub fn oauth_up(&mut self) {
        self.oauth_sel = self.oauth_sel.saturating_sub(1);
        self.oauth_armed = None;
    }

    /// Move selection down in the connections list (max index =
    /// `oauth_drafts.len()`, the `[+connect]` row); clears the delete-armed flag.
    pub fn oauth_down(&mut self) {
        self.oauth_sel = (self.oauth_sel + 1).min(self.oauth_drafts.len());
        self.oauth_armed = None;
    }

    /// `true` when the `[+connect]` button row is highlighted.
    pub fn oauth_on_add_button(&self) -> bool {
        self.oauth_sel == self.oauth_drafts.len()
    }

    /// First Ctrl+X arms the delete on the current row; the second CONFIRMS it
    /// and returns the connection's `uuid` for the caller (the runtime action) to
    /// actually remove from `config.oauth_conns` + persist + evict its token
    /// cache. No effect (returns `None`) on the `[+connect]` row.
    pub fn oauth_arm_or_delete(&mut self) -> Option<String> {
        if self.oauth_on_add_button() {
            return None;
        }
        if self.oauth_armed == Some(self.oauth_sel) {
            let uuid = self.oauth_drafts.get(self.oauth_sel).map(|d| d.uuid.clone());
            self.oauth_armed = None;
            uuid
        } else {
            self.oauth_armed = Some(self.oauth_sel);
            None
        }
    }

    /// Cancel the armed-delete state (any key other than Ctrl+X).
    pub fn oauth_disarm(&mut self) {
        self.oauth_armed = None;
    }

    // --- Connect flow: provider picker ---

    /// Open the provider picker (Enter on `[+connect]`).
    pub fn oauth_open_picker(&mut self) {
        self.oauth_flow = OAuthFlowState::Pick(0);
    }

    /// Move the picker cursor up (clamps at 0).
    pub fn oauth_pick_up(&mut self) {
        if let OAuthFlowState::Pick(c) = &mut self.oauth_flow {
            *c = c.saturating_sub(1);
        }
    }

    /// Move the picker cursor down (clamps at the last option, index 7).
    pub fn oauth_pick_down(&mut self) {
        if let OAuthFlowState::Pick(c) = &mut self.oauth_flow {
            *c = (*c + 1).min(7);
        }
    }

    // --- Connect flow: paste-token text field ---

    /// Append `c` to the paste-token draft (no-op off `CodexPaste`).
    pub fn oauth_paste_push_char(&mut self, c: char) {
        if let OAuthFlowState::CodexPaste { input, .. } = &mut self.oauth_flow {
            input.push(c);
        }
    }

    /// Delete the last character of the paste-token draft (no-op off `CodexPaste`).
    pub fn oauth_paste_backspace(&mut self) {
        if let OAuthFlowState::CodexPaste { input, .. } = &mut self.oauth_flow {
            input.pop();
        }
    }
}
