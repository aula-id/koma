//! Goal text and trajectory goals.

use std::collections::HashSet;

use serde_json::Value;

pub const GOAL_PROMPT_CAP: usize = 20_000;
const MAX_OPEN_GOALS: usize = 12;

/// Keep the registered goal and append a later round's sentence. A repeat or
/// an addition that would overflow the cap changes nothing.
pub fn append_goal(existing: &str, added: &str) -> String {
    let added = added.trim();
    if added.is_empty() || existing.contains(added) {
        return existing.to_string();
    }
    let block = format!("{existing}\n\nAdded goal:\n{added}");
    if block.chars().count() <= GOAL_PROMPT_CAP {
        block
    } else {
        existing.to_string()
    }
}

/// A trajectory goal with one of these statuses is finished. Anything else,
/// including a blank status, stays open.
fn is_exhausted(status: &str) -> bool {
    let status = status.trim().to_lowercase();
    status.contains("exhausted")
        || matches!(
            status.as_str(),
            "met" | "done" | "stopped" | "stop" | "complete" | "completed" | "cancelled" | "canceled" | "dismissed" | "closed"
        )
}

/// Labels of the trajectory goal nodes that are not met, done, stopped, or
/// budget exhausted. `contents` are the stored trajectory records.
pub fn open_trajectory_goals<S: AsRef<str>>(contents: &[S]) -> Vec<String> {
    let mut open = Vec::new();
    let mut seen = HashSet::new();
    for content in contents {
        let Ok(Value::Object(graph)) = serde_json::from_str::<Value>(content.as_ref()) else {
            continue;
        };
        let Some(nodes) = graph.get("nodes").and_then(Value::as_array) else {
            continue;
        };
        for node in nodes {
            let kind = node.get("kind").and_then(Value::as_str).unwrap_or("").to_lowercase();
            if kind != "goal" {
                continue;
            }
            let status = node.get("status").and_then(Value::as_str).unwrap_or("");
            if is_exhausted(status) {
                continue;
            }
            let text = |name: &str| node.get(name).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
            let label: String = text("label").or_else(|| text("id")).unwrap_or("goal").chars().take(80).collect();
            if !seen.insert(label.clone()) {
                continue;
            }
            open.push(label);
            if open.len() >= MAX_OPEN_GOALS {
                return open;
            }
        }
    }
    open
}
