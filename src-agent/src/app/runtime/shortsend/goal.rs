//! Deterministic session-goal commitment (no LLM judge).
//!
//! DRSS must not invent “the plan” from draft-shaped archive text. The only
//! doctrine plan on the wire is [`Settings::session_goal`], updated only when the
//! user clearly steers or cancels.

/// Result of inspecting the latest user message for a goal update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalPatch {
    /// Replace session goal with this one-line text.
    Set(String),
    /// Clear the committed goal.
    Clear,
}

/// Cap stored goal length so doctrine stays a one-liner, not a pasted essay.
const GOAL_MAX_CHARS: usize = 240;

/// Detect an explicit goal set/clear from the latest user text.
///
/// Conservative on purpose: never promote assistant drafts. Short accepts
/// (`lgtm`, `do that`, …) only store the **user’s own** short line as a steer
/// note — not the prior assistant plan body.
pub fn detect_goal_update(user_text: &str) -> Option<GoalPatch> {
    let raw = user_text.trim();
    if raw.is_empty() {
        return None;
    }
    let lower = raw.to_ascii_lowercase();

    // Clear first so "cancel goal: …" doesn't also set.
    if is_clear_goal(&lower) {
        return Some(GoalPatch::Clear);
    }

    if let Some(rest) = strip_goal_prefix(&lower, raw) {
        let g = clip_goal(rest);
        if g.is_empty() {
            return None;
        }
        return Some(GoalPatch::Set(g));
    }

    // Short accept: store the user line itself as the committed steer (not a plan dump).
    if is_short_accept(&lower, raw) {
        return Some(GoalPatch::Set(clip_goal(raw)));
    }

    None
}

fn is_clear_goal(lower: &str) -> bool {
    lower.starts_with("cancel goal")
        || lower.starts_with("clear goal")
        || lower.starts_with("never mind the plan")
        || lower.starts_with("forget the plan")
        || lower == "cancel goal"
        || lower == "clear goal"
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
    // "forget the plan, do X" already cleared above when whole-message clear;
    // "forget the plan and do X" — if longer, treat trailing clause after comma.
    if let Some(idx) = lower.find("forget the plan") {
        let after = raw[idx + "forget the plan".len()..].trim_start_matches(|c: char| {
            c == ',' || c == ';' || c == ':' || c.is_whitespace()
        });
        let after = after.trim();
        if after.len() >= 8 {
            return Some(after);
        }
    }
    None
}

fn is_short_accept(lower: &str, raw: &str) -> bool {
    // Only very short user messages — avoids treating long essays as accepts.
    if raw.chars().count() > 80 {
        return false;
    }
    const PHRASES: &[&str] = &[
        "lgtm",
        "do that",
        "do it",
        "ship it",
        "yes implement",
        "yes, implement",
        "go ahead",
        "sounds good, do it",
        "approved",
    ];
    let t = lower.trim().trim_end_matches(|c: char| c == '.' || c == '!');
    PHRASES.iter().any(|p| t == *p || t.starts_with(&format!("{p} ")))
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
    fn short_accept_sets_user_line() {
        assert_eq!(
            detect_goal_update("lgtm"),
            Some(GoalPatch::Set("lgtm".into()))
        );
        assert_eq!(
            detect_goal_update("do that"),
            Some(GoalPatch::Set("do that".into()))
        );
    }

    #[test]
    fn long_message_not_accept() {
        let long = "lgtm but also please reconsider the entire architecture and rewrite everything carefully now";
        assert!(long.chars().count() > 80);
        // starts with lgtm but too long → not short accept; no goal: prefix
        assert_eq!(detect_goal_update(long), None);
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
}
