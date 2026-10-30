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

/// OS file drops navigate the document to `file://…` and replace the GUI with a
/// white or transparent page. App pages stay on `koma://` (Windows WebView2
/// shows those as `http://koma.`). Every other scheme is left alone.
pub(crate) fn allow_webview_navigation(url: &str) -> bool {
    !url.trim().to_ascii_lowercase().starts_with("file:")
}

/// WebKitGTK can SIGSEGV in `WebLoaderStrategy::preconnectTo` (custom `koma://`
/// schemes and some HTTPS early-hints / `<link rel=preconnect>`). Turning off
/// DNS prefetch keeps the network process off that path. Best-effort.
///
/// Also force the hardware-acceleration policy to never, and turn off View
/// Transitions. WebKitGTK 2.52 still enters accelerated compositing with the
/// policy set to never, then SIGSEGVs in `AcceleratedBackingStore::update`
/// (null `this`, fault offset 0x48) the first time a page calls
/// `document.startViewTransition()`. n8n Execute does that after its run HTTP
/// returns and the editor swaps in the result. The rust bindings do not wrap
/// `webkit_settings_set_feature_enabled`; call it directly.
#[cfg(target_os = "linux")]
pub(crate) fn linux_disable_webkit_prefetch(webview: &wry::WebView) {
    use webkit2gtk::glib::translate::ToGlibPtr;
    use webkit2gtk::{HardwareAccelerationPolicy, SettingsExt, WebViewExt};
    use wry::WebViewExtUnix;
    if let Some(settings) = webview.webview().settings() {
        settings.set_enable_dns_prefetching(false);
        settings.set_hardware_acceleration_policy(HardwareAccelerationPolicy::Never);
        linux_disable_crash_features(settings.to_glib_none().0);
    }
    // Same webviews. An RGBA toplevel only keeps the pixels WebKit copies on
    // this draw; see `linux_keep_full_frame`. An opaque window keeps them all.
    if crate::model::linux_window::transparent() {
        linux_keep_full_frame(webview);
    }
}

/// WebKitGTK 2.54 copies only the damage clip into an RGBA toplevel
/// (`webkitWebViewBaseDraw` → `AcceleratedBackingStore::paint`). Pixels
/// outside that clip stay alpha 0, so the window shows a strip under the
/// cursor and the rest falls through to the desktop.
///
/// The committed WebKit buffer is already the whole page, transparent
/// corners included. This runs before WebKit's draw handler and widens the
/// clip to the widget so that buffer is copied whole. `#app` keeps its
/// anti-aliased radius. On Wayland, GDK also records only the original
/// damage rect (`wl_surface_damage` / the cairo backfill), and
/// `gdk_window_shape_combine_region` does not clip the frame, so a partial
/// clip also schedules one full invalidate. That full pass is not partial,
/// and this handler does not schedule another.
#[cfg(target_os = "linux")]
fn linux_keep_full_frame(webview: &wry::WebView) {
    use gtk::prelude::*;
    use wry::WebViewExtUnix;
    webview.webview().connect_draw(|widget, cr| {
        let alloc = widget.allocation();
        let w = f64::from(alloc.width());
        let h = f64::from(alloc.height());
        if w > 0.0 && h > 0.0 {
            let partial = matches!(
                cr.clip_extents(),
                Ok((x1, y1, x2, y2))
                    if x1 > 0.5 || y1 > 0.5 || x2 < w - 0.5 || y2 < h - 0.5
            );
            cr.reset_clip();
            cr.rectangle(0.0, 0.0, w, h);
            cr.clip();
            // The follow-up paint's clip is the whole widget, so this does
            // not queue itself again.
            if partial {
                widget.queue_draw();
            }
        }
        gtk::glib::Propagation::Proceed
    });
}

/// View Transitions are the confirmed null-backing-store trigger (WebKit 321683).
/// Disabling accelerated compositing here freezes scroll on a display with no
/// DRI3, so only the transition features are turned off.
#[cfg(target_os = "linux")]
fn linux_disable_crash_features(settings: *mut webkit2gtk::ffi::WebKitSettings) {
    use std::ffi::{c_char, c_void, CStr};
    extern "C" {
        fn webkit_settings_get_all_features() -> *mut c_void;
        fn webkit_feature_list_get_length(list: *mut c_void) -> usize;
        fn webkit_feature_list_get(list: *mut c_void, index: usize) -> *mut c_void;
        fn webkit_feature_get_identifier(feature: *mut c_void) -> *const c_char;
        fn webkit_settings_set_feature_enabled(
            settings: *mut webkit2gtk::ffi::WebKitSettings,
            feature: *mut c_void,
            enabled: i32,
        );
    }
    // Safety: settings is the live WebKitSettings from this webview. The feature
    // list and each feature pointer are owned by WebKit for the process lifetime
    // (transfer-none). Identifiers are NUL-terminated C strings.
    unsafe {
        if settings.is_null() {
            return;
        }
        let list = webkit_settings_get_all_features();
        if list.is_null() {
            return;
        }
        let n = webkit_feature_list_get_length(list);
        for i in 0..n {
            let feature = webkit_feature_list_get(list, i);
            if feature.is_null() {
                continue;
            }
            let id = webkit_feature_get_identifier(feature);
            if id.is_null() {
                continue;
            }
            let id = CStr::from_ptr(id);
            if id == c"ViewTransitions" || id == c"CrossDocumentViewTransitions" {
                webkit_settings_set_feature_enabled(settings, feature, 0);
            }
        }
    }
}

