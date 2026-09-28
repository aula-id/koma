//! User-mediated screen/application Wayland portal observations. Standard portals do
//! not expose target focus/obstruction identity, so they cannot authorize Koma's
//! input contract. Never substitute XWayland or global unverified injection.
use super::*;
use anyhow::{ensure, Context, Result};
use futures_util::StreamExt;
use std::{
    collections::HashMap,
    io::Read,
    os::fd::{AsRawFd, OwnedFd},
    process::{Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};
use zbus::{
    zvariant::{OwnedObjectPath, OwnedValue, Value},
    Connection, Proxy,
};
const DEST: &str = "org.freedesktop.portal.Desktop";
const ROOT: &str = "/org/freedesktop/portal/desktop";
const CAST: &str = "org.freedesktop.portal.ScreenCast";
const PICKER: &str = "portal:choose";
type Values = HashMap<String, OwnedValue>;
type Cache = Arc<Mutex<Option<Arc<Portal>>>>;

fn executable() -> std::path::PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let sibling = parent.join("gst-launch-1.0");
            if sibling.is_file() {
                return sibling;
            }
        }
    }
    "gst-launch-1.0".into()
}
pub fn capabilities() -> Capabilities {
    let check = (|| -> Result<()> {
        let binary = executable();
        ensure!(
            binary.is_file()
                || std::env::var_os("PATH").is_some_and(
                    |paths| std::env::split_paths(&paths).any(|p| p.join(&binary).is_file())
                ),
            "Install GStreamer tools, PipeWire and PNG plugins for portal observation"
        );
        let conn = zbus::blocking::connection::Builder::session()?
            .method_timeout(Duration::from_millis(500))
            .build()?;
        let proxy = zbus::blocking::Proxy::new(&conn, DEST, ROOT, CAST)?;
        let sources: u32 = proxy.get_property("AvailableSourceTypes")?;
        ensure!(
            sources & 3 != 0,
            "This compositor's portal does not support screen or application capture"
        );
        Ok(())
    })();
    let mut limitations = vec!["Wayland: select the source in the system portal dialog. Display listing and switching require system consent. This ScreenCast adapter is observation-only; RemoteDesktop input is not implemented. The in-app preview is used.".into()];
    if let Err(e) = &check {
        limitations.push(e.to_string());
    }
    Capabilities {
        capture: check.is_ok(),
        windows: false,
        ocr: super::super::enrichment::ocr_available(),
        limitations,
        ..Default::default()
    }
}
async fn request<B: serde::Serialize + zbus::zvariant::DynamicType>(
    conn: &Connection,
    method: &str,
    body: &B,
    token: &str,
    cancelled: &AtomicBool,
) -> Result<Values> {
    let sender = conn
        .unique_name()
        .context("Portal connection has no unique name")?
        .as_str()
        .trim_start_matches(':')
        .replace('.', "_");
    let path = format!("{ROOT}/request/{sender}/{token}");
    let reply = Proxy::new(conn, DEST, path.as_str(), "org.freedesktop.portal.Request").await?;
    let mut signals = reply.receive_signal("Response").await?;
    let proxy = Proxy::new(conn, DEST, ROOT, CAST).await?;
    let returned: OwnedObjectPath = proxy.call(method, body).await?;
    ensure!(
        returned.as_str() == path,
        "Portal returned an unexpected request handle"
    );
    let deadline = Instant::now() + Duration::from_secs(110);
    loop {
        tokio::select! {
            response=signals.next()=>{
                let message=response.context("Portal disconnected before its response")?;
                let (code,values):(u32,Values)=message.body().deserialize()?;
                ensure!(code==0,"Portal source selection cancelled or denied (response {code})");
                return Ok(values);
            }
            _=tokio::time::sleep(Duration::from_millis(20))=>{
                if cancelled.load(Ordering::SeqCst)||Instant::now()>deadline {
                    let _=reply.call::<_,_,()>("Close",&()).await;
                    anyhow::bail!("Portal request cancelled or timed out");
                }
            }
        }
    }
}
fn token() -> String {
    format!("koma{}", uuid::Uuid::new_v4().simple())
}
fn options<'a>(token: &'a str) -> HashMap<&'a str, Value<'a>> {
    HashMap::from([("handle_token", Value::from(token))])
}

