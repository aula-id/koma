//! fleet-board-daemon: the live bridge demo. It contributes a kanban panel
//! (`ui/index.html`, backed by the copyable `ui/koma-panel.js` helper) AND
//! drives koma's sub-agents — both directions of the protocol at once,
//! wired through the same UI. Run `cargo run -p fleet-board-daemon` to see
//! it drive koma in demo mode; install it for real and click "Spawn card"
//! in the panel to watch an actual sub-agent get spawned and its progress
//! flow back as panel pushes.
//!
//! # Threading model — read this before copying the pattern
//!
//! `koma_extension::DaemonDemo::driver` is a bare `fn(&mut Koma)` — a
//! function pointer, not a closure — so it cannot capture anything from
//! `main()` or from the `Extension` struct. And per the SDK's DEADLOCK RULE
//! (see `koma_extension::sdk::Extension` docs), `on_invoke`/`on_event` run
//! ON the host's single duplex-serve-loop thread, so calling `Koma::call`
//! from either of them would deadlock: it would block that thread waiting
//! for a `Result` frame that only that same thread could ever read off the
//! socket.
//!
//! The fix is one `std::sync::mpsc` channel, with the receiving half parked
//! in a `OnceLock` so the fn-pointer driver can find it without a capture:
//! `on_invoke` and `on_event` both just push a `Cmd` and return immediately
//! — neither ever touches `Koma`. The driver thread, which DOES own a live
//! `Koma` handle, is the only place in this file that calls `koma.call` or
//! `koma.panel_push`.
//!
//! This is also why `on_event` forwards through the SAME cmd channel
//! instead of holding its own cloned `Koma` handle (via `Koma::try_clone`,
//! which the SDK does support): routing every koma-touching action through
//! one thread is simpler than synchronizing two independent writers onto
//! the same socket, and it sidesteps the deadlock rule for free since
//! neither handler ever blocks on a reply.

use koma_extension::{run_daemon, DaemonDemo, Extension, ExtensionManifest, Koma};
use serde_json::Value;
use std::sync::mpsc;
use std::sync::OnceLock;

/// Work handed from `on_invoke`/`on_event` (host-loop thread) to the driver
/// thread (the only thread allowed to touch `Koma`).
enum Cmd {
    /// A "spawn" action from the panel's "Spawn card" button.
    Spawn { task: String },
    /// A koma->ext `Event` this daemon subscribed to via
    /// `contributes.events`, forwarded here instead of being handled inline
    /// in `on_event`.
    Event { name: String, params: Value },
}

/// The receiving half of the cmd channel, claimed once by the driver
/// thread. `on_invoke`/`on_event` never touch this directly — they only
/// ever see the sender, stored on the `FleetBoard` struct itself.
static CMD_RX: OnceLock<std::sync::Mutex<mpsc::Receiver<Cmd>>> = OnceLock::new();

struct FleetBoard {
    cmd_tx: mpsc::Sender<Cmd>,
}

impl Extension for FleetBoard {
    fn manifest(&self) -> ExtensionManifest {
        serde_json::from_str(include_str!("../manifest.json")).expect("manifest.json is valid")
    }

    /// koma invokes this for every panel bridge message from `ui/index.html`
    /// — routed as `{ "panelId": "board", "payload": <whatever the panel
    /// sent> }`. We only understand one payload shape: `{ "action": "spawn",
    /// "task": <string> }`, sent by the "Spawn card" button (see
    /// `ui/koma-panel.js` / `ui/index.html`).
    ///
    /// This is the DEADLOCK RULE in practice: we do NOT call `koma.call`
    /// here to actually spawn the agent — that would block this very
    /// thread waiting for a reply that only this same thread could ever
    /// deliver. Instead we queue the request and reply immediately with
    /// `{"ok":true,"queued":true}`: the panel finds out the REAL outcome
    /// later, through an unsolicited `panel.push` the driver thread sends
    /// once the spawn actually happens (see `handle_cmd` below).
    fn on_invoke(&mut self, method: &str, params: Value) -> Value {
        match method {
            "panel.msg" => {
                let action = params
                    .get("payload")
                    .and_then(|p| p.get("action"))
                    .and_then(|a| a.as_str())
                    .unwrap_or("");
                match action {
                    "spawn" => {
                        let task = params
                            .get("payload")
                            .and_then(|p| p.get("task"))
                            .and_then(|t| t.as_str())
                            .unwrap_or("untitled card")
                            .to_string();
                        // Best-effort: a send failure just means the driver
                        // thread is gone (daemon shutting down) — nothing
                        // more to do from here.
                        let _ = self.cmd_tx.send(Cmd::Spawn { task });
                        serde_json::json!({ "ok": true, "queued": true })
                    }
                    other => {
                        serde_json::json!({ "error": format!("unknown panel action: {other}") })
                    }
                }
            }
            other => serde_json::json!({ "error": format!("unknown method: {other}") }),
        }
    }

