//! Thin helper layer on top of `protocol`.
//!
//! There is no real koma host yet, so this SDK ships a **standalone demo
//! mode**: every sample can run on its own with `cargo run` and prints the
//! handshake and the contribute/require interaction it would have with koma,
//! frame by frame, so the shape of the protocol is visible without a host to
//! talk to.
//!
//! Mode is picked by the `KOMA_EXT_SOCKET` env var: if it is set a real koma host
//! is on the other end — we connect to that endpoint (a unix domain socket path on
//! unix, a `\\.\pipe\koma-ext-<id>` named-pipe path on Windows — same env var, same
//! host-side handoff, just a platform-shaped value), complete the `Hello`/`Welcome`
//! handshake, and run the duplex serve loop (koma `Invoke`s us; we `Call` back into
//! koma). If it is unset we run the scripted demo. The host client is std-only
//! (blocking stream + threads); it pulls no async runtime into the SDK.

use crate::protocol::*;
use std::io::IsTerminal;
use std::io::Read;

/// Implemented by a sample to answer koma -> extension invocations
/// (the "contributes" side: koma is calling into the extension).
///
/// # DEADLOCK RULE
///
/// The host-mode serve loop ([`host_run`]) is single-threaded, and `on_invoke`/
/// `on_event` run ON that loop. Calling [`Koma::call`] from inside either handler
/// deadlocks: `call` blocks waiting for a `KomaMsg::Result`, but the only thing that
/// can read that `Result` off the socket is the very serve loop your handler is
/// currently blocking. There is no other reader.
///
/// The safe pattern: reply immediately from `on_invoke` (or return from `on_event`),
/// and hand any real work off to the driver thread (or your own worker thread) via a
/// `std::sync::mpsc` channel you own. `Koma::notify` / `Koma::panel_push` are
/// write-only (no reply is awaited) and are safe to call from either handler.
///
/// # OAuth providers (W11 — DELEGATED login)
///
/// If your manifest declares `contributes.oauth_providers` and requires the
/// `oauth:contribute` grant, koma surfaces each provider as a row in its OAuth
/// picker and delegates the WHOLE login to you over three `on_invoke` methods.
/// koma stores the resulting token as a connection; it never sees your provider's
/// client secret. Every call carries `{ "providerId": "<your provider id>" }`.
///
/// Your extension MUST be `kind: "daemon"` (the begin/poll handshake needs state
/// held across invokes; a oneshot is respawned per invoke and can't remember a
/// pending code).
///
/// - `oauth.begin { providerId }` → start a login. Reply EITHER
///   `{ "url": "https://…" }` (browser method → koma shows a `waiting_url` phase
///   with the URL for the user to open; koma does NOT auto-open it in v1) OR
///   `{ "userCode": "ABCD-1234", "verificationUrl": "https://…" }` (device_code
///   method → `waiting_code` phase) OR `{ "error": "…" }` (→ terminal `failed`).
/// - `oauth.poll { providerId }` → koma polls this every ~3s after `begin`. Reply
///   `{ "status": "pending" }` to keep waiting, `{ "status": "success", "token":
///   { "access_token": "…", "refresh_token"?: "…", "expires_at"?: <unix secs>,
///   "email"?: "…", "label"?: "…" } }` on completion (only `access_token` is
///   required), or `{ "status": "failed", "error": "…" }`. A malformed reply or a
///   bare `{ "error": "…" }` is treated as `failed`.
/// - `oauth.cancel { providerId }` → best-effort teardown when the user cancels or
///   a new flow supersedes this one. Reply anything; koma ignores the result.
///
/// Budgets koma enforces: each `oauth.*` invoke is bounded at 25s, and the whole
/// begin→poll loop at 5 minutes overall (then `failed: timed out`). Reply to
/// `begin`/`poll` promptly — do the real network waiting on your own thread and let
/// `poll` report progress, exactly like the DEADLOCK RULE above.
///
/// # Registering models (W12 — `models.register` / `models.unregister`)
///
/// Once your user has connected one of your OAuth providers, you can register the models
/// that account can serve into koma's global catalogue. Unlike the `oauth.*` methods above
/// (which koma INVOKES on you), these are ext→koma CALLS you make with [`Koma::call`] — so,
/// per the DEADLOCK RULE, make them from your driver/worker thread (e.g. right after a
/// successful `oauth.poll`), never from inside `on_invoke`/`on_event`.
///
/// Your manifest must declare the model provider's `chat_endpoint` + `api_type` on its
/// `OAuthProviderDef` (`api_type` must be `"openai"` or `"anthropic"` — the two wire
/// protocols koma dispatches; an account-login-only provider omits them and `models.register`
/// then refuses with `"provider is account-login only"`), and your extension must hold the
/// `models:contribute` grant (registering models an OAuth account serves almost always means
/// requiring BOTH `oauth:contribute` and `models:contribute`).
///
/// - `models.register { "models": [ { "id": "<model id>", "name": "<display name>" }, … ] }`
///   → registers each model, SERVED BY your connected account. `id` and `name` are non-empty
///   and ≤ 200 chars; at most 100 models per call. Re-registering a model you already
///   registered UPDATES its display name IN PLACE and keeps its stable koma uuid (so a
///   sub-agent already bound to it keeps resolving). Reply:
///   `{ "registered": <n>, "uuids": [ "<uuid>", … ] }` (the stable per-model uuids). Errors:
///   `{ "error": "no connected oauth account for this extension" }` (connect first) or
///   `{ "error": "provider is account-login only" }` (declare `chat_endpoint`+`api_type`).
/// - `models.unregister { "ids"?: [ "<id-or-uuid>", … ] }` → removes models you registered.
///   Omit `ids` to remove ALL of yours; pass `ids` (each matching a `model_id` OR a returned
///   uuid) to remove a subset. You can only ever remove YOUR OWN models — koma enforces an
///   ownership wall, so another extension's or the user's own models are untouchable. Reply:
///   `{ "removed": <n> }`.
///
/// ## The binding guarantee
///
/// Declare the model slugs your sub-agents use in your manifest `contributes.sub_agents`
/// (each `SubAgentDef.model`). After your user connects your provider and you
/// `models.register`, those sub-agents run on YOUR registered models: koma binds an
/// extension-authored sub-agent's `model:` slug to a model served by that SAME extension's
/// OAuth connection FIRST (matched uuid-deterministically), so a same-named model elsewhere
/// in the user's catalogue can NEVER hijack your sub-agent's route. If you have not connected
/// / registered yet, the slug resolves by the normal catalogue rules (and ultimately falls
/// back to the user's Main model), so your sub-agents still run — just not on your models
/// until the account is live.
pub trait Extension {
    fn manifest(&self) -> ExtensionManifest;

