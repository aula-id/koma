//! Feature-gated desktop GUI (`koma gui`).
//!
//! A `tao` event loop + `wry` WebView (single window, main thread) that hosts
//! xterm.js rendering the *real* koma client. Wave 1 was scaffold (empty
//! window), Wave 2 vendored xterm.js under `src-webgui/` (repo root, sibling of
//! `src-agent/`), embedded that tree via `include_dir!`, and served it through a
//! `koma://` custom protocol. Wave 3 (this file) spawns a bare `koma` client in
//! a PTY and bridges it bidirectionally to xterm.js so the window is a fully
//! interactive terminal:
//!
//! - **pty -> xterm**: a reader thread pumps pty output as base64 through an
//!   [`EventLoopProxy`] user event; the main thread `evaluate_script`s it into
//!   `window.__koma.write`.
//! - **xterm -> pty**: the wry ipc handler receives JSON from `koma.js`
//!   (`data`/`resize`/`ready`) and writes bytes / resizes the pty accordingly.
//!
//! The child is a *bare* `koma` (no args): its default path mints its own
//! session uuid, ensures its own daemon, and runs the client — exactly like
//! launching `koma` in a terminal. Never spawn `koma gui` (infinite recursion).

use anyhow::{Context, Result};
use include_dir::{include_dir, Dir};
use std::borrow::Cow;
use wry::http::{Request, Response, StatusCode};

/// The `src-webgui/dist/` directory (Vite-built React app), embedded at
/// compile time. `$CARGO_MANIFEST_DIR` is `src-agent/`, so this resolves to
/// the repo-root `src-webgui/dist/` tree produced by `build.rs` running
/// `npm run build` when the `gui` feature is enabled.
static WEBUI: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../src-webgui/dist");

/// Guess a MIME type from a file extension for the `koma://` protocol handler.
fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html",
        "js" => "text/javascript",
        "css" => "text/css",
        "json" => "application/json",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        _ => "application/octet-stream",
    }
}

/// Handle a `koma://localhost/<path>` request by serving the matching file out
/// of the embedded [`WEBUI`] tree. Empty path or `/` maps to `index.html`.
fn handle_koma_request(request: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let path = request.uri().path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    match WEBUI.get_file(path) {
        Some(file) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", mime_for(path))
            .body(Cow::Borrowed(file.contents()))
            .unwrap_or_else(|_| {
                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Cow::Borrowed(&[][..]))
                    .expect("static empty response is valid")
            }),
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Cow::Borrowed(&b"not found"[..]))
            .expect("static 404 response is valid"),
    }
}

/// Events delivered to the main `tao` event loop from the ipc handler (window
/// commands) or the host-relay client-thread (state pushes).
enum UserEvent {
    /// A custom-titlebar window command posted from the webview.
    Win(WinCmd),
    /// A ready-to-inject JSON envelope from the host-relay client-thread. The main
    /// thread hands it to `window.__komaClient.push(...)` via `evaluate_script`. The
    /// payload is a COMPLETE JSON object (tagged on `k` — `Snapshot`/`StreamMsg`/
    /// `Reasoning`/`Status`/`Hub`), so it is embedded verbatim (not quoted).
    Push(String),
}

/// Window-management commands the HTML titlebar (drag region, minimize /
/// maximize / close buttons, edge resize handles) posts over ipc, since the
/// window is undecorated (`with_decorations(false)`) and has no native
/// titlebar to drive these.
#[derive(Clone, Copy)]
enum WinCmd {
    Drag,
    Minimize,
    ToggleMax,
    Close,
    Resize(tao::window::ResizeDirection),
}

/// Messages posted from `koma.js` via `window.ipc.postMessage(JSON.stringify(..))`.
/// Internally tagged on `t`; unknown tags / malformed JSON fail to deserialize
/// and are ignored (the ipc handler must never panic).
#[derive(serde::Deserialize)]
#[serde(tag = "t")]
enum ClientMsg {
    /// Custom-titlebar window command: drag / minimize / toggle-maximize / close.
    #[serde(rename = "win")]
    Win { a: String },
    /// Custom edge/corner resize-handle drag; `dir` is one of
    /// `e`/`w`/`n`/`s`/`ne`/`nw`/`se`/`sw`.
    #[serde(rename = "winresize")]
    WinResize { dir: String },
    /// The native-React client protocol (host-relay bridge). Tagged `"req"` on the
    /// outer `t`; the inner [`GuiReq`] carries the actual request keyed on `r`
    /// (`Ready` / `Submit` / `SelectSession` / `NewSession`). This is the ONLY
    /// inbound channel once the PTY-for-chat path is retired — the page drives the
    /// daemon through it, and the host pushes authoritative state back via
    /// `window.__komaClient.push(...)`.
    #[serde(rename = "req")]
    Req(GuiReq),
}

