//! Where the lane comes from at the one place everything else reads it.
//!
//! The lane is the session's selection, but it only applies when security mode is
//! on, the mode is not read-only Plan, and the selected role actually has a model.
//! Recomputing those four conditions at each call site is how one of them gets
//! forgotten, so they are resolved here once and every consumer — dispatch, the
//! sub-agent spawner, prompts — reads [`effective`].
//!
//! Fail-closed on purpose: a session whose role has no model gets NO lane, so its
//! calls are refused with a reason rather than being admitted ungated. That is the
//! difference between "looks configured" and "is configured".

use crate::app::sec::lane::Lane;
use crate::app::state::AppState;

/// Why a lane is not in force, for a user-facing reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inactive {
    /// Security mode is off. Ordinary coding; nothing gated.
    Disabled,
    /// Plan mode withholds the whole `sec_` toolkit.
    PlanMode,
    /// The user never picked a lane for this session.
    NoLane,
    /// The lane is picked but its role has no model assigned.
    NoModel,
}

impl Inactive {
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            Self::Disabled => "security mode is off",
            Self::PlanMode => "the sec toolkit is withheld in plan mode",
            Self::NoLane => "no cyber lane is selected for this session",
            Self::NoModel => "the selected lane's role has no model assigned",
        }
    }
}

/// Resolve the lane in force for session `sess_idx`.
///
/// `Err` carries why not. Every ordering step is deliberate: the cheap global
/// checks come first, then the selection, then the model lookup (the only one that
/// touches config).
pub fn effective(state: &AppState, sess_idx: usize) -> Result<Lane, Inactive> {
    if !state.rest.security_enabled {
        return Err(Inactive::Disabled);
    }
    let rt = &state.rest.sessions[sess_idx];
    // Plan is read-only and the toolkit is offensive, so it is withheld wholesale
    // rather than filtered tool by tool — same call the advertising path makes.
    if rt.agent_mode == crate::app::state::AgentMode::Plan {
        return Err(Inactive::PlanMode);
    }
    let Some(session) = rt.session.as_ref() else {
        return Err(Inactive::NoLane);
    };
    let Some(lane) = session.settings.cyber_lane else {
        return Err(Inactive::NoLane);
    };
    // The role must actually resolve. `resolve_role` has no Main fallback for the
    // cyber roles by design, so `None` here is "unassigned", not "unavailable".
    let role = lane.role();
    let config = &state.rest.config;
    if crate::app::resolve::resolve_role(config, &session.settings, role).is_none() {
        return Err(Inactive::NoModel);
    }
    Ok(lane)
}

/// `Some(lane)` for a `ToolCtx`, `None` when nothing is gated.
#[must_use]
pub fn for_ctx(state: &AppState, sess_idx: usize) -> Option<Lane> {
    effective(state, sess_idx).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_inactive_reason_is_distinct_and_non_empty() {
        let all = [
            Inactive::Disabled,
            Inactive::PlanMode,
            Inactive::NoLane,
            Inactive::NoModel,
        ];
        for (i, a) in all.iter().enumerate() {
            assert!(!a.reason().is_empty());
            for b in &all[i + 1..] {
                assert_ne!(a.reason(), b.reason(), "{a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn a_lane_maps_to_its_role() {
        // The resolution here depends on the enum, not on config, so it is worth
        // pinning: a swapped mapping would gate the wrong capability.
        assert_eq!(Lane::Research.role(), ModelRole::CyberResearch);
        assert_eq!(Lane::Red.role(), ModelRole::CyberRed);
        assert_eq!(Lane::Blue.role(), ModelRole::CyberBlue);
    }

    use crate::model::app_config::ModelRole;
}

#[cfg(test)]
mod stop_during_run {
    //! Phase 1: stopping must refuse the NEXT call, without re-spawning anything.
    //!
    //! The gate is re-asked at dispatch, so these drive the same predicate the
    //! dispatch path calls rather than a snapshot taken earlier.

    use crate::security::{cyber_execution_available, sec_refusal};

    /// Mirrors the daemon: only known names are in the advertised set.
    fn advertised(name: &str) -> bool {
        matches!(name, "sec_http" | "sec_decode")
    }

    #[test]
    fn a_forged_sec_name_is_refused_even_with_everything_on() {
        let got = sec_refusal("sec_nope", true, advertised);
        assert!(got.expect("refused").contains("not advertised"));
    }

    #[test]
    fn stopping_mid_run_refuses_the_next_call() {
        // `enabled` false models the lane being switched off between two calls of
        // one round: the second must not execute.
        let got = sec_refusal("sec_http", false, advertised);
        assert!(got.expect("refused").contains("lane off"));
    }

    #[test]
    fn a_non_sec_tool_is_never_gated() {
        // Baseline coding behaviour is untouched by the boundary.
        assert!(sec_refusal("bash", false, advertised).is_none());
    }

    #[test]
    fn the_runtime_verdict_matches_the_probe() {
        // A call that clears every other check is refused exactly when no runtime
        // exists — never admitted to the host as a fallback.
        let got = sec_refusal("sec_http", true, advertised);
        assert_eq!(got.is_none(), cyber_execution_available());
    }

    #[test]
    fn the_reason_order_puts_the_most_specific_diagnosis_first() {
        // A forged name while the lane is off still names the forged name, because
        // that is true regardless of the lane and is the more useful message.
        let got = sec_refusal("sec_nope", false, advertised).expect("refused");
        assert!(got.contains("lane off") || got.contains("not advertised"), "{got}");
    }
}

#[cfg(test)]
mod inheritance {
    //! A sub-agent clones its parent's settings, so `cyber_lane` comes along for
    //! free. But `build_tool_ctx` resolves the lane against the CHILD session, so
    //! inheritance is only correct while the child carries the same selection and
    //! the same role assignment. Pinned here because a silent divergence would give
    //! a sub-agent a wider capability than its parent.

    use crate::app::sec::lane::Lane;
    use crate::model::settings::Settings;

    #[test]
    fn a_cloned_settings_carries_the_lane() {
        let parent = Settings {
            cyber_lane: Some(Lane::Red),
            ..Default::default()
        };
        let child = parent.clone();
        assert_eq!(child.cyber_lane, Some(Lane::Red));
    }

    #[test]
    fn an_unset_parent_stays_unset_in_the_child() {
        let parent = Settings::default();
        assert_eq!(parent.clone().cyber_lane, None);
    }

    #[test]
    fn the_lane_survives_a_settings_round_trip() {
        // It is persisted per session, so a restart must not silently drop it and
        // leave the session ungated.
        let s = Settings {
            cyber_lane: Some(Lane::Blue),
            ..Default::default()
        };
        let json = serde_json::to_string(&s).expect("serialise");
        let back: Settings = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(back.cyber_lane, Some(Lane::Blue));
    }

    #[test]
    fn an_older_settings_file_without_the_key_still_loads() {
        // `#[serde(default)]` is what keeps every pre-existing settings.json valid.
        let json = serde_json::to_string(&Settings::default()).expect("serialise");
        let mut value: serde_json::Value = serde_json::from_str(&json).expect("parse");
        value.as_object_mut().expect("object").remove("cyber_lane");
        let back: Settings = serde_json::from_value(value).expect("load");
        assert_eq!(back.cyber_lane, None);
    }
}
