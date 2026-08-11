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
use std::path::PathBuf;
use wry::http::{Request, Response, StatusCode};

// The ipc-bridge wire types (`UserEvent`/`WinCmd`/`ClientMsg`/`GuiReq`) and the
// `GuiReq` dispatcher (`handle_gui_req` + its `GuiReqCtx`) live in the sibling
// `proto`/`dispatch` modules (file size); re-imported here so `run_gui` keeps
// compiling unchanged. `dispatch_git`/`dispatch_forward` are `dispatch`'s own
// split-out git/key routing + generic forwarding helpers (file size).
mod dispatch;
mod dispatch_forward;
mod dispatch_git;
mod proto;
use proto::{ClientMsg, UserEvent, WinCmd};

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
/// of the embedded [`WEBUI`] tree, OR a `koma://extension/<id>/<rest>` request
/// by serving an installed extension's own `ui/` directory straight off disk
/// (see [`handle_extension_request`]). The two are dispatched by the request
/// URI's HOST/authority (`localhost` vs `extension`) — a DIFFERENT origin from
/// the host chrome's, so an extension's panel can never script it even though
/// both ride the same `koma` scheme. Empty path or `/` maps to `index.html`.
fn handle_koma_request(request: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let uri = request.uri();
    // `extension` is the native authority (`koma://extension/...`). On Windows,
    // WebView2/Chromium can't register a real custom scheme, so wry serves the
    // `koma://` protocol over an http fake-domain: it navigates
    // `koma://extension/...` as `http://koma.extension/...` (wry `webview2` mod
    // `work_around_uri_prefix` = `{http_or_https}://{scheme}.`, and koma never
    // calls `with_https_scheme`, so it's `http`). wry REVERTS that fake URL back
    // to `koma://extension/...` before invoking this handler
    // (`revert_uri_work_around` in `prepare_request`), so the native authority is
    // what we actually see on every platform — which is why the main GUI
    // (`koma://localhost`) already works on Windows. We ALSO accept the
    // un-reverted fake authority `koma.extension` defensively, so a request that
    // ever reaches here still in the work-around form routes to the extension UI
    // instead of falling through into the host chrome tree.
    if matches!(uri.host(), Some("extension" | "koma.extension")) {
        return handle_extension_request(uri.path());
    }

    let url_path = uri.path().trim_start_matches('/');
    let url_path = if url_path.is_empty() {
        "index.html"
    } else {
        url_path
    };

    // `koma://localhost/image/<encoded_path>`: serve a local image file for
    // inline rendering in the React transcript. The path is URL-encoded to
    // handle spaces/special chars. Only image extensions are served; the
    // path must be absolute and exist on disk.
    if url_path.starts_with("image/") {
        use percent_encoding::percent_decode_str;
        let encoded = &url_path["image/".len()..];
        if let Ok(decoded) = percent_decode_str(encoded).decode_utf8() {
            let p = std::path::Path::new(decoded.as_ref());
            if p.is_absolute() && p.exists() {
                let mime = match p.extension().and_then(|e| e.to_str()).unwrap_or("") {
                    "png" => "image/png",
                    "jpg" | "jpeg" => "image/jpeg",
                    "gif" => "image/gif",
                    "webp" => "image/webp",
                    "svg" => "image/svg+xml",
                    _ => "application/octet-stream",
                };
                if let Ok(bytes) = std::fs::read(p) {
                    return Response::builder()
                        .status(StatusCode::OK)
                        .header("Content-Type", mime)
                        .header("Cache-Control", "private, max-age=3600")
                        .body(Cow::Owned(bytes))
                        .unwrap_or_else(|_| internal_error_response());
                }
            }
        }
        return not_found_response();
    }

    match WEBUI.get_file(url_path) {
        Some(file) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", mime_for(url_path))
            .body(Cow::Borrowed(file.contents()))
            .unwrap_or_else(|_| internal_error_response()),
        None => not_found_response(),
    }
}

