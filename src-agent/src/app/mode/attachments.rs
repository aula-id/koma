//! Attachments panel (`Ctrl+P`): staged composer images + pasted-text chips.
//!
//! List overlay over Chat. Enter on a paste opens a nested full-screen
//! [`TextEditorState`]; Esc saves the body back to disk (replace only — no new
//! chip). `d` removes the selected attachment and strips its marker from the
//! composer. Image rows show path + remove; preview is deferred to existing
//! show_image / transcript paths.

use crate::app::mode::editor::TextEditorState;
use crate::dto::chat::{Attachment, AttachmentKind};

/// Working state for the attachments list overlay.
#[derive(Debug, Clone)]
pub struct AttachmentsState {
    /// Snapshot of staged attachments at open / last refresh (kind + n + path).
    pub items: Vec<Attachment>,
    /// LIST cursor into `items`.
    pub selected: usize,
    /// Nested paste editor: `(marker_n, editor)`. `None` = list view.
    pub editor: Option<(usize, TextEditorState)>,
}

impl AttachmentsState {
    /// Build from the current pending list (call when opening Ctrl+P).
    pub fn from_pending(pending: &[Attachment]) -> Self {
        Self {
            items: pending.to_vec(),
            selected: 0,
            editor: None,
        }
    }

    /// Re-sync list from live `pending_attachments` without dropping an open editor.
    pub fn refresh_from_pending(&mut self, pending: &[Attachment]) {
        if self.editor.is_some() {
            return;
        }
        let prev_key = self
            .items
            .get(self.selected)
            .map(|a| (a.kind, a.marker_n));
        self.items = pending.to_vec();
        self.selected = prev_key
            .and_then(|(k, n)| {
                self.items
                    .iter()
                    .position(|a| a.kind == k && a.marker_n == n)
            })
            .unwrap_or(0)
            .min(self.items.len().saturating_sub(1));
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if !self.items.is_empty() && self.selected + 1 < self.items.len() {
            self.selected += 1;
        }
    }

    pub fn current(&self) -> Option<&Attachment> {
        self.items.get(self.selected)
    }

    /// Open nested editor for the selected PastedText row. Loads body from
    /// `session_dir/rel_path`. No-op for images or missing selection.
    pub fn open_paste_editor(&mut self, session_dir: &std::path::Path) {
        let Some(att) = self.current().filter(|a| a.is_pasted_text()).cloned() else {
            return;
        };
        let body = std::fs::read_to_string(session_dir.join(&att.rel_path)).unwrap_or_default();
        self.editor = Some((att.marker_n, TextEditorState::from_text(&body)));
    }

    /// Commit editor buffer to disk for marker_n; clear nested editor.
    /// Returns `Ok(true)` if a file was written.
    pub fn commit_editor(&mut self, session_dir: &std::path::Path) -> anyhow::Result<bool> {
        let Some((n, ed)) = self.editor.take() else {
            return Ok(false);
        };
        let Some(att) = self
            .items
            .iter()
            .find(|a| a.is_pasted_text() && a.marker_n == n)
        else {
            return Ok(false);
        };
        let path = session_dir.join(&att.rel_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, ed.text())?;
        Ok(true)
    }

    /// Kind label for list rows.
    pub fn kind_label(kind: AttachmentKind) -> &'static str {
        match kind {
            AttachmentKind::Image => "image",
            AttachmentKind::PastedText => "paste",
        }
    }
}
