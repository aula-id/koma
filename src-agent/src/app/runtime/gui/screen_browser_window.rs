//! Detached multi-tab on-screen browser window (wry), owned by the GUI event loop.
use super::proto::UserEvent;
use crate::app::runtime::{
    client::HostCtl,
    screen_browser::{
        act::{action_js, wait_is_delay},
        allow_content_url, duckduckgo_search_url, nav_grant_label, nav_never_allow,
        nav_remember_key, navigation_allowed, resolve_omnibox_input, validate_content_url,
        webview_can_load_url, Action, BlockedNav, Observation, Operation, Reply, Request, Status,
        TabInfo, Transform, Viewport, MAX_TABS,
    },
};
use anyhow::{bail, Context, Result};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};
use tao::{
    dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize},
    event_loop::EventLoopWindowTarget,
    window::{Window, WindowBuilder},
};
use wry::{NewWindowResponse, PageLoadEvent, Rect, WebContext, WebView, WebViewBuilder};

/// New-tab welcome (about:blank) — koma logo + short copy, no chrome border.
fn welcome_html() -> String {
    use base64::Engine;
    let logo = base64::engine::general_purpose::STANDARD.encode(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../assets/icon-128.png"
    )));
    let radius = window_radius_px();
    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width,initial-scale=1"/>
<title>New Tab</title>
<style>
  :root {{
    color-scheme: dark light;
    --bg: #0b0e14;
    --fg: #c8d3f5;
    --dim: #8b93a7;
    --accent: #39ff14;
    --panel: #151922;
  }}
  @media (prefers-color-scheme: light) {{
    :root {{
      --bg: #f4f5f8;
      --fg: #1a1d27;
      --dim: #6b7280;
      --accent: #1a9e2e;
      --panel: #ffffff;
    }}
  }}
  * {{ box-sizing: border-box; margin: 0; padding: 0; }}
  html {{
    clip-path: inset(0 round 0 0 {radius}px {radius}px);
  }}
  html, body {{
    height: 100%;
    background: var(--bg);
    color: var(--fg);
    font-family: ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif;
  }}
  body {{
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 100%;
    padding: 2rem;
  }}
  .card {{
    text-align: center;
    max-width: 28rem;
  }}
  .logo {{
    width: 72px; height: 72px;
    border-radius: 18px;
    display: block;
    margin: 0 auto 1.25rem;
    box-shadow: 0 8px 28px color-mix(in srgb, var(--fg) 12%, transparent);
  }}
  h1 {{
    font-size: 1.35rem;
    font-weight: 650;
    letter-spacing: -0.02em;
    margin-bottom: 0.4rem;
  }}
  p {{
    color: var(--dim);
    font-size: 0.95rem;
    line-height: 1.5;
  }}
  .hint {{
    margin-top: 1.5rem;
    font-size: 0.8rem;
    color: var(--dim);
    opacity: 0.85;
  }}
  kbd {{
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 0.78em;
    padding: 0.12em 0.4em;
    border-radius: 4px;
    background: color-mix(in srgb, var(--fg) 8%, var(--bg));
  }}
</style>
</head>
<body>
  <div class="card">
    <img class="logo" width="72" height="72" alt="koma" src="data:image/png;base64,{logo}"/>
    <h1>koma browser</h1>
    <p>Type a URL or search in the address bar to get started.</p>
    <p class="hint">New tab · <kbd>about:blank</kbd></p>
  </div>
</body>
</html>
"##
    )
}

/// Fallback chrome strip height (tabs + address bar). Overwritten when the
/// chrome webview reports its measured height so no black gap remains.
/// Tabs-in-titlebar + address row (measured live via chrome_metrics).
const CHROME_H_DEFAULT: f64 = 88.0;

fn window_radius_px() -> i32 {
    if cfg!(target_os = "macos") {
        16
    } else {
        10
    }
}

/// Overlay 6px edge/corner handles on every page webview so the south/east/west
/// of the frameless window can drag-resize (chrome strip only covers the top).
/// Mirrors `ResizeHandles` in the main GUI. Only `win_resize` is accepted from
/// page ipc — never chrome commands like close.
///
/// Do **not** inject `clip-path` / `background-color:Canvas` into third-party
/// pages. WKWebView (and some WebView2 builds) go blank after first paint when
/// the root element is clipped; Linux survived because `linux_apply_window_shape`
/// rounds the native window instead. Welcome HTML still has its own clip-path.
fn page_init_script() -> String {
    format!(
        r#"(function(){{
  if(!window.__komaNoVT){{
    window.__komaNoVT=1;
    try{{
      var done=Promise.resolve();
      Document.prototype.startViewTransition=function(){{
        return {{finished:done,ready:done,updateCallbackDone:done,types:new Set(),skipTransition:function(){{}}}};
      }};
    }}catch(e){{}}
  }}
}})();
{EDGE_RESIZE_JS}"#
    )
}

const EDGE_RESIZE_JS: &str = r#"(function(){
  if (window.__komaRz) return;
  window.__komaRz = 1;
  var D=['e','w','s','se','sw'];
  var C={
    e:'top:0;right:0;width:6px;height:100%;cursor:ew-resize;',
    w:'top:0;left:0;width:6px;height:100%;cursor:ew-resize;',
    s:'bottom:0;left:6px;right:6px;height:6px;cursor:ns-resize;',
    se:'bottom:0;right:0;width:12px;height:12px;cursor:nwse-resize;',
    sw:'bottom:0;left:0;width:12px;height:12px;cursor:nesw-resize;'
  };
  function go(d){
    try{window.ipc&&window.ipc.postMessage(JSON.stringify({t:'sb',a:'win_resize',dir:d}))}catch(e){}
  }
  function mount(){
    if(!document.body) return;
    D.forEach(function(d){
      if(document.getElementById('koma-rz-'+d)) return;
      var el=document.createElement('div');
      el.id='koma-rz-'+d;
      el.style.cssText='position:fixed;z-index:2147483647;margin:0;padding:0;border:0;background:transparent;pointer-events:auto;'+C[d];
      el.addEventListener('mousedown',function(e){
        if(e.button!==0) return;
        e.preventDefault();e.stopPropagation();
        go(d);
      });
      document.body.appendChild(el);
    });
  }
  if(document.readyState==='loading') document.addEventListener('DOMContentLoaded',mount);
  else mount();
})();"#;

/// Desktop browser UA so sites treat the page like a normal browser (not a bot/webview).
/// macOS uses a Safari-shaped UA (we are WKWebView) so Google WebAuthn/passkey prompts
/// align with the real engine; Chromium UA on Windows/Linux matches WebView2 / webkitgtk quirks less.
fn chrome_user_agent() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.2 Safari/605.1.15"
    }
    #[cfg(target_os = "windows")]
    {
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
    }
    #[cfg(target_os = "linux")]
    {
        // Must match WebKitGTK. A Chrome UA here makes Google serve a Chromium
        // GIS/popup flow that then hangs because we deny native windows.
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.2 Safari/605.1.15"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.2 Safari/605.1.15"
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Placement {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    #[serde(default)]
    always_on_top: bool,
}

struct Tab {
    id: String,
    webview: WebView,
    title: String,
    url: String,
    loading: bool,
    error: Option<String>,
}

/// macOS WKWebView remote layers paint outside their frame after navigation
/// (AppKit no longer clips by default). Wrap each page in a clipping NSView
/// so it cannot cover the chrome strip even if WebKit resets the webview frame.
#[cfg(target_os = "macos")]
fn macos_wrap_tab_in_clip(webview: &WebView) -> Result<()> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSAutoresizingMaskOptions, NSView};
    use objc2_web_kit::WKWebView;
    use wry::WebViewExtMacOS;

    let mtm = MainThreadMarker::new().context("screen-browser wrap needs main thread")?;
    let wk_retain = webview.webview();
    let wk: &WKWebView = wk_retain.as_ref();
    let tab_view: &NSView = wk.as_ref();
    unsafe {
        let parent = tab_view
            .superview()
            .context("tab webview has no superview")?;
        let current = tab_view.frame();
        let clip = NSView::new(mtm);
        clip.setFrame(current);
        clip.setClipsToBounds(true);
        clip.setWantsLayer(true);
        clip.setAutoresizingMask(NSAutoresizingMaskOptions::ViewNotSizable);
        parent.addSubview(&clip);
        clip.addSubview(tab_view);
        let mut inner = current;
        inner.origin.x = 0.0;
        inner.origin.y = 0.0;
        tab_view.setFrame(inner);
        tab_view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        // Never clipsToBounds on the WKWebView itself — that blanks the
        // out-of-process layer after navigation. The wrapper clips.
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn macos_child_frame(
    parent: &objc2_app_kit::NSView,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> objc2_foundation::NSRect {
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    let origin = if parent.isFlipped() {
        NSPoint::new(x, y)
    } else {
        let pf = parent.frame();
        NSPoint::new(x, pf.size.height - y - height)
    };
    NSRect::new(origin, NSSize::new(width, height))
}

#[cfg(target_os = "macos")]
fn macos_layout_tab(
    webview: &WebView,
    chrome: &WebView,
    visible: bool,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) {
    use objc2_app_kit::{NSAutoresizingMaskOptions, NSView};
    use objc2_web_kit::WKWebView;
    use wry::WebViewExtMacOS;

    let tab_wk_owned = webview.webview();
    let tab_wk: &WKWebView = tab_wk_owned.as_ref();
    let tab_view: &NSView = tab_wk.as_ref();
    let chrome_wk_owned = chrome.webview();
    let chrome_wk: &WKWebView = chrome_wk_owned.as_ref();
    let chrome_view: &NSView = chrome_wk.as_ref();
    unsafe {
        let Some(chrome_parent) = chrome_view.superview() else {
            return;
        };
        let frame = macos_child_frame(&chrome_parent, x, y, w, h);
        if let Some(tab_parent) = tab_view.superview() {
            if !std::ptr::eq(&*tab_parent, &*chrome_parent) {
                tab_parent.setFrame(frame);
                tab_parent.setHidden(!visible);
                tab_parent.setClipsToBounds(true);
                tab_parent.setWantsLayer(true);
                tab_parent.setAutoresizingMask(NSAutoresizingMaskOptions::ViewNotSizable);
                tab_view.setFrame(tab_parent.bounds());
                tab_view.setHidden(false);
                return;
            }
        }
        tab_view.setFrame(frame);
        tab_view.setHidden(!visible);
        tab_view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewNotSizable);
    }
}

#[cfg(target_os = "macos")]
fn macos_pin_chrome(chrome: &WebView, tabs: &[Tab], active: Option<&str>) {
    use objc2_app_kit::{NSAutoresizingMaskOptions, NSView, NSWindowOrderingMode};
    use objc2_web_kit::WKWebView;
    use wry::WebViewExtMacOS;

    let chrome_retain = chrome.webview();
    let chrome_wk: &WKWebView = chrome_retain.as_ref();
    let chrome_view: &NSView = chrome_wk.as_ref();
    unsafe {
        chrome_view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        chrome_view.setClipsToBounds(true);
        let Some(parent) = chrome_view.superview() else {
            return;
        };
        for tab in tabs {
            let tab_retain = tab.webview.webview();
            let tab_wk: &WKWebView = tab_retain.as_ref();
            let tab_view: &NSView = tab_wk.as_ref();
            if Some(tab.id.as_str()) != active {
                continue;
            }
            let relative = tab_view
                .superview()
                .filter(|sv| !std::ptr::eq(&**sv, &*parent));
            parent.addSubview_positioned_relativeTo(
                chrome_view,
                NSWindowOrderingMode::Above,
                relative.as_deref(),
            );
        }
        parent.addSubview_positioned_relativeTo(chrome_view, NSWindowOrderingMode::Above, None);
    }
}

/// Keep the page webview opaque across navigations. The window is transparent
/// so chrome corners show the desktop; without this, WKWebView / WebView2 clear
/// to alpha 0 between documents and only the tab strip and URI remain.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn pin_page_opaque(webview: &WebView, blank: bool) {
    let (r, g, b, a) = if blank {
        (11u8, 14, 20, 255)
    } else {
        (255, 255, 255, 255)
    };
    #[cfg(target_os = "windows")]
    {
        let _ = webview.set_background_color((r, g, b, a));
    }
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSColor;
        use objc2_foundation::{ns_string, NSNumber, NSObjectNSKeyValueCoding};
        use wry::WebViewExtMacOS;
        unsafe {
            let wk = webview.webview();
            // wry sets drawsBackground=false whenever a background color is
            // passed at build. Put it back, or navigation punches through.
            let yes = NSNumber::numberWithBool(true);
            wk.setValue_forKey(Some(&yes), ns_string!("drawsBackground"));
            let color = NSColor::colorWithSRGBRed_green_blue_alpha(
                f64::from(r) / 255.0,
                f64::from(g) / 255.0,
                f64::from(b) / 255.0,
                f64::from(a) / 255.0,
            );
            wk.setUnderPageBackgroundColor(Some(&color));
        }
    }
}