/// Serve `koma://extension/<id>/<rest>` from `extensions_dir()/<id>/ui/<rest>` on disk.
/// This is a SEPARATE origin from `koma://localhost/` (the baked-in host UI) — wry/
/// WebKit key script isolation off the URI's authority, so a different host means a
/// different origin even under the same `koma` scheme; an extension's own UI running
/// under `koma://extension/<id>/` can never script the host chrome under
/// `koma://localhost/`. Empty `<rest>` (or `/`) maps to `index.html`.
///
/// HARD path-safety (both `<id>` and `<rest>` are attacker-reachable — they come
/// straight off the request URI, which the extension's own page controls via its
/// relative asset links): `<id>` must be a well-formed reverse-DNS-style component
/// (whitelist: non-empty, only `[A-Za-z0-9._-]`, at least one alphanumeric character,
/// no leading/trailing `.` — mirrors `app::ext::install::validate_id`'s whitelist) and
/// `<rest>` must resolve to a path that can never escape `<id>/ui/` (no `..` component,
/// no absolute path, no embedded backslash — mirrors `app::ext::install::safe_rel_path`'s
/// zip-slip guard, applied here to protocol paths instead of zip entries). Anything that
/// fails either check, or names a file that doesn't exist on disk, is a 404 — never a
/// panic, never a path leaked into the response body. A rejection (as opposed to a
/// plain missing file) is logged via `append_global_error_log` since it's the shape of
/// an escape attempt, not ordinary 404 traffic.
fn handle_extension_request(path: &str) -> Response<Cow<'static, [u8]>> {
    let path = path.trim_start_matches('/');
    let (id, rest) = path.split_once('/').unwrap_or((path, ""));
    let rest = if rest.is_empty() { "index.html" } else { rest };

    let (Some(safe_id), Some(rel)) = (safe_ext_id(id), safe_ext_rel_path(rest)) else {
        crate::model::store::append_global_error_log(
            "gui",
            &format!("koma://extension rejected unsafe request path: id={id:?} rest={rest:?}"),
        );
        return not_found_response();
    };

    let extensions_dir = match crate::model::store::extensions_dir() {
        Ok(d) => d,
        Err(_) => return internal_error_response(),
    };
    let file_path = extensions_dir.join(safe_id).join("ui").join(&rel);

    match std::fs::read(&file_path) {
        Ok(bytes) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", mime_for(rest))
            .body(Cow::Owned(bytes))
            .unwrap_or_else(|_| internal_error_response()),
        Err(_) => not_found_response(),
    }
}

/// Whitelist guard for the `<id>` path segment of a `koma://extension/<id>/...` request
/// — mirrors `app::ext::install::validate_id`'s reverse-DNS whitelist so a request can
/// never smuggle `..`, `/`, `\`, or a bare `.`/`..` id past this check.
fn safe_ext_id(id: &str) -> Option<&str> {
    let all_allowed = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-');
    let has_alnum = id.chars().any(|c| c.is_ascii_alphanumeric());
    let dot_wrapped = id.starts_with('.') || id.ends_with('.');
    if !all_allowed || !has_alnum || dot_wrapped {
        return None;
    }
    Some(id)
}

/// Turn the `<rest>` of a `koma://extension/<id>/<rest>` request into a clean relative
/// path under `<id>/ui/`, or `None` if unsafe — mirrors `app::ext::install::safe_rel_path`
/// (there, guarding zip-entry names against zip-slip) applied here to protocol-request
/// paths instead: reject an absolute path or any `..` component; empty and `.` components
/// are dropped.
fn safe_ext_rel_path(name: &str) -> Option<PathBuf> {
    if name.starts_with('/') || name.starts_with('\\') {
        return None; // absolute
    }
    let mut out = PathBuf::new();
    for part in name.split('/') {
        match part {
            "" | "." => continue,
            ".." => return None, // escape attempt
            _ => {
                if part.contains('\\') {
                    return None; // windows-style separator sneaking in
                }
                out.push(part);
            }
        }
    }
    Some(out)
}

/// Shared 404 body for both `koma://` request kinds (baked-in host UI + extension UI).
fn not_found_response() -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Cow::Borrowed(&b"not found"[..]))
        .unwrap_or_else(|_| Response::new(Cow::Borrowed(&b"not found"[..])))
}

