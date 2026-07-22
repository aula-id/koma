//! Controller – key handler for the `/usage` cost dashboard (Usage mode).
//!
//! Handled keys:
//! - `Tab`   – toggle Global / Session view.
//! - `1`–`3` – set date range: Today / Week / Year (View A only).
//! - `m`     – toggle metric: Cost / Tokens.
//! - `Esc`   – close dashboard and return to Chat.
//!
//! All other keys are silently ignored.

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use super::Action;
use crate::app::mode::{UsageMetric, UsageNavState, UsageRange, UsageView};

/// Handle a key press while the usage dashboard is open.
///
/// Mutates `nav` in place for navigation keys; returns the appropriate
/// [`Action`] for keys that require runtime involvement.
pub(super) fn handle_usage(nav: &mut UsageNavState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => Action::CloseUsage,

        // Tab toggles between Global and Session views.
        KeyCode::Tab => {
            nav.view = match nav.view {
                UsageView::Global => UsageView::Session,
                UsageView::Session => UsageView::Global,
            };
            Action::None
        }

        // 1-3 set the date range (only meaningful in Global view, but we
        // accept the key in both views so the user doesn't have to think
        // about which view they're in).
        KeyCode::Char('1') => {
            nav.range = UsageRange::Today;
            Action::None
        }
        KeyCode::Char('2') => {
            nav.range = UsageRange::Week;
            Action::None
        }
        KeyCode::Char('3') => {
            nav.range = UsageRange::Year;
            Action::None
        }

        // 'm' toggles between cost and token intensity.
        KeyCode::Char('m') => {
            nav.metric = match nav.metric {
                UsageMetric::Cost => UsageMetric::Tokens,
                UsageMetric::Tokens => UsageMetric::Cost,
            };
            Action::None
        }

        _ => Action::None,
    }
}
