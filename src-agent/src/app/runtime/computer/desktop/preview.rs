//! Bounded live frames for an open GUI preview. They never become observations.
use super::*;
use anyhow::{ensure, Result};
use base64::Engine;
use std::time::{Duration, Instant};

impl Worker {
    pub fn accepts_preview(&self, request: &PreviewRequest) -> bool {
        self.sharing
            && self.preview_allowed
            && request.session == self.session
            && request.generation == self.generation
            && self
                .preview_window
                .as_ref()
                .is_some_and(|w| w.id == request.window)
    }

    pub fn preview(&mut self, request: PreviewRequest, tx: Sender<PreviewFrame>) {
        let Some(window) = self.preview_window.clone() else {
            let _ = tx.send(frame(
                request,
                Err(anyhow::anyhow!("Select a window first")),
            ));
            return;
        };
        if !self.accepts_preview(&request)
            || self.busy.load(Ordering::SeqCst)
            || self
                .last_preview
                .is_some_and(|t| t.elapsed() < Duration::from_millis(400))
            || self.preview_busy.swap(true, Ordering::SeqCst)
        {
            let _ = tx.send(frame(
                request,
                Err(anyhow::anyhow!("Preview waiting for desktop")),
            ));
            return;
        }
        self.last_preview = Some(Instant::now());
        let busy = self.preview_busy.clone();
        let input_busy = self.busy.clone();
        let gate = self.native_gate.clone();
        let cancelled = self.cancelled.clone();
        #[cfg(target_os = "linux")]
        let portal = self.portal.clone();
        std::thread::spawn(move || {
            let _busy = scopeguard::guard((), |_| busy.store(false, Ordering::SeqCst));
            let result = (|| -> Result<(String, u32, u32)> {
                let _native = gate
                    .try_lock()
                    .map_err(|_| anyhow::anyhow!("Desktop operation in progress"))?;
                ensure!(
                    !input_busy.load(Ordering::SeqCst) && !cancelled.load(Ordering::SeqCst),
                    "Preview paused"
                );
                let lock = acquire_native_lock()?;
                let _lock = scopeguard::guard(lock, |file| {
                    let _ = file.unlock();
                });
                let png = capture(
                    &window,
                    &cancelled,
                    #[cfg(target_os = "linux")]
                    &portal,
                )?;
                ensure!(!cancelled.load(Ordering::SeqCst), "Preview cancelled");
                let reader = image::ImageReader::with_format(
                    std::io::Cursor::new(&png),
                    image::ImageFormat::Png,
                );
                let (width, height) = reader.into_dimensions()?;
                ensure!(
                    width > 0 && height > 0 && capture_size(width, height, true) == (width, height),
                    "Preview dimensions exceed limit"
                );
                let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png)?
                    .thumbnail(1280, 960)
                    .to_rgb8();
                let mut bytes = Vec::new();
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 80)
                    .encode_image(&image)?;
                ensure!(bytes.len() <= 2 * 1024 * 1024, "Preview exceeds size limit");
                Ok((
                    format!(
                        "data:image/jpeg;base64,{}",
                        base64::engine::general_purpose::STANDARD.encode(bytes)
                    ),
                    image.width(),
                    image.height(),
                ))
            })();
            if !cancelled.load(Ordering::SeqCst) {
                let _ = tx.send(frame(request, result));
            }
        });
    }
}

fn frame(request: PreviewRequest, result: Result<(String, u32, u32)>) -> PreviewFrame {
    let (image, error, width, height) = match result {
        Ok((image, width, height)) => (Some(image), None, width, height),
        Err(e) => (None, Some(e.to_string()), 0, 0),
    };
    PreviewFrame {
        request,
        image,
        error,
        width,
        height,
        captured_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
    }
}

fn capture(
    window: &Window,
    cancelled: &Arc<AtomicBool>,
    #[cfg(target_os = "linux")] portal: &Arc<std::sync::Mutex<Option<Arc<wayland::Portal>>>>,
) -> Result<Vec<u8>> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        native::Native::open(cancelled.clone())?.preview(&window.id)
    }
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            return wayland::preview(window, cancelled, portal);
        }
        use super::super::executor::Desktop;
        let mut desktop = x11::X11::open(cancelled.clone())?;
        let current = desktop.inspect(&window.id)?;
        Ok(desktop.capture_source(&current, None, true)?.1)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (window, cancelled);
        anyhow::bail!("Live preview is unavailable on this platform")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_results_are_scoped_to_active_source_and_generation() {
        let window = Window {
            id: "window-a".into(),
            application: "fixture".into(),
            title: "one".into(),
            geometry: Rect {
                x: 0.0,
                y: 0.0,
                width: 80.0,
                height: 60.0,
            },
            focused: false,
            focus: None,
        };
        let observation = Observation {
            id: "model-frame".into(),
            session: "session".into(),
            generation: "generation".into(),
            window: window.clone(),
            transform: Transform {
                desktop: window.geometry,
                width: 80,
                height: 60,
            },
            captured_ms: 0,
            elements: vec![],
            accessibility_status: String::new(),
            ocr_status: String::new(),
            image_path: String::new(),
        };
        let mut status = Status {
            session: "session".into(),
            generation: "generation".into(),
            desktop: identity(),
            enabled: true,
            capabilities: Capabilities {
                capture: true,
                ..Default::default()
            },
            observation: Some(observation.clone()),
            ..Default::default()
        };
        let request = PreviewRequest {
            id: "frame-1".into(),
            session: status.session.clone(),
            generation: status.generation.clone(),
            window: window.id,
        };
        let mut worker = Worker::default();
        worker.status(&status);
        assert!(worker.accepts_preview(&request));
        status.paused = true;
        worker.status(&status);
        assert!(
            worker.accepts_preview(&request),
            "take back keeps sharing live"
        );
        assert!(!worker.active, "paused sharing cannot inject input");
        status.paused = false;
        status.busy = true;
        worker.status(&status);
        assert!(!worker.accepts_preview(&request));
        let (tx, rx) = std::sync::mpsc::channel();
        worker.preview(request.clone(), tx);
        assert!(rx.recv().unwrap().image.is_none());
        assert!(!worker.preview_busy.load(Ordering::SeqCst));
        status.busy = false;
        status.observation.as_mut().unwrap().window.id = "window-b".into();
        worker.status(&status);
        assert!(!worker.accepts_preview(&request));
        status.observation = Some(observation.clone());
        status.generation = "replacement".into();
        worker.status(&status);
        assert!(!worker.accepts_preview(&request));
        status.generation = request.generation.clone();
        worker.status(&status);
        worker.cancel();
        assert!(!worker.accepts_preview(&request));
        assert_eq!(status.observation, Some(observation));
    }
}
