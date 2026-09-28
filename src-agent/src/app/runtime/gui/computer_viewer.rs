//! Detached observation viewer, owned by the same GUI event loop/connection.
use super::proto::UserEvent;
use crate::app::runtime::{client::HostCtl, computer::Status};
use anyhow::Result;
use std::cell::{Cell, RefCell};
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
    aspect: Cell<f64>,
    framed: Cell<bool>,
    last_size: Cell<PhysicalSize<u32>>,
    /// Origin and target while a source-change contain is in flight.
    settle: Cell<Option<(PhysicalSize<u32>, PhysicalSize<u32>)>>,
    source: RefCell<Option<(String, String, String)>>,
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
            .with_min_inner_size(PhysicalSize::new(80, 80))
            .with_focused(false)
            .with_focusable(false)
            .with_always_on_top(true)
            .with_content_protection(true);
        if let Some(p) = saved {
            if p.width >= 80 && p.width <= 4000 && p.height >= 80 && p.height <= 4000 {
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
        let viewer = Self {
            last_size: Cell::new(window.inner_size()),
            aspect: Cell::new(16.0 / 9.0),
            framed: Cell::new(false),
            settle: Cell::new(None),
            source: RefCell::new(None),
            window,
            webview,
            preferences,
        };
        if let Some(status) = status {
            viewer.update(status);
        }
        Ok(viewer)
    }
    pub fn update(&self, status: &Status) {
        *self.source.borrow_mut() = status.observation.as_ref().map(|o| {
            (
                status.session.clone(),
                status.generation.clone(),
                o.window.id.clone(),
            )
        });
        if let Some(observation) = &status.observation {
            self.set_aspect(observation.transform.width, observation.transform.height);
        }
        if let Ok(json) = serde_json::to_string(status) {
            // Status serialization produces JSON data, never executable desktop text.
            let _ = self.webview.evaluate_script(&format!(
                "window.__komaComputerInitial={json};window.dispatchEvent(new CustomEvent('koma-computer',{{detail:window.__komaComputerInitial}}));"
            ));
        }
    }
    pub fn frame(&self, frame: &serde_json::Value) {
        if self
            .source
            .borrow()
            .as_ref()
            .is_some_and(|(session, generation, window)| {
                frame["request"]["session"].as_str() == Some(session.as_str())
                    && frame["request"]["generation"].as_str() == Some(generation.as_str())
                    && frame["request"]["window"].as_str() == Some(window.as_str())
            })
        {
            if let (Some(width), Some(height)) = (frame["width"].as_u64(), frame["height"].as_u64())
            {
                if width <= 1280 && height <= 960 {
                    self.set_aspect(width as u32, height as u32);
                }
            }
        }
        let _ = self.webview.evaluate_script(&format!(
            "window.dispatchEvent(new CustomEvent('koma-computer-preview',{{detail:{frame}}}));"
        ));
    }
    pub fn palette(&self, palette: &serde_json::Value) {
        let _ = self.webview.evaluate_script(&format!(
            "window.__komaComputerPalette={palette};window.dispatchEvent(new CustomEvent('koma-computer-palette',{{detail:window.__komaComputerPalette}}));"
        ));
    }
    fn set_aspect(&self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        let aspect = f64::from(width) / f64::from(height);
        let first = !self.framed.get();
        let changed = (aspect / self.aspect.get() - 1.0).abs() >= 0.005;
        if !first && !changed {
            return;
        }
        self.aspect.set(aspect);
        self.framed.set(true);
        if first {
            self.resized();
        } else {
            // A new screen or window fits inside the box the user already has.
            // Dragging the window still resizes through resized().
            self.contain();
        }
    }
    fn monitor_size(&self) -> PhysicalSize<u32> {
        self.window
            .current_monitor()
            .map(|m| m.size())
            .unwrap_or(PhysicalSize::new(1920, 1080))
    }
    fn contain(&self) {
        let size = self.window.inner_size();
        let fitted = contain_size(size, self.aspect.get(), self.monitor_size());
        self.last_size.set(fitted);
        if fitted != size {
            self.settle.set(Some((size, fitted)));
            self.window.set_inner_size(fitted);
        }
    }
    pub fn resized(&self) {
        let size = self.window.inner_size();
        // An empty viewer has no image ratio. Let the window manager resize
        // either axis until a source observation arrives.
        if self.source.borrow().is_none() {
            self.last_size.set(size);
            return;
        }
        let aspect = self.aspect.get();
        if let Some((origin, target)) = self.settle.get() {
            if size == target || matches_aspect(size, aspect) {
                self.settle.set(None);
                self.last_size.set(size);
                return;
            }
            if size == origin {
                return;
            }
            self.settle.set(None);
        }
        let previous = self.last_size.get();
        let width = if size.height.abs_diff(previous.height) as f64 * aspect
            > size.width.abs_diff(previous.width) as f64
        {
            f64::from(size.height) * aspect
        } else {
            f64::from(size.width)
        };
        let fitted = fit_size(width, aspect, self.monitor_size());
        self.last_size.set(fitted);
        // Some window managers impose their own minimum. Do not loop forever
        // re-requesting a size the OS just refused.
        if fitted != size && fitted != previous {
            self.window.set_inner_size(fitted);
        }
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

/// Fit `aspect` inside `current` by shrinking one side. Neither side grows.
fn contain_size(
    current: PhysicalSize<u32>,
    aspect: f64,
    screen: PhysicalSize<u32>,
) -> PhysicalSize<u32> {
    let max_width = f64::from(screen.width.saturating_sub(48).max(1));
    let max_height = f64::from(screen.height.saturating_sub(100).max(1));
    let box_w = f64::from(current.width.max(1)).min(max_width);
    let box_h = f64::from(current.height.max(1)).min(max_height);
    let (width, height) = if aspect >= box_w / box_h {
        let height = box_w / aspect;
        (height * aspect, height)
    } else {
        let width = box_h * aspect;
        (width, width / aspect)
    };
    PhysicalSize::new(
        width.round().max(1.0) as u32,
        height.round().max(1.0) as u32,
    )
}

fn matches_aspect(size: PhysicalSize<u32>, aspect: f64) -> bool {
    size.height > 0 && (f64::from(size.width) - f64::from(size.height) * aspect).abs() <= 1.5
}

fn fit_size(width: f64, aspect: f64, screen: PhysicalSize<u32>) -> PhysicalSize<u32> {
    let max_width = f64::from(screen.width.saturating_sub(48).max(1));
    let max_height = f64::from(screen.height.saturating_sub(100).max(1));
    let limit = max_width.min(max_height * aspect);
    let minimum = 240.0_f64.max(120.0 * aspect).min(limit);
    let width = width.clamp(minimum, limit);
    PhysicalSize::new(
        width.round().max(1.0) as u32,
        (width / aspect).round().max(1.0) as u32,
    )
}

#[cfg(test)]
mod sizing_tests {
    use super::*;

    #[test]
    fn computer_preview_fits_complete_landscape_and_portrait_frames() {
        let monitor = PhysicalSize::new(1440, 900);
        for aspect in [16.0 / 9.0, 4.0 / 3.0, 9.0 / 16.0, 3.0] {
            for width in [50.0, 560.0, 4000.0] {
                let size = fit_size(width, aspect, monitor);
                assert!(size.width <= monitor.width - 48);
                assert!(size.height <= monitor.height - 100);
                assert!((f64::from(size.width) / aspect - f64::from(size.height)).abs() <= 1.0);
            }
        }
    }

    #[test]
    fn source_switch_contains_inside_the_current_box() {
        let monitor = PhysicalSize::new(1440, 900);
        let tall = PhysicalSize::new(480, 800);
        let wide = contain_size(tall, 32.0 / 9.0, monitor);
        assert!(wide.width <= tall.width);
        assert!(wide.height < tall.height);
        assert!((f64::from(wide.width) / f64::from(wide.height) - 32.0 / 9.0).abs() < 0.05);

        let landscape = PhysicalSize::new(800, 450);
        let portrait = contain_size(landscape, 9.0 / 16.0, monitor);
        assert!(portrait.width < landscape.width);
        assert!(portrait.height <= landscape.height);
        assert!((f64::from(portrait.width) / f64::from(portrait.height) - 9.0 / 16.0).abs() < 0.05);

        let same = contain_size(landscape, 16.0 / 9.0, monitor);
        assert!((same.width as i32 - landscape.width as i32).abs() <= 1);
        assert!((same.height as i32 - landscape.height as i32).abs() <= 1);
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
                        .evaluate_script(
                            "window.ipc.postMessage(JSON.stringify({action:'windows'}))",
                        )
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