/// The native-React client -> host request, carried inside [`ClientMsg::Req`] and
/// internally tagged on `r`. Mirrors the JS→Rust half of the host-relay bridge
/// contract exactly:
///   - `Ready` — the page booted; the host sends its first push (a `Hub` if it is
///     in the swapper, else a `Snapshot`).
///   - `Submit { text }` — a chat send; forwarded to the attached daemon as
///     [`ClientRequest::SubmitInput`].
///   - `SelectSession { id }` — a hub pick; the host-thread attaches to that daemon.
///   - `NewSession` — the hub `[+ new session]` row; mint a fresh uuid + attach.
///   - `RefreshHub` — the ResumePalette overlay opened (and may re-emit while open):
///     ask the host to re-run cross-daemon discovery and push a FRESH `Hub` envelope,
///     so the live-session list is current even while ATTACHED (it was previously only
///     built once, cold, in the swapper). This is the live-session-listing fix.
///
/// Deserialised from the SAME JSON map as the outer [`ClientMsg`] (serde internal
/// tagging strips `t`, then this reads `r`), so `{ "t":"req", "r":"Submit",
/// "text":"…" }` round-trips into `ClientMsg::Req(GuiReq::Submit { text })`.
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "r")]
enum GuiReq {
    Ready,
    Submit { text: String },
    SelectSession { id: String },
    NewSession,
    RefreshHub,
    /// Cancel an in-progress session switch (the full-screen loader's Cancel button):
    /// best-effort bail back to the hub. Forwarded as [`HostCtl::ToSwapper`]. The swap
    /// itself can't be interrupted (the host-thread blocks in the attach), so this is
    /// acted on once the target lands — the host then drops to the swapper and pushes a
    /// fresh `Hub`, which clears the loader React-side.
    CancelSwitch,
    /// Attach RAW file bytes from the page (a clipboard-image paste, a drag-drop, or a
    /// file-picker pick). The host base64-decodes `bytes_b64`, writes them to a
    /// host-writable scratch path (preserving `name`'s extension so the daemon's
    /// image-path sniff still fires), then forwards a [`ClientRequest::Paste`] of that
    /// path — reusing the daemon's EXISTING attachment ingest (image paths land in
    /// `pending_attachments`; other files fall through to the daemon's paste handling).
    /// `mime` is carried for the contract but the daemon sniffs by extension/bytes.
    AttachFile {
        name: String,
        // Carried for the bridge contract; the daemon sniffs by extension/bytes so the
        // host never needs to read it (the scratch write preserves `name`'s extension).
        #[serde(default)]
        #[allow(dead_code)]
        mime: Option<String>,
        #[serde(rename = "bytesB64")]
        bytes_b64: String,
    },
    /// Attach an EXISTING on-disk file by path (an omnisearch pick — the file already
    /// lives in the workspace, so no bytes are shipped). Forwarded verbatim as a
    /// [`ClientRequest::Paste`]: an image path is ingested into `pending_attachments`;
    /// a non-image path is handled by the daemon's paste path as before.
    AttachPath { path: String },
    /// Drop a staged attachment chip by its `[Image #N]` marker number (`markerN`).
    /// Forwarded as [`ClientRequest::RemoveAttachment`], which unstages it daemon-side;
    /// the resulting `pending_attachments` change re-emits the Snapshot (chips update).
    RemoveAttachment {
        #[serde(rename = "markerN")]
        marker_n: usize,
    },
    /// Omnisearch: fuzzy-search the workspace file index. Forwarded as
    /// [`ClientRequest::FileSearch`]; the daemon's one-shot reply is re-pushed to JS as a
    /// `SearchResults` envelope by the host `push_loop`. Select a result → `AttachPath`.
    FileSearch {
        query: String,
        #[serde(default)]
        limit: Option<usize>,
    },
    /// Rename the foreground session (the RenameOverlay submit). Forwarded verbatim as
    /// [`ClientRequest::RenameSession`], which sets the session's name + persists it
    /// (registry + settings) daemon-side; the resulting title change re-emits the
    /// Snapshot so `Snapshot.title` — which the overlay prefills from — updates.
    Rename { name: String },

