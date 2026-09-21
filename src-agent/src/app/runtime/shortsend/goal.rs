//! Deterministic session-goal commitment (no LLM judge).
//!
//! DRSS must not invent “the plan” from draft-shaped archive text. Doctrine on
//! the wire is resolved from ranked sources: **user > mission active leaf >
//! charter > none**. Archive indexing never writes doctrine.

use crate::model::settings::Settings;

/// Result of inspecting the latest user message for a goal update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalPatch {
    /// Replace session goal with this one-line text.
    Set(String),
    /// Clear the committed goal (charter is preserved).
    Clear,
}

/// Ranked provenance of the effective objective on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalSource {
    None,
    User,
    Mission,
    Charter,
}

impl GoalSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::User => "user",
            Self::Mission => "mission",
            Self::Charter => "charter",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "user" => Self::User,
            "mission" => Self::Mission,
            "charter" => Self::Charter,
            _ => Self::None,
        }
    }
}

/// Best-effort mission snapshot for objective resolve (read-only).
#[derive(Debug, Clone, Default)]
pub struct MissionSnap {
    pub approved: bool,
    /// Frozen mission.goal (fallback if leaf title empty — rarely used).
    pub mission_goal: String,
    /// Exactly one active open leaf: `(id, title)`. Multiple actives → None.
    pub active_leaf: Option<(String, String)>,
}

/// Resolved doctrine for inject + fingerprinting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveGoal {
    pub source: GoalSource,
    /// Current doctrine line shown as objective (may be empty).
    pub objective: String,
    /// Kickoff charter (may equal objective for charter-only).
    pub charter: String,
    /// Stable id for transition detection (`user:…` / `mission:{id}` / `charter:…`).
    pub fingerprint: String,
}

/// Wire payload cloned into the reshape task (no settings re-resolve needed).
#[derive(Debug, Clone, Default)]
pub struct GoalWire {
    pub source: String,
    pub objective: String,
    pub charter: String,
}

impl GoalWire {
    pub fn from_effective(eg: &EffectiveGoal) -> Self {
        Self {
            source: eg.source.as_str().to_string(),
            objective: eg.objective.clone(),
            charter: eg.charter.clone(),
        }
    }
}

/// A stream-start refresh separates persistence changes from objective changes.
/// Only an effective fingerprint transition requests a consolidation fold.
pub struct GoalRefresh {
    pub wire: GoalWire,
    pub settings_changed: bool,
    #[cfg_attr(not(test), allow(dead_code))] // Transition regression tests inspect this flag.
    pub objective_changed: bool,
}

pub fn refresh_goal_state(
    settings: &mut Settings,
    last_user: Option<&str>,
    mission: Option<&MissionSnap>,
) -> GoalRefresh {
    let mut settings_changed = false;
    if let Some(user) = last_user {
        if let Some(patch) = detect_goal_update(user) {
            let (goal, source) = match patch {
                GoalPatch::Set(goal) => (goal, "user"),
                GoalPatch::Clear => (String::new(), "none"),
            };
            // Re-reading the same user message on every tool continuation is
            // idempotent, including the existing message provenance.
            if settings.session_goal != goal || settings.session_goal_source != source {
                settings.session_goal = goal;
                settings.session_goal_source = source.into();
                settings.session_goal_msg_id = 0;
                settings_changed = true;
            }
        }
        settings_changed |= seed_charter_if_empty(settings, user);
    }
    let effective = resolve_effective_goal(settings, mission);
    let objective_changed = settings.session_objective_fp != effective.fingerprint;
    if objective_changed {
        settings.session_objective_fp = effective.fingerprint.clone();
        settings_changed = true;
    }
    GoalRefresh {
        wire: GoalWire::from_effective(&effective),
        settings_changed,
        objective_changed,
    }
}

/// Cap stored goal length so doctrine stays a one-liner, not a pasted essay.
const GOAL_MAX_CHARS: usize = 240;

/// Detect an explicit goal set/clear from the latest user text.
///
/// Conservative on purpose: never promote assistant drafts. Short accepts
/// (`lgtm`, `do that`, …) are **not** doctrine — they return `None`.
pub fn detect_goal_update(user_text: &str) -> Option<GoalPatch> {
    let raw = user_text.trim();
    if raw.is_empty() {
        return None;
    }
    let lower = raw.to_ascii_lowercase();

    // Clear first so "cancel goal: …" doesn't also set.
    if is_clear_goal(&lower) {
        // "forget the plan, do X" → set trailing clause instead of bare clear.
        if let Some(rest) = strip_after_forget_plan(&lower, raw) {
            let g = clip_goal(rest);
            if !g.is_empty() {
                return Some(GoalPatch::Set(g));
            }
        }
        return Some(GoalPatch::Clear);
    }

    if let Some(rest) = strip_goal_prefix(&lower, raw) {
        let g = clip_goal(rest);
        if g.is_empty() {
            return None;
        }
        return Some(GoalPatch::Set(g));
    }

    None
}

