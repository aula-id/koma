//! fleet-board-daemon: a komatica-style daemon. It contributes a kanban
//! panel, but its real job is driving koma: it requires `agents:orchestrate`
//! and spawns one sub-agent per card. Run with
//! `cargo run -p fleet-board-daemon` to see it drive koma in demo mode.

use koma_extension::{run_daemon, DaemonDemo, Extension, ExtensionManifest, Koma};

struct FleetBoard;

impl Extension for FleetBoard {
    fn manifest(&self) -> ExtensionManifest {
        serde_json::from_str(include_str!("../manifest.json")).expect("manifest.json is valid")
    }
}

/// Hand a product spec's cards to koma, one sub-agent each, then check in
/// on the first one.
fn drive(koma: &mut Koma) {
    koma.call("agents.spawn", serde_json::json!({ "task": "card 1" }));
    koma.call("agents.spawn", serde_json::json!({ "task": "card 2" }));
    koma.call("agents.status", serde_json::json!({ "agentId": "demo-1" }));
}

fn main() {
    run_daemon(
        FleetBoard,
        DaemonDemo {
            invoke: None,
            driver: Some(drive),
        },
    );
}
