//! [`ExtScreenState`] — the client-side state for one EXTENSION-DRIVEN TUI screen
//! (server-driven UI over the TUI SCREEN PROTOCOL v1; see
//! [`crate::app::ext::screen`] for the wire contract).
//!
//! koma renders a `Screen` model the extension supplies (title + body nodes + footer) and
//! owns ONLY the menu cursor: Up/Down walk the union of every menu node's items, Enter sends
//! the highlighted item id back as a `tui-select`. Everything else (which screen to show,
//! what the menu says) is decided by the extension and pushed/replied over the socket. The
//! reconstructed state is render-only on an attached client — keys are forwarded to the
//! daemon, which owns the invoke + the mode.

/// Working state for one open extension TUI screen.
#[derive(Debug, Clone)]
pub struct ExtScreenState {
    /// The backing extension's manifest id (the invoke target).
    pub ext_id: String,
    /// The tui-screen id (the `panelId` passed on every `panel.msg` invoke for this screen).
    pub screen_id: String,
    /// The declared screen title (the fallback header shown until — and if — the
    /// extension's `Screen` supplies its own `title`, and while `waiting`).
    pub screen_title: String,
    /// The current `Screen` model to render (`{ title?, body:[Node], footer? }`), or `None`
    /// before the first reply/push has landed.
    pub screen: Option<serde_json::Value>,
    /// Cursor over the UNION of every menu node's items (host-side menu navigation).
    pub menu_cursor: usize,
    /// `true` while an invoke (tui-open / tui-select) is in flight — the view shows a
    /// one-line "loading…" state and Enter is inert until the reply lands.
    pub waiting: bool,
    /// Last invoke error/timeout, shown as a one-line error (Esc still works).
    pub error: Option<String>,
}

impl ExtScreenState {
    /// Open a fresh screen for `ext_id`/`screen_id` with the declared `title`. Starts with no
    /// `Screen` (the caller kicks off the `tui-open` invoke and flips `waiting`).
    pub fn new(ext_id: String, screen_id: String, screen_title: String) -> Self {
        Self {
            ext_id,
            screen_id,
            screen_title,
            screen: None,
            menu_cursor: 0,
            waiting: false,
            error: None,
        }
    }

    /// The `(id, label)` of every menu item in the current screen, in body order across ALL
    /// menu nodes (the navigable union).
    pub fn menu_entries(&self) -> Vec<(String, String)> {
        screen_menu_entries(self.screen.as_ref())
    }

    /// The id of the menu item under the cursor, or `None` when the screen has no menu.
    pub fn selected_menu_item(&self) -> Option<String> {
        self.menu_entries()
            .get(self.menu_cursor)
            .map(|(id, _)| id.clone())
    }

    /// Move the menu cursor up.
    pub fn menu_up(&mut self) {
        self.menu_cursor = self.menu_cursor.saturating_sub(1);
    }

    /// Move the menu cursor down (bounded by the menu length).
    pub fn menu_down(&mut self) {
        let n = self.menu_entries().len();
        if n > 0 && self.menu_cursor + 1 < n {
            self.menu_cursor += 1;
        }
    }

    /// Clamp the menu cursor into range after a new screen lands (empty menu → 0).
    pub fn clamp_menu(&mut self) {
        let n = self.menu_entries().len();
        if n == 0 {
            self.menu_cursor = 0;
        } else if self.menu_cursor >= n {
            self.menu_cursor = n - 1;
        }
    }
}

/// Extract the `(id, label)` of every menu item in `screen`'s body, in order across ALL
/// `{ "t": "menu" }` nodes (the navigable union). Items with an empty/missing `id` are
/// skipped (they can't be selected). Shared by [`ExtScreenState`] (navigation) and the
/// renderer (`crate::view::extscreen`), so the highlighted row and the sent id never drift.
/// A `None` screen — or a screen with no menu — yields an empty list.
pub(crate) fn screen_menu_entries(screen: Option<&serde_json::Value>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Some(screen) = screen else {
        return out;
    };
    let Some(body) = screen.get("body").and_then(|b| b.as_array()) else {
        return out;
    };
    for node in body {
        if node.get("t").and_then(|t| t.as_str()) != Some("menu") {
            continue;
        }
        let Some(items) = node.get("items").and_then(|i| i.as_array()) else {
            continue;
        };
        for item in items {
            let id = item
                .get("id")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string();
            if id.is_empty() {
                continue;
            }
            let label = item
                .get("label")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string();
            out.push((id, label));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn menu_entries_union_and_skips_blank_ids() {
        let screen = json!({
            "title": "Home",
            "body": [
                { "t": "text", "text": "pick one" },
                { "t": "menu", "items": [
                    { "id": "a", "label": "Alpha" },
                    { "id": "", "label": "skipme" }
                ]},
                { "t": "divider" },
                { "t": "menu", "items": [ { "id": "b", "label": "Beta" } ] }
            ]
        });
        let mut st = ExtScreenState::new("x".into(), "s".into(), "Home".into());
        st.screen = Some(screen);
        let entries = st.menu_entries();
        assert_eq!(entries, vec![
            ("a".to_string(), "Alpha".to_string()),
            ("b".to_string(), "Beta".to_string()),
        ]);
        // Cursor clamps + selects across the union.
        st.menu_cursor = 5;
        st.clamp_menu();
        assert_eq!(st.menu_cursor, 1);
        assert_eq!(st.selected_menu_item().as_deref(), Some("b"));
    }

    #[test]
    fn no_menu_is_empty_and_selects_nothing() {
        let st = ExtScreenState::new("x".into(), "s".into(), "t".into());
        assert!(st.menu_entries().is_empty());
        assert!(st.selected_menu_item().is_none());
    }
}