/// wry `set_bounds` is a no-op unless the webview lives in a `gtk::Fixed`.
/// `window.default_vbox()` is a GtkBox that `pack_start(..., expand=true)`s every
/// child, so chrome and the page split the window — dark gap under the toolbar,
/// page content starting halfway down (and vanishing into that gap while loading).
#[cfg(target_os = "linux")]
fn linux_fixed_overlay(window: &Window) -> Result<gtk::Fixed> {
    use gtk::prelude::*;
    use tao::platform::unix::WindowExtUnix;
    let vbox = window
        .default_vbox()
        .ok_or_else(|| anyhow::anyhow!("browser GTK container unavailable"))?;
    let fixed = gtk::Fixed::new();
    fixed.set_hexpand(true);
    fixed.set_vexpand(true);
    // Window is transparent so chrome radius shows the desktop. Do NOT paint
    // an opaque plate on the full vbox — that fills the top corner pockets.
    vbox.set_app_paintable(true);
    fixed.set_app_paintable(true);
    fixed.show();
    vbox.pack_start(&fixed, true, true, 0);
    vbox.show_all();
    Ok(fixed)
}

/// Opaque native window covering the page slot while WebKitGTK has cleared
/// to alpha 0 (`load-started` → `load-committed`). A GTK draw on the parent
/// cannot fill that hole: the webview is its own GdkWindow and composites
/// through the transparent toplevel to the wallpaper. Tab-switch "fixed" it
/// because hide/show remapped the webview; this plate is the same idea
/// without changing tabs. Hidden again on committed/finished.
#[cfg(target_os = "linux")]
fn linux_page_plate(fixed: &gtk::Fixed) -> gtk::EventBox {
    use gtk::prelude::*;
    let plate = gtk::EventBox::new();
    plate.set_widget_name("koma-sb-plate");
    plate.set_visible_window(true);
    plate.set_size_request(1, 1);
    plate.set_app_paintable(false);
    plate.set_visible(false);
    plate.connect_draw(|w, cr| {
        let a = w.allocation();
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.rectangle(0.0, 0.0, f64::from(a.width()), f64::from(a.height()));
        let _ = cr.fill();
        gtk::glib::Propagation::Proceed
    });
    fixed.put(&plate, 0, 0);
    plate.show();
    plate.set_visible(false);
    plate
}

/// Cover the page slot for an in-place navigation. wry only forwards
/// Committed as `page_started`, which is after the transparent flash.
#[cfg(target_os = "linux")]
fn linux_cover_on_load(webview: &WebView, plate: &gtk::EventBox, chrome: &WebView) {
    use gtk::prelude::*;
    use webkit2gtk::WebViewExt;
    use wry::WebViewExtUnix;
    let plate = plate.clone();
    let chrome_w = chrome.webview();
    let view = webview.webview();
    let _ = webview.set_background_color((255, 255, 255, 255));
    view.set_background_color(&gtk::gdk::RGBA::new(1.0, 1.0, 1.0, 1.0));
    view.connect_load_changed(move |view, ev| {
        match ev {
            webkit2gtk::LoadEvent::Started => {
                view.set_background_color(&gtk::gdk::RGBA::new(1.0, 1.0, 1.0, 1.0));
                // Don't raise a blocking plate here. Google login is
                // Started → Redirected × N → Committed; a plate that waits
                // for Committed freezes the page and later load_uri no-ops.
            }
            webkit2gtk::LoadEvent::Redirected
            | webkit2gtk::LoadEvent::Committed
            | webkit2gtk::LoadEvent::Finished => {
                plate.set_visible(false);
                if let Some(win) = chrome_w.window() {
                    win.raise();
                }
            }
            _ => {}
        }
    });
}

/// GtkFixed sizes children to their size-request. We keep that request tiny so
/// the window can shrink; this runs *after* GtkFixed's allocate and stretches
/// chrome + the visible page to the real window size.
#[cfg(target_os = "linux")]
fn linux_follow_allocate(
    fixed: &gtk::Fixed,
    chrome: &WebView,
    chrome_h: std::rc::Rc<std::cell::Cell<f64>>,
    plate: gtk::EventBox,
) {
    use gtk::prelude::*;
    use wry::WebViewExtUnix;
    let chrome_widget = chrome.webview();
    chrome_widget.set_widget_name("koma-sb-chrome");
    fixed.connect_size_allocate(move |fixed, alloc| {
        let h = alloc.height().max(1);
        let w = alloc.width().max(1);
        let ax = alloc.x();
        let ay = alloc.y();
        let ch = chrome_h.get().round().clamp(1.0, h as f64) as i32;
        chrome_widget.size_allocate(&gtk::Allocation::new(ax, ay, w, ch));
        let rest = (h - ch).max(1);
        let page = gtk::Allocation::new(ax, ay + ch, w, rest);
        plate.set_size_request(1, 1);
        if plate.parent().is_some() {
            fixed.move_(&plate, ax, ay + ch);
        }
        plate.size_allocate(&page);
        fixed.foreach(|child| {
            let name = child.widget_name();
            if name == "koma-sb-chrome" || name == "koma-sb-plate" {
                return;
            }
            if child.is_visible() {
                child.size_allocate(&page);
            }
        });
    });
}

/// Clip the frameless GTK window to a rounded rect so opaque page webviews
/// cannot paint square corners. No-op on some Wayland compositors (CSS clip
/// on the page is the fallback).
#[cfg(target_os = "linux")]
fn linux_apply_window_shape(window: &Window, width: i32, height: i32, scale: f64) {
    use gtk::gdk::prelude::GdkSurfaceExt;
    use gtk::prelude::*;
    use tao::platform::unix::WindowExtUnix;
    let w = width.max(1);
    let h = height.max(1);
    let r = (window_radius_px() as f64 * scale).max(1.0);
    let Ok(surface) = cairo::ImageSurface::create(cairo::Format::A8, w, h) else {
        return;
    };
    let Ok(cr) = cairo::Context::new(&surface) else {
        return;
    };
    let deg = std::f64::consts::PI / 180.0;
    let wf = f64::from(w);
    let hf = f64::from(h);
    let r = r.min(wf / 2.0).min(hf / 2.0);
    cr.new_sub_path();
    cr.arc(wf - r, r, r, -90.0 * deg, 0.0);
    cr.arc(wf - r, hf - r, r, 0.0, 90.0 * deg);
    cr.arc(r, hf - r, r, 90.0 * deg, 180.0 * deg);
    cr.arc(r, r, r, 180.0 * deg, 270.0 * deg);
    cr.close_path();
    cr.set_source_rgb(1.0, 1.0, 1.0);
    let _ = cr.fill();
    drop(cr);
    let Some(region) = surface.create_region() else {
        return;
    };
    if let Some(gdk_win) = window.gtk_window().window() {
        gdk_win.shape_combine_region(Some(&region), 0, 0);
    }
}

/// Round the Win32 window so we do not need CSS `clip-path` on page HTML.
/// Layered/transparent parents make WebView2 paint a white surface; the
/// browser window is opaque on Windows and this region cuts the corners.
#[cfg(target_os = "windows")]
fn windows_apply_window_shape(window: &Window, width: i32, height: i32, scale: f64) {
    use tao::platform::windows::WindowExtWindows;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::Graphics::Gdi::{CreateRoundRectRgn, DeleteObject, SetWindowRgn};
    let w = width.max(1);
    let h = height.max(1);
    let r = ((window_radius_px() as f64) * scale).round().max(1.0) as i32;
    let rgn = unsafe { CreateRoundRectRgn(0, 0, w + 1, h + 1, r * 2, r * 2) };
    if rgn.is_null() {
        return;
    }
    let hwnd = window.hwnd() as HWND;
    let ok = unsafe { SetWindowRgn(hwnd, rgn, 1) };
    if ok == 0 {
        unsafe {
            DeleteObject(rgn);
        }
    }
}

/// WebView2 blanks after the parent moves/resizes unless the controller
/// is told the HWND geometry changed.
#[cfg(target_os = "windows")]
fn windows_sync_webview(webview: &WebView) {
    use wry::WebViewExtWindows;
    let _ = unsafe { webview.controller().NotifyParentWindowPositionChanged() };
}

/// Force position on Linux. Size-request stays 1×1 so GTK will let the window
/// shrink; `linux_follow_allocate` stretches children to the real allocation.
#[cfg(target_os = "linux")]
fn linux_place(
    fixed: &gtk::Fixed,
    webview: &WebView,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    visible: bool,
) {
    use gtk::prelude::*;
    use wry::WebViewExtUnix;
    let widget = webview.webview();
    let xi = x.round() as i32;
    let yi = y.round() as i32;
    widget.set_size_request(1, 1);
    widget.set_visible(visible);
    if widget.parent().is_some() {
        fixed.move_(&widget, xi, yi);
    } else {
        fixed.put(&widget, xi, yi);
    }
    let _ = webview.set_visible(visible);
    if visible {
        let _ = webview.set_bounds(Rect {
            position: LogicalPosition::new(x, y).into(),
            size: LogicalSize::new(w, h).into(),
        });
    }
}

/// Keep the toolbar's GDK window above tab webviews without unparenting
/// (unparent+put on every layout unmaps chrome while a page is loading).
#[cfg(target_os = "linux")]
fn linux_pin_chrome(chrome: &WebView) {
    use gtk::prelude::WidgetExt;
    use wry::WebViewExtUnix;
    let widget = chrome.webview();
    if let Some(win) = widget.window() {
        win.raise();
    }
}

struct NavGrantState {
    always: Vec<String>,
    once: HashSet<String>,
}

type NavGrants = Rc<RefCell<NavGrantState>>;

fn load_nav_grants(profile_dir: &std::path::Path) -> Vec<String> {
    let path = profile_dir.join("allowed_nav.json");
    std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|k| k.starts_with("scheme:") || k.starts_with("host:"))
        .collect()
}

