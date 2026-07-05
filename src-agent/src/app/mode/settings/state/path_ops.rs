//! Path-list accessors, list management, and filesystem picker operations for
//! [`SettingsState`].

use super::super::picker::{PathPicker, PickerMode};
use super::super::SettingField;
use super::SettingsState;

impl SettingsState {
    // --- Path-list field accessors ---

    /// Return a mutable reference to the text draft for `f`, or `None` for
    /// non-text fields (Theme, Accent, the awareness toggles).
    pub fn text_draft_mut(&mut self, f: SettingField) -> Option<&mut String> {
        match f {
            SettingField::ApiKey   => Some(&mut self.api_key),
            SettingField::Model    => Some(&mut self.model),
            SettingField::Provider => Some(&mut self.provider),
            SettingField::Name     => Some(&mut self.name),
            SettingField::AwarenessModel if !self.awareness_inherit => {
                Some(&mut self.awareness_model)
            }
            SettingField::AwarenessProvider if !self.awareness_inherit => {
                Some(&mut self.awareness_provider)
            }
            SettingField::ClassifierModel    => Some(&mut self.classifier_model),
            SettingField::ClassifierProvider => Some(&mut self.classifier_provider),
            SettingField::Workdir
            | SettingField::AllowedFolders
            | SettingField::Theme
            | SettingField::Accent
            | SettingField::AwarenessEnabled
            | SettingField::AwarenessSource
            | SettingField::AwarenessModel
            | SettingField::AwarenessProvider
            | SettingField::ClassifierEnabled
            | SettingField::ShortSendEnabled
            | SettingField::SlidingCache
            | SettingField::BashSaving
            | SettingField::InternetMode => None,
        }
    }

    /// Whether `f` is a managed PATH LIST (Workdir or Allowed dirs).
    pub fn is_path_list(f: SettingField) -> bool {
        matches!(f, SettingField::Workdir | SettingField::AllowedFolders)
    }

    /// Mutable handle to the path-list draft vec for `f`, or `None` if `f` is
    /// not a path-list field.
    pub fn path_list_mut(&mut self, f: SettingField) -> Option<&mut Vec<String>> {
        match f {
            SettingField::Workdir        => Some(&mut self.workdir),
            SettingField::AllowedFolders => Some(&mut self.allowed_folders),
            _ => None,
        }
    }

    /// Immutable handle to the path-list draft vec for `f` (view-side reads).
    pub fn path_list(&self, f: SettingField) -> Option<&Vec<String>> {
        match f {
            SettingField::Workdir        => Some(&self.workdir),
            SettingField::AllowedFolders => Some(&self.allowed_folders),
            _ => None,
        }
    }

    // --- Path-list management ---

    /// Move the highlighted list entry up (clamps at 0).
    pub fn list_up(&mut self) {
        self.list_sel = self.list_sel.saturating_sub(1);
    }

    /// Move the highlighted list entry down (clamps at the last entry).
    pub fn list_down(&mut self) {
        let len = self
            .path_list(self.current_field())
            .map(|v| v.len())
            .unwrap_or(0);
        if self.list_sel + 1 < len {
            self.list_sel += 1;
        }
    }

    /// Remove the highlighted entry, honouring the min-1 rule.
    pub fn list_remove(&mut self) {
        let f = self.current_field();
        let sel = self.list_sel;
        if let Some(v) = self.path_list_mut(f) {
            if v.len() > 1 && sel < v.len() {
                v.remove(sel);
            }
        }
        let len = self.path_list(f).map(|v| v.len()).unwrap_or(0);
        if self.list_sel >= len {
            self.list_sel = len.saturating_sub(1);
        }
    }

    // --- Filesystem picker operations ---

    /// Open the FS picker in ADD mode.
    pub fn open_picker_add(&mut self) {
        self.picker = Some(PathPicker::new(PickerMode::Add, String::new(), &self.cwd));
    }

    /// Open the FS picker in REPLACE mode for the highlighted entry.
    pub fn open_picker_replace(&mut self) {
        let f = self.current_field();
        let sel = self.list_sel;
        let seed = self
            .path_list(f)
            .and_then(|v| v.get(sel))
            .cloned()
            .unwrap_or_default();
        self.picker = Some(PathPicker::new(PickerMode::Replace(sel), seed, &self.cwd));
    }

    /// Confirm the active picker: apply the chosen path to the target list.
    pub fn picker_confirm(&mut self) {
        let Some(picker) = self.picker.take() else {
            return;
        };
        let chosen = picker
            .selected()
            .cloned()
            .unwrap_or_else(|| picker.query.clone());
        let chosen = chosen.strip_prefix('@').unwrap_or(&chosen).trim().to_string();
        if chosen.is_empty() {
            return;
        }
        let f = self.current_field();
        match picker.mode {
            PickerMode::Add => {
                if let Some(v) = self.path_list_mut(f) {
                    v.push(chosen);
                    self.list_sel = v.len().saturating_sub(1);
                }
            }
            PickerMode::Replace(i) => {
                if let Some(v) = self.path_list_mut(f) {
                    if let Some(slot) = v.get_mut(i) {
                        *slot = chosen;
                    }
                }
            }
        }
    }

    /// Cancel the active picker without applying anything.
    pub fn picker_cancel(&mut self) {
        self.picker = None;
    }

    /// Append `c` to the picker query and recompute matches.
    pub fn picker_push_char(&mut self, c: char) {
        let cwd = self.cwd.clone();
        if let Some(p) = self.picker.as_mut() {
            p.query.push(c);
            p.recompute(&cwd);
        }
    }

    /// Delete the last char of the picker query and recompute matches.
    pub fn picker_backspace(&mut self) {
        let cwd = self.cwd.clone();
        if let Some(p) = self.picker.as_mut() {
            p.query.pop();
            p.recompute(&cwd);
        }
    }

    /// Drill into the currently highlighted match.
    pub fn picker_descend(&mut self) {
        let cwd = self.cwd.clone();
        if let Some(p) = self.picker.as_mut() {
            if let Some(sel) = p.selected().cloned() {
                p.query = format!("{sel}/");
                p.sel = 0;
                p.recompute(&cwd);
            }
        }
    }
}