    /// Handle a koma->ext Invoke (contributes side). Return the result value.
    fn on_invoke(&mut self, _method: &str, _params: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "error": "unimplemented" })
    }

    /// Handle a koma->ext fire-and-forget `Event` (contributes side, `events` in the
    /// manifest). No reply is sent back — unlike `on_invoke` there is no `Result`
    /// frame. Default is a no-op. See the DEADLOCK RULE above.
    fn on_event(&mut self, _name: &str, _params: serde_json::Value) {}
}

/// Handle passed to samples that need to DRIVE koma (the "requires" side:
/// the extension is calling into koma). In demo mode there is no real
/// socket; calls are answered with plausible canned responses so the shape
/// of the interaction is still visible.
pub struct Koma {
    next_agent_id: u32,
    /// Live host connection when driving a real koma (host mode). `None` in demo mode,
    /// where [`Koma::call`] returns canned stubs. Unix/Windows-only (the transport is a
    /// unix socket or, on Windows, a named pipe — see [`SdkStream`]); the field simply
    /// does not exist on other platforms.
    #[cfg(any(unix, windows))]
    host: Option<HostHandle>,
}

/// Platform stream used by the SDK's blocking host-mode transport: on unix this is
/// `std::os::unix::net::UnixStream` (unchanged); on Windows it is [`WindowsPipeStream`],
/// a small wrapper over a `std::fs::File` opened on the pipe path koma hands us via
/// `KOMA_EXT_SOCKET` — the SAME env var and handoff mechanism, just holding a
/// `\\.\pipe\koma-ext-<id>` name instead of a unix socket path. Private: this type
/// never appears in the crate's public API (only inside [`HostHandle`]).
#[cfg(unix)]
type SdkStream = std::os::unix::net::UnixStream;
#[cfg(windows)]
type SdkStream = WindowsPipeStream;

/// Windows twin of `std::os::unix::net::UnixStream` for the SDK's blocking transport,
/// mirroring the host's `SyncIpcStream` (`src-agent/src/ipc/win.rs`): a `std::fs::File`
/// opened read+write on the pipe path, with the same bounded retry on
/// `ERROR_PIPE_BUSY` (231, "all pipe instances are busy" — the server is momentarily
/// between pre-armed instances). A truly-absent pipe (`NotFound`) is not retried; it
/// propagates verbatim as the "no daemon" signal.
#[cfg(windows)]
struct WindowsPipeStream {
    file: std::fs::File,
}

