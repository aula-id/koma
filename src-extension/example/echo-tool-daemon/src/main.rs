//! echo-tool-daemon: the simplest possible daemon extension. It contributes
//! one tool, `echo`, and does not require anything from koma. Run with
//! `cargo run -p echo-tool-daemon` to see koma invoke it in demo mode.

use koma_extension::{run_daemon, DaemonDemo, Extension, ExtensionManifest};
use serde_json::Value;

#[derive(Default)]
struct EchoTool {
    /// The last koma->ext `Event` this extension saw, recorded by `on_event`.
    /// Exposed via the `debug.last_event` invoke method as a test hook consumed
    /// by the host-side wave-2 tests.
    last_event: Option<(String, Value)>,
}

impl Extension for EchoTool {
    fn manifest(&self) -> ExtensionManifest {
        serde_json::from_str(include_str!("../manifest.json")).expect("manifest.json is valid")
    }

    fn on_invoke(&mut self, method: &str, params: Value) -> Value {
        match method {
            "tool.call" => {
                let text = params
                    .get("args")
                    .and_then(|a| a.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                serde_json::json!({ "output": text })
            }
            "debug.last_event" => match &self.last_event {
                Some((name, params)) => serde_json::json!({ "name": name, "params": params }),
                None => serde_json::json!({ "name": Value::Null }),
            },
            other => serde_json::json!({ "error": format!("unknown method: {other}") }),
        }
    }

    fn on_event(&mut self, name: &str, params: Value) {
        self.last_event = Some((name.to_string(), params));
    }
}

fn main() {
    run_daemon(
        EchoTool::default(),
        DaemonDemo {
            invoke: Some((
                "tool.call".to_string(),
                serde_json::json!({ "name": "echo", "args": { "text": "hello" } }),
            )),
            driver: None,
        },
    );
}
