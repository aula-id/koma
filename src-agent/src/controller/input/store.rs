//! Key handler for the `/store` marketplace browser (`Mode::ExtStore`).
//!
//! A network-backed sibling of [`super::extensions`]: three sub-modes (deepest first):
//!
//! 0. **InstallConfirm** – modal y/n; `y` kicks off the async install download
//!    (`Action::StoreInstallConfirm`, gated on a koma.run bearer being on file), `n`/Esc
//!    cancels back to Detail.
//! 1. **Detail** – `i` arms InstallConfirm for a not-yet-installed extension (refreshing
//!    the bearer check right at the boundary); Esc returns to Browse.
//! 2. **Browse** – ↑/↓ move the LIST cursor; Enter opens the selected row's Detail
//!    (`Action::StoreOpenDetail`); `r` retries a failed catalogue fetch
//!    (`Action::StoreRetryBrowse`); Esc closes the browser (`Action::CloseStore`).

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::app::mode::{ExtStoreState, StoreSubMode};
use crate::app::state::AppStateRest;
use crate::model::app_config::OAuthProvider;

use super::Action;

/// Handle a key press inside the `/store` marketplace browser.
pub fn handle_store(s: &mut ExtStoreState, rest: &mut AppStateRest, key: KeyEvent) -> Action {
    match s.sub_mode {
        // --- InstallConfirm: modal y/n, gated on a koma.run bearer ---
        StoreSubMode::InstallConfirm => match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                if s.komarun_connected {
                    Action::StoreInstallConfirm
                } else {
                    Action::None
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                s.sub_mode = StoreSubMode::Detail;
                Action::None
            }
            _ => Action::None,
        },

        // --- Detail: read the fetched detail + arm an install ---
        StoreSubMode::Detail => match key.code {
            KeyCode::Esc => {
                s.sub_mode = StoreSubMode::Browse;
                Action::None
            }
            KeyCode::Char('i') | KeyCode::Char('I') => {
                if s.current().map(|r| !r.installed).unwrap_or(false) {
                    // Refresh the bearer check right at the confirm boundary — the
                    // connection could have been added/removed since Browse opened.
                    s.komarun_connected = rest
                        .config
                        .oauth_conns
                        .iter()
                        .any(|c| c.provider == OAuthProvider::KomaRun);
                    s.install_error = None;
                    s.sub_mode = StoreSubMode::InstallConfirm;
                }
                Action::None
            }
            _ => Action::None,
        },

        // --- Browse: navigate the fetched catalogue LIST ---
        StoreSubMode::Browse => match key.code {
            KeyCode::Esc => Action::CloseStore,
            KeyCode::Up => {
                s.list_up();
                Action::None
            }
            KeyCode::Down | KeyCode::Tab => {
                s.list_down();
                Action::None
            }
            KeyCode::Enter => {
                if s.current().is_some() {
                    Action::StoreOpenDetail
                } else {
                    Action::None
                }
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                if s.error.is_some() {
                    Action::StoreRetryBrowse
                } else {
                    Action::None
                }
            }
            _ => Action::None,
        },
    }
}
