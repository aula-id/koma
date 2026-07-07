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

    {
        let push_proxy = proxy.clone();
        let live_req = Arc::clone(&live_req);
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
            );
        });
    }

    // Handles captured by the (Fn) ipc handler: a proxy for titlebar `Win` commands,
    // the control sender for session intents, and the shared live-request sender a
    // chat `Submit` is forwarded through.
    let win_proxy = proxy.clone();
    let ipc_ctl = ctl_tx;
    let ipc_req = Arc::clone(&live_req);

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
                    GuiReq::Submit { text } => {
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
                    GuiReq::NewSession => {
                        let _ = ipc_ctl.send(HostCtl::New);
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
            // native-React client. `json` is a complete JSON object, embedded verbatim.
            Event::UserEvent(UserEvent::Push(json)) => {
                let _ = webview.evaluate_script(&format!("window.__komaClient.push({json})"));
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
