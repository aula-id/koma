//! agent-peek-oneshot: contributes nothing at all — no tools, no panel, no
//! model. It only requires `agents:read` and uses it to look at koma's
//! sub-agents. Run with `cargo run -p agent-peek-oneshot` to see a non-daemon
//! extension that just reads koma state, in demo mode.

use koma_extension::{run_oneshot, Extension, ExtensionManifest, Koma, OneshotDemo};

struct AgentPeek;

impl Extension for AgentPeek {
    fn manifest(&self) -> ExtensionManifest {
        serde_json::from_str(include_str!("../manifest.json")).expect("manifest.json is valid")
    }
}

fn drive(koma: &mut Koma) {
    let list = koma.call("agents.list", serde_json::json!({}));
    println!(
        "\nagent status list:\n{}",
        serde_json::to_string_pretty(&list).unwrap_or_default()
    );
}

fn main() {
    run_oneshot(
        AgentPeek,
        OneshotDemo {
            request: None,
            driver: Some(drive),
        },
    );
}
