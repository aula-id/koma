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
#[path = "active_test.rs"]
mod tests;


#[cfg(test)]
#[path = "active_stop_during_run_test.rs"]
mod stop_during_run;


#[cfg(test)]
#[path = "active_inheritance_test.rs"]
mod inheritance;


#[cfg(test)]
#[path = "active_roles_test.rs"]
mod roles;

