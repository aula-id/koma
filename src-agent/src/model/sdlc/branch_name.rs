//! Mission branch naming: sanitize user intent and classify from goal/lane.

/// Sanitize a user-requested branch name.
pub fn sanitize_branch_name(user: &str) -> Result<String, String> {
    let s = user.trim();
    if s.is_empty() {
        return Err("branch name is empty".into());
    }
    if s.len() > 200 {
        return Err("branch name too long (max 200)".into());
    }
    if s.starts_with('-') {
        return Err("branch name must not start with '-'".into());
    }
    if s.contains("..") {
        return Err("branch name must not contain '..'".into());
    }
    if s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("branch name must not contain whitespace or control characters".into());
    }
    for c in s.chars() {
        if !(c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '.')) {
            return Err(format!(
                "branch name has invalid character '{c}' (allow alnum / _ - .)"
            ));
        }
    }
    Ok(s.to_string())
}

/// Classify a default mission branch from goal keywords + lane.
///
/// Prefix rules (first match wins on lowercased goal tokens):
/// fix|bug|error|crash|regression → fix/
/// feat|add|implement|new → feat/
/// chore|deps|ci|tooling → chore/
/// docs|readme → docs/
/// refactor|cleanup → refactor/
/// perf|optim → perf/
/// test|spec → test/
/// default: feat/ for standard|full|express; chore/ only when clearly chore.
///
/// Suffix is a short slug from the goal (alnum/-/_ lower, max ~40) plus a short
/// fingerprint for uniqueness when the slug is empty or truncated collision risk.
pub fn classify_mission_branch(goal: &str, lane: &str, _non_goals: &[String]) -> String {
    let lower = goal.to_ascii_lowercase();
    let prefix = classify_prefix(&lower, lane);
    let slug = goal_slug(goal);
    let fp = goal_fingerprint(goal);
    if slug.is_empty() {
        format!("{prefix}{fp}")
    } else {
        format!("{prefix}{slug}-{fp}")
    }
}

fn classify_prefix(goal_lower: &str, lane: &str) -> &'static str {
    // Token-ish scan: match whole-word-ish substrings separated by non-alnum.
    let tokens: Vec<&str> = goal_lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    let has = |words: &[&str]| tokens.iter().any(|t| words.contains(t));

    if has(&["fix", "bug", "error", "crash", "regression"]) {
        return "fix/";
    }
    if has(&["feat", "feature", "add", "implement", "new"]) {
        return "feat/";
    }
    if has(&["chore", "deps", "ci", "tooling"]) {
        return "chore/";
    }
    if has(&["docs", "readme", "documentation"]) {
        return "docs/";
    }
    if has(&["refactor", "cleanup"]) {
        return "refactor/";
    }
    if has(&["perf", "optim", "optimize", "performance"]) {
        return "perf/";
    }
    if has(&["test", "spec", "tests"]) {
        return "test/";
    }
    let _ = lane; // lane reserved; default prefer feat/
    "feat/"
}

fn goal_slug(goal: &str) -> String {
    let slug: String = goal
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else if c.is_whitespace() {
                '-'
            } else {
                '\0'
            }
        })
        .filter(|c| *c != '\0')
        .collect();
    // Collapse repeated dashes and trim.
    let mut out = String::new();
    let mut prev_dash = false;
    for c in slug.chars() {
        if c == '-' {
            if !prev_dash && !out.is_empty() {
                out.push('-');
                prev_dash = true;
            }
        } else {
            out.push(c);
            prev_dash = false;
        }
    }
    let out = out.trim_matches('-').to_string();
    out.chars().take(40).collect()
}

fn goal_fingerprint(goal: &str) -> String {
    // Deterministic FNV-1a 32-bit — stable across process restarts.
    let mut h: u32 = 2_166_136_261;
    for b in goal.as_bytes() {
        h ^= u32::from(*b);
        h = h.wrapping_mul(16_777_619);
    }
    format!("{h:08x}")[..8].to_string()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn sanitize_rejects_bad_names() {
        assert!(sanitize_branch_name("").is_err());
        assert!(sanitize_branch_name("   ").is_err());
        assert!(sanitize_branch_name("-bad").is_err());
        assert!(sanitize_branch_name("a..b").is_err());
        assert!(sanitize_branch_name("has space").is_err());
        assert!(sanitize_branch_name("feat/ok-name").is_ok());
        assert_eq!(sanitize_branch_name("  fix/bug-1  ").unwrap(), "fix/bug-1");
    }

    #[test]
    fn classify_picks_prefix_from_goal() {
        let b = classify_mission_branch("fix crash on startup", "standard", &[]);
        assert!(b.starts_with("fix/"), "{b}");
        let b = classify_mission_branch("add new feature for login", "full", &[]);
        assert!(b.starts_with("feat/"), "{b}");
        let b = classify_mission_branch("chore deps bump", "express", &[]);
        assert!(b.starts_with("chore/"), "{b}");
        let b = classify_mission_branch("update readme docs", "standard", &[]);
        assert!(b.starts_with("docs/"), "{b}");
        let b = classify_mission_branch("something vague", "standard", &[]);
        assert!(b.starts_with("feat/"), "{b}");
    }
}