    /// koma delivers only the events this daemon listed in
    /// `contributes.events` (`subagent.done`, `agent.turn_end` — see
    /// `docs/EXTENSIONS.md`'s events section for the full vocabulary and
    /// payload shapes). We don't hold a `Koma` handle on this struct (the
    /// driver thread owns the only one — see the module doc comment above),
    /// so, same as a spawn request, we queue it and let the driver thread
    /// turn it into a `panel_push`.
    fn on_event(&mut self, name: &str, params: Value) {
        let _ = self.cmd_tx.send(Cmd::Event {
            name: name.to_string(),
            params,
        });
    }
}

/// Runs on its own thread with a live `Koma` handle (host mode) or a demo
/// stub (demo mode) — see `koma_extension::sdk::run_daemon`. This is the
/// ONLY function in this sample that calls `Koma::call` or
/// `Koma::panel_push`.
fn drive(koma: &mut Koma) {
    let rx = CMD_RX
        .get()
        .expect("main() sets CMD_RX before starting the daemon")
        .lock()
        .expect("cmd channel mutex poisoned");

    // Host mode: block forever, servicing "Spawn card" clicks (and event
    // forwards) as they arrive for as long as this daemon runs — `rx.iter()`
    // only ends when every `Sender` (the one on `FleetBoard`) drops, which
    // only happens at process exit. Demo mode has no live socket and
    // nothing will ever send another `Cmd` after the one scripted invoke in
    // `main()` below, so blocking forever there would just hang `cargo
    // run` — drain what's already queued with `try_iter()` and return
    // instead.
    if std::env::var_os("KOMA_EXT_SOCKET").is_some() {
        for cmd in rx.iter() {
            handle_cmd(koma, cmd);
        }
    } else {
        for cmd in rx.try_iter() {
            handle_cmd(koma, cmd);
        }
    }
}

fn handle_cmd(koma: &mut Koma, cmd: Cmd) {
    match cmd {
        Cmd::Spawn { task } => {
            // The actual spawn, now safely off the host loop thread.
            // `agent: "card-worker"` picks the sub-agent this manifest
            // declares under `contributes.sub_agents` — it ships its own
            // prompt/model/effort, so koma runs it as-authored rather than
            // falling back to the generic default agent. `notify: true`
            // means koma will ALSO fire a private "agents.done" event
            // straight to this extension when it finishes, independent of
            // whether "agents.done" is in `contributes.events` (see
            // `event-watcher-daemon` for more on that distinction).
            let result = koma.call(
                "agents.spawn",
                serde_json::json!({ "agent": "card-worker", "task": task, "notify": true }),
            );
            let agent_id = result.get("agentId").cloned().unwrap_or(Value::Null);
            let status = result
                .get("status")
                .and_then(|s| s.as_str())
                .unwrap_or("unknown");
            koma.panel_push(
                "board",
                serde_json::json!({ "kind": "agent", "agentId": agent_id, "status": status, "task": task }),
            );
        }
        Cmd::Event { name, params } => {
            koma.panel_push(
                "board",
                serde_json::json!({ "kind": "event", "name": name, "params": params }),
            );
        }
    }
}

fn main() {
    let (cmd_tx, cmd_rx) = mpsc::channel();
    CMD_RX
        .set(std::sync::Mutex::new(cmd_rx))
        .unwrap_or_else(|_| unreachable!("CMD_RX is only ever set here"));

    run_daemon(
        FleetBoard { cmd_tx },
        DaemonDemo {
            // Simulates the panel's "Spawn card" button POSTing
            // {koma:'panel', v:1, kind:'msg', reqId, payload:{action:'spawn',
            // task:'demo card'}} (see ui/koma-panel.js) — koma relays a real
            // click here as a "panel.msg" invoke with exactly this shape.
            invoke: Some((
                "panel.msg".to_string(),
                serde_json::json!({ "panelId": "board", "payload": { "action": "spawn", "task": "demo card" } }),
            )),
            driver: Some(drive),
        },
    );
}