    // ─── GUI config setters (Connector + MCP panels) ─────────────────────────
    // Forwarded to the attached daemon (which owns `AppConfig`) as the matching
    // gui-gated [`ClientRequest`]; the daemon mutates + persists config and re-emits a
    // fresh `Config` push. Field shapes mirror the panel form models exactly.
    /// Upsert an MCP server (McpPanel add/edit). `uuid` is absent for a new server.
    SetMcpServer {
        #[serde(default)]
        uuid: Option<String>,
        name: String,
        enabled: bool,
        transport: String,
        command: String,
        args: String,
        env: String,
        url: String,
    },
    /// Remove an MCP server by uuid (McpPanel arm-delete).
    DeleteMcpServer { uuid: String },
    /// Toggle an MCP server's enabled flag by uuid (McpPanel list switch).
    EnableMcpServer { uuid: String, enabled: bool },
    /// Upsert a provider (Connector ProviderForm). `uuid` is absent for a new provider.
    SetProvider {
        #[serde(default)]
        uuid: Option<String>,
        name: String,
        endpoint: String,
        #[serde(rename = "apiKey")]
        api_key: String,
    },
    /// Remove a provider by uuid (Connector arm-delete).
    DeleteProvider { uuid: String },
    /// Upsert a model (Connector ModelForm). `uuid` is absent for a new model; `roles`
    /// are lowercase tokens; `scope` is `"global"`/`"local"`.
    SetModel {
        #[serde(default)]
        uuid: Option<String>,
        name: String,
        #[serde(rename = "modelId")]
        model_id: String,
        #[serde(rename = "providerUuid")]
        provider_uuid: String,
        #[serde(default)]
        route: Option<String>,
        roles: Vec<String>,
        scope: String,
    },
    /// Remove a model by uuid from the addressed `scope` (Connector arm-delete).
    DeleteModel { uuid: String, scope: String },
    /// Fetch the live model-id catalogue for a provider (Connector model picker). The
    /// daemon replies out-of-band; the host re-pushes it as a `ModelList` envelope.
    ListModels { provider: String },
}

/// Write `bytes` to a host-writable scratch file, returning its absolute path.
///
/// Used by the [`GuiReq::AttachFile`] raw-bytes route: the host can't address the
/// daemon's per-session `images/` dir (it knows neither `pwd_hash` nor the session
/// uuid), so it drops the incoming bytes into `<tmp>/koma/gui-attach/<uuid>-<name>`
/// and hands the daemon that path via [`ClientRequest::Paste`] — the daemon then
/// re-copies it into the session's `images/` on ingest. The original basename +
/// extension are preserved (behind a uuid to avoid collisions) so the daemon's
/// extension-based image sniff still fires. Returns `None` on any fs error (the ipc
/// handler must never panic).
fn write_attach_scratch(name: &str, bytes: &[u8]) -> Option<std::path::PathBuf> {
    let mut dir = std::env::temp_dir();
    dir.push("koma");
    dir.push("gui-attach");
    std::fs::create_dir_all(&dir).ok()?;
    let base = std::path::Path::new(name)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "file".to_string());
    let unique = format!("{}-{}", uuid::Uuid::new_v4(), base);
    let path = dir.join(unique);
    std::fs::write(&path, bytes).ok()?;
    Some(path)
}

/// Forward a `ClientRequest::Paste { text: path }` to the currently-attached daemon
/// through the shared live-request slot. Shared by the [`GuiReq::AttachFile`] and
/// [`GuiReq::AttachPath`] arms — both funnel a filesystem path into the daemon's
/// existing paste/attachment ingest. A missing live sender (no session attached yet)
/// is a silent no-op.
fn forward_paste(
    live_req: &std::sync::Mutex<Option<std::sync::mpsc::Sender<crate::ipc::proto::ClientRequest>>>,
    path: String,
) {
    if let Ok(g) = live_req.lock() {
        if let Some(tx) = g.as_ref() {
            let _ = tx.send(crate::ipc::proto::ClientRequest::Paste { text: path });
        }
    }
}