/// Shared 500 body for both `koma://` request kinds.
fn internal_error_response() -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .body(Cow::Borrowed(&[][..]))
        .unwrap_or_else(|_| Response::new(Cow::Borrowed(&[][..])))
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

    // --- 0. Windows: drop the orphan console on shortcut / double-click launch ---
    // koma.exe is a console-subsystem binary, so launching it from a Start Menu /
    // Desktop shortcut (or a double-click) makes Windows allocate a fresh console
    // that pops up as an empty window behind the GUI. `GetConsoleProcessList` with a
    // 2-slot buffer reports how many processes are attached to THIS console: exactly
    // 1 (only us) means Windows created the console solely for this launch — i.e. a
    // shortcut / double-click — so we `FreeConsole` to make it vanish. More than 1
    // (our parent shell is attached too) means we were launched from a real terminal,
    // where the user wants their console output kept, so we leave it alone. Runs
    // before any terminal / webview setup. (After detaching, runtime logging goes
    // through `store::append_global_error_log` per repo convention — never
    // println!/eprintln!, which a detached console would swallow anyway.)
    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
        let mut pids = [0u32; 2];
        // Count of processes attached to the current console (0 on failure → skip).
        let attached = GetConsoleProcessList(pids.as_mut_ptr(), pids.len() as u32);
        if attached == 1 {
            FreeConsole();
        }
    }

    // WebKitGTK rasterizes text inside GPU-composited scroll layers through a
    // different (stem-darkened) path — any overflowing container renders its text
    // visibly BOLDER than the rest of the app, and the cached layer tile never
    // repaints (live-confirmed on Linux: scrollbar appears → text bolds; fixed by
    // this env). Disabling accelerated compositing removes layer promotion
    // entirely, so all text shares one raster path; for a chat UI the GPU loss is
    // imperceptible and on old Mesa stacks it is the more stable path anyway.
    // Must be set BEFORE any gtk/webkit init; respect an explicit user override.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_COMPOSITING_MODE").is_none() {
        std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
    }

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

    // --- 1b. macOS: minimal native menu bar (Cmd+V/C/X/A fix) -------------------
    // WKWebView dispatches editing commands (paste/copy/cut/select-all) through
    // the app's NSMenu Edit-menu items via the responder chain; with no menu bar
    // at all (frameless window, `with_decorations(false)`), those shortcuts have
    // nothing to route through and never reach the webview. `muda`'s
    // `PredefinedMenuItem::{cut,copy,paste,select_all,undo,redo}` bind the
    // standard AppKit selectors, so installing just an App + Edit submenu is
    // enough — no custom accelerator handling needed on our end. Mirrors muda's
    // own tao example (github.com/tauri-apps/muda examples/tao.rs): build the
    // `Menu` + submenus after the window exists, then `init_for_nsapp()` (main
    // thread only — `run_gui` never leaves the main thread before this point).
    #[cfg(target_os = "macos")]
    {
        use muda::{Menu, PredefinedMenuItem, Submenu};

        let menu_bar = Menu::new();

        // App submenu must be the menu bar's first submenu on macOS; a bare
        // Quit item is enough (`Cmd+Q` otherwise has nowhere to route either).
        let app_menu = Submenu::new("koma", true);
        let _ = menu_bar.append(&app_menu);
        let _ = app_menu.append(&PredefinedMenuItem::quit(None));

        let edit_menu = Submenu::new("Edit", true);
        let _ = menu_bar.append(&edit_menu);
        let _ = edit_menu.append_items(&[
            &PredefinedMenuItem::undo(None),
            &PredefinedMenuItem::redo(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::cut(None),
            &PredefinedMenuItem::copy(None),
            &PredefinedMenuItem::paste(None),
            &PredefinedMenuItem::select_all(None),
        ]);

        menu_bar.init_for_nsapp();
    }

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
    // The webview's current Explore STREAM VIEW (which sub-agent / bash job is streaming
    // into the active stream tab). Written by the ipc thread on a `SetStreamView`, read by
    // the fold loop to fold that one target's transcript / output tail into the push.
    let live_view: Arc<Mutex<super::client::StreamView>> =
        Arc::new(Mutex::new(super::client::StreamView::default()));

    {
        let push_proxy = proxy.clone();
        // A SELF-clone of the control-channel sender rides into the relay so its off-thread
        // session-lifecycle workers (kill / delete) can route a follow-up `RefreshHub` back
        // into whichever host state is active once a daemon is dead / a session deleted. The
        // original `ctl_tx` stays behind for the ipc handler (`ipc_ctl`, below).
        let ctl_tx = ctl_tx.clone();
        let live_req = Arc::clone(&live_req);
        let live_marks = Arc::clone(&live_marks);
        let live_view = Arc::clone(&live_view);
        std::thread::spawn(move || {
            run_host_relay(
                opts,
                // The event loop may already be gone (window closed) — a failed send
                // is fine (the thread is about to be torn down with the process).
                move |json| {
                    let _ = push_proxy.send_event(UserEvent::Push(json));
                },
                ctl_tx,
                ctl_rx,
                live_req,
                live_marks,
                live_view,
            );
        });
    }

    // Handles captured by the (Fn) ipc handler: a proxy for titlebar `Win` commands,
    // plus the `GuiReq` dispatch context (control sender for session intents, the
    // shared live-request sender a chat `Submit` is forwarded through, the staged-
    // attachment markers, and the current Explore stream-tab view) — bundled into
    // one `GuiReqCtx` so `dispatch::handle_gui_req` gets it as a single reference.
    let win_proxy = proxy.clone();
    let gui_ctx = dispatch::GuiReqCtx {
        ctl: ctl_tx,
        req: Arc::clone(&live_req),
        marks: Arc::clone(&live_marks),
        view: Arc::clone(&live_view),
    };

    // --- 3. WebView + ipc handler ----------------------------------------------
    // Windows: WebView2 defaults its user-data folder to a directory beside the
    // exe. When koma is installed under Program Files (not user-writable), creating
    // that folder fails, the WebView2 environment never comes up, and the GUI dies
    // at startup. wry 0.52's `WebContext { data_directory }` is the documented fix —
    // its own docs call it "useful in Windows when a bundled application can't have
    // the webview data inside Program Files" (wry-0.52.1 src/web_context.rs), and the
    // webview2 backend feeds that path straight into
    // `CreateCoreWebView2EnvironmentWithOptions` as the userDataFolder
    // (wry-0.52.1 src/webview2/mod.rs::create_environment). Point it at a writable
    // dir under ~/.koma. Bound here because the builder borrows it until `.build()`;
    // left entirely untouched on Linux/macOS (no WebContext), so their webview data
    // handling is exactly as before.
    #[cfg(target_os = "windows")]
    let mut web_context = {
        let dir = crate::model::store::base_dir()?.join("gui-webdata");
        // ~/.koma already exists (ensure_dirs); create_dir_all is mkdir -p, so an
        // already-present dir is not an error — only a genuine IO failure propagates.
        std::fs::create_dir_all(&dir).context("failed to create GUI webview data dir")?;
        wry::WebContext::new(Some(dir))
    };

    #[cfg(target_os = "windows")]
    let base_builder = WebViewBuilder::new_with_web_context(&mut web_context);
    #[cfg(not(target_os = "windows"))]
    let base_builder = WebViewBuilder::new();

    let wv_builder = base_builder
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
                // Host-relay bridge (native-React client): dispatch the decoded GuiReq
                // via the extracted handler — routes to the client-thread (session
                // intents) or the live daemon (chat submit); see gui::dispatch.
                ClientMsg::Req(req) => dispatch::handle_gui_req(req, &gui_ctx),
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
    let webview = wv_builder
        .build(&window)
        .context("failed to build webview")?;

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
                    crate::model::store::append_global_error_log(
                        "gui",
                        "titlebar close -> closing",
                    );
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
                crate::model::store::append_global_error_log("gui", "window close requested");
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod koma_request_tests {
    use super::*;

    /// A well-formed reverse-DNS id passes; anything that could escape `extensions_dir()`
    /// as a directory name (empty, `.`/`..`, embedded `/`) is rejected.
    #[test]
    fn safe_ext_id_rejects_path_escapes() {
        assert_eq!(
            safe_ext_id("run.koma.example.fleet-board-daemon"),
            Some("run.koma.example.fleet-board-daemon")
        );
        assert_eq!(safe_ext_id(""), None);
        assert_eq!(safe_ext_id("."), None);
        assert_eq!(safe_ext_id(".."), None);
        assert_eq!(safe_ext_id("../etc"), None);
        assert_eq!(safe_ext_id("a/b"), None);
        assert_eq!(safe_ext_id(".hidden"), None);
    }

    /// A plain relative asset path resolves cleanly; `..` (any position), an absolute
    /// path, or an embedded backslash is rejected outright — the zip-slip-style guard
    /// applied to protocol request paths instead of zip entries.
    #[test]
    fn safe_ext_rel_path_rejects_escapes() {
        assert_eq!(
            safe_ext_rel_path("index.html"),
            Some(PathBuf::from("index.html"))
        );
        assert_eq!(
            safe_ext_rel_path("assets/index-abc123.js"),
            Some(PathBuf::from("assets/index-abc123.js"))
        );
        assert_eq!(safe_ext_rel_path(""), Some(PathBuf::new()));
        assert_eq!(safe_ext_rel_path(".."), None);
        assert_eq!(safe_ext_rel_path("../../etc/passwd"), None);
        assert_eq!(safe_ext_rel_path("assets/../../escape"), None);
        assert_eq!(safe_ext_rel_path("/etc/passwd"), None);
        assert_eq!(safe_ext_rel_path("a\\b"), None);
    }
}
