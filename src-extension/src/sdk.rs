//! Thin helper layer on top of `protocol`.
//!
//! There is no real koma host yet, so this SDK ships a **standalone demo
//! mode**: every sample can run on its own with `cargo run` and prints the
//! handshake and the contribute/require interaction it would have with koma,
//! frame by frame, so the shape of the protocol is visible without a host to
//! talk to.
//!
//! Mode is picked by the `KOMA_EXT_SOCKET` env var: if it is set we assume a
//! real host is on the other end (not implemented in v0, so we just say so
//! and exit); if it is unset we run the scripted demo.

use crate::protocol::*;
use std::io::IsTerminal;
use std::io::Read;

/// Implemented by a sample to answer koma -> extension invocations
/// (the "contributes" side: koma is calling into the extension).
pub trait Extension {
    fn manifest(&self) -> ExtensionManifest;

    /// Handle a koma->ext Invoke (contributes side). Return the result value.
    fn on_invoke(&mut self, _method: &str, _params: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "error": "unimplemented" })
    }
}

/// Handle passed to samples that need to DRIVE koma (the "requires" side:
/// the extension is calling into koma). In demo mode there is no real
/// socket; calls are answered with plausible canned responses so the shape
/// of the interaction is still visible.
pub struct Koma {
    next_agent_id: u32,
}

impl Koma {
    fn new_demo() -> Self {
        Koma { next_agent_id: 1 }
    }

    /// ext->koma Call. In demo mode this returns a canned stub based on
    /// `method` and prints both the call and the canned reply to stderr.
    pub fn call(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        let call = ExtMsg::Call { id: 0, method: method.to_string(), params: params.clone() };
        print_err(&format!("EXT->KOMA Call {method}"), &to_value(&call));

        let result = self.canned_result(method, &params);
        let result_msg = KomaMsg::Result { id: 0, result: result.clone() };
        print_err(&format!("KOMA->EXT Result (reply to {method})"), &to_value(&result_msg));
        result
    }

    fn canned_result(&mut self, method: &str, params: &serde_json::Value) -> serde_json::Value {
        match method {
            "agents.spawn" => {
                let agent_id = format!("demo-{}", self.next_agent_id);
                self.next_agent_id += 1;
                serde_json::json!({ "agentId": agent_id, "status": "spawned" })
            }
            "agents.list" => serde_json::json!([
                { "agentId": "demo-1", "status": "running", "task": "card 1" },
                { "agentId": "demo-2", "status": "queued", "task": "card 2" }
            ]),
            "agents.status" => {
                let agent_id = params
                    .get("agentId")
                    .cloned()
                    .unwrap_or(serde_json::Value::String("demo-1".to_string()));
                serde_json::json!({ "agentId": agent_id, "status": "running", "progress": 0.42 })
            }
            "agents.result" => {
                let agent_id = params
                    .get("agentId")
                    .cloned()
                    .unwrap_or(serde_json::Value::String("demo-1".to_string()));
                serde_json::json!({ "agentId": agent_id, "output": "demo output" })
            }
            other => serde_json::json!({ "error": format!("unknown method: {other}") }),
        }
    }
}

/// Scripted demo for a daemon sample.
#[derive(Default)]
pub struct DaemonDemo {
    /// A (method, params) pair koma would send as an Invoke — exercises the
    /// extension's `on_invoke` (contributes side).
    pub invoke: Option<(String, serde_json::Value)>,
    /// A driver run against a demo `Koma` handle — exercises the extension
    /// driving koma (requires side).
    pub driver: Option<fn(&mut Koma)>,
}

/// Scripted demo for a oneshot sample.
#[derive(Default)]
pub struct OneshotDemo {
    /// The request the extension would receive on stdin, as
    /// `{"method": ..., "params": ...}`. Used as a fallback when nothing is
    /// piped in on stdin. `None` if the sample contributes nothing to invoke.
    pub request: Option<serde_json::Value>,
    /// A driver run against a demo `Koma` handle — exercises the extension
    /// driving koma (requires side).
    pub driver: Option<fn(&mut Koma)>,
}