/// WebKitGTK rasterizes overflowing scroll layers on a different path. On this
/// TurboVNC display those tiles never repaint, so the wheel does nothing.
/// `WEBKIT_DISABLE_COMPOSITING_MODE=1` is the path that scrolls. It SIGSEGVs in
/// `AcceleratedBackingStore::update` only if a page then calls
/// `document.startViewTransition()` while that feature is still on (WebKit
/// 324730). View Transitions are disabled before the page runs, so this stays
/// on for every Linux display, including ones with no DRI3.
#[cfg(target_os = "linux")]
fn linux_software_egl_if_no_dri3() {
    if std::env::var_os("WEBKIT_DISABLE_COMPOSITING_MODE").is_none() {
        std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
    }
}

/// Dock icon from the bundle written by [`crate::app::launcher::install_for_launch`].
/// Missing on the first open, until that install finishes.
#[cfg(target_os = "macos")]
fn apply_dock_icon() {
    let Some(path) = crate::app::launcher::dock_icon_path() else {
        return;
    };
    let Ok(path) = std::ffi::CString::new(path.to_string_lossy().as_bytes()) else {
        return;
    };
    // Safety: `path` is a live NUL-terminated path. Callers run on the main
    // thread after the window build has created NSApplication.
    unsafe { agent::computer_native::koma_set_app_icon(path.as_ptr()) };
}

