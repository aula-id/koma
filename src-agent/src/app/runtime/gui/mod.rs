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

/// Events delivered from the pty reader thread (or the ipc handler) to the
/// main event loop.
enum UserEvent {
    /// A chunk of raw pty output, base64-encoded, to hand to xterm.js.
    Pty(String),
    /// The pty reader hit EOF/error: the koma client exited — tear the window down.
    ChildExited,
    /// A custom-titlebar window command posted from `koma.js`.
    Win(WinCmd),
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
    /// Keystrokes / paste: `d` is base64-encoded UTF-8 bytes to write to the pty.
    #[serde(rename = "data")]
    Data { d: String },
    /// xterm computed a new grid size -> resize the pty (TIOCSWINSZ -> SIGWINCH).
    #[serde(rename = "resize")]
    Resize { cols: u16, rows: u16 },
    /// The page defined `window.__koma` and is ready for pty output: start the
    /// reader thread (exactly once) so no early bytes are lost.
    #[serde(rename = "ready")]
    Ready,
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

pub fn run_gui(_opts: crate::cli::Opts) -> Result<()> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};
    use tao::{
        dpi::LogicalSize,
        event::{Event, WindowEvent},
        event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
        window::WindowBuilder,
    };
    use wry::WebViewBuilder;

    // --- 1. Spawn the real koma client in a PTY --------------------------------
    // Bare `koma` (no args): the default path mints its own session + daemon and
    // runs the client. NO `--session` (default overwrites it), NO `koma gui`
    // (recursion), NO setsid/null-stdio (the PTY *is* its controlling terminal).
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("failed to open pty")?;

    let exe = std::env::current_exe().context("cannot resolve current executable path")?;
    let mut cmd = CommandBuilder::new(exe);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    // Tells the koma client it's running under the GUI host, so its render loop
    // emits a private OSC 5380 with its canvas bg whenever the palette changes
    // (see client/render.rs render_loop) — the webview listens and repaints its
    // window gutter live. Normal terminal use never sets this, so it's fully
    // gated off outside `koma gui`.
    cmd.env("KOMA_GUI", "1");
    if let Ok(cwd) = std::env::current_dir() {
        cmd.cwd(cwd);
    }

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .context("failed to spawn koma client in pty")?;
    eprintln!("[gui] spawned pty child pid={:?}", child.process_id());
    // Parent drops its slave handle so the master read EOFs when the child exits.
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .context("failed to clone pty reader")?;
    let writer = pair
        .master
        .take_writer()
        .context("failed to take pty writer")?;
    let master = pair.master; // retained for resize()

    // --- 1b. Resolve the *configured* palette canvas bg, for the webview gutter -
    // xterm's cell grid rarely divides the window's pixel size evenly, leaving a
    // remainder strip on the right/bottom that shows through as the container
    // background. That container must match koma's ACTUAL palette (not a
    // hardcoded near-black) or the gutter reads as a visible seam whenever the
    // user runs a non-default palette (e.g. `autumn` = #2e2a20). `AppConfig::load`
    // already falls back to `AppConfig::default()` on any error, and
    // `theme::palette` falls back to `dark()` for an unknown name, so this is
    // infallible; on top of that we defensively fall back to pure black.
    // Same rationale applies to the titlebar/button glyph FOREGROUND: resolve it
    // from the SAME palette so the custom titlebar text/buttons match the
    // configured theme instead of a hardcoded near-white, with a sane fallback
    // for non-Rgb palette variants.
    let (bg_hex, fg_hex) = {
        use ratatui::style::Color;
        let cfg = crate::model::app_config::AppConfig::load();
        let palette = crate::view::theme::palette(&cfg);
        let bg_hex = match palette.bg {
            Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
            Color::Black => "#000000".to_string(),
            _ => "#000000".to_string(),
        };
        let fg_hex = match palette.fg {
            Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
            _ => "#c8d3f5".to_string(),
        };
        (bg_hex, fg_hex)
    };

    // --- 2. Event loop + window ------------------------------------------------
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

    // Shared handles for the (Fn, not FnMut) ipc handler: the pty writer + master
    // live behind mutexes; the reader + proxy sit in an Option the `ready`
    // handshake takes exactly once to launch the reader thread. Starting the
    // reader only after `ready` guarantees the first pty bytes reach a page that
    // already defined `window.__koma.write`.
    let writer = Arc::new(Mutex::new(writer));
    let master = Arc::new(Mutex::new(master));
    // A dedicated clone the ipc handler sends `Win` commands through directly
    // (titlebar drag / min / max / close / edge-resize) — separate from the
    // reader/proxy pair below, which the `ready` handshake takes exactly once.
    let win_proxy = proxy.clone();
    #[allow(clippy::type_complexity)]
    let reader_boot: Arc<Mutex<Option<(Box<dyn Read + Send>, EventLoopProxy<UserEvent>)>>> =
        Arc::new(Mutex::new(Some((reader, proxy))));

    // --- 3. WebView + ipc handler (xterm -> pty) -------------------------------
    let ipc_writer = Arc::clone(&writer);
    let ipc_master = Arc::clone(&master);
    let ipc_boot = Arc::clone(&reader_boot);

    let wv_builder = WebViewBuilder::new()
        .with_devtools(true)
        .with_initialization_script(format!(
            "window.__komaBg='{bg_hex}';window.__komaFg='{fg_hex}';window.__komaOS='{}';",
            std::env::consts::OS
        ))
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
                ClientMsg::Data { d } => {
                    if let Ok(bytes) = STANDARD.decode(d) {
                        if let Ok(mut w) = ipc_writer.lock() {
                            let _ = w.write_all(&bytes);
                            let _ = w.flush();
                        }
                    }
                }
                ClientMsg::Resize { cols, rows } => {
                    eprintln!("[gui] ipc: resize {cols}x{rows}");
                    if let Ok(m) = ipc_master.lock() {
                        let _ = m.resize(PtySize {
                            rows,
                            cols,
                            pixel_width: 0,
                            pixel_height: 0,
                        });
                    }
                }
                ClientMsg::Ready => {
                    eprintln!("[gui] ipc: READY -> starting reader");
                    // Take reader+proxy once; lock is released before the thread
                    // spawns (the take happens inside `and_then`, dropping the guard).
                    let boot = ipc_boot.lock().ok().and_then(|mut g| g.take());
                    if let Some((mut reader, proxy)) = boot {
                        std::thread::spawn(move || {
                            let mut buf = [0u8; 65536];
                            let mut first = true;
                            loop {
                                match reader.read(&mut buf) {
                                    // EOF or read error (pty master EIO on child exit).
                                    Ok(0) | Err(_) => {
                                        eprintln!("[gui] reader: pty EOF/err -> ChildExited");
                                        let _ = proxy.send_event(UserEvent::ChildExited);
                                        break;
                                    }
                                    Ok(n) => {
                                        if first {
                                            first = false;
                                            eprintln!("[gui] reader: first {n} bytes from pty");
                                        }
                                        let b64 = STANDARD.encode(&buf[..n]);
                                        // base64 alphabet is quote-safe.
                                        if proxy.send_event(UserEvent::Pty(b64)).is_err() {
                                            break; // event loop gone
                                        }
                                    }
                                }
                            }
                        });
                    }
                }
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
                // Host-relay bridge (native-React client). R1: parse + log only, so
                // the wire format is validated before any behaviour is wired up.
                ClientMsg::Req(req) => {
                    eprintln!("[gui] ipc req: {req:?}");
                }
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

    // --- 4. Run: pty -> xterm on the main thread; child cleanup on close -------
    // `run` diverges (`!`); `window` stays live in this frame, `webview` + `child`
    // move into the closure. Killing the child tears down the client on window
    // close; its detached daemon persists for `/resume` (same as closing a term).
    let mut first_pty_event = true;
    event_loop.run(move |event, _target, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(UserEvent::Pty(b64)) => {
                if first_pty_event {
                    first_pty_event = false;
                    eprintln!("[gui] evaluate_script: first pty chunk pushed to xterm");
                }
                let _ = webview.evaluate_script(&format!("window.__koma.write('{b64}')"));
            }
            Event::UserEvent(UserEvent::ChildExited) => {
                eprintln!("[gui] child exited -> closing");
                let _ = child.kill();
                let _ = child.wait();
                *control_flow = ControlFlow::Exit;
            }
            // Custom-titlebar window commands: the window is undecorated, so
            // drag / minimize / maximize / close / edge-resize all have to be
            // driven from here via tao's `Window` methods rather than native
            // OS titlebar chrome.
            Event::UserEvent(UserEvent::Win(cmd)) => match cmd {
                WinCmd::Drag => {
                    let _ = window.drag_window();
                }
                WinCmd::Minimize => window.set_minimized(true),
                WinCmd::ToggleMax => window.set_maximized(!window.is_maximized()),
                WinCmd::Close => {
                    eprintln!("[gui] titlebar close -> closing");
                    let _ = child.kill();
                    let _ = child.wait();
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
                let _ = child.kill();
                let _ = child.wait();
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}
