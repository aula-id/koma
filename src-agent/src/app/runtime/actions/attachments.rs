//! Action handlers for the Ctrl+P attachments panel.

use anyhow::Result;

use crate::app::mode::Mode;
use crate::app::state::AppState;
use crate::dto::chat::AttachmentKind;

/// Esc: close panel → Chat.
pub(super) fn handle_close_attachments(state: &mut AppState) -> Result<()> {
    *state.mode_mut() = Mode::Chat;
    state.rest.fg_mut().status = "ready".into();
    Ok(())
}

/// Open the panel from Chat (Ctrl+P).
pub(super) fn handle_open_attachments(state: &mut AppState) -> Result<()> {
    let pending = state.rest.fg().pending_attachments.clone();
    *state.mode_mut() = Mode::Attachments(Box::new(
        crate::app::mode::AttachmentsState::from_pending(&pending),
    ));
    Ok(())
}

/// Remove the selected attachment: drop from pending, strip marker from input.
pub(super) fn handle_attachments_remove_selected(state: &mut AppState) -> Result<()> {
    let (kind, n) = {
        let Mode::Attachments(st) = state.mode() else {
            return Ok(());
        };
        match st.current() {
            Some(a) => (a.kind, a.marker_n),
            None => return Ok(()),
        }
    };
    let marker = match kind {
        AttachmentKind::Image => format!("[Image #{n}]"),
        AttachmentKind::PastedText => crate::model::attachment::paste_marker(n),
    };
    // Strip first occurrence of the marker from the composer.
    {
        let fg = state.rest.fg_mut();
        if let Some(pos) = fg.input.find(&marker) {
            let end = pos + marker.len();
            fg.input.replace_range(pos..end, "");
            // Collapse double spaces left by the strip.
            while fg.input.contains("  ") {
                fg.input = fg.input.replace("  ", " ");
            }
            fg.cursor = fg.cursor.min(fg.input.chars().count());
        }
        fg.pending_attachments
            .retain(|a| !(a.kind == kind && a.marker_n == n));
        fg.reconcile_attachments();
    }
    // Refresh the open panel's list.
    let pending = state.rest.fg().pending_attachments.clone();
    if let Mode::Attachments(st) = state.mode_mut() {
        st.refresh_from_pending(&pending);
    }
    Ok(())
}
