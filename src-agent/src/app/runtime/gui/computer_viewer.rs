//! Detached observation viewer, owned by the same GUI event loop/connection.
use super::proto::UserEvent;
use crate::app::runtime::{client::HostCtl, computer::Status};
use anyhow::Result;
use tao::{
    dpi::{PhysicalPosition, PhysicalSize},
    event_loop::EventLoopWindowTarget,
    window::{Window, WindowBuilder},
};

#[derive(serde::Serialize, serde::Deserialize)]
struct Placement {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}
pub(super) struct Viewer {
    pub window: Window,
    webview: wry::WebView,
    preferences: Option<std::path::PathBuf>,
}
impl Viewer {
    pub fn new(
        target: &EventLoopWindowTarget<UserEvent>,
        status: Option<&Status>,
        palette: &serde_json::Value,
        ctl: std::sync::mpsc::Sender<HostCtl>,
    ) -> Result<Self> {
        Self::build(
            target,
            status,
            palette,
            ctl,
            crate::model::store::base_dir()
                .ok()
                .map(|p| p.join("computer-viewer.json")),
        )
    }
    fn build(
        target: &EventLoopWindowTarget<UserEvent>,
        status: Option<&Status>,
        palette: &serde_json::Value,
        ctl: std::sync::mpsc::Sender<HostCtl>,
        preferences: Option<std::path::PathBuf>,
    ) -> Result<Self> {
        let saved = preferences
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|v| serde_json::from_slice::<Placement>(&v).ok());
        let mut builder = WindowBuilder::new()
            .with_title("Koma · Computer")
            .with_inner_size(PhysicalSize::new(420, 360))
            .with_min_inner_size(PhysicalSize::new(300, 220))
            .with_focused(false)
            .with_focusable(false)
            .with_always_on_top(true)
            .with_content_protection(true);
        if let Some(p) = saved {
            if p.width >= 300 && p.width <= 2000 && p.height >= 220 && p.height <= 2000 {
                builder = builder.with_inner_size(PhysicalSize::new(p.width, p.height));
                // Only restore positions that intersect a current monitor.
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
        let initial = serde_json::to_string(&status)?;
        let builder = wry::WebViewBuilder::new()
            .with_url("koma://localhost/index.html#computer-preview")
            .with_initialization_script(format!(
                "window.__komaComputerInitial={initial};window.__komaComputerPalette={palette};"
            ))
            .with_custom_protocol("koma".into(), |_, request| {
                super::handle_koma_request(request)
            })
            .with_ipc_handler(move |request| {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(request.body()) else {
                    return;
                };
                if let Some(preview) = value.get("preview") {
                    if let Ok(request) = serde_json::from_value(preview.clone()) {
                        let _ = ctl.send(HostCtl::ComputerPreview(request));
                    }
                    return;
                }
                if let Some(action) = value
                    .get("action")
                    .and_then(|v| v.as_str())
                    .filter(|v| matches!(*v, "windows" | "select"))
                {
                    let _ = ctl.send(HostCtl::Computer {
                        action: action.into(),
                        window: value
                            .get("window")
                            .and_then(|v| v.as_str())
                            .map(str::to_owned),
                    });
                }
            });
        #[cfg(target_os = "linux")]
        let webview = {
            use tao::platform::unix::WindowExtUnix;
            use wry::WebViewBuilderExtUnix;
            builder.build_gtk(
                window
                    .default_vbox()
                    .ok_or_else(|| anyhow::anyhow!("viewer GTK container unavailable"))?,
            )?
        };
        #[cfg(not(target_os = "linux"))]
        let webview = builder.build(&window)?;
        Ok(Self {
            window,
            webview,
            preferences,
        })
    }
    pub fn update(&self, status: &Status) {
        if let Ok(json) = serde_json::to_string(status) {
            // Status serialization produces JSON data, never executable desktop text.
            let _ = self.webview.evaluate_script(&format!(
                "window.__komaComputerInitial={json};window.dispatchEvent(new CustomEvent('koma-computer',{{detail:window.__komaComputerInitial}}));"
            ));
        }
    }
    pub fn frame(&self, frame: &serde_json::Value) {
        let _ = self.webview.evaluate_script(&format!(
            "window.dispatchEvent(new CustomEvent('koma-computer-preview',{{detail:{frame}}}));"
        ));
    }
    pub fn palette(&self, palette: &serde_json::Value) {
        let _ = self.webview.evaluate_script(&format!(
            "window.__komaComputerPalette={palette};window.dispatchEvent(new CustomEvent('koma-computer-palette',{{detail:window.__komaComputerPalette}}));"
        ));
    }
    pub fn save(&self) {
        if let (Ok(position), Some(path)) = (self.window.outer_position(), &self.preferences) {
            let size = self.window.inner_size();
            let placement = Placement {
                x: position.x,
                y: position.y,
                width: size.width,
                height: size.height,
            };
            if let Ok(json) = serde_json::to_vec(&placement) {
                let _ = std::fs::write(path, json);
            }
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod native_tests {
    use super::*;
    use std::time::{Duration, Instant};
    use tao::{
        event::Event,
        event_loop::{ControlFlow, EventLoopBuilder},
        platform::{run_return::EventLoopExtRunReturn, unix::EventLoopBuilderExtUnix},
    };

    #[test]
    #[ignore = "opens disposable native GUI windows; run explicitly on a local X11 desktop"]
    fn native_viewer_preserves_focus_and_routes_controls() {
        assert!(std::env::var_os("WAYLAND_DISPLAY").is_none());
        let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event()
            .with_any_thread(true)
            .build();
        let target = WindowBuilder::new()
            .with_title("Koma viewer smoke target")
            .with_inner_size(PhysicalSize::new(300, 200))
            .with_focused(true)
            .build(&event_loop)
            .unwrap();
        target.set_focus();
        let deadline = Instant::now() + Duration::from_secs(2);
        event_loop.run_return(|_, _, control| {
            *control = if target.is_focused() || Instant::now() >= deadline {
                ControlFlow::Exit
            } else {
                ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(20))
            };
        });
        assert!(
            target.is_focused(),
            "window manager did not focus disposable target"
        );
        let (tx, rx) = std::sync::mpsc::channel();
        // No preferences path: the smoke test must not overwrite user placement.
        let viewer = Viewer::build(&event_loop, None, &serde_json::Value::Null, tx, None).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut injected = false;
        let mut routed = false;
        let mut focus_stolen = false;
        event_loop.run_return(|event, _, control| {
            if matches!(event, Event::MainEventsCleared) {
                focus_stolen |= viewer.window.is_focused() || !target.is_focused();
                if !injected {
                    viewer
                        .webview
                        .evaluate_script("window.ipc.postMessage(JSON.stringify({action:'windows'}))")
                        .unwrap();
                    injected = true;
                }
                while let Ok(message) = rx.try_recv() {
                    if matches!(message, HostCtl::Computer {action, ..} if action == "windows") {
                        routed = true;
                    }
                }
            }
            *control = if routed || Instant::now() >= deadline {
                ControlFlow::Exit
            } else {
                ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(20))
            };
        });
        assert!(routed, "viewer IPC did not route the window refresh");
        assert!(!focus_stolen, "floating viewer stole focus from the target");
    }
}
impl Drop for Viewer {
    fn drop(&mut self) {
        self.save();
    }
}
