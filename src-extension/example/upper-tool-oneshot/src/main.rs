//! upper-tool-oneshot: the simplest possible oneshot extension. It
//! contributes one tool, `upper`, and does not require anything from koma.
//! Run with `cargo run -p upper-tool-oneshot` to see it answer a single
//! invocation in demo mode.

use koma_extension::{run_oneshot, Extension, ExtensionManifest, OneshotDemo};
use serde_json::Value;

struct UpperTool;

impl Extension for UpperTool {
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
                serde_json::json!({ "output": text.to_uppercase() })
            }
            other => serde_json::json!({ "error": format!("unknown method: {other}") }),
        }
    }
}

fn main() {
    run_oneshot(
        UpperTool,
        OneshotDemo {
            request: Some(serde_json::json!({
                "method": "tool.call",
                "params": { "name": "upper", "args": { "text": "hi there" } }
            })),
            driver: None,
        },
    );
}