#[cfg(windows)]
impl WindowsPipeStream {
    const ERROR_PIPE_BUSY: i32 = 231;
    const CONNECT_BUSY_RETRIES: usize = 100;
    const CONNECT_BUSY_BACKOFF: std::time::Duration = std::time::Duration::from_millis(50);

    /// Client-side connect to the pipe at `path`. Same signature shape as
    /// `UnixStream::connect` so call sites need no per-platform changes.
    fn connect(path: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
        let path = path.as_ref();
        let mut attempts: usize = 0;
        loop {
            match std::fs::OpenOptions::new().read(true).write(true).open(path) {
                Ok(file) => return Ok(WindowsPipeStream { file }),
                Err(e) if e.raw_os_error() == Some(Self::ERROR_PIPE_BUSY) => {
                    attempts += 1;
                    if attempts > Self::CONNECT_BUSY_RETRIES {
                        return Err(e);
                    }
                    std::thread::sleep(Self::CONNECT_BUSY_BACKOFF);
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Mirrors `UnixStream::try_clone`: an independent handle onto the same pipe
    /// instance, used for the reader/writer split in [`host_run`].
    fn try_clone(&self) -> std::io::Result<Self> {
        Ok(WindowsPipeStream { file: self.file.try_clone()? })
    }
}

#[cfg(windows)]
impl std::io::Read for WindowsPipeStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buf)
    }
}

#[cfg(windows)]
impl std::io::Write for WindowsPipeStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

/// Shared pieces of a live host connection a [`Koma`] handle needs to drive koma:
/// the write half (guarded so the serve loop and any driver thread can both send), the
/// `pending` map the read loop fulfils when a `KomaMsg::Result` arrives, and a request
/// id source.
#[cfg(any(unix, windows))]
struct HostHandle {
    writer: std::sync::Arc<std::sync::Mutex<SdkStream>>,
    pending: std::sync::Arc<
        std::sync::Mutex<std::collections::HashMap<u64, std::sync::mpsc::Sender<serde_json::Value>>>,
    >,
    next_id: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

#[cfg(any(unix, windows))]
fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Koma {
    fn new_demo() -> Self {
        Koma {
            next_agent_id: 1,
            #[cfg(any(unix, windows))]
            host: None,
        }
    }

    /// A handle bound to a live host connection (host mode).
    #[cfg(any(unix, windows))]
    fn new_host(
        writer: std::sync::Arc<std::sync::Mutex<SdkStream>>,
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
        #[cfg(any(unix, windows))]
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

    /// ext->koma fire-and-forget `Notify`: unlike [`Koma::call`], this does not wait
    /// for (or expect) a `Result` reply — it writes the frame and returns
    /// immediately. HOST mode: writes `ExtMsg::Notify` on the live socket (best
    /// effort; write failures are swallowed the same way other fire-and-forget
    /// sends are). DEMO mode: prints the frame in the same style [`Koma::call`]
    /// prints its demo output. Safe to call from `on_invoke`/`on_event` (see the
    /// DEADLOCK RULE on [`Extension`]).
    pub fn notify(&mut self, name: &str, params: serde_json::Value) {
        let msg = ExtMsg::Notify { name: name.to_string(), params };

        #[cfg(any(unix, windows))]
        if let Some(h) = &self.host {
            let _ = write_line(&h.writer, &msg);
            return;
        }

        print_err(&format!("EXT->KOMA Notify {name}"), &to_value(&msg));
    }

    /// Convenience wrapper over [`Koma::notify`] for `panel.push`: sends
    /// `{"panelId": panel_id, "payload": payload}` so a panel extension's live UI
    /// can push updates to koma. Cheap insurance against the host's frame-size
    /// kill: if the encoded payload exceeds 1 MiB it is logged and DROPPED rather
    /// than sent (the host's frame limit is a few MiB higher than this, but a
    /// misbehaving extension shouldn't get itself killed over a panel update).
    pub fn panel_push(&mut self, panel_id: &str, payload: serde_json::Value) {
        const MAX_PANEL_PUSH_BYTES: usize = 1024 * 1024;

        let encoded_len = serde_json::to_string(&payload).map(|s| s.len());
        match encoded_len {
            Ok(len) if len > MAX_PANEL_PUSH_BYTES => {
                eprintln!(
                    "koma-ext: panel_push({panel_id}) payload is {len} bytes (> {MAX_PANEL_PUSH_BYTES} cap); dropping"
                );
                return;
            }
            Err(e) => {
                eprintln!("koma-ext: panel_push({panel_id}) payload failed to serialize: {e}; dropping");
                return;
            }
            Ok(_) => {}
        }

        self.notify("panel.push", serde_json::json!({ "panelId": panel_id, "payload": payload }));
    }

    /// Cheaply clone this handle so it can be shared across threads (e.g. a driver
    /// thread and a background worker both holding their own `Koma`). HOST mode:
    /// the live connection state (`writer`, `pending`, `next_id`) is already
    /// `Arc`-shared inside [`HostHandle`], so this is a shallow clone onto the SAME
    /// connection — calls/notifies from either handle round-trip through it.
    /// DEMO mode (and builds on platforms without a live transport): there is no live
    /// connection to share, so this returns a fresh demo handle instead of failing.
    pub fn try_clone(&self) -> Koma {
        #[cfg(any(unix, windows))]
        {
            if let Some(h) = &self.host {
                return Koma {
                    next_agent_id: self.next_agent_id,
                    host: Some(HostHandle {
                        writer: std::sync::Arc::clone(&h.writer),
                        pending: std::sync::Arc::clone(&h.pending),
                        next_id: std::sync::Arc::clone(&h.next_id),
                    }),
                };
            }
        }
        Koma::new_demo()
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

/// Host-mode entry point shared by [`run_daemon`] and [`run_oneshot`]: on unix or
/// Windows, run the real duplex client ([`host_run`]) over the platform [`SdkStream`];
/// elsewhere no transport is available, so print a notice and exit cleanly.
fn host_serve(ext: impl Extension, driver: Option<fn(&mut Koma)>) {
    #[cfg(any(unix, windows))]
    {
        host_run(ext, driver);
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (ext, driver);
        println!("koma-ext: host mode needs a unix socket or named pipe (unsupported on this platform)");
    }
}

/// The real host client (unix + Windows): connect to `KOMA_EXT_SOCKET` (a unix socket
/// path, or on Windows a `\\.\pipe\koma-ext-<id>` name — see [`SdkStream`]), send
/// `Hello`, read `Welcome`/`Reject`, then run the duplex loop — koma `Invoke`s us
/// (dispatched to [`Extension::on_invoke`], answered with `ExtMsg::Result`), `Ping`s us
/// (answered with `ExtMsg::Health`), and `Shutdown`s us (clean exit). Inbound
/// `KomaMsg::Result` frames complete a pending ext→koma `Call`. If the sample ships a
/// `driver`, it runs on a side thread with a live [`Koma`] handle so the "extension
/// drives koma" direction is exercised too. Any connect/handshake failure logs to
/// stderr and exits.
#[cfg(any(unix, windows))]
fn host_run(mut ext: impl Extension, driver: Option<fn(&mut Koma)>) {
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader};
    use std::sync::atomic::AtomicU64;
    use std::sync::{mpsc, Arc, Mutex};

    let socket = match std::env::var("KOMA_EXT_SOCKET") {
        Ok(s) => s,
        Err(_) => return,
    };
    let token = std::env::var("KOMA_EXT_TOKEN").unwrap_or_default();

    let stream = match SdkStream::connect(&socket) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("koma-ext: connect to {socket} failed: {e}");
            return;
        }
    };
    // A second handle for the read half; reading and writing on independent clones of
    // the stream (unix socket, or Windows named-pipe file handle) concurrently is fine.
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
            KomaMsg::Event { name, params } => ext.on_event(&name, params),
            KomaMsg::Result { id, result } => {
                if let Some(tx) = lock(&pending).remove(&id) {
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
#[cfg(any(unix, windows))]
fn host_call(h: &HostHandle, method: &str, params: serde_json::Value) -> serde_json::Value {
    use std::sync::atomic::Ordering;
    let id = h.next_id.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = std::sync::mpsc::channel();
    lock(&h.pending).insert(id, tx);

    let call = ExtMsg::Call { id, method: method.to_string(), params };
    if write_line(&h.writer, &call).is_err() {
        lock(&h.pending).remove(&id);
        return serde_json::json!({ "error": "koma call: write failed" });
    }
    match rx.recv_timeout(std::time::Duration::from_secs(120)) {
        Ok(v) => v,
        Err(_) => {
            lock(&h.pending).remove(&id);
            serde_json::json!({ "error": "koma call: timed out" })
        }
    }
}

/// Serialize `msg` as one newline-delimited JSON frame and write+flush it under the
/// writer lock.
#[cfg(any(unix, windows))]
fn write_line<T: serde::Serialize>(
    writer: &std::sync::Arc<std::sync::Mutex<SdkStream>>,
    msg: &T,
) -> std::io::Result<()> {
    use std::io::Write;
    let mut line = serde_json::to_string(msg).unwrap_or_default();
    line.push('\n');
    let mut w = lock(writer);
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
