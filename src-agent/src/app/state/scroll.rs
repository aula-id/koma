//! Scroll methods on [`super::AppStateRest`].
//!
//! The transcript `scroll` offset + `follow` flag now live on the foreground
//! [`super::SessionRuntime`] (each session carries its own view position), so
//! these wrappers route through `fg_mut()`. `last_max_scroll` STAYS on
//! `AppStateRest`: it is a render-feedback [`std::cell::Cell`] the renderer
//! writes through a shared `&AppStateRest` (no `&mut`), and it is shared with the
//! global agent-viewer scroll path (`agent_viewer_scroll_*` below) — so it is
//! read here into a local BEFORE taking the `&mut` borrow of the foreground
//! session. `scroll` is an offset-from-top used only when NOT following; `follow`
//! pins the view to the bottom.

use super::rest::AppStateRest;

impl AppStateRest {
    pub fn scroll_up(&mut self) {
        // Read the render-feedback cell first (it stays on AppStateRest) so the
        // subsequent `fg_mut()` mutable borrow doesn't conflict.
        let max = self.last_max_scroll.get();
        let fg = self.fg_mut();
        if fg.follow {
            // Leave follow starting from the current bottom offset.
            fg.follow = false;
            fg.scroll = max;
        }
        fg.scroll = fg.scroll.saturating_sub(1);
    }

    pub fn scroll_down(&mut self) {
        let max = self.last_max_scroll.get();
        let fg = self.fg_mut();
        if fg.follow {
            return; // already pinned to the bottom
        }
        fg.scroll = fg.scroll.saturating_add(1);
        if fg.scroll >= max {
            fg.follow = true; // back at the bottom → resume following
        }
    }

    pub fn reset_scroll(&mut self) {
        let fg = self.fg_mut();
        fg.follow = true;
        fg.scroll = 0;
    }

    /// Scroll the full-screen sub-agent viewer up by `n` lines.
    ///
    /// If currently following (pinned to bottom), detaches follow and seeds the
    /// offset from the current bottom so the view doesn't jump to the top.
    /// Clamped at 0.
    pub fn agent_viewer_scroll_up(&mut self, n: u16) {
        if self.agent_viewer_follow {
            // Detach: start from the current bottom offset.
            self.agent_viewer_follow = false;
            self.agent_viewer_scroll = self.last_max_scroll.get();
        } else if self.agent_viewer_scroll > self.last_max_scroll.get() {
            self.agent_viewer_scroll = self.last_max_scroll.get();
        }
        self.agent_viewer_scroll = self.agent_viewer_scroll.saturating_sub(n);
    }

    /// Scroll the full-screen sub-agent viewer down by `n` lines, clamped to the
    /// last-published max. Re-attaches follow when the offset reaches the bottom.
    pub fn agent_viewer_scroll_down(&mut self, n: u16) {
        if self.agent_viewer_follow {
            return; // already pinned to the bottom
        }
        let max = self.last_max_scroll.get();
        self.agent_viewer_scroll = self.agent_viewer_scroll.saturating_add(n).min(max);
        if self.agent_viewer_scroll >= max {
            self.agent_viewer_follow = true; // back at the bottom -> resume following
        }
    }
}
