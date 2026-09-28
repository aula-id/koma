//! A narrow, in-process JSON bridge to the platform SDKs. Only this GUI worker
//! can call it; consent, ownership and action ordering remain in Rust.
use super::*;
use crate::app::runtime::computer::executor::Desktop;
use agent::computer_native::{koma_computer_call, koma_computer_cancel, koma_computer_free};
use anyhow::{ensure, Result};
use base64::Engine;
use serde::de::DeserializeOwned;
use serde_json::json;
use std::ffi::{CStr, CString};
fn call<T: DeserializeOwned>(request: serde_json::Value) -> Result<T> {
    let text = CString::new(serde_json::to_vec(&request)?)?;
    let ptr = unsafe { koma_computer_call(text.as_ptr()) };
    ensure!(
        !ptr.is_null(),
        "native desktop bridge failed to allocate a response"
    );
    let release = scopeguard::guard(ptr, |p| unsafe { koma_computer_free(p) });
    let bytes = unsafe { CStr::from_ptr(*release) }.to_bytes();
    ensure!(
        bytes.len() <= 30 * 1024 * 1024,
        "native response exceeds size limit"
    );
    let mut value: serde_json::Value = serde_json::from_slice(bytes)?;
    if let Some(error) = value.get("error").and_then(|v| v.as_str()) {
        if value.get("input_busy").and_then(|v| v.as_bool()) == Some(true) {
            return Err(InputBusy {
                input_started: value
                    .get("input_started")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
            }
            .into());
        }
        anyhow::bail!("{error}");
    }
    Ok(serde_json::from_value(value["result"].take())?)
}
pub fn capabilities() -> Capabilities {
    call(json!({"command":"capabilities","prompt":true})).unwrap_or_else(|e| Capabilities {
        limitations: vec![e.to_string()],
        ..Default::default()
    })
}
#[derive(serde::Deserialize)]
struct Capture {
    png: String,
    transform: Transform,
    #[serde(default)]
    elements: Vec<Element>,
    accessibility_status: String,
    ocr_status: String,
}
pub struct Native {
    done: Arc<AtomicBool>,
    watch: Option<std::thread::JoinHandle<()>>,
    capture: Option<Capture>,
}
impl Native {
    fn capture_source(
        &mut self,
        window: &Window,
        region: Option<Rect>,
    ) -> Result<(Transform, Vec<u8>)> {
        let mut request = json!({"command":"capture","window":window.id});
        if let Some(region) = region {
            request["region"] = serde_json::to_value(region)?;
        }
        let mut capture: Capture = call(request)?;
        ensure!(
            capture.elements.len() <= 512,
            "native extraction exceeds element limit"
        );
        let png =
            base64::engine::general_purpose::STANDARD.decode(std::mem::take(&mut capture.png))?;
        ensure!(
            png.len() <= 20 * 1024 * 1024,
            "native PNG exceeds size limit"
        );
        let transform = capture.transform.clone();
        ensure!(
            capture_size(transform.width, transform.height, false)
                == (transform.width, transform.height),
            "Native observation exceeds resolution budget"
        );
        self.capture = Some(capture);
        Ok((transform, png))
    }
    pub fn preview(&mut self, window: &str) -> Result<Vec<u8>> {
        let capture: Capture = call(json!({"command":"preview","window":window}))?;
        let png = base64::engine::general_purpose::STANDARD.decode(capture.png)?;
        ensure!(
            png.len() <= 20 * 1024 * 1024,
            "native preview exceeds size limit"
        );
        Ok(png)
    }
    pub fn open(cancelled: Arc<AtomicBool>) -> Result<Self> {
        call::<serde_json::Value>(json!({"command":"reset"}))?;
        let done = Arc::new(AtomicBool::new(false));
        let finished = done.clone();
        let watch = std::thread::spawn(move || {
            while !finished.load(Ordering::SeqCst) {
                if cancelled.load(Ordering::SeqCst) {
                    // Only sets a native atomic; SDK calls and key cleanup stay
                    // on the worker thread that owns the native operation.
                    unsafe { koma_computer_cancel() };
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        });
        Ok(Self {
            done,
            watch: Some(watch),
            capture: None,
        })
    }
    pub fn enrich(&mut self, reply: &mut Reply) {
        if let (Some(capture), Some(obs)) = (self.capture.take(), reply.observation.as_mut()) {
            obs.elements = capture.elements;
            for (i, element) in obs.elements.iter_mut().enumerate() {
                element.id = format!("{}:{}:{i}", obs.id, element.source);
            }
            obs.accessibility_status = capture.accessibility_status;
            obs.ocr_status = capture.ocr_status;
        }
    }
}
impl Desktop for Native {
    fn windows(&mut self) -> Result<Vec<Window>> {
        call(json!({"command":"windows"}))
    }
    fn select(&mut self, id: &str) -> Result<Window> {
        call(json!({"command":"select","window":id}))
    }
    fn inspect(&mut self, id: &str) -> Result<Window> {
        call(json!({"command":"inspect","window":id}))
    }
    fn capture(&mut self, window: &Window) -> Result<(Transform, Vec<u8>)> {
        self.capture_source(window, None)
    }
    fn capture_region(&mut self, window: &Window, region: Rect) -> Result<(Transform, Vec<u8>)> {
        self.capture_source(window, Some(region))
    }
    fn validate_input(&mut self, action: &Action) -> Result<()> {
        call::<serde_json::Value>(json!({"command":"validate_input","action":action}))?;
        Ok(())
    }
    fn input(&mut self, action: &Action, transform: &Transform) -> Result<()> {
        call::<serde_json::Value>(
            json!({"command":"input","action":action,"transform":transform}),
        )?;
        Ok(())
    }
    fn release(&mut self) {
        let _ = call::<serde_json::Value>(json!({"command":"release"}));
    }
}
impl Drop for Native {
    fn drop(&mut self) {
        self.release();
        self.done.store(true, Ordering::SeqCst);
        if let Some(watch) = self.watch.take() {
            let _ = watch.join();
        }
    }
}