/// Daemon lifecycle: Hello/Welcome, then the scripted demo interaction.
///
/// In host mode (`KOMA_EXT_SOCKET` set) this is a stub for v0: it prints a
/// notice and exits cleanly. Only demo mode is fully implemented.
pub fn run_daemon(mut ext: impl Extension, demo: DaemonDemo) {
    if host_mode() {
        println!("host mode not implemented in v0");
        return;
    }

    let manifest = ext.manifest();
    println!("=== koma-extension demo :: daemon :: {} ===", manifest.id);

    handshake(&manifest);

    if let Some((method, params)) = demo.invoke {
        let invoke = KomaMsg::Invoke { id: 1, method: method.clone(), params: params.clone() };
        print_out(&format!("KOMA->EXT Invoke {method}"), &to_value(&invoke));

        let result = ext.on_invoke(&method, params);
        let result_msg = ExtMsg::Result { id: 1, result: result.clone() };
        print_out(&format!("EXT->KOMA Result (reply to {method})"), &to_value(&result_msg));
    }

    if let Some(drive) = demo.driver {
        let mut koma = Koma::new_demo();
        drive(&mut koma);
    }

    println!("\n=== demo complete (daemon exiting; a real daemon would keep running) ===");
}

/// Oneshot: read one request from stdin (or fall back to the sample's demo
/// request if stdin is a tty/empty), produce a response, print it, exit.
///
/// In host mode (`KOMA_EXT_SOCKET` set) this is a stub for v0: it prints a
/// notice and exits cleanly. Only demo mode is fully implemented.
pub fn run_oneshot(mut ext: impl Extension, demo: OneshotDemo) {
    if host_mode() {
        println!("host mode not implemented in v0");
        return;
    }

    let manifest = ext.manifest();
    println!("=== koma-extension demo :: oneshot :: {} ===", manifest.id);

    handshake(&manifest);

    if let Some(fallback) = demo.request {
        let request = read_stdin_request().unwrap_or(fallback);
        let method = request
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let params = request.get("params").cloned().unwrap_or(serde_json::Value::Null);

        let invoke = KomaMsg::Invoke { id: 1, method: method.clone(), params: params.clone() };
        print_out(&format!("KOMA->EXT Invoke {method}"), &to_value(&invoke));

        let result = ext.on_invoke(&method, params);
        let result_msg = ExtMsg::Result { id: 1, result: result.clone() };
        print_out("EXT->KOMA Result (response)", &to_value(&result_msg));
    }

    if let Some(drive) = demo.driver {
        let mut koma = Koma::new_demo();
        drive(&mut koma);
    }

    println!("=== demo complete ===");
}

fn handshake(manifest: &ExtensionManifest) {
    let hello = ExtMsg::Hello {
        protocol: PROTOCOL_VERSION.to_string(),
        token: "demo-token".to_string(),
        manifest: manifest.clone(),
    };
    print_out("EXT->KOMA Hello", &to_value(&hello));

    let welcome = KomaMsg::Welcome {
        protocol: PROTOCOL_VERSION.to_string(),
        koma_version: "0.0.0-demo".to_string(),
        granted: manifest.requires.clone(),
    };
    print_out("KOMA->EXT Welcome", &to_value(&welcome));
}

fn host_mode() -> bool {
    std::env::var_os("KOMA_EXT_SOCKET").is_some()
}

/// Reads a JSON `{"method": ..., "params": ...}` request piped into stdin.
/// Returns `None` if stdin is a tty (nothing piped) or isn't valid JSON, so
/// callers can fall back to a built-in demo request.
fn read_stdin_request() -> Option<serde_json::Value> {
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        return None;
    }
    let mut buf = String::new();
    stdin.lock().read_to_string(&mut buf).ok()?;
    if buf.trim().is_empty() {
        return None;
    }
    serde_json::from_str(&buf).ok()
}

fn to_value<T: serde::Serialize>(v: &T) -> serde_json::Value {
    serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
}

fn print_out(label: &str, value: &serde_json::Value) {
    println!("\n--- {label} ---");
    println!("{}", serde_json::to_string_pretty(value).unwrap_or_default());
}

fn print_err(label: &str, value: &serde_json::Value) {
    eprintln!("\n--- {label} ---");
    eprintln!("{}", serde_json::to_string_pretty(value).unwrap_or_default());
}
