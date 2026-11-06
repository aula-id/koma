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
mod tests {
    use super::*;

    /// The baseline. Regenerate deliberately (and say why in the commit) when the
    /// disabled-mode contract genuinely changes.
    #[test]
    fn disabled_mode_refusals_are_byte_stable() {
        let names = ["sec_http", "sec_ffuf", "sec_nope"];
        let got = render(&refusals(false, &names));
        let want = "\
sec_http -> refused: cyber lane off — enable security mode in /security first
sec_ffuf -> refused: cyber lane off — enable security mode in /security first
sec_nope -> refused: cyber lane off — enable security mode in /security first";
        assert_eq!(got, want, "\n--- got ---\n{got}\n--- want ---\n{want}");
    }

    #[test]
    fn ordinary_tools_are_never_gated() {
        for tool in ordinary_tools() {
            assert!(
                crate::security::sec_refusal(tool, false, |_| false).is_none(),
                "{tool} was gated"
            );
        }
    }

    #[test]
    fn enabling_changes_only_the_sec_names() {
        // The ordinary set stays admitted whatever the lane state.
        let off = refusals(false, &ordinary_tools());
        let on = refusals(true, &ordinary_tools());
        assert_eq!(off, on);
        for (_, got) in on {
            assert!(got.is_none());
        }
    }

    #[test]
    fn a_forged_name_is_refused_in_both_states_but_diagnosed_differently() {
        // The refusal order is lane-off, then advertised-ness, then availability. So
        // OFF reports the lane for a forged name too — the lane is the thing the user
        // turned off — and ON reports the name. Pinned in both states because
        // reordering the checks silently changes the diagnosis without changing the
        // refusal.
        let off = refusals(false, &["sec_nope"]);
        let on = refusals(true, &["sec_nope"]);
        assert!(off[0].1.as_deref().unwrap_or("").contains("lane off"));
        assert!(on[0].1.as_deref().unwrap_or("").contains("not advertised"));
    }

    #[test]
    fn a_lane_off_never_reaches_the_advertised_check() {
        // Off wins, so a real tool and a forged one are indistinguishable while off.
        // That is intended: the lane, not the name, is what the user turned off.
        let rows = refusals(false, &["sec_http", "sec_nope"]);
        assert_eq!(rows[0].1, rows[1].1);
    }

    #[test]
    fn a_signalled_tool_is_never_refused_while_on() {
        // Availability is the LAST check, so with a lane on and a real name the
        // verdict follows the machine's runtime, not the fixture.
        let rows = refusals(true, &["sec_http"]);
        assert_eq!(rows[0].1.is_none(), crate::security::cyber_execution_available());
    }

    #[test]
    fn the_fake_advertises_deterministically() {
        let a = refusals(true, &["sec_http"]);
        let b = refusals(true, &["sec_http"]);
        assert_eq!(a, b);
    }

    #[test]
    fn lane_state_does_not_leak_into_the_fixture() {
        // `Lane` is per-session; the refusal predicate takes only `enabled`. If a
        // future change makes it lane-aware, this fixture is where that shows up.
        let lanes = [Lane::Research, Lane::Red, Lane::Blue];
        let first = refusals(true, &["sec_http"]);
        for _ in &lanes {
            assert_eq!(refusals(true, &["sec_http"]), first);
        }
    }
}
