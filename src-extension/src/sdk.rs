//! Thin helper layer on top of `protocol`.
//!
//! There is no real koma host yet, so this SDK ships a **standalone demo
//! mode**: every sample can run on its own with `cargo run` and prints the
//! handshake and the contribute/require interaction it would have with koma,
//! frame by frame, so the shape of the protocol is visible without a host to
//! talk to.
//!
//! Mode is picked by the `KOMA_EXT_SOCKET` env var: if it is set a real koma host
//! is on the other end — we connect to that unix socket, complete the
//! `Hello`/`Welcome` handshake, and run the duplex serve loop (koma `Invoke`s us; we
//! `Call` back into koma). If it is unset we run the scripted demo. The host client is
//! std-only (blocking `UnixStream` + threads); it pulls no async runtime into the SDK.

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
    /// Live host connection when driving a real koma (host mode). `None` in demo mode,
    /// where [`Koma::call`] returns canned stubs. Unix-only (the transport is a unix
    /// socket); the field simply does not exist on other platforms.
    #[cfg(unix)]
    host: Option<HostHandle>,
}

/// Shared pieces of a live host connection a [`Koma`] handle needs to drive koma:
/// the write half (guarded so the serve loop and any driver thread can both send), the
/// `pending` map the read loop fulfils when a `KomaMsg::Result` arrives, and a request
/// id source.
#[cfg(unix)]
struct HostHandle {
    writer: std::sync::Arc<std::sync::Mutex<std::os::unix::net::UnixStream>>,
    pending: std::sync::Arc<
        std::sync::Mutex<std::collections::HashMap<u64, std::sync::mpsc::Sender<serde_json::Value>>>,
    >,
    next_id: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl Koma {
    fn new_demo() -> Self {
        Koma {
            next_agent_id: 1,
            #[cfg(unix)]
            host: None,
        }
    }

    /// A handle bound to a live host connection (host mode).
    #[cfg(unix)]
    fn new_host(
        writer: std::sync::Arc<std::sync::Mutex<std::os::unix::net::UnixStream>>,
        pending: std::sync::Arc<
            std::sync::Mutex<
                std::collections::HashMap<u64, std::sync::mpsc::Sender<serde_json::Value>>,
            >,
        >,
        next_id: std::sync::Arc<std::sync::atomic::AtomicU64>,
    ) -> Self {
        Koma {
            next_agent_id: 1,
            host: Some(HostHandle {
                writer,
                pending,
                next_id,
            }),
        }
    }

    /// ext->koma Call. In HOST mode this sends `ExtMsg::Call` on the live socket and
    /// blocks until the matching `KomaMsg::Result` arrives (bounded). In DEMO mode it
    /// returns a canned stub based on `method` and prints both the call and the canned
    /// reply to stderr.
    pub fn call(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        #[cfg(unix)]
        if let Some(h) = &self.host {
            return host_call(h, method, params);
        }

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
/// In HOST mode (`KOMA_EXT_SOCKET` set) this connects to koma's socket, completes the
/// handshake, and runs the duplex serve loop until koma sends `Shutdown` or the socket
/// closes (see [`host_serve`]). In DEMO mode it runs the scripted interaction below.
pub fn run_daemon(mut ext: impl Extension, demo: DaemonDemo) {
    if host_mode() {
        host_serve(ext, demo.driver);
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
/// In HOST mode (`KOMA_EXT_SOCKET` set) this connects and runs the same duplex serve
/// loop as [`run_daemon`] (koma spawns a oneshot per invoke; the loop serves whatever
/// koma sends, then exits on `Shutdown`/close). In DEMO mode it runs the scripted
/// stdin/one-request interaction below.
pub fn run_oneshot(mut ext: impl Extension, demo: OneshotDemo) {
    if host_mode() {
        host_serve(ext, demo.driver);
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

/// Host-mode entry point shared by [`run_daemon`] and [`run_oneshot`]: on unix, run the
/// real duplex client ([`host_run`]); elsewhere the unix-socket transport is
/// unavailable, so print a notice and exit cleanly.
fn host_serve(ext: impl Extension, driver: Option<fn(&mut Koma)>) {
    #[cfg(unix)]
    {
        host_run(ext, driver);
    }
    #[cfg(not(unix))]
    {
        let _ = (ext, driver);
        println!("koma-ext: host mode needs a unix socket (unsupported on this platform)");
    }
}

/// The real host client (unix): connect to `KOMA_EXT_SOCKET`, send `Hello`, read
/// `Welcome`/`Reject`, then run the duplex loop — koma `Invoke`s us (dispatched to
/// [`Extension::on_invoke`], answered with `ExtMsg::Result`), `Ping`s us (answered with
/// `ExtMsg::Health`), and `Shutdown`s us (clean exit). Inbound `KomaMsg::Result` frames
/// complete a pending ext→koma `Call`. If the sample ships a `driver`, it runs on a side
/// thread with a live [`Koma`] handle so the "extension drives koma" direction is exercised
/// too. Any connect/handshake failure logs to stderr and exits.
#[cfg(unix)]
fn host_run(mut ext: impl Extension, driver: Option<fn(&mut Koma)>) {
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::AtomicU64;
    use std::sync::{mpsc, Arc, Mutex};

    let socket = match std::env::var("KOMA_EXT_SOCKET") {
        Ok(s) => s,
        Err(_) => return,
    };
    let token = std::env::var("KOMA_EXT_TOKEN").unwrap_or_default();

    let stream = match UnixStream::connect(&socket) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("koma-ext: connect to {socket} failed: {e}");
            return;
        }
    };
    // A second handle for the read half; reading and writing on independent clones of a
    // unix socket concurrently is fine.
    let read_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("koma-ext: socket clone failed: {e}");
            return;
        }
    };
    let writer = Arc::new(Mutex::new(stream));

    // Hello.
    let hello = ExtMsg::Hello {
        protocol: PROTOCOL_VERSION.to_string(),
        token,
        manifest: ext.manifest(),
    };
    if write_line(&writer, &hello).is_err() {
        return;
    }

    let mut reader = BufReader::new(read_stream);

    // Welcome / Reject.
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) | Err(_) => return, // socket closed before a reply
        Ok(_) => {}
    }
    match serde_json::from_str::<KomaMsg>(line.trim()) {
        Ok(KomaMsg::Welcome { .. }) => {}
        Ok(KomaMsg::Reject { reason }) => {
            eprintln!("koma-ext: rejected by koma: {reason}");
            return;
        }
        _ => {
            eprintln!("koma-ext: expected Welcome from koma");
            return;
        }
    }

    // Pending ext->koma calls, fulfilled by this read loop when a Result arrives.
    let pending: Arc<Mutex<HashMap<u64, mpsc::Sender<serde_json::Value>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let next_id = Arc::new(AtomicU64::new(1));

    // If the sample drives koma, run its driver on a side thread with a host handle that
    // shares this connection's writer + pending map (so its `call()`s round-trip here).
    if let Some(drive) = driver {
        let mut koma = Koma::new_host(Arc::clone(&writer), Arc::clone(&pending), Arc::clone(&next_id));
        std::thread::spawn(move || drive(&mut koma));
    }

    // Duplex serve loop on the main thread.
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break, // koma closed the connection
            Ok(_) => {}
            Err(_) => break,
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg: KomaMsg = match serde_json::from_str(trimmed) {
            Ok(m) => m,
            Err(_) => continue,
        };
        match msg {
            KomaMsg::Invoke { id, method, params } => {
                let result = ext.on_invoke(&method, params);
                let _ = write_line(&writer, &ExtMsg::Result { id, result });
            }
            KomaMsg::Result { id, result } => {
                if let Some(tx) = pending.lock().unwrap().remove(&id) {
                    let _ = tx.send(result);
                }
            }
            KomaMsg::Ping => {
                let _ = write_line(&writer, &ExtMsg::Health { ok: true });
            }
            KomaMsg::Shutdown => break,
            KomaMsg::Welcome { .. } | KomaMsg::Reject { .. } => {}
        }
    }
}