pub struct Portal {
    runtime: tokio::runtime::Runtime,
    connection: Connection,
    session: OwnedObjectPath,
    remote: OwnedFd,
    node: u32,
    window: Window,
    live: Arc<AtomicBool>,
}
impl Portal {
    fn open(cancelled: &AtomicBool, screen: bool) -> Result<Arc<Self>> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()?;
        let (connection, session, remote, node, size, live) = runtime.block_on(async {
            let connection = zbus::connection::Builder::session()?
                .method_timeout(Duration::from_secs(5))
                .build()
                .await?;
            let t = token();
            let session_token = token();
            let mut args = options(&t);
            args.insert("session_handle_token", Value::from(session_token.as_str()));
            let mut result = request(&connection, "CreateSession", &(args,), &t, cancelled).await?;
            let path = String::try_from(
                result
                    .remove("session_handle")
                    .context("Portal omitted session handle")?,
            )?;
            let session = OwnedObjectPath::try_from(path)?;
            let result = async {
                let t = token();
                let mut args = options(&t);
                args.insert("types", Value::from(if screen { 1u32 } else { 2u32 }));
                args.insert("multiple", Value::from(false));
                args.insert("cursor_mode", Value::from(1u32));
                request(
                    &connection,
                    "SelectSources",
                    &(&session, args),
                    &t,
                    cancelled,
                )
                .await?;
                let t = token();
                let mut result = request(
                    &connection,
                    "Start",
                    &(&session, "", options(&t)),
                    &t,
                    cancelled,
                )
                .await?;
                let streams: Vec<(u32, Values)> =
                    Vec::try_from(result.remove("streams").context("Portal omitted streams")?)?;
                ensure!(
                    streams.len() == 1,
                    "Select exactly one source in the portal"
                );
                let (node, properties) = streams
                    .into_iter()
                    .next()
                    .context("Portal returned no stream")?;
                if let Some(kind) = properties.get("source_type") {
                    ensure!(
                        u32::try_from(kind)? == if screen { 1 } else { 2 },
                        "Portal returned the wrong source type"
                    );
                }
                let size = properties
                    .get("size")
                    .and_then(|v| <(i32, i32)>::try_from(v.try_clone().ok()?).ok());
                ensure!(
                    size.is_some_and(|(w, h)| w > 0
                        && h > 0
                        && i64::from(w) * i64::from(h) <= 67_108_864),
                    "Portal did not report valid bounded display dimensions"
                );
                let proxy = Proxy::new(&connection, DEST, ROOT, CAST).await?;
                let fd: zbus::zvariant::OwnedFd = proxy
                    .call(
                        "OpenPipeWireRemote",
                        &(&session, HashMap::<&str, Value>::new()),
                    )
                    .await?;
                let live = Arc::new(AtomicBool::new(true));
                let monitor = Proxy::new_owned(
                    connection.clone(),
                    DEST,
                    session.clone(),
                    "org.freedesktop.portal.Session",
                )
                .await?;
                let mut closed = monitor.receive_signal("Closed").await?;
                let marker = live.clone();
                tokio::spawn(async move {
                    closed.next().await;
                    marker.store(false, Ordering::SeqCst);
                });
                Ok::<_, anyhow::Error>((OwnedFd::from(fd), node, size, live))
            }
            .await;
            match result {
                Ok((remote, node, size, live)) => {
                    Ok((connection, session, remote, node, size, live))
                }
                Err(error) => {
                    if let Ok(proxy) = Proxy::new(
                        &connection,
                        DEST,
                        session.clone(),
                        "org.freedesktop.portal.Session",
                    )
                    .await
                    {
                        let _ = proxy.call::<_, _, ()>("Close", &()).await;
                    }
                    Err(error)
                }
            }
        })?;
        let (width, height) =
            size.context("Portal did not report display dimensions; bounded capture unavailable")?;
        ensure!(
            width > 0 && height > 0,
            "Portal reported invalid logical source dimensions"
        );
        Ok(Arc::new(Self {
            runtime,
            connection,
            session,
            remote,
            node,
            window: Window {
                id: format!(
                    "portal:{}:{}",
                    if screen { "screen" } else { "application" },
                    uuid::Uuid::new_v4()
                ),
                application: "Desktop portal".into(),
                title: if screen {
                    "User-selected Wayland screen"
                } else {
                    "User-selected Wayland application (assist)"
                }
                .into(),
                geometry: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: width as f64,
                    height: height as f64,
                },
                focused: false,
                focus: None,
            },
            live,
        }))
    }
    pub fn close(&self) {
        if !self.live.swap(false, Ordering::SeqCst) {
            return;
        }
        self.runtime.block_on(async {
            if let Ok(proxy) = Proxy::new(
                &self.connection,
                DEST,
                self.session.clone(),
                "org.freedesktop.portal.Session",
            )
            .await
            {
                let _ = proxy.call::<_, _, ()>("Close", &()).await;
            }
        });
    }
    fn capture(&self, cancelled: &AtomicBool, preview: bool) -> Result<(Transform, Vec<u8>)> {
        ensure!(
            !cancelled.load(Ordering::SeqCst) && self.live.load(Ordering::SeqCst),
            "Portal capture cancelled, source closed or permission revoked"
        );
        // Start a one-frame consumer only at observation time. Pass the portal's
        // restricted PipeWire fd; never connect to the compositor's unrestricted socket.
        let (width, height) = capture_size(
            self.window.geometry.width as u32,
            self.window.geometry.height as u32,
            preview,
        );
        let caps = format!("video/x-raw,width={width},height={height},pixel-aspect-ratio=1/1");
        let fd = self.remote.try_clone()?;
        let raw = fd.as_raw_fd();
        let mut command = Command::new(executable());
        command
            .args([
                "-q",
                "pipewiresrc",
                &format!("fd={raw}"),
                &format!("path={}", self.node),
                "num-buffers=1",
                "!",
                "videoconvert",
                "!",
                "videoscale",
                "add-borders=false",
                "!",
                &caps,
                "!",
                "pngenc",
                "snapshot=true",
                "!",
                "fdsink",
                "fd=1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(move || {
                if libc::fcntl(raw, libc::F_SETFD, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                let plugins = parent.join("../lib/gstreamer-1.0");
                if plugins.is_dir() {
                    command.env("GST_PLUGIN_PATH_1_0", plugins);
                }
                let scanner = parent.join("../libexec/gstreamer-1.0/gst-plugin-scanner");
                if scanner.is_file() {
                    command.env("GST_PLUGIN_SCANNER_1_0", scanner);
                }
                for (variable, relative) in [
                    ("PIPEWIRE_MODULE_DIR", "../lib/pipewire-0.3"),
                    ("SPA_PLUGIN_DIR", "../lib/spa-0.2"),
                    ("PIPEWIRE_CONFIG_DIR", "../share/pipewire"),
                ] {
                    let directory = parent.join(relative);
                    if directory.is_dir() {
                        command.env(variable, directory);
                    }
                }
            }
        }
        let mut child = command
            .spawn()
            .context("GStreamer/PipeWire capture process could not start")?;
        let stdout = child.stdout.take().context("Missing capture pipe")?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = vec![];
            let result = stdout
                .take(20 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes);
            let _ = tx.send(result);
        });
        let mut child = scopeguard::guard(child, |mut c| {
            let _ = c.kill();
            let _ = c.wait();
        });
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            ensure!(
                !cancelled.load(Ordering::SeqCst) && self.live.load(Ordering::SeqCst),
                "Portal capture cancelled or permission revoked"
            );
            ensure!(
                Instant::now() < deadline,
                "Portal frame timed out; check PipeWire/GStreamer plugins"
            );
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(bytes) => {
                    let bytes = bytes?;
                    ensure!(
                        bytes.len() <= 20 * 1024 * 1024,
                        "Portal PNG exceeds size limit"
                    );
                    // Closing the image output normally coincides with process exit;
                    // don't block cancellation on an unresponsive plugin's teardown.
                    let (width, height) = image::ImageReader::with_format(
                        std::io::Cursor::new(&bytes),
                        image::ImageFormat::Png,
                    )
                    .into_dimensions()?;
                    ensure!(
                        capture_size(width, height, preview) == (width, height),
                        "Portal image exceeds pixel limit"
                    );
                    let desktop = self.window.geometry;
                    let _ = child.try_wait();
                    return Ok((
                        Transform {
                            desktop,
                            width,
                            height,
                        },
                        bytes,
                    ));
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(e) => return Err(e.into()),
            }
        }
    }
}
impl Drop for Portal {
    fn drop(&mut self) {
        self.close();
    }
}
fn picker() -> Window {
    Window {
        id: PICKER.into(),
        application: "Desktop portal".into(),
        title: "Choose a display in the system dialog…".into(),
        geometry: Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
        focused: false,
        focus: None,
    }
}
pub(super) fn preview(window: &Window, cancelled: &AtomicBool, cache: &Cache) -> Result<Vec<u8>> {
    let portal = cache
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .context("No shared portal source")?;
    ensure!(portal.window.id == window.id, "Portal source changed");
    Ok(portal.capture(cancelled, true)?.1)
}

