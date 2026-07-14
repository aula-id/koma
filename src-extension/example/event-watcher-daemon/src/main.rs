//! event-watcher-daemon: THE starter sample for `contributes.events`. It
//! subscribes to every broadcast koma event — `subagent.done`,
//! `agent.turn_end`, `session.foreground_change` (see `docs/EXTENSIONS.md`'s
//! events section for the full vocabulary and payload shapes) — and counts
//! how many times each has fired. The counts are exposed back to koma
//! through a single tool, `watcher.stats`, so you can ask the chat model
//! "how many sub-agents have finished?" and it can call this tool to find
//! out.
//!
//! This is the smallest useful example of the koma->ext `on_event`
//! direction: no `requires`, no driving koma, just "listen and remember".
//! Run `cargo run -p event-watcher-daemon` to see a few faked event
//! deliveries and a `watcher.stats` call answered in demo mode.

use koma_extension::{run_daemon, DaemonDemo, Extension, ExtensionManifest};
use serde_json::Value;
use std::collections::HashMap;

/// Counts how many times each event name has fired since this daemon
/// started. A real extension would probably do more per event — update a
/// live panel, wake a driver thread, write to disk — this sample keeps it to
/// the minimum that's still useful, so the `on_event` shape stays easy to
/// see.
#[derive(Default)]
struct EventWatcher {
    counts: HashMap<String, u64>,
}

impl Extension for EventWatcher {
    fn manifest(&self) -> ExtensionManifest {
        serde_json::from_str(include_str!("../manifest.json")).expect("manifest.json is valid")
    }

    fn on_invoke(&mut self, method: &str, _params: Value) -> Value {
        match method {
            // The only tool this daemon contributes. Every contributed tool
            // is invoked the same way regardless of its own name: koma sends
            // "tool.call" with `{ "name": <tool name>, "args": {...} }`. We
            // only have one tool, so there's nothing to branch on here — a
            // daemon with more than one tool would match on
            // `params.get("name")` before deciding what to do.
            "tool.call" => {
                let stats: serde_json::Map<String, Value> = self
                    .counts
                    .iter()
                    .map(|(name, count)| (name.clone(), Value::from(*count)))
                    .collect();
                serde_json::json!({ "output": Value::Object(stats) })
            }
            other => serde_json::json!({ "error": format!("unknown method: {other}") }),
        }
    }

    // koma only ever calls this for events this daemon LISTED in
    // manifest.json's `contributes.events` — anything else is filtered out
    // before it reaches us (see `docs/EXTENSIONS.md`'s "only-subscribed"
    // rule). No reply is expected or sent: `on_event` is fire-and-forget by
    // design, same as koma->ext `Event` on the wire (see the SDK's
    // `Extension` trait docs, the DEADLOCK RULE section).
    fn on_event(&mut self, name: &str, _params: Value) {
        *self.counts.entry(name.to_string()).or_insert(0) += 1;
    }
}

fn main() {
    let mut watcher = EventWatcher::default();

    // Demo mode has no live koma to fire real events at us, so we fake a
    // few deliveries here first — the exact same call the host duplex loop
    // would make — so `watcher.stats` below has something to show. Run with
    // KOMA_EXT_SOCKET set against a real koma to see genuine events
    // accumulate instead of these three.
    watcher.on_event(
        "subagent.done",
        serde_json::json!({ "session": "demo-session", "subagentId": 1, "agent": "general", "status": "done" }),
    );
    watcher.on_event(
        "subagent.done",
        serde_json::json!({ "session": "demo-session", "subagentId": 2, "agent": "general", "status": "error" }),
    );
    watcher.on_event(
        "agent.turn_end",
        serde_json::json!({ "session": "demo-session" }),
    );

    run_daemon(
        watcher,
        DaemonDemo {
            invoke: Some((
                "tool.call".to_string(),
                serde_json::json!({ "name": "watcher.stats", "args": {} }),
            )),
            driver: None,
        },
    );
}