/// Map a `koma.js` resize-handle direction string to tao's [`tao::window::ResizeDirection`].
/// Unknown strings are ignored by the caller (returns `None`).
fn parse_resize_dir(dir: &str) -> Option<tao::window::ResizeDirection> {
    use tao::window::ResizeDirection;
    match dir {
        "e" => Some(ResizeDirection::East),
        "w" => Some(ResizeDirection::West),
        "n" => Some(ResizeDirection::North),
        "s" => Some(ResizeDirection::South),
        "ne" => Some(ResizeDirection::NorthEast),
        "nw" => Some(ResizeDirection::NorthWest),
        "se" => Some(ResizeDirection::SouthEast),
        "sw" => Some(ResizeDirection::SouthWest),
        _ => None,
    }
}

pub fn run_gui(opts: crate::cli::Opts) -> Result<()> {
    use std::sync::{Arc, Mutex};
    use tao::{
        dpi::LogicalSize,
        event::{Event, WindowEvent},
        event_loop::{ControlFlow, EventLoopBuilder},
        window::WindowBuilder,
    };
    use wry::WebViewBuilder;

    use super::client::{run_host_relay, HostCtl};
    use crate::ipc::proto::ClientRequest;

    // --- 1. Event loop + window (frameless, transparent) -----------------------
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title("koma")
        .with_inner_size(LogicalSize::new(1024.0, 680.0))
        .with_decorations(false)
        .with_resizable(true)
        .with_transparent(true)
        .build(&event_loop)
        .context("failed to build GUI window")?;
    let proxy = event_loop.create_proxy();

    // --- 2. Host-relay wiring --------------------------------------------------
    // The GUI host IS the daemon client, but this main thread is owned by tao/GTK
    // (`event_loop.run` diverges — no tokio here). So the daemon connection + the
    // headless fold loop run on a BACKGROUND client-thread with its own tokio runtime
    // (`run_host_relay`). daemon->JS: the client-thread pushes JSON envelopes out
    // through a closure that fires `UserEvent::Push` at this event loop, which the §4
    // arm injects via `window.__komaClient.push(...)`. JS->daemon: the ipc handler
    // sends `HostCtl` intents (Ready / SelectSession / NewSession) over `ctl_tx` and
    // forwards a chat `Submit` straight to the live daemon through the shared `live_req`.
    let (ctl_tx, ctl_rx) = std::sync::mpsc::channel::<HostCtl>();
    let live_req: Arc<Mutex<Option<std::sync::mpsc::Sender<ClientRequest>>>> =
        Arc::new(Mutex::new(None));
    // The marker numbers of the currently-STAGED attachments (mirrors the attached
    // session's `pending_attachments`, maintained by the fold loop). A chat `Submit`
    // carries only React's typed text, so the host appends any staged `[Image #N]`
    // markers to it before forwarding — otherwise the daemon's submit-time reconcile
    // (which keeps only attachments whose marker survived in the sent text) would drop
    // every staged image. Empty whenever detached.
    let live_marks: Arc<Mutex<Vec<usize>>> = Arc::new(Mutex::new(Vec::new()));

    {
        let push_proxy = proxy.clone();
        let live_req = Arc::clone(&live_req);
        let live_marks = Arc::clone(&live_marks);
        std::thread::spawn(move || {
            run_host_relay(
                opts,
                // The event loop may already be gone (window closed) — a failed send
                // is fine (the thread is about to be torn down with the process).
                move |json| {
                    let _ = push_proxy.send_event(UserEvent::Push(json));
                },
                ctl_rx,
                live_req,
                live_marks,
            );
        });
    }

    // Handles captured by the (Fn) ipc handler: a proxy for titlebar `Win` commands,
    // the control sender for session intents, and the shared live-request sender a
    // chat `Submit` is forwarded through.
    let win_proxy = proxy.clone();
    let ipc_ctl = ctl_tx;
    let ipc_req = Arc::clone(&live_req);
    let ipc_marks = Arc::clone(&live_marks);

    // --- 3. WebView + ipc handler ----------------------------------------------
    let wv_builder = WebViewBuilder::new()
        .with_devtools(true)
        // Palette now rides `Snapshot.palette` (pushed live), so only the platform
        // hint the React chrome reads at boot is injected here.
        .with_initialization_script(format!("window.__komaOS='{}';", std::env::consts::OS))
        .with_url("koma://localhost/index.html")
        .with_transparent(true)
        .with_custom_protocol("koma".into(), |_webview_id, request| {
            handle_koma_request(request)
        })
        .with_ipc_handler(move |req: Request<String>| {
            let msg: ClientMsg = match serde_json::from_str(req.body()) {
                Ok(m) => m,
                Err(_) => return, // malformed / unknown -> ignore, never panic
            };
            match msg {
                // Custom-titlebar window commands (the window is undecorated).
                ClientMsg::Win { a } => {
                    let cmd = match a.as_str() {
                        "drag" => Some(WinCmd::Drag),
                        "min" => Some(WinCmd::Minimize),
                        "max" => Some(WinCmd::ToggleMax),
                        "close" => Some(WinCmd::Close),
                        _ => None,
                    };
                    if let Some(cmd) = cmd {
                        let _ = win_proxy.send_event(UserEvent::Win(cmd));
                    }
                }
                ClientMsg::WinResize { dir } => {
                    if let Some(dir) = parse_resize_dir(&dir) {
                        let _ = win_proxy.send_event(UserEvent::Win(WinCmd::Resize(dir)));
                    }
                }
                // Host-relay bridge (native-React client): route each request to the
                // client-thread (session intents) or the live daemon (chat submit).
                ClientMsg::Req(req) => match req {
                    // Page (re)booted: ask the client-thread to re-push full state.
                    GuiReq::Ready => {
                        let _ = ipc_ctl.send(HostCtl::Ready);
                    }
                    // Chat send: forward straight to the currently-attached daemon.
                    // Append any staged attachment markers React's text doesn't already
                    // carry, so the daemon's submit-time reconcile keeps the images.
                    GuiReq::Submit { text } => {
                        let mut text = text;
                        if let Ok(marks) = ipc_marks.lock() {
                            for n in marks.iter() {
                                let marker = format!("[Image #{n}]");
                                if !text.contains(&marker) {
                                    if !text.is_empty() {
                                        text.push(' ');
                                    }
                                    text.push_str(&marker);
                                }
                            }
                        }
                        if let Ok(g) = ipc_req.lock() {
                            if let Some(tx) = g.as_ref() {
                                let _ = tx.send(ClientRequest::SubmitInput { text });
                            }
                        }
                    }
                    // Hub pick / new session: the client-thread (re)attaches.
                    GuiReq::SelectSession { id } => {
                        let _ = ipc_ctl.send(HostCtl::Select(id));
                    }
                    // `[+ new session]`: open a NATIVE folder picker off the tao event
                    // loop (rfd's dialog is modal/blocking — running it on this thread
                    // would stall the 16ms push loop), and only mint the session once a
                    // folder is confirmed. React raises its switch loader optimistically on
                    // click, so on CANCEL create nothing but kick a hub RE-PUSH so the
                    // loader (`switchingTo`) clears instead of stranding.
                    GuiReq::NewSession => {
                        let ctl = ipc_ctl.clone();
                        std::thread::spawn(move || {
                            match rfd::FileDialog::new().pick_folder() {
                                Some(folder) => {
                                    let _ = ctl.send(HostCtl::New(Some(folder)));
                                }
                                None => {
                                    let _ = ctl.send(HostCtl::RefreshHub);
                                }
                            }
                        });
                    }
                    // ResumePalette opened: re-discover live sessions + re-push the hub
                    // (works while attached too — see `host_swapper` / `push_loop`).
                    GuiReq::RefreshHub => {
                        let _ = ipc_ctl.send(HostCtl::RefreshHub);
                    }
                    // Cancel-switch: best-effort bail to the hub (acted on once the
                    // in-flight attach lands — the swap can't be interrupted mid-flight).
                    GuiReq::CancelSwitch => {
                        let _ = ipc_ctl.send(HostCtl::ToSwapper);
                    }
                    // Attach raw file bytes: decode, spill to a scratch path, and forward
                    // as a Paste of that path so the daemon's existing ingest stages it.
                    GuiReq::AttachFile { name, bytes_b64, .. } => {
                        use base64::Engine;
                        if let Ok(bytes) =
                            base64::engine::general_purpose::STANDARD.decode(bytes_b64.as_bytes())
                        {
                            if let Some(path) = write_attach_scratch(&name, &bytes) {
                                forward_paste(&ipc_req, path.to_string_lossy().into_owned());
                            }
                        }
                    }
                    // Attach an existing on-disk file by path (omnisearch pick).
                    GuiReq::AttachPath { path } => {
                        forward_paste(&ipc_req, path);
                    }
                    // Drop a staged attachment chip by its marker number.
                    GuiReq::RemoveAttachment { marker_n } => {
                        if let Ok(g) = ipc_req.lock() {
                            if let Some(tx) = g.as_ref() {
                                let _ = tx.send(ClientRequest::RemoveAttachment { marker_n });
                            }
                        }
                    }
                    // Omnisearch: run the daemon's @-palette fuzzy search; its one-shot
                    // reply is re-pushed to JS as a `SearchResults` envelope by `push_loop`.
                    GuiReq::FileSearch { query, limit } => {
                        if let Ok(g) = ipc_req.lock() {
                            if let Some(tx) = g.as_ref() {
                                let _ = tx.send(ClientRequest::FileSearch { query, limit });
                            }
                        }
                    }
                    // Rename the foreground session: forward to the attached daemon,
                    // which persists it and re-emits the Snapshot (title updates).
                    GuiReq::Rename { name } => {
                        if let Ok(g) = ipc_req.lock() {
                            if let Some(tx) = g.as_ref() {
                                let _ = tx.send(ClientRequest::RenameSession { name });
                            }
                        }
                    }
                    // GUI config setters: forward each to the attached daemon, which owns
                    // `AppConfig`, persists the change, and re-pushes a fresh `Config`.
                    GuiReq::SetMcpServer {
                        uuid,
                        name,
                        enabled,
                        transport,
                        command,
                        args,
                        env,
                        url,
                    } => {
                        if let Ok(g) = ipc_req.lock() {
                            if let Some(tx) = g.as_ref() {
                                let _ = tx.send(ClientRequest::SetMcpServer {
                                    uuid,
                                    name,
                                    enabled,
                                    transport,
                                    command,
                                    args,
                                    env,
                                    url,
                                });
                            }
                        }
                    }
                    GuiReq::DeleteMcpServer { uuid } => {
                        if let Ok(g) = ipc_req.lock() {
                            if let Some(tx) = g.as_ref() {
                                let _ = tx.send(ClientRequest::DeleteMcpServer { uuid });
                            }
                        }
                    }
                    GuiReq::EnableMcpServer { uuid, enabled } => {
                        if let Ok(g) = ipc_req.lock() {
                            if let Some(tx) = g.as_ref() {
                                let _ = tx.send(ClientRequest::EnableMcpServer { uuid, enabled });
                            }
                        }
                    }
                    GuiReq::SetProvider {
                        uuid,
                        name,
                        endpoint,
                        api_key,
                    } => {
                        if let Ok(g) = ipc_req.lock() {
                            if let Some(tx) = g.as_ref() {
                                let _ = tx.send(ClientRequest::SetProvider {
                                    uuid,
                                    name,
                                    endpoint,
                                    api_key,
                                });
                            }
                        }
                    }
                    GuiReq::DeleteProvider { uuid } => {
                        if let Ok(g) = ipc_req.lock() {
                            if let Some(tx) = g.as_ref() {
                                let _ = tx.send(ClientRequest::DeleteProvider { uuid });
                            }
                        }
                    }
                    GuiReq::SetModel {
                        uuid,
                        name,
                        model_id,
                        provider_uuid,
                        route,
                        roles,
                        scope,
                    } => {
                        if let Ok(g) = ipc_req.lock() {
                            if let Some(tx) = g.as_ref() {
                                let _ = tx.send(ClientRequest::SetModel {
                                    uuid,
                                    name,
                                    model_id,
                                    provider_uuid,
                                    route,
                                    roles,
                                    scope,
                                });
                            }
                        }
                    }
                    GuiReq::DeleteModel { uuid, scope } => {
                        if let Ok(g) = ipc_req.lock() {
                            if let Some(tx) = g.as_ref() {
                                let _ = tx.send(ClientRequest::DeleteModel { uuid, scope });
                            }
                        }
                    }
                    GuiReq::ListModels { provider } => {
                        if let Ok(g) = ipc_req.lock() {
                            if let Some(tx) = g.as_ref() {
                                let _ = tx.send(ClientRequest::ListModels { provider });
                            }
                        }
                    }
                },
            }
        });
    // wry's default `.build(&window)` on Linux attaches the webview via a
    // fragile X11 foreign-window reparenting path (a second GtkWindow bolted
    // onto tao's X11 surface) that, on some GPUs, renders the DOM to an
    // uncomposited surface -> a live but invisible (blank/gray) window. The
    // fix (same one Tauri uses) is to attach the webview directly to tao's
    // real GTK widget hierarchy via `build_gtk(window.default_vbox())`
    // instead of going through the X11 reparenting path at all.
    #[cfg(target_os = "linux")]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window
            .default_vbox()
            .ok_or_else(|| anyhow::anyhow!("tao window has no default_vbox (GTK)"))?;
        wv_builder
            .build_gtk(vbox)
            .context("failed to build webview (gtk)")?
    };
    #[cfg(not(target_os = "linux"))]
    let webview = wv_builder.build(&window).context("failed to build webview")?;

    // --- 3b. macOS: clear WKWebView's `underPageBackgroundColor` -----------------
    // wry 0.52.1's "transparent" feature (enabled on our `wry` dependency above)
    // only clears the legacy `drawsBackground` WKWebViewConfiguration flag on
    // macOS/iOS; it never touches `underPageBackgroundColor`. On macOS 12+,
    // WKWebView paints that color (opaque by default) behind the page
    // independently of `drawsBackground`, which produces exactly the "opaque
    // square behind the rounded #app corners" symptom even though both
    // `with_transparent(true)` calls (tao window + wry webview) are honored.
    // Clear it explicitly via the real `WKWebView` handle wry exposes through
    // `WebViewExtMacOS`. macOS-only; no-op on Linux/Windows.
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSColor;
        use wry::WebViewExtMacOS;
        unsafe {
            let ns_webview = webview.webview();
            ns_webview.setUnderPageBackgroundColor(Some(&NSColor::clearColor()));
        }
    }

    // --- 4. Run: push host state to JS + drive the frameless titlebar ----------
    // `run` diverges (`!`); `window` + `webview` move into the closure. On close we
    // just exit the loop — the host-relay client-thread's daemon is a SEPARATE
    // detached process that keeps cooking (resumable via the swapper), exactly like
    // closing a terminal; the process exit drops the client-thread with it.
    event_loop.run(move |event, _target, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            // Host-relay state push: inject the authoritative JSON envelope into the
            // native-React client. `json` is a complete JSON object; it must be
            // re-encoded as a quoted, escaped JS string literal (not embedded
            // verbatim as a raw object) so the JS side's `JSON.parse(j)` receives
            // an actual string to parse, robust to arbitrary chat content.
            Event::UserEvent(UserEvent::Push(json)) => {
                let _ = webview.evaluate_script(&format!(
                    "window.__komaClient.push({})",
                    serde_json::to_string(&json).unwrap_or_else(|_| "\"\"".to_string())
                ));
            }
            // Custom-titlebar window commands: the window is undecorated, so
            // drag / minimize / maximize / close / edge-resize all have to be driven
            // from here via tao's `Window` methods rather than native OS chrome.
            Event::UserEvent(UserEvent::Win(cmd)) => match cmd {
                WinCmd::Drag => {
                    let _ = window.drag_window();
                }
                WinCmd::Minimize => window.set_minimized(true),
                WinCmd::ToggleMax => window.set_maximized(!window.is_maximized()),
                WinCmd::Close => {
                    eprintln!("[gui] titlebar close -> closing");
                    *control_flow = ControlFlow::Exit;
                }
                WinCmd::Resize(dir) => {
                    let _ = window.drag_resize_window(dir);
                }
            },
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                eprintln!("[gui] window close requested");
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}