pub fn execute(request: &Request, cancelled: &AtomicBool, cache: &Cache) -> Reply {
    let mut reply = Reply {
        id: request.id.clone(),
        session: request.session.clone(),
        generation: request.generation.clone(),
        ..Default::default()
    };
    let result = (|| -> Result<()> {
        ensure!(!cancelled.load(Ordering::SeqCst), "Cancelled");
        let mut portal = cache.lock().unwrap_or_else(|p| p.into_inner()).clone();
        match &request.operation {
            Operation::Windows => {
                reply.windows = vec![picker()];
                if let Some(p) = portal.filter(|p| p.live.load(Ordering::SeqCst)) {
                    reply.windows.push(p.window.clone());
                }
                return Ok(());
            }
            Operation::InspectWindow { window }
                if window == PICKER
                    || window == "portal:choose:screen"
                    || window == "portal:choose:application" =>
            {
                if let Some(old) = portal.take() {
                    old.close();
                }
                let selected = Portal::open(cancelled, window != "portal:choose:application")?;
                ensure!(
                    !cancelled.load(Ordering::SeqCst),
                    "Portal selection cancelled"
                );
                *cache.lock().unwrap_or_else(|p| p.into_inner()) = Some(selected.clone());
                portal = Some(selected);
            }
            Operation::Act { .. } | Operation::Select { .. } => {
                anyhow::bail!("Wayland ScreenCast sharing is observation-only; RemoteDesktop input is not implemented. Choose a display in the GUI.");
            }
            _ => {}
        }
        let portal =
            portal.context("Choose a display using the GUI's system source picker first")?;
        if let Some(obs) = &request.observation {
            if !matches!(request.operation, Operation::InspectWindow { .. }) {
                ensure!(
                    obs.window.id == portal.window.id,
                    "Portal source changed; select again"
                );
            }
        }
        if let Operation::InspectWindow { window } = &request.operation {
            ensure!(
                window == PICKER
                    || window == "portal:choose:screen"
                    || window == "portal:choose:application"
                    || window == &portal.window.id,
                "Portal source no longer selected"
            );
        }
        let (transform, png) = portal.capture(cancelled, false)?;
        ensure!(
            !cancelled.load(Ordering::SeqCst),
            "Late portal observation discarded"
        );
        reply.png = png;
        reply.observation = Some(Observation {
            id: uuid::Uuid::new_v4().to_string(),
            session: request.session.clone(),
            generation: request.generation.clone(),
            window: portal.window.clone(),
            transform,
            captured_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default().as_millis() as u64,
            elements: vec![],
            accessibility_status: "unavailable: portal does not expose a verifiable application/window identity for selected-window AT-SPI traversal".into(),
            ocr_status: "pending local Tesseract".into(),
            image_path: String::new(),
        });
        Ok(())
    })();
    if let Err(e) = result {
        reply.error = Some(e.to_string());
        let old = cache.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(p) = old {
            p.close();
        }
    }
    reply
}