/// Seed `session_charter` once from a real kickoff prompt. Returns true if written.
/// Does not set `session_goal` or source — charter alone feeds resolve step 3.
pub fn seed_charter_if_empty(settings: &mut Settings, last_user: &str) -> bool {
    if !settings.session_charter.trim().is_empty() {
        return false;
    }
    let raw = last_user.trim();
    if raw.is_empty() || looks_like_slash_only(raw) {
        return false;
    }
    let c = clip_goal(raw);
    if c.is_empty() {
        return false;
    }
    settings.session_charter = c;
    true
}

/// Ranked resolve: user > mission active leaf > charter > none.
pub fn resolve_effective_goal(
    settings: &Settings,
    mission: Option<&MissionSnap>,
) -> EffectiveGoal {
    let charter = settings.session_charter.trim().to_string();
    let user_goal = settings.session_goal.trim();
    let stored = GoalSource::parse(&settings.session_goal_source);

    // 1. User-owned text. Legacy: non-empty goal with source none still counts as user.
    let user_owned = !user_goal.is_empty()
        && matches!(stored, GoalSource::User | GoalSource::None);
    if user_owned {
        return EffectiveGoal {
            source: GoalSource::User,
            objective: user_goal.to_string(),
            charter: charter.clone(),
            fingerprint: format!("user:{user_goal}"),
        };
    }

    // 2. Mission: approved + exactly one active leaf with a title.
    if let Some(m) = mission {
        if m.approved {
            if let Some((id, title)) = m.active_leaf.as_ref() {
                let title = title.trim();
                if !title.is_empty() {
                    return EffectiveGoal {
                        source: GoalSource::Mission,
                        objective: title.to_string(),
                        charter: charter.clone(),
                        fingerprint: format!("mission:{id}"),
                    };
                }
            }
            // Leaf missing/empty: fall through (do not invent from mission.goal alone
            // unless we have no charter — still prefer charter for unattended kickoff).
            let _ = &m.mission_goal;
        }
    }

    // 3. Charter as objective.
    if !charter.is_empty() {
        return EffectiveGoal {
            source: GoalSource::Charter,
            objective: charter.clone(),
            charter: charter.clone(),
            fingerprint: format!("charter:{charter}"),
        };
    }

    EffectiveGoal {
        source: GoalSource::None,
        objective: String::new(),
        charter: String::new(),
        fingerprint: String::new(),
    }
}

/// Load mission + active leaf for resolve (best-effort, never claims leaves).
pub fn load_mission_snap(session_dir: &std::path::Path) -> Option<MissionSnap> {
    let m = crate::model::sdlc::Mission::load(session_dir)?;
    let mut snap = MissionSnap {
        approved: m.approved,
        mission_goal: m.goal.clone(),
        active_leaf: None,
    };
    if !m.approved {
        return Some(snap);
    }
    let conn = crate::model::msglog::open(session_dir).ok()?;
    let open = crate::model::sdlc::graph::list_open_leaves(&conn).ok()?;
    let actives: Vec<_> = open
        .into_iter()
        .filter(|n| n.status == "active")
        .collect();
    // Multiple actives = invalid; don't pick randomly.
    if actives.len() == 1 {
        let a = &actives[0];
        let title = a.title.trim();
        if !title.is_empty() {
            snap.active_leaf = Some((a.id.clone(), title.to_string()));
        }
    }
    Some(snap)
}

fn is_clear_goal(lower: &str) -> bool {
    lower.starts_with("cancel goal")
        || lower.starts_with("clear goal")
        || lower.starts_with("never mind the plan")
        || lower.starts_with("forget the plan")
        || lower == "cancel goal"
        || lower == "clear goal"
}

fn strip_after_forget_plan<'a>(lower: &str, raw: &'a str) -> Option<&'a str> {
    let idx = lower.find("forget the plan")?;
    let after = raw[idx + "forget the plan".len()..]
        .trim_start_matches(|c: char| c == ',' || c == ';' || c == ':' || c.is_whitespace());
    let after = after.trim();
    if after.len() >= 8 {
        Some(after)
    } else {
        None
    }
}

