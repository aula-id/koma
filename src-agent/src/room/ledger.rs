//! What each goal cost, and the setup it ran with. Both are plain shelf notes, so the shelf screen
//! shows them and a later reader can see why a run went the way it did. Nothing here changes how a
//! lane runs.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::room::lane::Usage;
use crate::room::registry::Registry;
use crate::room::shelf::Shelf;

/// Spend of one lane inside one goal.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Entry {
    pub calls: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cost: f64,
}

/// A goal's spend, by lane.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Book {
    pub by_avatar: BTreeMap<String, Entry>,
}

impl Book {
    pub fn add(&mut self, avatar: &str, usage: Usage) {
        let entry = self.by_avatar.entry(avatar.to_string()).or_default();
        entry.calls += 1;
        entry.prompt_tokens += usage.prompt_tokens;
        entry.completion_tokens += usage.completion_tokens;
        entry.cost += usage.cost;
    }

    pub fn total(&self) -> Entry {
        self.by_avatar.values().fold(Entry::default(), |mut all, e| {
            all.calls += e.calls;
            all.prompt_tokens += e.prompt_tokens;
            all.completion_tokens += e.completion_tokens;
            all.cost += e.cost;
            all
        })
    }

    fn to_json(&self, goal: &str) -> Value {
        let total = self.total();
        json!({
            "kind": "spend", "goal": goal,
            "calls": total.calls, "prompt_tokens": total.prompt_tokens, "completion_tokens": total.completion_tokens,
            "cost": (total.cost * 1_000_000.0).round() / 1_000_000.0,
            "by_avatar": self.by_avatar.iter().map(|(who, e)| (who.clone(), json!({
                "calls": e.calls, "prompt_tokens": e.prompt_tokens, "completion_tokens": e.completion_tokens,
                "cost": (e.cost * 1_000_000.0).round() / 1_000_000.0,
            }))).collect::<serde_json::Map<_, _>>(),
        })
    }
}

pub fn spend_key(goal: &str) -> String {
    format!("notes/spend/{goal}")
}

/// A book as the shelf last had it, so a room entered again carries on from it.
pub fn load(shelf: &Shelf, goal: &str) -> Book {
    let mut book = Book::default();
    let Some(body) = shelf.peek(&spend_key(goal)).ok().flatten().and_then(|row| serde_json::from_str::<Value>(row.content.as_deref()?).ok()) else {
        return book;
    };
    if let Some(map) = body.get("by_avatar").and_then(Value::as_object) {
        for (who, e) in map {
            book.by_avatar.insert(
                who.clone(),
                Entry {
                    calls: e.get("calls").and_then(Value::as_u64).unwrap_or(0),
                    prompt_tokens: e.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0),
                    completion_tokens: e.get("completion_tokens").and_then(Value::as_u64).unwrap_or(0),
                    cost: e.get("cost").and_then(Value::as_f64).unwrap_or(0.0),
                },
            );
        }
    }
    book
}

fn put(shelf: &Shelf, key: String, body: Value) {
    let expected = shelf.peek(&key).ok().flatten().map_or(0, |row| row.revision.max(0));
    let _ = shelf.call("put", &json!({ "key": key, "content": body.to_string(), "expected_revision": expected }));
}

pub fn save(shelf: &Shelf, goal: &str, book: &Book) {
    put(shelf, spend_key(goal), book.to_json(goal));
}

/// What a goal ran with: who was on the team, on which models, and the limits. Written once, when the goal first appears.
pub fn snapshot(shelf: &Shelf, goal: &str, registry: &Registry, started_ms: u64, oracle_on: bool, chat_steps: usize, work_steps: usize) {
    let key = format!("notes/run/{goal}");
    if shelf.peek(&key).ok().flatten().is_some() {
        return;
    }
    let team: Vec<Value> = registry
        .avatars
        .values()
        .map(|a| {
            json!({
                "avatar": a.id, "role": a.role,
                "chat_model": a.chat.model, "work_model": a.work.model,
                "tools": registry.role(&a.role).map(|r| r.tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>()).unwrap_or_default(),
            })
        })
        .collect();
    let scopes: Vec<String> = shelf.scan("scope/").unwrap_or_default().into_iter().map(|e| format!("{}@{}", e.key, e.revision)).collect();
    put(
        shelf,
        key,
        json!({
            "kind": "run", "goal": goal, "started_ms": started_ms, "registry_version": registry.version,
            "code_check": oracle_on, "chat_max_steps": chat_steps, "work_max_steps": work_steps,
            "scope": scopes, "team": team,
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spend_adds_up_per_lane_and_survives_a_reload() {
        let shelf = Shelf::open_in_memory().unwrap();
        let mut book = load(&shelf, "g1");
        book.add("red", Usage { prompt_tokens: 100, completion_tokens: 20, cost: 0.01 });
        book.add("red", Usage { prompt_tokens: 50, completion_tokens: 10, cost: 0.005 });
        book.add("reviewer", Usage { prompt_tokens: 10, completion_tokens: 1, cost: 0.001 });
        assert_eq!(book.total().calls, 3);
        save(&shelf, "g1", &book);
        let again = load(&shelf, "g1");
        assert_eq!(again.by_avatar["red"].calls, 2);
        assert_eq!(again.by_avatar["red"].prompt_tokens, 150);
        assert!((again.total().cost - 0.016).abs() < 1e-9);
        // Saving again moves the revision on instead of failing.
        let mut more = again;
        more.add("red", Usage { prompt_tokens: 1, completion_tokens: 1, cost: 0.0 });
        save(&shelf, "g1", &more);
        assert_eq!(load(&shelf, "g1").by_avatar["red"].calls, 3);
    }

    #[test]
    fn the_run_snapshot_is_written_once() {
        let shelf = Shelf::open_in_memory().unwrap();
        let registry = crate::room::registry::RegistryHandle::embedded().unwrap().current();
        snapshot(&shelf, "g1", &registry, 1, true, 60, 60);
        let first = shelf.peek("notes/run/g1").unwrap().unwrap();
        snapshot(&shelf, "g1", &registry, 2, false, 1, 1);
        let second = shelf.peek("notes/run/g1").unwrap().unwrap();
        assert_eq!(first.revision, second.revision);
        let body: Value = serde_json::from_str(first.content.as_deref().unwrap()).unwrap();
        assert_eq!(body["code_check"], true);
        assert!(body["team"].as_array().unwrap().iter().any(|a| a["avatar"] == "red"));
    }
}