fn save_nav_grants(profile_dir: &std::path::Path, always: &[String]) {
    let path = profile_dir.join("allowed_nav.json");
    if always.is_empty() {
        let _ = std::fs::remove_file(&path);
        return;
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(always) {
        let _ = std::fs::write(path, bytes);
    }
}

pub(super) struct BrowserWindow {
    pub window: Window,
    chrome: WebView,
    tabs: Vec<Tab>,
    active: Option<String>,
    /// Logical height of the chrome strip (tabs + toolbar). Matches measured UI.
    chrome_h: f64,
    /// Shared GtkFixed so chrome + tabs are absolutely positioned (not GtkBox-split).
    #[cfg(target_os = "linux")]
    linux_fixed: gtk::Fixed,
    /// Live chrome height for the GtkFixed size-allocate follower.
    #[cfg(target_os = "linux")]
    linux_chrome_h: std::rc::Rc<std::cell::Cell<f64>>,
    /// Opaque page-slot cover shown between WebKit load-started and committed.
    #[cfg(target_os = "linux")]
    linux_plate: gtk::EventBox,
    /// Kept alive so content webviews share cookies/storage.
    #[allow(dead_code)]
    context: WebContext,
    preferences: Option<std::path::PathBuf>,
    profile_dir: std::path::PathBuf,
    /// `store::pwd_hash` of the workdir this profile belongs to.
    pwd_hash: String,
    /// Project workdir (for `.screenshoot/` catalog + composer attach context).
    workdir: std::path::PathBuf,
    ctl: std::sync::mpsc::Sender<HostCtl>,
    status: Status,
    always_on_top: bool,
    /// Window stays hidden until chrome React posts chrome_ready / first .sb-chrome metrics.
    chrome_shown: bool,
    nav_grants: NavGrants,
    blocked_nav: Option<BlockedNav>,
}

impl BrowserWindow {
    pub fn new(
        target: &EventLoopWindowTarget<UserEvent>,
        status: &Status,
        palette: &serde_json::Value,
        ctl: std::sync::mpsc::Sender<HostCtl>,
        workdir: &std::path::Path,
    ) -> Result<Self> {
        // Cookies/storage live under the project pwd bucket so every session with
        // the same workdir reuses logins. Tabs stay ephemeral (closed on stop).
        let hash = crate::model::store::pwd_hash(workdir);
        let profile_dir = crate::model::store::screen_browser_profile_dir(&hash)
            .context("screen-browser profile dir")?;
        let preferences = crate::model::store::screen_browser_window_path(&hash).ok();
        let saved = preferences
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|v| serde_json::from_slice::<Placement>(&v).ok());
        let always_on_top =
            status.prefs.always_on_top || saved.as_ref().map(|s| s.always_on_top).unwrap_or(false);
        #[cfg(target_os = "linux")]
        let linux_chrome_h = std::rc::Rc::new(std::cell::Cell::new(CHROME_H_DEFAULT));
        // Opaque on Windows, and on Linux where install.sh chose it.
        let transparent =
            cfg!(not(target_os = "windows")) && crate::model::linux_window::transparent();
        let mut builder = WindowBuilder::new()
            .with_title("Koma · Browser")
            .with_inner_size(PhysicalSize::new(1100, 760))
            .with_min_inner_size(PhysicalSize::new(480, 320))
            .with_focused(true)
            // Frameless. Transparent on macOS/Linux so chrome CSS radius shows the
            // desktop. Windows WebView2 in a WS_EX_LAYERED host paints white —
            // stay opaque there and round with SetWindowRgn.
            .with_decorations(false)
            .with_transparent(transparent)
            .with_resizable(true)
            .with_always_on_top(always_on_top)
            // Hidden until chrome React has painted tabs (avoids empty "New Tab" flash).
            .with_visible(false);
        if let Some(p) = &saved {
            if p.width >= 480 && p.width <= 4000 && p.height >= 320 && p.height <= 4000 {
                builder = builder.with_inner_size(PhysicalSize::new(p.width, p.height));
                let visible = target.available_monitors().any(|m| {
                    let o = m.position();
                    let s = m.size();
                    p.x >= o.x
                        && p.y >= o.y
                        && i64::from(p.x) < i64::from(o.x) + i64::from(s.width)
                        && i64::from(p.y) < i64::from(o.y) + i64::from(s.height)
                });
                if visible {
                    builder = builder.with_position(PhysicalPosition::new(p.x, p.y));
                }
            }
        }
        let window = builder.build(target)?;
        let mut context = WebContext::new(Some(profile_dir.clone()));
        let mut boot_status = status.clone();
        if boot_status.tabs.is_empty() {
            boot_status.tabs.push(TabInfo {
                id: "_boot".into(),
                title: "New Tab".into(),
                url: "about:blank".into(),
                active: true,
                loading: false,
                favicon: None,
                error: None,
            });
            boot_status.active_tab = Some("_boot".into());
        }
        let initial = serde_json::to_string(&boot_status)?;
        let ctl_chrome = ctl.clone();
        // Chrome is a *child* strip only — never a full-window webview (that painted
        // white over the page and made takeSnapshot look empty).
        let chrome_builder = WebViewBuilder::new_with_web_context(&mut context)
            .with_url("koma://localhost/index.html#screen-browser")
            .with_bounds(Rect {
                position: LogicalPosition::new(0.0, 0.0).into(),
                size: LogicalSize::new(1100.0, CHROME_H_DEFAULT).into(),
            })
            // Same as main GUI: transparent webview so CSS radius on `.sb-chrome`
            // is not filled with an opaque plate in the corner pockets.
            // Windows: opaque (layered parent + transparent child = white page).
            .with_transparent(transparent)
            .with_background_color(if !transparent {
                (14, 17, 23, 255)
            } else {
                (0, 0, 0, 0)
            })
            .with_navigation_handler(|url| super::allow_webview_navigation(&url))
            .with_initialization_script(format!(
                "window.__komaOS='{}';window.__komaScreenBrowserInitial={initial};window.__komaScreenBrowserPalette={palette};\
                 window.addEventListener('koma-screen-browser',()=>{{}});\
                 document.addEventListener('DOMContentLoaded',()=>{{try{{window.dispatchEvent(new CustomEvent('koma-screen-browser',{{detail:window.__komaScreenBrowserInitial}}));window.dispatchEvent(new CustomEvent('koma-screen-browser-palette',{{detail:window.__komaScreenBrowserPalette}}));}}catch(e){{}}}});\
                 (function(){{const report=()=>{{try{{const root=document.querySelector('.sb-chrome'); if(!root) return; const h=Math.ceil(root.getBoundingClientRect().height); if(h>0) window.ipc&&window.ipc.postMessage(JSON.stringify({{t:'sb',a:'chrome_metrics',h}}));}}catch(e){{}}}}; window.addEventListener('load',report); window.addEventListener('resize',report); new ResizeObserver(report).observe(document.documentElement); setTimeout(report,0); setTimeout(report,50); setTimeout(report,200); setTimeout(report,500);}})();",
                std::env::consts::OS
            ))
            .with_custom_protocol("koma".into(), |_, request| super::handle_koma_request(request))
            .with_ipc_handler(move |request| {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(request.body()) else {
                    return;
                };
                let action = value
                    .get("a")
                    .or_else(|| value.get("action"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if action.is_empty() {
                    return;
                }
                let url = value
                    .get("url")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
                    .or_else(|| {
                        // chrome_metrics sends height as `h`
                        value.get("h").and_then(|v| v.as_f64()).map(|h| h.to_string())
                    })
                    .or_else(|| {
                        // win_resize sends direction as `dir` (n/s/e/w/ne/…)
                        value
                            .get("dir")
                            .and_then(|v| v.as_str())
                            .map(str::to_owned)
                    });
                let tab_id = value
                    .get("tab_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned);
                let _ = ctl_chrome.send(HostCtl::ScreenBrowserChrome {
                    action: action.into(),
                    url,
                    tab_id,
                });
            });

        #[cfg(target_os = "linux")]
        let linux_fixed = linux_fixed_overlay(&window)?;
        #[cfg(target_os = "linux")]
        let chrome = {
            use wry::WebViewBuilderExtUnix;
            chrome_builder.build_gtk(&linux_fixed)?
        };
        #[cfg(target_os = "linux")]
        super::linux_disable_webkit_prefetch(&chrome);
        #[cfg(not(target_os = "linux"))]
        let chrome = chrome_builder.build_as_child(&window)?;
        #[cfg(target_os = "linux")]
        let linux_plate = linux_page_plate(&linux_fixed);
        #[cfg(target_os = "linux")]
        linux_follow_allocate(
            &linux_fixed,
            &chrome,
            linux_chrome_h.clone(),
            linux_plate.clone(),
        );
        #[cfg(target_os = "macos")]
        {
            // wry's transparent flag does not clear underPageBackgroundColor
            // (see main GUI webview boot). Opaque default = black in the radius.
            use objc2_app_kit::NSColor;
            use wry::WebViewExtMacOS;
            unsafe {
                chrome
                    .webview()
                    .setUnderPageBackgroundColor(Some(&NSColor::clearColor()));
            }
        }

        let mut always = load_nav_grants(&profile_dir);
        for k in &status.prefs.allowed_nav {
            if (k.starts_with("scheme:") || k.starts_with("host:")) && !always.contains(k) {
                always.push(k.clone());
            }
        }
        let nav_grants = Rc::new(RefCell::new(NavGrantState {
            always,
            once: HashSet::new(),
        }));

        let mut browser = Self {
            window,
            chrome,
            tabs: Vec::new(),
            active: None,
            chrome_h: CHROME_H_DEFAULT,
            #[cfg(target_os = "linux")]
            linux_fixed,
            #[cfg(target_os = "linux")]
            linux_chrome_h,
            #[cfg(target_os = "linux")]
            linux_plate,
            context,
            preferences,
            profile_dir,
            pwd_hash: hash,
            workdir: workdir.to_path_buf(),
            ctl,
            status: status.clone(),
            always_on_top,
            chrome_shown: false,
            nav_grants,
            blocked_nav: None,
        };
        browser.open_tab(Some("about:blank".into()), true)?;
        browser.layout()?;
        {
            let always = browser.nav_grants.borrow().always.clone();
            if !always.is_empty() {
                browser.status.prefs.allowed_nav = always;
                let _ = browser.ctl.send(HostCtl::ScreenBrowser {
                    action: "set_prefs".into(),
                    window: None,
                    prefs: Some(browser.status.prefs.clone()),
                    url: None,
                    tab_id: None,
                    tabs: None,
                    active_tab: None,
                    window_open: true,
                    reply: None,
                });
            }
        }
        browser.push_status();
        Ok(browser)
    }

    pub fn apply_status(&mut self, mut status: Status) {
        let local = self.nav_grants.borrow().always.clone();
        if status.prefs.allowed_nav != local {
            if status.prefs.allowed_nav.is_empty() && !local.is_empty() {
                status.prefs.allowed_nav = local;
            } else {
                self.nav_grants.borrow_mut().always = status.prefs.allowed_nav.clone();
                save_nav_grants(&self.profile_dir, &status.prefs.allowed_nav);
            }
        }
        status.blocked_nav = self.blocked_nav.clone();
        self.status = status.clone();
        if status.always_on_top != self.always_on_top {
            self.always_on_top = status.always_on_top;
            self.window.set_always_on_top(self.always_on_top);
        }
        let _ = self.chrome.evaluate_script(&format!(
            "window.dispatchEvent(new CustomEvent('koma-screen-browser',{{detail:{}}}));",
            serde_json::to_string(&status).unwrap_or_else(|_| "null".into())
        ));
        if !status.enabled {
            // Host will drop the window.
        }
    }

    pub fn always_on_top(&self) -> bool {
        self.always_on_top
    }

    /// Live theme push from the main GUI host (same palette as chat chrome).
    pub fn palette(&self, palette: &serde_json::Value) {
        let _ = self.chrome.evaluate_script(&format!(
            "window.__komaScreenBrowserPalette={palette};window.dispatchEvent(new CustomEvent('koma-screen-browser-palette',{{detail:window.__komaScreenBrowserPalette}}));"
        ));
    }

    /// Project key this window's cookies/storage belong to (`store::pwd_hash`).
    pub fn pwd_hash(&self) -> &str {
        &self.pwd_hash
    }

    pub fn resized(&mut self) {
        let _ = self.layout();
        self.save();
    }

    /// Unlock GTK min-size, then start a native edge/corner drag-resize.
    pub fn drag_resize(&self, dir: tao::window::ResizeDirection) {
        #[cfg(target_os = "linux")]
        {
            use gtk::prelude::*;
            use tao::platform::unix::WindowExtUnix;
            use wry::WebViewExtUnix;
            self.chrome.webview().set_size_request(-1, -1);
            for tab in &self.tabs {
                tab.webview.webview().set_size_request(-1, -1);
            }
            self.linux_fixed.set_size_request(-1, -1);
            self.window.gtk_window().set_size_request(-1, -1);
        }
        let _ = self.window.drag_resize_window(dir);
    }

    pub fn save(&self) {
        let Some(path) = &self.preferences else {
            return;
        };
        let size = self.window.inner_size();
        let pos = self.window.outer_position().unwrap_or_default();
        let placement = Placement {
            x: pos.x,
            y: pos.y,
            width: size.width,
            height: size.height,
            always_on_top: self.always_on_top,
        };
        if let Ok(bytes) = serde_json::to_vec_pretty(&placement) {
            let _ = std::fs::write(path, bytes);
        }
    }

    fn layout(&mut self) -> Result<()> {
        let size = self.window.inner_size();
        let scale = self.window.scale_factor();
        let logical_w = size.width as f64 / scale;
        let logical_h = size.height as f64 / scale;
        // Keep chrome strip tight: never more than ~1/3 of the window, never tiny.
        let chrome_h = self
            .chrome_h
            .clamp(64.0, (logical_h * 0.35).clamp(64.0, 160.0));
        self.chrome_h = chrome_h;
        #[cfg(target_os = "linux")]
        self.linux_chrome_h.set(chrome_h);
        #[cfg(target_os = "linux")]
        linux_apply_window_shape(&self.window, size.width as i32, size.height as i32, scale);
        #[cfg(target_os = "windows")]
        windows_apply_window_shape(&self.window, size.width as i32, size.height as i32, scale);
        let content_h = (logical_h - chrome_h).max(40.0);
        let active = self.active.clone();

        let width = logical_w.max(200.0);
        // Content first (lower z), chrome last (must stay above the page).
        for tab in &self.tabs {
            let is_active = Some(tab.id.as_str()) == active.as_deref();
            #[cfg(target_os = "macos")]
            macos_layout_tab(
                &tab.webview,
                &self.chrome,
                is_active,
                0.0,
                chrome_h,
                width,
                content_h,
            );
            #[cfg(target_os = "linux")]
            linux_place(
                &self.linux_fixed,
                &tab.webview,
                0.0,
                chrome_h,
                width,
                content_h,
                is_active,
            );
            #[cfg(not(any(target_os = "macos", target_os = "linux")))]
            {
                let _ = tab.webview.set_visible(is_active);
                if is_active {
                    let _ = tab.webview.set_bounds(Rect {
                        position: LogicalPosition::new(0.0, chrome_h).into(),
                        size: LogicalSize::new(width, content_h).into(),
                    });
                    #[cfg(target_os = "windows")]
                    windows_sync_webview(&tab.webview);
                }
            }
        }

        #[cfg(target_os = "linux")]
        linux_place(
            &self.linux_fixed,
            &self.chrome,
            0.0,
            0.0,
            width,
            chrome_h,
            true,
        );
        #[cfg(not(target_os = "linux"))]
        {
            let _ = self.chrome.set_bounds(Rect {
                position: LogicalPosition::new(0.0, 0.0).into(),
                size: LogicalSize::new(width, chrome_h).into(),
            });
            let _ = self.chrome.set_visible(true);
            #[cfg(target_os = "windows")]
            windows_sync_webview(&self.chrome);
        }
        #[cfg(target_os = "linux")]
        linux_pin_chrome(&self.chrome);
        #[cfg(target_os = "macos")]
        macos_pin_chrome(&self.chrome, &self.tabs, self.active.as_deref());
        Ok(())
    }

    fn open_tab(&mut self, url: Option<String>, activate: bool) -> Result<String> {
        if self.tabs.len() >= MAX_TABS {
            bail!("tab limit ({MAX_TABS}) reached");
        }
        let id = uuid::Uuid::new_v4().to_string();
        let start = url.unwrap_or_else(|| "about:blank".into());
        validate_content_url(&start)?;
        let id_nav = id.clone();
        let id_load = id.clone();
        let id_title = id.clone();
        let ctl_load = self.ctl.clone();
        let ctl_title = self.ctl.clone();
        let size = self.window.inner_size();
        let scale = self.window.scale_factor();
        let logical_w = size.width as f64 / scale;
        let logical_h = size.height as f64 / scale;
        let chrome_h = self.chrome_h;
        let content_h = (logical_h - chrome_h).max(40.0);
        let blank = crate::app::runtime::screen_browser::is_blank_url(&start);
        // GPU compositing is on by default in WKWebView / WebView2 / WebKitGTK.
        // WebGPU is available when the OS webview supports it (Safari 17+ / Edge 113+).
        let mut builder = WebViewBuilder::new_with_web_context(&mut self.context)
            .with_user_agent(chrome_user_agent())
            // Opaque plate like main GUI `#app`. Widget stays non-transparent so
            // pages cannot composite with the desktop. Native window shape (Linux /
            // Windows) rounds the frame; do not clip third-party HTML.
            // macOS: do not pass a color here. wry's transparent feature treats
            // any background color as `drawsBackground = false`, which is the
            // navigation punch-through (tab + URI, no body). Color is applied
            // after build.
            .with_back_forward_navigation_gestures(true)
            .with_bounds(Rect {
                position: LogicalPosition::new(0.0, chrome_h).into(),
                size: LogicalSize::new(logical_w.max(200.0), content_h).into(),
            });
        #[cfg(not(target_os = "macos"))]
        {
            builder = builder.with_background_color(if blank {
                (11, 14, 20, 255)
            } else {
                (255, 255, 255, 255)
            });
        }
        builder = if blank {
            builder.with_html(welcome_html())
        } else {
            builder.with_url(&start)
        };
        let builder = builder
            .with_navigation_handler({
                let ctl = self.ctl.clone();
                let grants = self.nav_grants.clone();
                move |u| {
                    let ok = {
                        let g = grants.borrow();
                        navigation_allowed(&u, &g.always, &g.once)
                    };
                    if !ok {
                        let _ = ctl.send(HostCtl::ScreenBrowserChrome {
                            action: "nav_blocked".into(),
                            url: Some(u),
                            tab_id: Some(id_nav.clone()),
                        });
                    }
                    ok
                }
            })
            .with_on_page_load_handler(move |event, url| {
                let action = match event {
                    PageLoadEvent::Started => "page_started",
                    PageLoadEvent::Finished => "page_finished",
                };
                let _ = ctl_load.send(HostCtl::ScreenBrowserChrome {
                    action: action.into(),
                    url: Some(url),
                    tab_id: Some(id_load.clone()),
                });
            })
            .with_document_title_changed_handler(move |title| {
                let _ = ctl_title.send(HostCtl::ScreenBrowserChrome {
                    action: "page_title".into(),
                    url: Some(title),
                    tab_id: Some(id_title.clone()),
                });
            })
            .with_new_window_req_handler({
                let ctl = self.ctl.clone();
                move |u, _features| {
                    // Always deny a native OS window. Real popups become tabs via
                    // "popup"; ad/tracker beacons are dropped in handle_chrome.
                    // (wry >=0.55 only calls this for window.open / target=_blank,
                    // not for iframe navigations as 0.52 did on macOS.)
                    if allow_content_url(&u) {
                        let _ = ctl.send(HostCtl::ScreenBrowserChrome {
                            action: "popup".into(),
                            url: Some(u),
                            tab_id: None,
                        });
                    }
                    NewWindowResponse::Deny
                }
            })
            .with_initialization_script(page_init_script())
            .with_ipc_handler({
                let ctl = self.ctl.clone();
                move |request| {
                    let Ok(value) = serde_json::from_str::<serde_json::Value>(request.body())
                    else {
                        return;
                    };
                    let action = value.get("a").and_then(|v| v.as_str()).unwrap_or("");
                    if action != "win_resize" {
                        return;
                    }
                    let dir = value.get("dir").and_then(|v| v.as_str()).map(str::to_owned);
                    let _ = ctl.send(HostCtl::ScreenBrowserChrome {
                        action: "win_resize".into(),
                        url: dir,
                        tab_id: None,
                    });
                }
            });

        #[cfg(target_os = "linux")]
        let webview = {
            use wry::WebViewBuilderExtUnix;
            builder.build_gtk(&self.linux_fixed)?
        };
        #[cfg(target_os = "linux")]
        super::linux_disable_webkit_prefetch(&webview);
        #[cfg(target_os = "linux")]
        linux_cover_on_load(&webview, &self.linux_plate, &self.chrome);
        // Content tabs are child webviews so the page sits under the chrome strip
        // (Chrome multi-tab style) and native takeSnapshot sees real page pixels.
        #[cfg(not(target_os = "linux"))]
        let webview = builder.build_as_child(&self.window)?;
        #[cfg(target_os = "macos")]
        macos_wrap_tab_in_clip(&webview)?;
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        pin_page_opaque(&webview, blank);

        let loading = !crate::app::runtime::screen_browser::is_blank_url(&start);
        self.tabs.push(Tab {
            id: id.clone(),
            webview,
            title: if crate::app::runtime::screen_browser::is_blank_url(&start) {
                "New Tab".into()
            } else {
                start.clone()
            },
            url: if crate::app::runtime::screen_browser::is_blank_url(&start) {
                "about:blank".into()
            } else {
                start
            },
            loading,
            error: None,
        });
        if activate {
            self.active = Some(id.clone());
        }
        self.layout()?;
        self.push_status();
        Ok(id)
    }

    fn tab_mut(&mut self, tab_id: Option<&str>) -> Result<&mut Tab> {
        let id = tab_id
            .map(str::to_owned)
            .or_else(|| self.active.clone())
            .ok_or_else(|| anyhow::anyhow!("no active tab"))?;
        self.tabs
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or_else(|| anyhow::anyhow!("unknown tab_id"))
    }

    fn tab_infos(&self) -> Vec<TabInfo> {
        self.tabs
            .iter()
            .map(|t| TabInfo {
                id: t.id.clone(),
                title: t.title.clone(),
                url: t.url.clone(),
                active: Some(t.id.as_str()) == self.active.as_deref(),
                loading: t.loading,
                favicon: None,
                error: t.error.clone(),
            })
            .collect()
    }

    fn push_status(&self) {
        let _ = self.ctl.send(HostCtl::ScreenBrowser {
            action: "window_state".into(),
            window: None,
            prefs: None,
            url: None,
            tab_id: None,
            tabs: Some(self.tab_infos()),
            active_tab: self.active.clone(),
            window_open: true,
            reply: None,
        });
        let mut status = self.status.clone();
        status.tabs = self.tab_infos();
        status.active_tab = self.active.clone();
        status.window_open = true;
        status.blocked_nav = self.blocked_nav.clone();
        status.prefs.allowed_nav = self.nav_grants.borrow().always.clone();
        let _ = self.chrome.evaluate_script(&format!(
            "window.dispatchEvent(new CustomEvent('koma-screen-browser',{{detail:{}}}));",
            serde_json::to_string(&status).unwrap_or_else(|_| "null".into())
        ));
    }

    fn chrome_toast(&self, text: &str) {
        let payload = serde_json::json!({ "text": text, "kind": "error" });
        let _ = self.chrome.evaluate_script(&format!(
            "window.dispatchEvent(new CustomEvent('koma-screen-browser-toast',{{detail:{payload}}}));"
        ));
    }

    fn fail_navigate(&mut self, tab_id: Option<&str>, message: &str) {
        if let Ok(tab) = self.tab_mut(tab_id) {
            tab.loading = false;
            tab.error = Some(message.to_string());
        }
        self.chrome_toast(message);
    }

    /// Load a tab URL without letting wry's NSURL `unwrap` take down the process.
    fn load_tab(&mut self, tab_id: Option<&str>, url: &str, blank: bool) -> Result<()> {
        let tab = self.tab_mut(tab_id)?;
        tab.url = if blank {
            "about:blank".into()
        } else {
            url.to_string()
        };
        tab.title = if blank {
            "New Tab".into()
        } else {
            url.to_string()
        };
        tab.loading = !blank;
        tab.error = None;
        if blank {
            tab.webview.load_html(&welcome_html())?;
            return Ok(());
        }
        if !webview_can_load_url(url) {
            bail!("invalid url");
        }
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        pin_page_opaque(&tab.webview, false);
        // WebKitGTK ignores load_uri while a previous load (Google redirects)
        // is stuck in started. Stop first so omnibox / other pages work.
        #[cfg(target_os = "linux")]
        {
            use webkit2gtk::WebViewExt;
            use wry::WebViewExtUnix;
            tab.webview.webview().stop_loading();
        }
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tab.webview.load_url(url))) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(anyhow::anyhow!("{e}")),
            Err(_) => bail!("webview rejected url"),
        }
    }

    pub fn handle_chrome(
        &mut self,
        action: &str,
        url: Option<String>,
        tab_id: Option<String>,
    ) -> Result<()> {
        match action {
            "open" => {
                self.open_tab(url, true)?;
            }
            "popup" => {
                // Page `window.open` — honor prefs + drop ad/tracker noise so Gmail
                // (etc.) does not spawn a second tab for doubleclick beacons.
                let u = url.unwrap_or_default();
                if crate::app::runtime::screen_browser::allow_popup_url(
                    &u,
                    &self.status.prefs.popup_policy,
                ) {
                    // Login/OAuth must be foreground: GIS waits on that window.
                    // Background tab = original page looks frozen, and on Linux
                    // WebKitGTK then ignores later loads.
                    let login = {
                        let l = u.to_ascii_lowercase();
                        l.contains("accounts.google.")
                            || l.contains("/oauth")
                            || l.contains("/signin")
                            || l.contains("/authorize")
                            || l.contains("login.")
                    };
                    self.open_tab(Some(u), login)?;
                }
            }
            "close" => {
                let id = tab_id.or_else(|| self.active.clone());
                if let Some(id) = id {
                    self.tabs.retain(|t| t.id != id);
                    if self.active.as_deref() == Some(id.as_str()) {
                        self.active = self.tabs.last().map(|t| t.id.clone());
                    }
                    if self.tabs.is_empty() {
                        self.open_tab(Some("about:blank".into()), true)?;
                    } else {
                        self.layout()?;
                        self.push_status();
                    }
                }
            }
            "select" => {
                if let Some(id) = tab_id {
                    if self.tabs.iter().any(|t| t.id == id) {
                        self.active = Some(id);
                        self.layout()?;
                        self.push_status();
                    }
                }
            }
            "navigate" => {
                let raw = url.ok_or_else(|| anyhow::anyhow!("url required"))?;
                let u = match resolve_omnibox_input(&raw) {
                    Ok(u) => u,
                    Err(e) => {
                        self.chrome_toast(&e.to_string());
                        return Ok(());
                    }
                };
                if let Err(e) = validate_content_url(&u) {
                    self.chrome_toast(&e.to_string());
                    return Ok(());
                }
                let blank = crate::app::runtime::screen_browser::is_blank_url(&u);
                let load = self.load_tab(tab_id.as_deref(), &u, blank);
                if let Err(e) = load {
                    // wry panics on garbage NSURL — treat as "not a URL" → DDG.
                    let search = duckduckgo_search_url(&raw);
                    if search != u {
                        if let Err(e2) = self.load_tab(tab_id.as_deref(), &search, false) {
                            self.fail_navigate(tab_id.as_deref(), &e2.to_string());
                        }
                    } else {
                        self.fail_navigate(tab_id.as_deref(), &e.to_string());
                    }
                }
                let _ = self.layout();
                self.push_status();
            }
            "reload" => {
                let u = {
                    let tab = self.tab_mut(tab_id.as_deref())?;
                    tab.url.clone()
                };
                let blank = crate::app::runtime::screen_browser::is_blank_url(&u);
                if let Err(e) = self.load_tab(tab_id.as_deref(), &u, blank) {
                    self.fail_navigate(tab_id.as_deref(), &e.to_string());
                } else if blank {
                    if let Ok(tab) = self.tab_mut(tab_id.as_deref()) {
                        tab.loading = false;
                        tab.title = "New Tab".into();
                    }
                }
                self.push_status();
            }
            "stop" => {
                // best-effort via JS
                let tab = self.tab_mut(tab_id.as_deref())?;
                let _ = tab.webview.evaluate_script("window.stop()");
                tab.loading = false;
                self.push_status();
            }
            "back" => {
                let tab = self.tab_mut(tab_id.as_deref())?;
                tab.loading = true;
                let _ = tab.webview.evaluate_script("history.back()");
                self.push_status();
            }
            "forward" => {
                let tab = self.tab_mut(tab_id.as_deref())?;
                tab.loading = true;
                let _ = tab.webview.evaluate_script("history.forward()");
                self.push_status();
            }
            "page_started" => {
                if let Some(id) = tab_id {
                    if let Some(t) = self.tabs.iter_mut().find(|t| t.id == id) {
                        if let Some(u) = url.filter(|u| allow_content_url(u)) {
                            // Keep about:blank as the canonical empty URL even if the
                            // engine reports a data:/file: shell from load_html.
                            if crate::app::runtime::screen_browser::is_blank_url(&u)
                                || crate::app::runtime::screen_browser::is_blank_url(&t.url)
                            {
                                // Don't overwrite a real page URL with engine noise while
                                // staying on welcome; only accept blank → blank.
                                if crate::app::runtime::screen_browser::is_blank_url(&t.url) {
                                    t.url = "about:blank".into();
                                    t.loading = false;
                                } else if !crate::app::runtime::screen_browser::is_blank_url(&u) {
                                    t.url = u;
                                    t.loading = true;
                                }
                            } else {
                                t.url = u;
                                t.loading = true;
                            }
                        } else if !crate::app::runtime::screen_browser::is_blank_url(&t.url) {
                            t.loading = true;
                        }
                        t.error = None;
                    }
                    // Don't layout() here: remapping the page webview on every
                    // start/finish blanks WKWebView/WebView2 and freezes WebKitGTK
                    // during redirect storms. Pin chrome only.
                    #[cfg(target_os = "linux")]
                    linux_pin_chrome(&self.chrome);
                    #[cfg(target_os = "macos")]
                    {
                        if let Some(t) = self.tabs.iter().find(|t| t.id == id) {
                            pin_page_opaque(&t.webview, false);
                        }
                        macos_pin_chrome(&self.chrome, &self.tabs, self.active.as_deref());
                    }
                    #[cfg(target_os = "windows")]
                    if let Some(t) = self.tabs.iter().find(|t| t.id == id) {
                        pin_page_opaque(&t.webview, false);
                    }
                    self.push_status();
                }
            }
            "page_finished" => {
                if let Some(id) = tab_id {
                    if let Some(t) = self.tabs.iter_mut().find(|t| t.id == id) {
                        t.loading = false;
                        if let Some(u) = url.filter(|u| allow_content_url(u)) {
                            if crate::app::runtime::screen_browser::is_blank_url(&u) {
                                t.url = "about:blank".into();
                                if t.title.trim().is_empty() {
                                    t.title = "New Tab".into();
                                }
                            } else if !crate::app::runtime::screen_browser::is_blank_url(&t.url)
                                || !u.is_empty()
                            {
                                // Prefer real navigations; ignore engine blank after welcome html.
                                if !crate::app::runtime::screen_browser::is_blank_url(&u) {
                                    t.url = u;
                                }
                            }
                        }
                    }
                    #[cfg(target_os = "linux")]
                    linux_pin_chrome(&self.chrome);
                    #[cfg(target_os = "macos")]
                    {
                        if let Some(t) = self.tabs.iter().find(|t| t.id == id) {
                            pin_page_opaque(&t.webview, false);
                        }
                        macos_pin_chrome(&self.chrome, &self.tabs, self.active.as_deref());
                    }
                    #[cfg(target_os = "windows")]
                    if let Some(t) = self.tabs.iter().find(|t| t.id == id) {
                        pin_page_opaque(&t.webview, false);
                    }
                    self.push_status();
                }
            }
            "page_title" => {
                if let Some(id) = tab_id {
                    if let Some(t) = self.tabs.iter_mut().find(|t| t.id == id) {
                        if let Some(title) = url.filter(|s| !s.trim().is_empty()) {
                            // Welcome html title is "New Tab"; keep that for blank shells.
                            if crate::app::runtime::screen_browser::is_blank_url(&t.url) {
                                t.title = "New Tab".into();
                            } else {
                                t.title = title;
                            }
                        }
                    }
                    self.push_status();
                }
            }
            "chrome_metrics" => {
                // `url` carries the measured height string from the IPC bridge.
                if let Some(h) = url.and_then(|s| s.parse::<f64>().ok()) {
                    // Clamp: title-tab strip + toolbar; never most of the window.
                    let next = h.clamp(64.0, 200.0);
                    if (next - self.chrome_h).abs() >= 0.5 {
                        self.chrome_h = next;
                        let _ = self.layout();
                    }
                }
                if !self.chrome_shown {
                    self.chrome_shown = true;
                    self.window.set_visible(true);
                    let _ = self.layout();
                }
                self.push_status();
            }
            "chrome_ready" => {
                if !self.chrome_shown {
                    self.chrome_shown = true;
                    self.window.set_visible(true);
                    let _ = self.layout();
                }
                self.push_status();
            }
            // Manual chrome screenshots → stage into chat composer as [Image #N].
            "shot" | "shot_full" => {
                let full_page = action == "shot_full";
                let path = self.capture_to_composer(tab_id.as_deref(), full_page)?;
                let _ = self.ctl.send(HostCtl::ScreenBrowser {
                    action: "composer_attach".into(),
                    window: None,
                    prefs: None,
                    url: Some(path),
                    tab_id: None,
                    tabs: None,
                    active_tab: None,
                    window_open: true,
                    reply: None,
                });
            }
            // Frameless window chrome (same contract as main koma titlebar).
            "win_drag" => {
                let _ = self.window.drag_window();
            }
            "win_min" => {
                self.window.set_minimized(true);
            }
            "win_max" => {
                self.window.set_maximized(!self.window.is_maximized());
            }
            // win_close is handled in the GUI event loop (drops BrowserWindow + stop).
            "pause" | "resume" | "take_over" | "stop_browser" | "continue_human" => {
                let mapped = match action {
                    "stop_browser" => "stop",
                    other => other,
                };
                let _ = self.ctl.send(HostCtl::ScreenBrowser {
                    action: mapped.into(),
                    window: None,
                    prefs: None,
                    url: None,
                    tab_id: None,
                    tabs: None,
                    active_tab: None,
                    window_open: true,
                    reply: None,
                });
            }
            "nav_blocked" => {
                let u = url.unwrap_or_default();
                if u.is_empty() {
                    return Ok(());
                }
                let can_allow = !nav_never_allow(&u);
                let key = nav_remember_key(&u).unwrap_or_default();
                if can_allow && key.is_empty() {
                    self.chrome_toast(&format!("Blocked navigation: {u}"));
                    return Ok(());
                }
                self.blocked_nav = Some(BlockedNav {
                    url: u.clone(),
                    key: key.clone(),
                    label: if can_allow {
                        nav_grant_label(&key)
                    } else {
                        u.clone()
                    },
                    tab_id,
                    can_allow,
                });
                self.push_status();
            }
            "nav_allow" | "nav_always" => {
                let remember = action == "nav_always";
                let pending = self.blocked_nav.take();
                let Some(pending) = pending else {
                    return Ok(());
                };
                if !pending.can_allow || pending.key.is_empty() {
                    self.push_status();
                    return Ok(());
                }
                {
                    let mut g = self.nav_grants.borrow_mut();
                    g.once.insert(pending.key.clone());
                    if remember && !g.always.iter().any(|k| k == &pending.key) {
                        g.always.push(pending.key.clone());
                        save_nav_grants(&self.profile_dir, &g.always);
                    }
                    if remember {
                        self.status.prefs.allowed_nav = g.always.clone();
                    }
                }
                if remember {
                    let _ = self.ctl.send(HostCtl::ScreenBrowser {
                        action: "set_prefs".into(),
                        window: None,
                        prefs: Some(self.status.prefs.clone()),
                        url: None,
                        tab_id: None,
                        tabs: None,
                        active_tab: None,
                        window_open: true,
                        reply: None,
                    });
                }
                let tab = pending.tab_id.clone();
                let _ = self.handle_chrome("reload", None, tab);
            }
            "nav_deny" => {
                self.blocked_nav = None;
                self.push_status();
            }
            "nav_forget" => {
                let key = url.unwrap_or_default();
                if key.is_empty() {
                    return Ok(());
                }
                {
                    let mut g = self.nav_grants.borrow_mut();
                    g.always.retain(|k| k != &key);
                    g.once.remove(&key);
                    save_nav_grants(&self.profile_dir, &g.always);
                    self.status.prefs.allowed_nav = g.always.clone();
                }
                if self.blocked_nav.as_ref().map(|b| b.key.as_str()) == Some(key.as_str()) {
                    self.blocked_nav = None;
                }
                let _ = self.ctl.send(HostCtl::ScreenBrowser {
                    action: "set_prefs".into(),
                    window: None,
                    prefs: Some(self.status.prefs.clone()),
                    url: None,
                    tab_id: None,
                    tabs: None,
                    active_tab: None,
                    window_open: true,
                    reply: None,
                });
                self.push_status();
            }
            _ => {}
        }
        Ok(())
    }

    pub fn execute(&mut self, request: Request) -> Reply {
        let mut reply = Reply {
            id: request.id.clone(),
            session: request.session.clone(),
            generation: request.generation.clone(),
            ..Default::default()
        };
        let result = (|| -> Result<()> {
            match &request.operation {
                Operation::Tabs {
                    action,
                    url,
                    tab_id,
                } => {
                    match action.as_str() {
                        "list" => {}
                        "open" => {
                            self.open_tab(url.clone(), true)?;
                        }
                        "close" => self.handle_chrome("close", None, tab_id.clone())?,
                        "select" => self.handle_chrome("select", None, tab_id.clone())?,
                        "navigate" => {
                            self.handle_chrome("navigate", url.clone(), tab_id.clone())?
                        }
                        "reload" => self.handle_chrome("reload", None, tab_id.clone())?,
                        "stop" => self.handle_chrome("stop", None, tab_id.clone())?,
                        "back" => self.handle_chrome("back", None, tab_id.clone())?,
                        "forward" => self.handle_chrome("forward", None, tab_id.clone())?,
                        other => bail!("unknown tabs action {other}"),
                    }
                    reply.tabs = self.tab_infos();
                    reply.completed = 1;
                }
                Operation::Observe { tab_id, full_page } => {
                    let obs = self.observe(tab_id.as_deref(), *full_page)?;
                    reply.observation = Some(obs.0);
                    reply.png = obs.1;
                    reply.tabs = self.tab_infos();
                    reply.completed = 1;
                }
                Operation::Act {
                    observation: _,
                    tab_id,
                    actions,
                    observe,
                } => {
                    let completed = self.run_actions(tab_id.as_deref(), actions)?;
                    reply.completed = completed;
                    if *observe {
                        let obs = self.observe(tab_id.as_deref(), false)?;
                        reply.observation = Some(obs.0);
                        reply.png = obs.1;
                    }
                    reply.tabs = self.tab_infos();
                }
                Operation::Read {
                    tab_id,
                    mode,
                    selector,
                    max_chars,
                } => {
                    let text = self.read_page(
                        tab_id.as_deref(),
                        mode,
                        selector.as_deref(),
                        max_chars.unwrap_or(crate::app::runtime::screen_browser::READ_MAX_CHARS),
                    )?;
                    reply.text = Some(text);
                    reply.tabs = self.tab_infos();
                    reply.completed = 1;
                }
                Operation::Eval {
                    tab_id,
                    script,
                    timeout_ms: _,
                } => {
                    let tab = self.tab_mut(tab_id.as_deref())?;
                    // Return value via wry's JSON callback (works macOS/Windows/Linux).
                    let result =
                        capture::js_eval(&tab.webview, &format!("(function(){{\n{script}\n}})()"))?;
                    if result.contains("\"__koma_err\"") {
                        reply.error = Some(result);
                    } else {
                        reply.text = Some(result);
                        reply.completed = 1;
                    }
                }
                Operation::WaitHuman { .. } => {
                    // handled daemon-side
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            reply.error = Some(e.to_string());
        }
        reply.tabs = if reply.tabs.is_empty() {
            self.tab_infos()
        } else {
            reply.tabs
        };
        reply
    }

    fn observe(&mut self, tab_id: Option<&str>, full_page: bool) -> Result<(Observation, Vec<u8>)> {
        {
            let tab = self.tab_mut(tab_id)?;
            if let Ok(u) = tab.webview.url() {
                if allow_content_url(&u) {
                    tab.url = u;
                }
            }
            let _ = tab.webview.evaluate_script(
                "try { window.__komaSbTitle = document.title || ''; } catch (e) {}",
            );
        }
        let chrome_h = self.chrome_h;
        let (tab_id_s, url, title) = {
            let tab = self.tab_mut(tab_id)?;
            (tab.id.clone(), tab.url.clone(), tab.title.clone())
        };
        // Engine snapshot of the page (GoFullPage-style). Never `screencapture -R`:
        // that grabs the desktop under the rect, so overlapping koma appears in the PNG.
        let (png, quality) = {
            let tab = self.tab_mut(Some(&tab_id_s))?;
            match capture::snapshot_tab(&tab.webview, full_page) {
                Ok(p) if !p.is_empty() => (p, "webview"),
                other => {
                    let why = match &other {
                        Ok(_) => "empty webview snapshot".to_string(),
                        Err(e) => e.to_string(),
                    };
                    #[cfg(target_os = "macos")]
                    {
                        match capture::snapshot_own_window(&self.window, chrome_h) {
                            Ok(p) if !p.is_empty() => (p, "window"),
                            _ => bail!("page screenshot unavailable ({why})"),
                        }
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        let _ = chrome_h;
                        bail!("page screenshot unavailable ({why})")
                    }
                }
            }
        };
        if png.is_empty() {
            bail!("page screenshot unavailable");
        }
        let (w, h) = image_size(&png).unwrap_or((1280, 720));
        let (tw, th) = crate::app::runtime::screen_browser::capture_size(w, h);
        let png = if tw != w || th != h {
            resize_png(&png, tw, th).unwrap_or(png)
        } else {
            png
        };
        let (fw, fh) = image_size(&png).unwrap_or((tw, th));
        let captured_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let scale = self.window.scale_factor();
        let obs = Observation {
            id: uuid::Uuid::new_v4().to_string(),
            session: self.status.session.clone(),
            generation: self.status.generation.clone(),
            tab_id: tab_id_s,
            url,
            title,
            transform: Transform {
                width: fw,
                height: fh,
                css_width: (fw as f64) / scale.max(0.01),
                css_height: (fh as f64) / scale.max(0.01),
            },
            viewport: Viewport {
                width: fw,
                height: fh,
                device_pixel_ratio: scale,
            },
            captured_ms,
            image_path: String::new(),
            capture_quality: quality.into(),
        };
        Ok((obs, png))
    }

    /// Manual chrome button: capture page PNG, save under `.screenshoot/`, return
    /// absolute path so the host can `Paste` it into the chat composer as an attachment.
    fn capture_to_composer(&mut self, tab_id: Option<&str>, full_page: bool) -> Result<String> {
        let (obs, png) = self.observe(tab_id, full_page)?;
        let kind = if full_page { "whole" } else { "ss" };
        let host = obs
            .url
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("page")
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .take(40)
            .collect::<String>();
        let host = if host.is_empty() { "page".into() } else { host };
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let stem = format!("sb_{kind}_{host}_{ts}");
        let dir = self.workdir.join(".screenshoot");
        std::fs::create_dir_all(&dir).context("create .screenshoot")?;
        let path = dir.join(format!("{stem}.png"));
        std::fs::write(&path, &png).context("write screenshot png")?;
        let _ =
            crate::model::screenshot_catalog::register_screenshot(&self.workdir, &stem, &obs.url);
        Ok(path.to_string_lossy().into_owned())
    }

    fn run_actions(&mut self, tab_id: Option<&str>, actions: &[Action]) -> Result<usize> {
        let mut completed = 0usize;
        for action in actions {
            if let Action::Wait {
                selector,
                url_contains,
                timeout_ms,
            } = action
            {
                let ms = timeout_ms.unwrap_or(5000);
                let deadline = std::time::Instant::now() + std::time::Duration::from_millis(ms);
                if wait_is_delay(selector, url_contains) {
                    // Delay: keep pumping the GUI loop. Never bail.
                    while std::time::Instant::now() < deadline {
                        capture::pump_brief();
                    }
                    completed += 1;
                    continue;
                }
                let sel = selector.clone().unwrap_or_default();
                let urlc = url_contains.clone().unwrap_or_default();
                loop {
                    let expr = format!(
                        "(function(){{ const sel={sel:?}; const urlc={urlc:?}; \
                         if(sel && document.querySelector(sel)) return true; \
                         if(urlc && location.href.includes(urlc)) return true; \
                         return false; }})()"
                    );
                    let tab = self.tab_mut(tab_id)?;
                    let raw =
                        capture::js_eval(&tab.webview, &expr).unwrap_or_else(|_| "false".into());
                    if raw.contains("true") {
                        break;
                    }
                    if std::time::Instant::now() > deadline {
                        bail!("wait timed out after {ms}ms");
                    }
                    capture::pump_brief();
                }
                completed += 1;
                continue;
            }
            let script = action_js(action)?;
            // Run as expression that returns true so js_eval confirms completion.
            let expr = format!("(function(){{ {script}; return true; }})()");
            let raw = {
                let tab = self.tab_mut(tab_id)?;
                capture::js_eval(&tab.webview, &expr).or_else(|_| -> Result<String> {
                    tab.webview.evaluate_script(&script)?;
                    Ok("true".into())
                })?
            };
            // js_eval catch returns {__koma_err} as a successful JSON string.
            if raw.contains("\"__koma_err\"") {
                bail!("act js: {raw}");
            }
            completed += 1;
            // Brief settle so layout/scroll finishes before the next act or observe.
            capture::pump_brief();
        }
        Ok(completed)
    }

    fn read_page(
        &mut self,
        tab_id: Option<&str>,
        mode: &str,
        selector: Option<&str>,
        max_chars: usize,
    ) -> Result<String> {
        let sel = selector.unwrap_or("body");
        let expression = match mode {
            "text" | "markdown" => format!(
                r#"(function(){{const el=document.querySelector({sel:?})||document.body; const t=(el&&el.innerText)||''; return t.slice(0,{max_chars});}})()"#
            ),
            "html" => format!(
                r#"(function(){{const el=document.querySelector({sel:?})||document.documentElement; const t=(el&&el.outerHTML)||''; return t.slice(0,{max_chars});}})()"#
            ),
            "query" => format!(
                r#"(function(){{const el=document.querySelector({sel:?}); if(!el) return {{found:false}}; return {{found:true,tag:el.tagName,text:(el.innerText||'').slice(0,500),value:el.value||null,href:el.href||null}};}})()"#
            ),
            "actions_inventory" => {
                r#"(function(){const out=[]; document.querySelectorAll('a[href],button,input,textarea,select,[role=button]').forEach((el,i)=>{if(i>=80)return; out.push({tag:el.tagName,text:(el.innerText||el.value||'').trim().slice(0,80),href:el.href||null,type:el.type||null});}); return out;})()"#
                    .to_string()
            }
            other => bail!("unknown read mode {other}"),
        };
        let tab = self.tab_mut(tab_id)?;
        let raw = capture::js_eval(&tab.webview, &expression)?;
        // wry returns a JSON string; unwrap string literals for text modes.
        if matches!(mode, "text" | "markdown" | "html") {
            if let Ok(serde_json::Value::String(s)) =
                serde_json::from_str::<serde_json::Value>(&raw)
            {
                return Ok(s);
            }
        }
        Ok(raw)
    }

    pub fn clear_profile(&mut self) -> Result<()> {
        self.tabs.clear();
        self.active = None;
        // Best-effort wipe; new context requires rebuild — reopen blank tab.
        let _ = std::fs::remove_dir_all(&self.profile_dir);
        std::fs::create_dir_all(&self.profile_dir)?;
        self.open_tab(Some("about:blank".into()), true)?;
        Ok(())
    }
}

fn image_size(png: &[u8]) -> Option<(u32, u32)> {
    let img = image::load_from_memory(png).ok()?;
    Some((img.width(), img.height()))
}

fn resize_png(png: &[u8], w: u32, h: u32) -> Result<Vec<u8>> {
    let img =
        image::load_from_memory(png)?.resize_exact(w, h, image::imageops::FilterType::Triangle);
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)?;
    Ok(out)
}

mod capture {
    //! Chrome-style **visible tab** screenshots of the page content webview
    //! (not koma chrome, not OS desktop). Plus a shared JS eval helper used by
    //! observe/read/eval/act verification across macOS / Windows / Linux.
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    const WAIT: Duration = Duration::from_secs(5);

    /// Chrome-style visible-tab capture of the page content webview only.
    pub fn snapshot_tab(webview: &WebView, full_page: bool) -> Result<Vec<u8>> {
        #[cfg(target_os = "macos")]
        {
            mac::snapshot_tab(webview, full_page)
        }
        #[cfg(target_os = "windows")]
        {
            win::snapshot_tab(webview, full_page)
        }
        #[cfg(target_os = "linux")]
        {
            linux::snapshot_tab(webview, full_page)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            let _ = (webview, full_page);
            bail!("page screenshot unsupported on this OS")
        }
    }

    /// Fallback: this window's backing store (not a desktop region).
    /// `screencapture -R` composites whatever is on screen — overlapping koma
    /// would appear in the PNG. `-l <windowNumber>` is this NSWindow only.
    #[cfg(target_os = "macos")]
    pub fn snapshot_own_window(window: &Window, chrome_h_logical: f64) -> Result<Vec<u8>> {
        use objc2::msg_send;
        use objc2::runtime::AnyObject;
        use tao::platform::macos::WindowExtMacOS;

        let ns = window.ns_window() as *mut AnyObject;
        if ns.is_null() {
            bail!("no NSWindow");
        }
        let id: isize = unsafe { msg_send![ns, windowNumber] };
        if id <= 0 {
            bail!("NSWindow windowNumber is 0");
        }
        let path = std::env::temp_dir().join(format!("koma-sb-{}.png", uuid::Uuid::new_v4()));
        let status = std::process::Command::new("screencapture")
            .args([
                "-x",
                "-o",
                "-l",
                &id.to_string(),
                path.to_str().unwrap_or("/tmp/koma-sb.png"),
            ])
            .status()
            .context("screencapture -l")?;
        if !status.success() {
            bail!("screencapture -l failed");
        }
        let bytes = std::fs::read(&path)?;
        let _ = std::fs::remove_file(&path);
        if bytes.is_empty() {
            bail!("empty window capture");
        }
        let scale = window.scale_factor();
        let top = (chrome_h_logical * scale).round().max(0.0) as u32;
        crop_top(&bytes, top)
    }

    #[cfg(target_os = "macos")]
    fn crop_top(png: &[u8], top_px: u32) -> Result<Vec<u8>> {
        if top_px == 0 {
            return Ok(png.to_vec());
        }
        let img = image::load_from_memory(png)?;
        if top_px >= img.height() {
            bail!("chrome crop exceeds capture");
        }
        let cropped = img.crop_imm(0, top_px, img.width(), img.height() - top_px);
        let mut out = Vec::new();
        cropped.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)?;
        Ok(out)
    }

    /// Evaluate JS and return the JSON-serialized result (wry portable path).
    /// Used by read / eval so agents can get real page data on every OS.
    pub fn js_eval(webview: &WebView, expression: &str) -> Result<String> {
        let (tx, rx) = mpsc::channel::<String>();
        // wry serializes the completion value as JSON text.
        let js = format!(
            "(function(){{ try {{ return ({expression}); }} catch (e) {{ return {{__koma_err: String(e)}}; }} }})()"
        );
        webview.evaluate_script_with_callback(&js, move |value| {
            let _ = tx.send(value);
        })?;
        wait_recv(rx).map_err(|e| anyhow::anyhow!("js eval: {e}"))
    }

    #[cfg(target_os = "macos")]
    fn page_content_size(webview: &WebView) -> Option<(f64, f64)> {
        let raw = js_eval(
            webview,
            "({w:Math.max(document.documentElement.scrollWidth||0, (document.body&&document.body.scrollWidth)||0), h:Math.max(document.documentElement.scrollHeight||0, (document.body&&document.body.scrollHeight)||0)})",
        )
        .ok()?;
        let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
        let w = v.get("w")?.as_f64()?;
        let h = v.get("h")?.as_f64()?;
        (w >= 8.0 && h >= 8.0).then_some((w, h))
    }

    fn wait_recv(rx: mpsc::Receiver<String>) -> Result<String, String> {
        let deadline = Instant::now() + WAIT;
        loop {
            match rx.try_recv() {
                Ok(v) => return Ok(v),
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err("callback dropped".into());
                }
                Err(mpsc::TryRecvError::Empty) => {
                    if Instant::now() > deadline {
                        return Err("timed out".into());
                    }
                    pump_main_events();
                }
            }
        }
    }

    fn pump_main_events() {
        #[cfg(target_os = "macos")]
        {
            use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSRunLoop};
            let run_loop = NSRunLoop::currentRunLoop();
            let until = NSDate::dateWithTimeIntervalSinceNow(0.02);
            let _ = unsafe { run_loop.runMode_beforeDate(NSDefaultRunLoopMode, &until) };
        }
        #[cfg(target_os = "linux")]
        {
            // wry builds on gtk — drain pending events so JS callbacks fire.
            while gtk::events_pending() {
                let _ = gtk::main_iteration_do(false);
            }
            std::thread::sleep(Duration::from_millis(5));
            return;
        }
        #[cfg(target_os = "windows")]
        {
            // Pump the Win32 queue so WebView2 completion handlers run.
            unsafe {
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
                };
                let mut msg: MSG = std::mem::zeroed();
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(Duration::from_millis(5));
            return;
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Short main-thread pump used between act steps.
    pub fn pump_brief() {
        for _ in 0..3 {
            pump_main_events();
        }
        std::thread::sleep(Duration::from_millis(30));
    }

    #[cfg(target_os = "macos")]
    mod mac {
        use super::*;
        use block2::RcBlock;
        use objc2::MainThreadMarker;
        use objc2_app_kit::NSImage;
        use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSError, NSRunLoop};
        use objc2_web_kit::{WKSnapshotConfiguration, WKWebView};
        use std::cell::RefCell;
        use std::rc::Rc;
        use wry::WebViewExtMacOS;

        type SnapshotSlot = Rc<RefCell<Option<Result<Vec<u8>, String>>>>;

        fn snapshot_error_msg(error: *mut NSError) -> String {
            if error.is_null() {
                return "WKWebView takeSnapshot failed".into();
            }
            match unsafe { objc2::rc::Retained::retain(error) } {
                Some(err) => {
                    format!(
                        "WKWebView takeSnapshot failed: {}",
                        err.localizedDescription()
                    )
                }
                None => "WKWebView takeSnapshot failed".into(),
            }
        }

        fn snapshot_once(
            as_wk: &WKWebView,
            mtm: MainThreadMarker,
            rect: Option<objc2_foundation::NSRect>,
            after_updates: bool,
        ) -> Result<Vec<u8>> {
            use objc2::rc::Retained;

            let slot: SnapshotSlot = Rc::new(RefCell::new(None));
            let out = Rc::clone(&slot);
            let config = unsafe { WKSnapshotConfiguration::new(mtm) };
            unsafe {
                config.setAfterScreenUpdates(after_updates);
                if let Some(r) = rect {
                    config.setRect(r);
                }
            }
            let block = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                let res = if !error.is_null() {
                    Err(snapshot_error_msg(error))
                } else {
                    match unsafe { Retained::retain(image) } {
                        Some(img) => nsimage_to_png(&img).map_err(|e| e.to_string()),
                        None => Err("WKWebView takeSnapshot returned no image".into()),
                    }
                };
                *out.borrow_mut() = Some(res);
            });
            unsafe {
                as_wk.takeSnapshotWithConfiguration_completionHandler(Some(&config), &block);
            }
            let deadline = Instant::now() + WAIT;
            while slot.borrow().is_none() {
                if Instant::now() > deadline {
                    bail!("WKWebView takeSnapshot timed out");
                }
                let run_loop = NSRunLoop::currentRunLoop();
                let until = NSDate::dateWithTimeIntervalSinceNow(0.02);
                let _ = unsafe { run_loop.runMode_beforeDate(NSDefaultRunLoopMode, &until) };
            }
            let taken = slot.borrow_mut().take();
            match taken {
                Some(Ok(png)) if !png.is_empty() => Ok(png),
                Some(Ok(_)) => bail!("empty WKWebView snapshot"),
                Some(Err(e)) => bail!(e),
                None => bail!("WKWebView takeSnapshot produced no result"),
            }
        }

        pub fn snapshot_tab(webview: &WebView, full_page: bool) -> Result<Vec<u8>> {
            let mtm = MainThreadMarker::new().ok_or_else(|| {
                anyhow::anyhow!("page screenshot must run on the GUI main thread")
            })?;
            let wk = webview.webview();
            let as_wk: &WKWebView = wk.as_ref();
            let bounds = as_wk.bounds();
            if bounds.size.width < 2.0 || bounds.size.height < 2.0 {
                bail!(
                    "webview has no layout ({:.0}x{:.0})",
                    bounds.size.width,
                    bounds.size.height
                );
            }
            let mut viewport = bounds;
            viewport.origin.x = 0.0;
            viewport.origin.y = 0.0;
            let mut custom = viewport;
            let mut used_custom_rect = false;
            if full_page {
                if let Some((cw, ch)) = super::page_content_size(webview) {
                    let nw = custom.size.width.max(cw);
                    let nh = custom.size.height.max(ch).min(8192.0);
                    if (nw - custom.size.width).abs() > 0.5 || (nh - custom.size.height).abs() > 0.5
                    {
                        custom.size.width = nw;
                        custom.size.height = nh;
                        used_custom_rect = true;
                    }
                }
            }

            let mut last = snapshot_once(as_wk, mtm, Some(custom), true);
            if last.is_ok() {
                return last;
            }
            super::pump_brief();
            last = snapshot_once(as_wk, mtm, Some(custom), true);
            if last.is_ok() {
                return last;
            }
            if used_custom_rect {
                last = snapshot_once(as_wk, mtm, Some(viewport), true);
                if last.is_ok() {
                    return last;
                }
            }
            let fallback_rect = if used_custom_rect { viewport } else { custom };
            last = snapshot_once(as_wk, mtm, Some(fallback_rect), false);
            last
        }

        fn nsimage_to_png(image: &NSImage) -> Result<Vec<u8>> {
            // TIFF → image crate → PNG (avoids NSBitmapImageRep feature maze).
            let tiff = image
                .TIFFRepresentation()
                .ok_or_else(|| anyhow::anyhow!("NSImage has no TIFF representation"))?;
            let bytes = tiff.to_vec();
            let img = image::load_from_memory(&bytes).context("decode WKWebView TIFF snapshot")?;
            let mut out = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)?;
            Ok(out)
        }
    }

    #[cfg(target_os = "windows")]
    mod win {
        use super::*;
        use std::fs;
        use std::sync::{Arc, Mutex};
        use wry::WebViewExtWindows;

        pub fn snapshot_tab(webview: &WebView, full_page: bool) -> Result<Vec<u8>> {
            if full_page {
                if let Ok(png) = snapshot_cdp(webview) {
                    if !png.is_empty() {
                        return Ok(png);
                    }
                }
            }
            snapshot_preview(webview)
        }

        /// GoFullPage-style: Chromium CDP `Page.captureScreenshot` from the renderer.
        fn snapshot_cdp(webview: &WebView) -> Result<Vec<u8>> {
            use base64::Engine;
            use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
            use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2;
            use windows::core::PCWSTR;

            let controller = webview.controller();
            let core: ICoreWebView2 = unsafe { controller.CoreWebView2() }
                .map_err(|e| anyhow::anyhow!("CoreWebView2: {e}"))?;

            let method: Vec<u16> = "Page.captureScreenshot\0".encode_utf16().collect();
            let params: Vec<u16> = "{\"format\":\"png\",\"captureBeyondViewport\":true}\0"
                .encode_utf16()
                .collect();

            let done = Arc::new(Mutex::new(None::<Result<String, String>>));
            let flag = Arc::clone(&done);
            let handler =
                CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |err, json| {
                    let res = if err.is_ok() {
                        Ok(json)
                    } else {
                        Err(format!("CDP captureScreenshot failed: {err:?}"))
                    };
                    *flag.lock().unwrap_or_else(|e| e.into_inner()) = Some(res);
                    Ok(())
                }));

            unsafe {
                core.CallDevToolsProtocolMethod(
                    PCWSTR(method.as_ptr()),
                    PCWSTR(params.as_ptr()),
                    &handler,
                )
            }
            .map_err(|e| anyhow::anyhow!("CallDevToolsProtocolMethod: {e}"))?;

            let deadline = Instant::now() + WAIT;
            let json = loop {
                if let Some(res) = done.lock().unwrap_or_else(|e| e.into_inner()).take() {
                    break res.map_err(|e| anyhow::anyhow!(e))?;
                }
                if Instant::now() > deadline {
                    bail!("WebView2 CDP captureScreenshot timed out");
                }
                super::pump_main_events();
            };

            let v: serde_json::Value =
                serde_json::from_str(&json).context("parse CDP captureScreenshot JSON")?;
            let b64 = v
                .get("data")
                .and_then(|d| d.as_str())
                .context("CDP captureScreenshot missing data")?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .context("decode CDP PNG")?;
            if bytes.is_empty() {
                bail!("empty CDP screenshot");
            }
            Ok(bytes)
        }

        fn snapshot_preview(webview: &WebView) -> Result<Vec<u8>> {
            use webview2_com::CapturePreviewCompletedHandler;
            use webview2_com::Microsoft::Web::WebView2::Win32::{
                ICoreWebView2, COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
            };
            use windows::core::PCWSTR;
            use windows::Win32::System::Com::{
                IStream, STGC, STGM_CREATE, STGM_READWRITE, STGM_SHARE_EXCLUSIVE,
            };
            use windows::Win32::UI::Shell::SHCreateStreamOnFileEx;

            let controller = webview.controller();
            let core: ICoreWebView2 = unsafe { controller.CoreWebView2() }
                .map_err(|e| anyhow::anyhow!("CoreWebView2: {e}"))?;

            let path = std::env::temp_dir().join(format!("koma-sb-{}.png", uuid::Uuid::new_v4()));
            let wide: Vec<u16> = path
                .to_string_lossy()
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();

            let stream: IStream = unsafe {
                SHCreateStreamOnFileEx(
                    PCWSTR(wide.as_ptr()),
                    STGM_CREATE.0 | STGM_READWRITE.0 | STGM_SHARE_EXCLUSIVE.0,
                    0,
                    true,
                    None,
                )
            }
            .map_err(|e| anyhow::anyhow!("SHCreateStreamOnFileEx: {e}"))?;

            let done = Arc::new(Mutex::new(None::<Result<(), String>>));
            let flag = Arc::clone(&done);
            let handler = CapturePreviewCompletedHandler::create(Box::new(move |err| {
                let res = if err.is_ok() {
                    Ok(())
                } else {
                    Err(format!("CapturePreview failed: {err:?}"))
                };
                *flag.lock().unwrap_or_else(|e| e.into_inner()) = Some(res);
                Ok(())
            }));

            unsafe {
                core.CapturePreview(
                    COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                    &stream,
                    &handler,
                )
            }
            .map_err(|e| anyhow::anyhow!("CapturePreview: {e}"))?;

            let deadline = Instant::now() + WAIT;
            loop {
                if let Some(res) = done.lock().unwrap_or_else(|e| e.into_inner()).take() {
                    res.map_err(|e| anyhow::anyhow!(e))?;
                    break;
                }
                if Instant::now() > deadline {
                    bail!("WebView2 CapturePreview timed out");
                }
                super::pump_main_events();
            }

            let _ = unsafe { stream.Commit(STGC(0)) };
            let bytes = fs::read(&path)?;
            let _ = fs::remove_file(&path);
            if bytes.is_empty() {
                bail!("empty WebView2 capture");
            }
            Ok(bytes)
        }
    }

    #[cfg(target_os = "linux")]
    mod linux {
        use super::*;
        use wry::WebViewExtUnix;

        pub fn snapshot_tab(webview: &WebView, full_page: bool) -> Result<Vec<u8>> {
            let region = if full_page {
                webkit2gtk::SnapshotRegion::FullDocument
            } else {
                webkit2gtk::SnapshotRegion::Visible
            };
            match snapshot_region(webview, region) {
                Ok(png) if !png.is_empty() => Ok(png),
                Ok(_) | Err(_) if full_page => {
                    snapshot_region(webview, webkit2gtk::SnapshotRegion::Visible)
                }
                other => other,
            }
        }

        fn snapshot_region(
            webview: &WebView,
            region: webkit2gtk::SnapshotRegion,
        ) -> Result<Vec<u8>> {
            use webkit2gtk::WebViewExt;

            let wk = webview.webview();
            let (tx, rx) = mpsc::channel::<Result<Vec<u8>, String>>();

            wk.snapshot(
                region,
                webkit2gtk::SnapshotOptions::NONE,
                None::<&gtk::gio::Cancellable>,
                move |res| {
                    let out = match res {
                        Ok(surface) => {
                            let mut buf = Vec::new();
                            match surface.write_to_png(&mut buf) {
                                Ok(()) if !buf.is_empty() => Ok(buf),
                                Ok(()) => Err("empty cairo snapshot".into()),
                                Err(e) => Err(format!("cairo png: {e}")),
                            }
                        }
                        Err(e) => Err(format!("webkit snapshot: {e}")),
                    };
                    let _ = tx.send(out);
                },
            );

            let deadline = Instant::now() + WAIT;
            loop {
                match rx.try_recv() {
                    Ok(Ok(png)) if !png.is_empty() => return Ok(png),
                    Ok(Ok(_)) => bail!("empty webkit snapshot"),
                    Ok(Err(e)) => bail!(e),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        bail!("webkit snapshot callback dropped")
                    }
                    Err(mpsc::TryRecvError::Empty) => {
                        if Instant::now() > deadline {
                            bail!("webkit snapshot timed out");
                        }
                        super::pump_main_events();
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod page_script_tests {
    use super::page_init_script;

    #[test]
    fn page_init_script_does_not_clip_third_party_pages() {
        let s = page_init_script();
        assert!(!s.contains("clip-path"), "{s}");
        assert!(!s.contains("koma-round"), "{s}");
        assert!(!s.contains("background-color:Canvas"), "{s}");
        assert!(s.contains("win_resize"), "{s}");
    }
}