// The ipc-bridge wire types (`UserEvent`/`WinCmd`/`ClientMsg`/`GuiReq`) and the
// `GuiReq` dispatcher (`handle_gui_req` + its `GuiReqCtx`) live in the sibling
// `proto`/`dispatch` modules (file size); re-imported here so `run_gui` keeps
// compiling unchanged. `dispatch_git`/`dispatch_forward` are `dispatch`'s own
// split-out git/key routing + generic forwarding helpers (file size).
mod computer_viewer;
mod dispatch;
mod dispatch_forward;
mod dispatch_git;
mod proto;
mod screen_browser_window;
mod usage_bar;
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

    // Product mark for chrome (browser tabs, welcome, etc.).
    if url_path == "product-icon.png" || url_path == "icon-128.png" {
        return Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "image/png")
            .header("Cache-Control", "private, max-age=86400")
            .body(Cow::Borrowed(
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../assets/icon-128.png"
                ))
                .as_slice(),
            ))
            .unwrap_or_else(|_| internal_error_response());
    }

    // `koma://localhost/image/<encoded_path>`: serve a local image file for
    // inline rendering in the React transcript. The path is URL-encoded to
    // handle spaces/special chars. Only image extensions are served; the
    // path must be absolute and exist on disk.
    if let Some(encoded) = url_path.strip_prefix("image/") {
        use percent_encoding::percent_decode_str;
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

/// Peek the `"k"` tag from a serialized push envelope without a full parse.
fn peek_push_kind(json: &str) -> Option<&str> {
    const KEY: &str = "\"k\":";
    let i = json.find(KEY)?;
    let rest = json[i + KEY.len()..].trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(&rest[..end])
}

pub fn run_gui(opts: crate::cli::Opts) -> Result<()> {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
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

    // Compositing-off is set before GTK/WebKit init. On a display with no DRI3
    // it is what makes scroll layers repaint; View Transitions are disabled so
    // that same setting does not SIGSEGV on `document.startViewTransition()`.
    #[cfg(target_os = "linux")]
    linux_software_egl_if_no_dri3();

    // --- 1. Event loop + window (frameless, transparent) -----------------------
    // The app-list entry is installed later, off this thread. Doing it here
    // stats the launch binary before any window exists. On macOS that stat is
    // the Documents / Desktop / Downloads dialog, so the app never appears.
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    // Opaque when install.sh chose it in /etc/koma/gui.conf (Linux outside
    // the Debian family); see `model::linux_window`.
    let transparent = crate::model::linux_window::transparent();
    let window_builder = WindowBuilder::new()
        .with_title("Koma")
        .with_inner_size(LogicalSize::new(1024.0, 680.0))
        .with_decorations(false)
        .with_resizable(true)
        .with_transparent(transparent);
    // Reassigned only on Linux, where the taskbar icon is applied here. On
    // macOS the Dock icon is set after the window exists.
    #[cfg(target_os = "linux")]
    let window_builder = match crate::app::launcher::window_icon() {
        Some(icon) => window_builder.with_window_icon(Some(icon)),
        None => window_builder,
    };
    let window = window_builder
        .build(&event_loop)
        .context("failed to build GUI window")?;
    #[cfg(target_os = "macos")]
    {
        // Safety: the window build above created NSApplication on this thread.
        unsafe { agent::computer_native::koma_activate_app() };
        apply_dock_icon();
    }
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
    let live_marks: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
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
    let coding_proxy = proxy.clone();
    let coding_service = crate::coding::Service::new(move |json| {
        let _ = coding_proxy.send_event(UserEvent::Push(json));
    });
    let coding_error_proxy = proxy.clone();
    let gui_ctx = dispatch::GuiReqCtx {
        ctl: ctl_tx.clone(),
        req: Arc::clone(&live_req),
        marks: Arc::clone(&live_marks),
        view: Arc::clone(&live_view),
        loop_proxy: proxy.clone(),
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
        .with_initialization_script(format!(
            "window.__komaOS='{}';window.__komaOpaque={};",
            std::env::consts::OS,
            !transparent
        ))
        .with_url("koma://localhost/index.html")
        .with_navigation_handler(|url| allow_webview_navigation(&url))
        .with_transparent(transparent)
        .with_custom_protocol("koma".into(), |_webview_id, request| {
            handle_koma_request(request)
        })
        .with_ipc_handler(move |req: Request<String>| {
            let msg: ClientMsg = match serde_json::from_str(req.body()) {
                Ok(m) => m,
                Err(_) => return, // malformed / unknown -> ignore, never panic
            };
            match msg {
                ClientMsg::ComputerPrepared { id } => {
                    let _ = gui_ctx
                        .ctl
                        .send(crate::app::runtime::client::HostCtl::ComputerPrepared { id });
                }
                ClientMsg::Coding { request } => {
                    let id = request.id.clone();
                    let workspace = request.workspace.clone();
                    if let Err(error) = coding_service.submit(request) {
                        let _ = coding_error_proxy.send_event(UserEvent::Push(
                            serde_json::json!({
                                "k":"CodingReply", "id":id, "workspace":workspace, "error":error
                            })
                            .to_string(),
                        ));
                    }
                }
                // Custom-titlebar window commands (the window is undecorated).
                ClientMsg::Win { a } => {
                    let cmd = match a.as_str() {
                        "computer-viewer" => Some(WinCmd::ComputerViewer),
                        "screen-browser" => Some(WinCmd::ScreenBrowser),
                        "screen-browser-close" => Some(WinCmd::ScreenBrowserClose),
                        "drag" => Some(WinCmd::Drag),
                        "min" => Some(WinCmd::Minimize),
                        "max" => Some(WinCmd::ToggleMax),
                        "close" => Some(WinCmd::Close),
                        "devtools" => Some(WinCmd::OpenDevTools),
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
    #[cfg(target_os = "linux")]
    linux_disable_webkit_prefetch(&webview);
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
    // WebKitGTK executes `evaluate_script` on its UI thread. A cold attach emits
    // several envelopes back-to-back (Loading/Snapshot/Status/Config, followed by
    // settings/repo replies); injecting each as its own tao user event starves
    // WebKit's paint pass and visibly freezes even the CSS boot spinner. Collect
    // everything that arrived in one event-loop turn and inject ONE array at most
    // once per display frame. The React bridge consumes the array in-order and,
    // while switching, deliberately applies one envelope per animation frame.
    const PUSH_FRAME_BUDGET: Duration = Duration::from_millis(16);
    let mut pending_pushes: VecDeque<String> = VecDeque::new();
    let mut next_push_at: Option<Instant> = None;
    let mut last_push_at: Option<Instant> = None;
    // Memory is local to this process. Wake at least every 2s so the menu bar
    // and the footer card move without a host Status tick.
    let mut usage_ticker = usage_bar::UsageTicker::new();
    let wait_for = |next_push_at: Option<Instant>, usage_at: Instant| -> ControlFlow {
        let at = match next_push_at {
            Some(push) => push.min(usage_at),
            None => usage_at,
        };
        ControlFlow::WaitUntil(at)
    };

    let mut computer_viewer: Option<computer_viewer::Viewer> = None;
    let mut computer_status: Option<crate::app::runtime::computer::Status> = None;
    let computer_ctl = ctl_tx.clone();
    let mut computer_palette = serde_json::Value::Null;
    let mut screen_browser: Option<screen_browser_window::BrowserWindow> = None;
    let mut screen_browser_status: Option<crate::app::runtime::screen_browser::Status> = None;
    let screen_browser_ctl = ctl_tx.clone();
    let mut screen_browser_palette = serde_json::Value::Null;
    // Primary session workdir — keys the per-PWD browser profile (cookies/storage).
    // Updated from SettingsValues; falls back to process cwd until the first push.
    let mut screen_browser_workdir =
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    // Taken on the first loop turn. The install must not run before `run`,
    // and it must not stay on this thread: a folder dialog here is the only
    // thing the user sees.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let mut launcher_install = Some(proxy.clone());
    event_loop.run(move |event, target, control_flow| {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if let Some(install_proxy) = launcher_install.take() {
            if let Err(e) = std::thread::Builder::new()
                .name("koma-launcher".into())
                .spawn(move || {
                    if let Err(e) = crate::app::launcher::install_for_launch() {
                        crate::model::store::append_global_error_log(
                            "gui",
                            &format!("app list entry was not installed: {e:#}"),
                        );
                    }
                    let _ = install_proxy.send_event(UserEvent::LauncherReady);
                })
            {
                crate::model::store::append_global_error_log(
                    "gui",
                    &format!("app list install did not start: {e}"),
                );
            }
        }
        *control_flow = wait_for(next_push_at, usage_ticker.deadline());
        match event {
            Event::UserEvent(UserEvent::PickSettingsFolder { request_id }) => {
                let path = dispatch::pick_session_folder(&window);
                let reply = serde_json::json!({ "requestId": request_id, "path": path.map(|p| p.to_string_lossy().into_owned()) });
                let _ = webview.evaluate_script(&format!("window.__komaFolderReply?.({reply});"));
            }
            Event::UserEvent(UserEvent::SettingsFolderPicked { request_id, path }) => {
                let reply = serde_json::json!({ "requestId": request_id, "path": path.map(|p| p.to_string_lossy().into_owned()) });
                let _ = webview.evaluate_script(&format!("window.__komaFolderReply?.({reply});"));
            }
            Event::UserEvent(UserEvent::UiScale { scale, request_id }) => {
                let error = if !proto::valid_ui_scale(scale) {
                    Some("Unsupported UI scale".to_string())
                } else {
                    webview.zoom(scale).err().map(|error| error.to_string())
                };
                let reply = serde_json::json!({ "requestId": request_id, "scale": scale, "error": error });
                let _ = webview.evaluate_script(&format!("window.__komaUiScaleReply?.({reply});"));
            }

            // Host-relay state push: queue the already-serialised object literal.
            // `MainEventsCleared` below drains a whole burst through one JS call,
            // never one synchronous `evaluate_script` per envelope.
            Event::UserEvent(UserEvent::Push(json)) => {
                // Any host push means the page has installed `__komaClient`
                // (it sends Ready only after that assignment).
                if let Some(live) = usage_ticker.note_host_push() {
                    pending_pushes.push_back(live);
                    if next_push_at.is_none() {
                        next_push_at = Some(Instant::now());
                    }
                }
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&json) {
                    usage_ticker.observe_push(&value);
                    if let Some(palette) = value.get("palette").filter(|v| v.is_object()) {
                        computer_palette = palette.clone();
                        screen_browser_palette = palette.clone();
                        if let Some(viewer) = &computer_viewer {
                            viewer.palette(&computer_palette);
                        }
                        if let Some(browser) = &screen_browser {
                            browser.palette(&screen_browser_palette);
                        }
                    }
                    let kind = value.get("k").and_then(|v| v.as_str());
                    if kind == Some("ComputerPrepare") {
                        if let Some(id) = value.get("id").and_then(|v| v.as_str()) {
                            if value.get("hide").and_then(|v| v.as_bool()) == Some(true) {
                                if let Some(viewer) = &computer_viewer { viewer.window.set_visible(false); }
                            }
                            if value.get("hide").and_then(|v| v.as_bool()) == Some(true) {
                                // The in-app preview is HTML in the main window. Wait
                                // for its hide to paint before capturing or injecting input.
                                let id = serde_json::to_string(id).unwrap_or_default();
                                let _ = webview.evaluate_script(&format!(
                                    "(()=>{{document.documentElement.dataset.computerPreparing='true';const ack=()=>window.ipc.postMessage(JSON.stringify({{t:'computer-prepared',id:{id}}}));if(document.hidden)ack();else requestAnimationFrame(()=>requestAnimationFrame(ack));}})();"
                                ));
                            } else {
                                let _ = computer_ctl.send(crate::app::runtime::client::HostCtl::ComputerPrepared { id: id.into() });
                            }
                        }
                        return;
                    }
                    if kind == Some("ComputerPreview") {
                        if let Some(viewer) = &computer_viewer { viewer.frame(&value["frame"]); }
                    }
                    if kind == Some("Computer") {
                        if let Some(status) = value.get("status").and_then(|v| {
                            serde_json::from_value::<crate::app::runtime::computer::Status>(v.clone()).ok()
                        }) {
                            if !status.enabled {
                                computer_viewer = None;
                            } else if let Some(viewer) = &computer_viewer {
                                viewer.update(&status);
                                if !status.busy { viewer.window.set_visible(true); }
                            }
                            if !status.busy {
                                let _ = webview.evaluate_script("delete document.documentElement.dataset.computerPreparing;");
                            }
                            computer_status = Some(status);
                        }
                    }
                    if kind == Some("ScreenBrowser") {
                        if let Some(status) = value.get("status").and_then(|v| {
                            serde_json::from_value::<crate::app::runtime::screen_browser::Status>(v.clone()).ok()
                        }) {
                            if !status.enabled {
                                screen_browser = None;
                            } else if let Some(browser) = &mut screen_browser {
                                if status.message.contains("Clear browsing data") {
                                    let _ = browser.clear_profile();
                                }
                                browser.apply_status(status.clone());
                            } else if status.enabled && screen_browser.is_none() {
                                // Status re-enabled (or first enable) without a separate
                                // ScreenBrowserOpen — open the window now.
                                let _ = proxy.send_event(UserEvent::Win(WinCmd::ScreenBrowser));
                            }
                            screen_browser_status = Some(status);
                        }
                    }
                    if kind == Some("ScreenBrowserOpen") {
                        let _ = proxy.send_event(UserEvent::Win(WinCmd::ScreenBrowser));
                    }
                    if kind == Some("ScreenBrowserChrome") {
                        let action = value
                            .get("action")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let url = value
                            .get("url")
                            .and_then(|v| v.as_str())
                            .map(str::to_owned);
                        let tab_id = value
                            .get("tab_id")
                            .and_then(|v| v.as_str())
                            .map(str::to_owned);
                        let _ = proxy.send_event(UserEvent::ScreenBrowserChrome {
                            action,
                            url,
                            tab_id,
                        });
                        return;
                    }
                    if kind == Some("ScreenBrowserOp") {
                        if let Some(request) = value.get("request").and_then(|v| {
                            serde_json::from_value::<
                                crate::app::runtime::screen_browser::Request,
                            >(v.clone())
                            .ok()
                        }) {
                            let _ = proxy.send_event(UserEvent::ScreenBrowserOp(request));
                        }
                        return;
                    }
                    if kind == Some("Palette") {
                        if let Some(p) = value.get("palette") {
                            computer_palette = p.clone();
                            screen_browser_palette = p.clone();
                            if let Some(viewer) = &computer_viewer {
                                viewer.palette(&computer_palette);
                            }
                            if let Some(browser) = &screen_browser {
                                browser.palette(&screen_browser_palette);
                            }
                        }
                    }
                    // Track primary workdir so the browser profile stays on the project
                    // pwd hash (cookies shared across sessions with the same workdir).
                    if kind == Some("SettingsValues") {
                        if let Some(first) = value
                            .get("workdir")
                            .and_then(|v| v.as_array())
                            .into_iter()
                            .flatten()
                            .filter_map(|v| v.as_str())
                            .map(str::trim)
                            .find(|s| !s.is_empty())
                        {
                            let next = std::path::PathBuf::from(first);
                            let next_hash = crate::model::store::pwd_hash(&next);
                            if screen_browser
                                .as_ref()
                                .is_some_and(|b| b.pwd_hash() != next_hash)
                            {
                                // Workdir moved to another project — drop the window so the
                                // next open loads that project's profile (tabs stay ephemeral).
                                screen_browser = None;
                            }
                            screen_browser_workdir = next;
                        }
                    }
                    let switched = kind == Some("Snapshot")
                        && computer_status.as_ref().is_some_and(|s| {
                            value.get("session").and_then(|v| v.as_str()) != Some(s.session.as_str())
                        });
                    if switched || matches!(kind, Some("Switching" | "Hub")) {
                        let _ = webview.evaluate_script("delete document.documentElement.dataset.computerPreparing;");
                        computer_viewer = None;
                        computer_status = None;
                        screen_browser = None;
                        screen_browser_status = None;
                    }
                }
                pending_pushes.push_back(json);
                if next_push_at.is_none() {
                    let now = Instant::now();
                    next_push_at = Some(
                        last_push_at
                            .map(|last| last + PUSH_FRAME_BUDGET)
                            .filter(|at| *at > now)
                            .unwrap_or(now),
                    );
                }
            }
            Event::MainEventsCleared => {
                let now = Instant::now();
                if let Some(live) = usage_ticker.poll(now) {
                    pending_pushes.push_back(live);
                    if next_push_at.is_none() {
                        next_push_at = Some(now);
                    }
                }
                if next_push_at.is_some_and(|at| now >= at) && !pending_pushes.is_empty() {
                    // Switching/Loading must reach WebKit before Snapshot so the
                    // overlay can paint. A fat Snapshot in the same evaluate_script
                    // freezes the UI thread (and CSS braille) until parse returns.
                    let mut batch_jsons: Vec<String> = Vec::new();
                    let mut hold_rest = false;
                    while let Some(json) = pending_pushes.pop_front() {
                        // Fat transcript envelopes ride alone so Switching/Loading
                        // paint first and WebKit never parses Snapshot+Head together.
                        let kind = peek_push_kind(&json);
                        let solo = matches!(
                            kind,
                            Some("Snapshot") | Some("SnapshotHead") | Some("HistoryPage")
                        );
                        if solo && !batch_jsons.is_empty() {
                            pending_pushes.push_front(json);
                            hold_rest = true;
                            break;
                        }
                        batch_jsons.push(json);
                        if solo {
                            hold_rest = !pending_pushes.is_empty();
                            break;
                        }
                    }
                    let n = batch_jsons.len();
                    let mut batch = String::from("[");
                    for json in batch_jsons {
                        if batch.len() > 1 {
                            batch.push(',');
                        }
                        // Keep each envelope as a JSON STRING inside the batch.
                        // WebKit only parses the cheap outer string array during
                        // evaluate_script; React JSON.parse's one queued bootstrap
                        // envelope per animation frame instead of parsing the fat
                        // Snapshot together with every sibling push.
                        // `json` is already a JSON text string; re-encode as a JSON
                        // string literal for the outer batch array.
                        let encoded = match serde_json::to_string(&json) {
                            Ok(s) => s,
                            Err(_) => continue,
                        };
                        batch.push_str(&encoded);
                    }
                    batch.push(']');
                    let script = format!(
                        "window.__komaClient&&(window.__komaClient.pushBatch?window.__komaClient.pushBatch({batch}):{batch}.forEach(x=>window.__komaClient.push(x)))"
                    );
                    let bytes = script.len();
                    let t0 = Instant::now();
                    let _ = webview.evaluate_script(&script);
                    let took = t0.elapsed();
                    // Always log fat/slow injects; sample small ones so we can
                    // see attach bursts without flooding ~/.koma/error.log.
                    if took >= Duration::from_millis(8) || bytes >= 32_768 || n > 1 {
                        crate::model::store::append_global_error_log(
                            "gui-pace",
                            &format!(
                                "inject n={n} bytes={bytes} took_ms={}",
                                took.as_millis()
                            ),
                        );
                    }
                    last_push_at = Some(now);
                    next_push_at = if hold_rest {
                        Some(now + PUSH_FRAME_BUDGET)
                    } else {
                        None
                    };
                }
            }
            // Custom-titlebar window commands: the window is undecorated, so
            // drag / minimize / maximize / close / edge-resize all have to be driven
            // from here via tao's `Window` methods rather than native OS chrome.
            Event::UserEvent(UserEvent::Win(cmd)) => match cmd {
                WinCmd::ComputerViewer => {
                    if computer_viewer.is_none()
                        && computer_status.as_ref().is_some_and(|s| s.enabled && !s.busy && s.capabilities.floating)
                    {
                        match computer_viewer::Viewer::new(
                            target, computer_status.as_ref(), &computer_palette, computer_ctl.clone(),
                        ) {
                            Ok(viewer) => computer_viewer = Some(viewer),
                            Err(e) => {
                                pending_pushes.push_back(serde_json::json!({
                                    "k": "ComputerError",
                                    "message": format!("Floating preview unavailable: {e}. Use the in-app preview."),
                                }).to_string());
                                next_push_at = Some(Instant::now());
                            }
                        }
                    }
                }
                WinCmd::ScreenBrowser => {
                    let want_hash = crate::model::store::pwd_hash(&screen_browser_workdir);
                    // Wrong project profile still open (workdir changed) → rebuild.
                    if screen_browser
                        .as_ref()
                        .is_some_and(|b| b.pwd_hash() != want_hash)
                    {
                        screen_browser = None;
                    }
                    if screen_browser.is_none() {
                        let status = screen_browser_status.clone().unwrap_or_default();
                        // Create when daemon says enabled. If status is stale-disabled
                        // after a partial stop, enable's status push will re-open via
                        // ScreenBrowserOpen — still try when enabled.
                        if status.enabled {
                            match screen_browser_window::BrowserWindow::new(
                                target,
                                &status,
                                &screen_browser_palette,
                                screen_browser_ctl.clone(),
                                &screen_browser_workdir,
                            ) {
                                Ok(browser) => screen_browser = Some(browser),
                                Err(e) => {
                                    pending_pushes.push_back(serde_json::json!({
                                        "k": "ScreenBrowserError",
                                        "message": format!("On-screen browser unavailable: {e}"),
                                    }).to_string());
                                    next_push_at = Some(Instant::now());
                                }
                            }
                        }
                    } else if let Some(browser) = &screen_browser {
                        // Show only if hidden. On macOS tao's set_visible(true) is
                        // makeKeyAndOrderFront — calling it on an already-visible
                        // window steals focus every time (e.g. after page load status).
                        if !browser.window.is_visible() {
                            browser.window.set_visible(true);
                            // Pin-on-top preference: raise when actually revealing.
                            if browser.always_on_top() {
                                browser.window.set_focus();
                            }
                        }
                    }
                }
                WinCmd::ScreenBrowserClose => {
                    screen_browser = None;
                    // Always stop daemon-side so ownership clears and enable works again.
                    let _ = screen_browser_ctl.send(crate::app::runtime::client::HostCtl::ScreenBrowser {
                        action: "stop".into(),
                        window: None,
                        prefs: None,
                        url: None,
                        tab_id: None,
                        tabs: None,
                        active_tab: None,
                        window_open: false,
                        reply: None,
                    });
                    // Optimistic UI: grey the globe immediately (don't wait for daemon round-trip).
                    if let Some(status) = screen_browser_status.as_mut() {
                        status.enabled = false;
                        status.window_open = false;
                        status.busy = false;
                        status.paused = false;
                        status.waiting_human = false;
                        status.tabs.clear();
                        status.active_tab = None;
                        status.observation = None;
                        status.message = "Stopped".into();
                        pending_pushes.push_back(
                            serde_json::json!({"k":"ScreenBrowser","status":status}).to_string(),
                        );
                        next_push_at = Some(Instant::now());
                    } else {
                        pending_pushes.push_back(
                            serde_json::json!({
                                "k": "ScreenBrowser",
                                "status": {
                                    "session": "",
                                    "generation": "",
                                    "enabled": false,
                                    "paused": false,
                                    "busy": false,
                                    "waiting_human": false,
                                    "window_open": false,
                                    "always_on_top": false,
                                    "capabilities": {
                                        "tabs": true, "capture": true, "input": true,
                                        "eval": false, "downloads": true, "floating": true,
                                        "limitations": []
                                    },
                                    "prefs": {
                                        "always_on_top": false, "auto_screenshot": true,
                                        "stop_on_close": true, "allow_eval": false,
                                        "popup_policy": "new_tab"
                                    },
                                    "tabs": [],
                                    "message": "Stopped"
                                }
                            })
                            .to_string(),
                        );
                        next_push_at = Some(Instant::now());
                    }
                }
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
                    crate::coding::shutdown();
                    *control_flow = ControlFlow::Exit;
                }
                WinCmd::Resize(dir) => {
                    let _ = window.drag_resize_window(dir);
                }
                WinCmd::OpenDevTools => {
                    // with_devtools(true) only enables the inspector; open it
                    // explicitly (custom context menu swallows the native one).
                    webview.open_devtools();
                }
            },
            Event::WindowEvent { window_id, event: WindowEvent::Focused(true), .. }
                if computer_viewer.as_ref().is_some_and(|v| v.window.id() == window_id) => {
                    let _ = computer_ctl.send(crate::app::runtime::client::HostCtl::Computer {
                        action: "pause".into(), window: None,
                    });
                    computer_viewer = None;
                    pending_pushes.push_back(serde_json::json!({"k":"ComputerError","message":"The desktop focused the floating viewer. Control is paused; use the in-app preview."}).to_string());
                    next_push_at = Some(Instant::now());
                }
            Event::WindowEvent { window_id, event: WindowEvent::CloseRequested, .. }
                if computer_viewer.as_ref().is_some_and(|v| v.window.id() == window_id) => {
                    computer_viewer = None;
                }
            Event::WindowEvent { window_id, event: WindowEvent::CloseRequested, .. }
                if screen_browser.as_ref().is_some_and(|v| v.window.id() == window_id) => {
                    // Closing the window = Stop (same as the titlebar Power control):
                    // tear down + disable so the browser does not re-spawn.
                    screen_browser = None;
                    let _ = screen_browser_ctl.send(crate::app::runtime::client::HostCtl::ScreenBrowser {
                        action: "stop".into(),
                        window: None,
                        prefs: None,
                        url: None,
                        tab_id: None,
                        tabs: None,
                        active_tab: None,
                        window_open: false,
                        reply: None,
                    });
                    // Optimistic UI: grey the globe immediately (mirror ScreenBrowserClose).
                    if let Some(status) = screen_browser_status.as_mut() {
                        status.enabled = false;
                        status.window_open = false;
                        status.busy = false;
                        status.paused = false;
                        status.waiting_human = false;
                        status.tabs.clear();
                        status.active_tab = None;
                        status.observation = None;
                        status.message = "Stopped".into();
                        pending_pushes.push_back(
                            serde_json::json!({"k":"ScreenBrowser","status":status}).to_string(),
                        );
                        next_push_at = Some(Instant::now());
                    }
                }
            Event::WindowEvent { window_id, event: WindowEvent::Resized(_), .. }
                if computer_viewer.as_ref().is_some_and(|v| v.window.id() == window_id) => {
                    if let Some(viewer) = &computer_viewer { viewer.resized(); viewer.save(); }
                }
            Event::WindowEvent { window_id, event: WindowEvent::Resized(_), .. }
                if screen_browser.as_ref().is_some_and(|v| v.window.id() == window_id) => {
                    if let Some(browser) = &mut screen_browser { browser.resized(); }
                }
            Event::WindowEvent { window_id, event: WindowEvent::Moved(_), .. }
                if computer_viewer.as_ref().is_some_and(|v| v.window.id() == window_id) => {
                    if let Some(viewer) = &computer_viewer { viewer.save(); }
                }
            Event::WindowEvent { window_id, event: WindowEvent::Moved(_), .. }
                if screen_browser.as_ref().is_some_and(|v| v.window.id() == window_id) => {
                    if let Some(browser) = &screen_browser { browser.save(); }
                }
            Event::UserEvent(UserEvent::ScreenBrowserOp(request)) => {
                if let Some(browser) = &mut screen_browser {
                    let reply = browser.execute(request);
                    let _ = screen_browser_ctl.send(crate::app::runtime::client::HostCtl::ScreenBrowser {
                        action: "result".into(),
                        window: None,
                        prefs: None,
                        url: None,
                        tab_id: None,
                        tabs: None,
                        active_tab: None,
                        window_open: true,
                        reply: Some(Box::new(reply)),
                    });
                } else {
                    let reply = crate::app::runtime::screen_browser::Reply {
                        id: request.id,
                        session: request.session,
                        generation: request.generation,
                        error: Some("On-screen browser window is not open".into()),
                        ..Default::default()
                    };
                    let _ = screen_browser_ctl.send(crate::app::runtime::client::HostCtl::ScreenBrowser {
                        action: "result".into(),
                        window: None,
                        prefs: None,
                        url: None,
                        tab_id: None,
                        tabs: None,
                        active_tab: None,
                        window_open: false,
                        reply: Some(Box::new(reply)),
                    });
                }
            }
            Event::UserEvent(UserEvent::ScreenBrowserChrome { action, url, tab_id }) => {
                // Custom close from frameless chrome — same teardown as native X.
                if action == "win_close" {
                    let _ = proxy.send_event(UserEvent::Win(WinCmd::ScreenBrowserClose));
                } else if action == "win_resize" {
                    if let Some(browser) = &screen_browser {
                        if let Some(dir) = url.as_deref().and_then(parse_resize_dir) {
                            browser.drag_resize(dir);
                        }
                    }
                } else if let Some(browser) = &mut screen_browser {
                    if let Err(e) = browser.handle_chrome(&action, url, tab_id) {
                        pending_pushes.push_back(serde_json::json!({
                            "k": "ScreenBrowserError",
                            "message": e.to_string(),
                        }).to_string());
                        next_push_at = Some(Instant::now());
                    }
                }
            }
            Event::UserEvent(UserEvent::LauncherReady) => {
                // The background install may have just written icon-256.png.
                #[cfg(target_os = "macos")]
                apply_dock_icon();
            }
            Event::UserEvent(UserEvent::PickFolder { kill }) => {
                // Posted on macOS and Windows. The dialog is modal and owned
                // by this window. Linux keeps its picker on a worker thread.
                match dispatch::pick_session_folder(&window) {
                    Some(folder) => {
                        let _ = computer_ctl.send(crate::app::runtime::client::HostCtl::New {
                            workdir: Some(folder),
                            kill,
                        });
                    }
                    None => {
                        let _ = computer_ctl.send(crate::app::runtime::client::HostCtl::RefreshHub);
                    }
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                crate::model::store::append_global_error_log("gui", "window close requested");
                crate::coding::shutdown();
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
        // A push queued during this callback must wake no later than its frame
        // deadline. Preserve Exit selected by a close event.
        if *control_flow != ControlFlow::Exit {
            *control_flow = wait_for(next_push_at, usage_ticker.deadline());
        }
    });
}

#[cfg(test)]
#[path = "mod_koma_request_tests.rs"]
mod koma_request_tests;

#[cfg(test)]
mod navigation_tests {
    use super::allow_webview_navigation;

    #[test]
    fn file_drops_cannot_replace_the_gui() {
        assert!(!allow_webview_navigation("file:///tmp/notes.md"));
        assert!(!allow_webview_navigation("FILE:///Users/a/page.html"));
        assert!(!allow_webview_navigation("  file://C:/code/app.js"));
        assert!(allow_webview_navigation("koma://localhost/index.html"));
        assert!(allow_webview_navigation(
            "koma://localhost/index.html#computer-preview"
        ));
        assert!(allow_webview_navigation("http://koma.localhost/index.html"));
        assert!(allow_webview_navigation(
            "http://koma.extension/panel/index.html"
        ));
        assert!(allow_webview_navigation("about:blank"));
    }
}