fn strip_goal_prefix<'a>(lower: &str, raw: &'a str) -> Option<&'a str> {
    const PREFIXES: &[&str] = &[
        "goal:",
        "new goal:",
        "session goal:",
        "instead:",
        "do this instead:",
        "do this instead",
    ];
    for p in PREFIXES {
        if lower.starts_with(p) {
            let rest = raw[p.len()..].trim();
            if rest.is_empty() && *p == "do this instead" {
                return Some(raw.trim());
            }
            if !rest.is_empty() {
                return Some(rest);
            }
        }
    }
    None
}

fn looks_like_slash_only(s: &str) -> bool {
    let t = s.trim();
    if !t.starts_with('/') {
        return false;
    }
    // Single-line slash command (e.g. `/help`, `/compact`) — not a kickoff charter.
    t.lines().count() == 1
}

fn clip_goal(s: &str) -> String {
    let t = s.trim();
    let mut out: String = t.chars().take(GOAL_MAX_CHARS).collect();
    if t.chars().count() > GOAL_MAX_CHARS {
        out.push('…');
    }
    // First line only — doctrine is a one-liner.
    if let Some(line) = out.lines().next() {
        return line.trim().to_string();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goal_prefix_sets() {
        assert_eq!(
            detect_goal_update("goal: fix DRSS amnesia"),
            Some(GoalPatch::Set("fix DRSS amnesia".into()))
        );
        assert_eq!(
            detect_goal_update("New goal: ship the hot window"),
            Some(GoalPatch::Set("ship the hot window".into()))
        );
    }

    #[test]
    fn clear_goal() {
        assert_eq!(detect_goal_update("cancel goal"), Some(GoalPatch::Clear));
        assert_eq!(
            detect_goal_update("never mind the plan"),
            Some(GoalPatch::Clear)
        );
    }

    #[test]
    fn short_accept_does_not_set_goal() {
        assert_eq!(detect_goal_update("lgtm"), None);
        assert_eq!(detect_goal_update("do that"), None);
        assert_eq!(detect_goal_update("go ahead"), None);
        assert_eq!(detect_goal_update("ship it"), None);
        assert_eq!(detect_goal_update("approved"), None);
    }

    #[test]
    fn ordinary_chat_no_patch() {
        assert_eq!(detect_goal_update("what is DRSS?"), None);
        assert_eq!(detect_goal_update("continue"), None);
    }

    #[test]
    fn instead_prefix() {
        assert_eq!(
            detect_goal_update("instead: only retune defaults"),
            Some(GoalPatch::Set("only retune defaults".into()))
        );
    }

    #[test]
    fn forget_the_plan_with_rest_sets() {
        assert_eq!(
            detect_goal_update("forget the plan, only fix the fold"),
            Some(GoalPatch::Set("only fix the fold".into()))
        );
    }

    #[test]
    fn resolve_user_over_mission_and_charter() {
        let mut s = Settings::default();
        s.session_goal = "user steer".into();
        s.session_goal_source = "user".into();
        s.session_charter = "kickoff charter".into();
        let mission = MissionSnap {
            approved: true,
            mission_goal: "mission goal".into(),
            active_leaf: Some(("leaf-1".into(), "active leaf".into())),
        };
        let eg = resolve_effective_goal(&s, Some(&mission));
        assert_eq!(eg.source, GoalSource::User);
        assert_eq!(eg.objective, "user steer");
        assert_eq!(eg.charter, "kickoff charter");
        assert_eq!(eg.fingerprint, "user:user steer");
    }

    #[test]
    fn resolve_mission_when_no_user() {
        let mut s = Settings::default();
        s.session_charter = "kickoff".into();
        let mission = MissionSnap {
            approved: true,
            mission_goal: "big goal".into(),
            active_leaf: Some(("n1".into(), "Implement fold".into())),
        };
        let eg = resolve_effective_goal(&s, Some(&mission));
        assert_eq!(eg.source, GoalSource::Mission);
        assert_eq!(eg.objective, "Implement fold");
        assert_eq!(eg.fingerprint, "mission:n1");
    }

    #[test]
    fn resolve_charter_fallback() {
        let mut s = Settings::default();
        s.session_charter = "build headless run".into();
        let eg = resolve_effective_goal(&s, None);
        assert_eq!(eg.source, GoalSource::Charter);
        assert_eq!(eg.objective, "build headless run");
        assert_eq!(eg.fingerprint, "charter:build headless run");
    }

    #[test]
    fn resolve_legacy_goal_without_source() {
        let mut s = Settings::default();
        s.session_goal = "legacy goal".into();
        // source empty → None parse, still user-owned when goal non-empty
        let eg = resolve_effective_goal(&s, None);
        assert_eq!(eg.source, GoalSource::User);
        assert_eq!(eg.objective, "legacy goal");
    }

    #[test]
    fn resolve_multi_active_skips_mission() {
        let mut s = Settings::default();
        s.session_charter = "charter only".into();
        // load_mission_snap would set active_leaf None; simulate here
        let mission = MissionSnap {
            approved: true,
            mission_goal: "g".into(),
            active_leaf: None,
        };
        let eg = resolve_effective_goal(&s, Some(&mission));
        assert_eq!(eg.source, GoalSource::Charter);
    }

    #[test]
    fn charter_seed_once() {
        let mut s = Settings::default();
        assert!(seed_charter_if_empty(&mut s, "Implement no-HITL goals"));
        assert_eq!(s.session_charter, "Implement no-HITL goals");
        assert!(!seed_charter_if_empty(&mut s, "second message does not overwrite"));
        assert_eq!(s.session_charter, "Implement no-HITL goals");
    }

    #[test]
    fn charter_seed_skips_slash() {
        let mut s = Settings::default();
        assert!(!seed_charter_if_empty(&mut s, "/help"));
        assert!(s.session_charter.is_empty());
    }

    #[test]
    fn clear_does_not_imply_wipe_charter_in_resolve() {
        let mut s = Settings::default();
        s.session_charter = "still here".into();
        s.session_goal.clear();
        s.session_goal_source = "none".into();
        let eg = resolve_effective_goal(&s, None);
        assert_eq!(eg.source, GoalSource::Charter);
        assert_eq!(eg.objective, "still here");
    }
}

#[cfg(test)]
mod refresh_tests {
    use super::*;

    #[test]
    fn unchanged_goal_and_clear_are_idempotent_across_tool_steps() {
        let mut settings = Settings::default();
        let first = refresh_goal_state(&mut settings, Some("goal: fix context recall"), None);
        assert!(first.settings_changed && first.objective_changed);
        settings.session_goal_msg_id = 42;
        let charter = settings.session_charter.clone();
        for _ in 0..10 {
            let repeated = refresh_goal_state(&mut settings, Some("goal: fix context recall"), None);
            assert!(!repeated.settings_changed && !repeated.objective_changed);
            assert_eq!(settings.session_goal_msg_id, 42);
        }
        let changed = refresh_goal_state(&mut settings, Some("goal: validate the fix"), None);
        assert!(changed.settings_changed && changed.objective_changed);
        assert_eq!(changed.wire.objective, "validate the fix");
        let cleared = refresh_goal_state(&mut settings, Some("cancel goal"), None);
        assert!(cleared.settings_changed && cleared.objective_changed);
        assert_eq!(cleared.wire.source, "charter");
        for _ in 0..10 {
            let repeated = refresh_goal_state(&mut settings, Some("cancel goal"), None);
            assert!(!repeated.settings_changed && !repeated.objective_changed);
        }
        assert_eq!(settings.session_charter, charter);
    }

    #[test]
    fn metadata_change_does_not_force_an_unchanged_objective() {
        let mut settings = Settings {
            session_goal: "fix recall".into(),
            session_goal_source: "none".into(),
            session_charter: "initial request".into(),
            session_objective_fp: "user:fix recall".into(),
            ..Settings::default()
        };
        let refresh = refresh_goal_state(&mut settings, Some("goal: fix recall"), None);
        assert!(refresh.settings_changed);
        assert!(!refresh.objective_changed);
        assert_eq!(refresh.wire.source, "user");
    }

    #[test]
    fn mission_transitions_respect_user_priority_and_only_arm_once() {
        let mut settings = Settings::default();
        let mut mission = MissionSnap {
            approved: true,
            active_leaf: Some(("first".into(), "Inspect context".into())),
            ..MissionSnap::default()
        };
        let first = refresh_goal_state(&mut settings, Some("initial task"), Some(&mission));
        assert!(first.objective_changed);
        assert_eq!(first.wire.source, "mission");
        assert!(!refresh_goal_state(&mut settings, Some("continue"), Some(&mission)).objective_changed);
        mission.active_leaf = Some(("second".into(), "Validate context".into()));
        assert!(refresh_goal_state(&mut settings, Some("continue"), Some(&mission)).objective_changed);
        assert!(!refresh_goal_state(&mut settings, Some("continue"), Some(&mission)).objective_changed);
        assert!(refresh_goal_state(&mut settings, Some("goal: fix recall"), Some(&mission)).objective_changed);
        mission.active_leaf = Some(("third".into(), "Review context".into()));
        let masked = refresh_goal_state(&mut settings, Some("goal: fix recall"), Some(&mission));
        assert!(!masked.objective_changed);
        assert_eq!(masked.wire.source, "user");
        let clear = refresh_goal_state(&mut settings, Some("cancel goal"), Some(&mission));
        assert!(clear.objective_changed);
        assert_eq!(clear.wire.objective, "Review context");
    }
}
