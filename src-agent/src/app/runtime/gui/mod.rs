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

/// The `src-webgui/` directory (vendored xterm.js + addons + glue), embedded
/// at compile time. `$CARGO_MANIFEST_DIR` is `src-agent/`, so this resolves to
/// the repo-root `src-webgui/` tree.
static WEBUI: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../src-webgui");

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

/// Events delivered from the pty reader thread to the main event loop.
enum UserEvent {
    /// A chunk of raw pty output, base64-encoded, to hand to xterm.js.
    Pty(String),
    /// The pty reader hit EOF/error: the koma client exited — tear the window down.
    ChildExited,
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

    // --- 2. Event loop + window ------------------------------------------------
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title("koma")
        .with_inner_size(LogicalSize::new(1024.0, 680.0))
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
    #[allow(clippy::type_complexity)]
    let reader_boot: Arc<Mutex<Option<(Box<dyn Read + Send>, EventLoopProxy<UserEvent>)>>> =
        Arc::new(Mutex::new(Some((reader, proxy))));

    // --- 3. WebView + ipc handler (xterm -> pty) -------------------------------
    let ipc_writer = Arc::clone(&writer);
    let ipc_master = Arc::clone(&master);
    let ipc_boot = Arc::clone(&reader_boot);

    let webview = WebViewBuilder::new()
        .with_devtools(true)
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
            }
        })
        .with_url("koma://localhost/index.html")
        .build(&window)
        .context("failed to build webview")?;

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