/// ext->koma `Call` on a live host connection: register a pending slot, write the frame,
/// and block (bounded) until the read loop delivers the matching `KomaMsg::Result`.
#[cfg(unix)]
fn host_call(h: &HostHandle, method: &str, params: serde_json::Value) -> serde_json::Value {
    use std::sync::atomic::Ordering;
    let id = h.next_id.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = std::sync::mpsc::channel();
    h.pending.lock().unwrap().insert(id, tx);

    let call = ExtMsg::Call { id, method: method.to_string(), params };
    if write_line(&h.writer, &call).is_err() {
        h.pending.lock().unwrap().remove(&id);
        return serde_json::json!({ "error": "koma call: write failed" });
    }
    match rx.recv_timeout(std::time::Duration::from_secs(120)) {
        Ok(v) => v,
        Err(_) => {
            h.pending.lock().unwrap().remove(&id);
            serde_json::json!({ "error": "koma call: timed out" })
        }
    }
}

/// Serialize `msg` as one newline-delimited JSON frame and write+flush it under the
/// writer lock.
#[cfg(unix)]
fn write_line<T: serde::Serialize>(
    writer: &std::sync::Arc<std::sync::Mutex<std::os::unix::net::UnixStream>>,
    msg: &T,
) -> std::io::Result<()> {
    use std::io::Write;
    let mut line = serde_json::to_string(msg).unwrap_or_default();
    line.push('\n');
    let mut w = writer.lock().unwrap();
    w.write_all(line.as_bytes())?;
    w.flush()
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
