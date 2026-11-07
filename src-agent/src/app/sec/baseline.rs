//! Disabled-mode baseline.
//!
//! The point of this fixture is narrower than it looks: it is not a snapshot of the
//! whole session, it pins the three things security mode must not disturb while it
//! is OFF — which tools an ordinary turn may reach, which names are refused, and
//! the exact refusal reasons. Those are byte-compared, because a refusal is
//! user-facing text and a reworded denial is a behaviour change even when the
//! verdict matches.
//!
//! Deliberately NOT a model transcript. A live-model answer includes temperature
//! and provider wobble, so an oracle built from one fails for reasons unrelated to
//! the change under test. Everything here is deterministic.
//!
//! Honest limitation, recorded rather than hidden: this was captured AFTER phase 1
//! had already landed, so it pins the post-wiring behaviour. It is a regression
//! guard from here, not a pre-change record of the original tree.

use super::fake_daemon::{self, Script};
use crate::app::sec::lane::Lane;

/// A daemon advertising the whole `sec_` namespace, as a configured install does.
fn advertised() -> Vec<&'static str> {
    vec![
        "sec_http",
        "sec_browser",
        "sec_ffuf",
        "sec_nuclei",
        "sec_sqlmap",
        "sec_decode",
        "sec_hashid",
        "sec_factor",
    ]
}

/// Refusal reasons for a set of names at a given lane state.
///
/// `advertised` mirrors what the manager holds, so the check exercises the real
/// predicate rather than a copy of it.
pub fn refusals(enabled: bool, names: &[&str]) -> Vec<(String, Option<String>)> {
    let fake = fake_daemon::shared(Script::new().advertising(&advertised()));
    let adv: Vec<String> = fake.lock().map(|s| s.advertised().to_vec()).unwrap_or_default();
    names
        .iter()
        .map(|n| {
            let got = crate::security::sec_refusal(n, enabled, |name| {
                adv.iter().any(|a| a == name)
            });
            ((*n).to_string(), got)
        })
        .collect()
}

/// Render refusals as the stable comparison text.
#[must_use]
pub fn render(rows: &[(String, Option<String>)]) -> String {
    rows.iter()
        .map(|(name, got)| match got {
            Some(reason) => format!("{name} -> refused: {reason}"),
            None => format!("{name} -> admitted"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Ordinary (non-`sec_`) tools must be untouched by security mode being off.
#[must_use]
pub fn ordinary_tools() -> Vec<&'static str> {
    vec!["read", "grep", "glob", "dir_list", "bash", "edit", "write", "task"]
}

#[cfg(test)]
#[path = "baseline_test.rs"]
mod tests;

