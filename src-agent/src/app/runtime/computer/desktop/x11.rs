//! Local X11 desktop adapter; no shell commands or polling capture loop.
use super::*;
use crate::app::runtime::computer::executor::Desktop;
use anyhow::{bail, ensure, Result};
use std::{
    ffi::{CStr, CString},
    os::raw::{c_int, c_ulong},
    ptr,
};
use x11_dl::{xlib, xtest};

// Xlib's default error handler exits the process for stale XIDs. Preserve the
// host's handler for other displays and catch errors on this worker's display.
type ErrorHandler = unsafe extern "C" fn(*mut xlib::Display, *mut xlib::XErrorEvent) -> c_int;
static PREVIOUS: std::sync::OnceLock<Option<ErrorHandler>> = std::sync::OnceLock::new();
thread_local! { static OWN: std::cell::Cell<*mut xlib::Display> = const {std::cell::Cell::new(ptr::null_mut())}; static ERROR: std::cell::Cell<bool> = const {std::cell::Cell::new(false)}; }
unsafe extern "C" fn error_handler(
    display: *mut xlib::Display,
    event: *mut xlib::XErrorEvent,
) -> c_int {
    if OWN.with(|v| v.get() == display) {
        ERROR.with(|v| v.set(true));
        return 0;
    }
    if let Some(Some(previous)) = PREVIOUS.get() {
        return previous(display, event);
    }
    0
}
pub fn capabilities() -> Capabilities {
    match X11::open(Arc::new(AtomicBool::new(false))) {
        Ok(x) => Capabilities {
            capture: true,
            windows: true,
            focus: true,
            pointer: x.test.is_some(),
            keyboard: x.test.is_some(),
            floating: true,
            accessibility: std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some(),
            ocr: crate::app::runtime::computer::enrichment::ocr_available(),
            limitations: vec![
                "X11: fully visible windows only; typing requires characters present in the active keyboard map. Floating viewer cannot be excluded from X11 capture; obstructed targets are rejected. Accessibility requires a uniquely matching AT-SPI window.".into()
            ],
        },
        Err(e)=>Capabilities {limitations:vec![e.to_string()],..Default::default()},
    }
}
pub struct X11 {
    x: xlib::Xlib,
    test: Option<xtest::Xf86vmode>,
    display: *mut xlib::Display,
    root: c_ulong,
    cancelled: Arc<AtomicBool>,
    held: Vec<u32>,
    buttons: Vec<u32>,
    target: Option<Window>,
}
impl X11 {
    pub fn open(cancelled: Arc<AtomicBool>) -> Result<Self> {
        let display_name = std::env::var("DISPLAY").unwrap_or_default();
        ensure!(
            display_name.starts_with(':') || display_name.starts_with("unix:"),
            "Computer control requires a local X11 Unix-socket display"
        );
        let x = xlib::Xlib::open()?;
        // GTK/tao initialize Xlib threading before this GUI worker is created.
        let display = unsafe { (x.XOpenDisplay)(ptr::null()) };
        ensure!(!display.is_null(), "Cannot open X11 display");
        OWN.with(|v| v.set(display));
        PREVIOUS.get_or_init(|| unsafe { (x.XSetErrorHandler)(Some(error_handler)) });
        let root = unsafe { (x.XDefaultRootWindow)(display) };
        let test = xtest::Xf86vmode::open().ok().filter(|t| {
            let (mut a, mut b, mut c, mut d) = (0, 0, 0, 0);
            unsafe { (t.XTestQueryExtension)(display, &mut a, &mut b, &mut c, &mut d) != 0 }
        });
        Ok(Self {
            x,
            test,
            display,
            root,
            cancelled,
            held: vec![],
            buttons: vec![],
            target: None,
        })
    }
    fn sync(&self) -> Result<()> {
        unsafe {
            (self.x.XSync)(self.display, 0);
        };
        ensure!(
            !ERROR.with(|v| v.replace(false)),
            "X11 window changed or permission denied"
        );
        Ok(())
    }
    fn atom(&self, name: &str) -> Result<c_ulong> {
        let name = CString::new(name)?;
        Ok(unsafe { (self.x.XInternAtom)(self.display, name.as_ptr(), 0) })
    }
    fn property(&self, window: c_ulong, name: &str) -> Result<Vec<c_ulong>> {
        let (mut actual, mut format, mut count, mut remaining, mut data) =
            (0, 0, 0, 0, ptr::null_mut());
        unsafe {
            (self.x.XGetWindowProperty)(
                self.display,
                window,
                self.atom(name)?,
                0,
                4096,
                0,
                0,
                &mut actual,
                &mut format,
                &mut count,
                &mut remaining,
                &mut data,
            );
        }
        let out = if format == 32 && !data.is_null() && count <= 4096 {
            unsafe { std::slice::from_raw_parts(data.cast::<c_ulong>(), count as usize).to_vec() }
        } else {
            vec![]
        };
        if !data.is_null() {
            unsafe {
                (self.x.XFree)(data.cast());
            }
        }
        self.sync()?;
        Ok(out)
    }
    fn xid(&self, id: &str) -> Result<c_ulong> {
        let (xid, pid) = id
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("invalid window identity"))?;
        let xid = xid.parse()?;
        let process: u32 = pid.parse()?;
        ensure!(
            process != 0 && process != std::process::id(),
            "cannot target Koma or a window without process identity"
        );
        ensure!(
            self.property(self.root, "_NET_CLIENT_LIST")?.contains(&xid),
            "window no longer listed by the window manager"
        );
        ensure!(
            self.property(xid, "_NET_WM_PID")?
                .first()
                .copied()
                .unwrap_or(0)
                .to_string()
                == pid,
            "window identity changed"
        );
        Ok(xid)
    }
    fn geometry(&self, window: c_ulong) -> Result<Rect> {
        let mut attr: xlib::XWindowAttributes = unsafe { std::mem::zeroed() };
        let ok = unsafe { (self.x.XGetWindowAttributes)(self.display, window, &mut attr) };
        self.sync()?;
        ensure!(
            ok != 0 && attr.map_state == xlib::IsViewable && attr.class == xlib::InputOutput,
            "window not visible"
        );
        let (mut x, mut y, mut child) = (0, 0, 0);
        unsafe {
            (self.x.XTranslateCoordinates)(
                self.display,
                window,
                self.root,
                0,
                0,
                &mut x,
                &mut y,
                &mut child,
            );
        }
        self.sync()?;
        let r = Rect {
            x: x as f64,
            y: y as f64,
            width: attr.width as f64,
            height: attr.height as f64,
        };
        ensure!(r.valid(), "invalid window bounds");
        Ok(r)
    }
    fn unobstructed(&self, window: c_ulong, bounds: Rect) -> Result<()> {
        // Root children include the WM frames, popups, panels and Koma viewer.
        let (mut root, mut parent, mut children, mut count) = (0, 0, ptr::null_mut(), 0);
        let mut top = window;
        for _ in 0..32 {
            unsafe {
                (self.x.XQueryTree)(
                    self.display,
                    top,
                    &mut root,
                    &mut parent,
                    &mut children,
                    &mut count,
                );
            }
            if !children.is_null() {
                unsafe {
                    (self.x.XFree)(children.cast());
                }
                children = ptr::null_mut();
            }
            self.sync()?;
            if parent == self.root {
                break;
            }
            ensure!(parent != 0, "window detached");
            top = parent;
        }
        unsafe {
            (self.x.XQueryTree)(
                self.display,
                self.root,
                &mut root,
                &mut parent,
                &mut children,
                &mut count,
            );
        }
        let list = if !children.is_null() {
            unsafe { std::slice::from_raw_parts(children, count as usize).to_vec() }
        } else {
            vec![]
        };
        if !children.is_null() {
            unsafe {
                (self.x.XFree)(children.cast());
            }
        }
        self.sync()?;
        let pos = list
            .iter()
            .position(|w| *w == top)
            .ok_or_else(|| anyhow::anyhow!("window is not on desktop"))?;
        for above in &list[pos + 1..] {
            if let Ok(r) = self.geometry(*above) {
                ensure!(
                    !(r.x < bounds.x + bounds.width
                        && r.x + r.width > bounds.x
                        && r.y < bounds.y + bounds.height
                        && r.y + r.height > bounds.y),
                    "target obstructed by another window; move the viewer or select again"
                );
            }
        }
        Ok(())
    }
    fn guard_input(&self) -> Result<()> {
        ensure!(!self.cancelled.load(Ordering::SeqCst), "cancelled");
        let target = self
            .target
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no input target"))?;
        let xid = self.xid(&target.id)?;
        ensure!(
            self.geometry(xid)? == target.geometry,
            "target geometry changed"
        );
        ensure!(
            self.property(self.root, "_NET_ACTIVE_WINDOW")?.first() == Some(&xid),
            "focus changed"
        );
        self.unobstructed(xid, target.geometry)
    }
    fn key(&mut self, code: u32, down: bool) -> Result<()> {
        let t = self
            .test
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("XTEST unavailable"))?;
        if down {
            self.guard_input()?;
            ensure!(!self.cancelled.load(Ordering::SeqCst), "cancelled");
            self.held.push(code);
        }
        unsafe {
            (t.XTestFakeKeyEvent)(self.display, code, i32::from(down), 0);
        }
        self.sync()?;
        if !down {
            self.held.retain(|v| *v != code);
        }
        Ok(())
    }
    fn button(&mut self, button: u32) -> Result<()> {
        self.guard_input()?;
        ensure!(!self.cancelled.load(Ordering::SeqCst), "cancelled");
        let t = self
            .test
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("XTEST unavailable"))?;
        self.buttons.push(button);
        unsafe {
            (t.XTestFakeButtonEvent)(self.display, button, 1, 0);
            (t.XTestFakeButtonEvent)(self.display, button, 0, 0);
        }
        self.sync()?;
        self.buttons.clear();
        Ok(())
    }
    fn mapped(&self, symbol: c_ulong) -> Result<(u32, bool)> {
        let code = unsafe { (self.x.XKeysymToKeycode)(self.display, symbol) };
        ensure!(
            code != 0,
            "character is not present in the active X11 keyboard map"
        );
        let first = unsafe { (self.x.XKeycodeToKeysym)(self.display, code, 0) };
        let second = unsafe { (self.x.XKeycodeToKeysym)(self.display, code, 1) };
        ensure!(
            symbol == first || symbol == second,
            "character needs an unsupported keyboard group"
        );
        Ok((u32::from(code), symbol != first))
    }
}
impl Desktop for X11 {
    fn windows(&mut self) -> Result<Vec<Window>> {
        let mut windows = vec![];
        for xid in self
            .property(self.root, "_NET_CLIENT_LIST")?
            .into_iter()
            .take(256)
        {
            let pid = self
                .property(xid, "_NET_WM_PID")?
                .first()
                .copied()
                .unwrap_or(0);
            // Never offer Koma's own native windows as targets.
            if pid == std::process::id() as c_ulong {
                continue;
            }
            if let Ok(window) = self.inspect(&format!("{xid}:{pid}")) {
                windows.push(window);
            }
        }
        Ok(windows)
    }
    fn select(&mut self, id: &str) -> Result<Window> {
        let xid = self.xid(id)?;
        self.geometry(xid)?;
        let mut event: xlib::XEvent = unsafe { std::mem::zeroed() };
        let mut message: xlib::XClientMessageEvent = unsafe { std::mem::zeroed() };
        message.type_ = xlib::ClientMessage;
        message.window = xid;
        message.message_type = self.atom("_NET_ACTIVE_WINDOW")?;
        message.format = 32;
        message.data.set_long(0, 2);
        message.data.set_long(1, 0);
        event.client_message = message;
        unsafe {
            (self.x.XSendEvent)(
                self.display,
                self.root,
                0,
                xlib::SubstructureRedirectMask | xlib::SubstructureNotifyMask,
                &mut event,
            );
        }
        self.sync()?;
        for _ in 0..50 {
            ensure!(!self.cancelled.load(Ordering::SeqCst), "cancelled");
            let window = self.inspect(id)?;
            if window.focused {
                return Ok(window);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        bail!("window manager did not grant focus")
    }
    fn inspect(&mut self, id: &str) -> Result<Window> {
        let xid = self.xid(id)?;
        let geometry = self.geometry(xid)?;
        let mut title = ptr::null_mut();
        unsafe {
            (self.x.XFetchName)(self.display, xid, &mut title);
        }
        let name = if title.is_null() {
            String::new()
        } else {
            unsafe {
                CStr::from_ptr(title)
                    .to_string_lossy()
                    .chars()
                    .take(512)
                    .collect()
            }
        };
        if !title.is_null() {
            unsafe {
                (self.x.XFree)(title.cast());
            }
        }
        let focused = self.property(self.root, "_NET_ACTIVE_WINDOW")?.first() == Some(&xid);
        self.sync()?;
        let result = Window {
            id: id.into(),
            application: format!(
                "pid {}",
                id.split_once(':').map(|v| v.1).unwrap_or("unknown")
            ),
            title: name,
            geometry,
            focused,
        };
        self.target = Some(result.clone());
        Ok(result)
    }
    fn capture(&mut self, window: &Window) -> Result<(Transform, Vec<u8>)> {
        let xid = self.xid(&window.id)?;
        self.unobstructed(xid, window.geometry)?;
        let w = window.geometry.width as u32;
        let h = window.geometry.height as u32;
        ensure!(
            u64::from(w) * u64::from(h) <= 32_000_000,
            "window too large"
        );
        let raw = unsafe { (self.x.XGetImage)(self.display, xid, 0, 0, w, h, !0, xlib::ZPixmap) };
        let guard = scopeguard::guard(raw, |image| unsafe {
            if !image.is_null() {
                (self.x.XDestroyImage)(image);
            }
        });
        self.sync()?;
        ensure!(!raw.is_null(), "capture failed");
        let mut image = image::RgbImage::new(w, h);
        let masks = unsafe { [(*raw).red_mask, (*raw).green_mask, (*raw).blue_mask] };
        ensure!(masks.iter().all(|v| *v != 0), "unsupported X11 visual");
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            let value = unsafe { (self.x.XGetPixel)(*guard, x as i32, y as i32) };
            let mut rgb = [0; 3];
            for (i, mask) in masks.iter().enumerate() {
                let shift = mask.trailing_zeros();
                rgb[i] = (((value & mask) >> shift) * 255 / (mask >> shift)) as u8;
            }
            *pixel = image::Rgb(rgb);
        }
        let mut png = std::io::Cursor::new(vec![]);
        image.write_to(&mut png, image::ImageFormat::Png)?;
        ensure!(
            png.get_ref().len() <= 20 * 1024 * 1024,
            "captured PNG exceeds the 20 MiB observation limit"
        );
        Ok((
            Transform {
                desktop: window.geometry,
                width: w,
                height: h,
            },
            png.into_inner(),
        ))
    }
    fn input(&mut self, action: &Action, transform: &Transform) -> Result<()> {
        self.guard_input()?;
        ensure!(!self.cancelled.load(Ordering::SeqCst), "cancelled");
        match action {
            Action::Move { x, y }
            | Action::Scroll { x, y, .. }
            | Action::Click {
                x: Some(x),
                y: Some(y),
                ..
            } => {
                let (x, y) = transform.map(*x, *y)?;
                let t = self
                    .test
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("XTEST unavailable"))?;
                unsafe {
                    (t.XTestFakeMotionEvent)(
                        self.display,
                        -1,
                        x.round() as i32,
                        y.round() as i32,
                        0,
                    );
                }
                self.sync()?;
                match action {
                    Action::Click { button, .. } => {
                        self.button(if *button == Button::Right { 3 } else { 1 })?;
                        if *button == Button::Double {
                            self.button(1)?;
                        }
                    }
                    Action::Scroll { delta, .. } => {
                        for _ in 0..delta.unsigned_abs() {
                            self.button(if *delta > 0 { 5 } else { 4 })?;
                        }
                    }
                    _ => {}
                }
            }
            Action::Type { text } => {
                let keys = text
                    .chars()
                    .map(|c| {
                        self.mapped(match c {
                            '\n' | '\r' => 0xff0d,
                            '\t' => 0xff09,
                            c if (c as u32) < 256 => c as c_ulong,
                            c => 0x01000000 | c as c_ulong,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                let shift = self.mapped(0xffe1)?.0;
                for (key, shifted) in keys {
                    if shifted {
                        self.key(shift, true)?;
                    }
                    self.key(key, true)?;
                    self.key(key, false)?;
                    if shifted {
                        self.key(shift, false)?;
                    }
                }
            }
            Action::Key { keys } => {
                let codes = keys
                    .iter()
                    .map(|k| {
                        let name = CString::new(match k.as_str() {
                            "Control" | "Ctrl" => "Control_L",
                            "Shift" => "Shift_L",
                            "Alt" => "Alt_L",
                            "Super" | "Command" => "Super_L",
                            "Meta" => "Meta_L",
                            "Enter" => "Return",
                            "Esc" => "Escape",
                            "Backspace" => "BackSpace",
                            "Space" => "space",
                            "ArrowLeft" => "Left",
                            "ArrowRight" => "Right",
                            "ArrowUp" => "Up",
                            "ArrowDown" => "Down",
                            other => other,
                        })?;
                        let symbol = unsafe { (self.x.XStringToKeysym)(name.as_ptr()) };
                        ensure!(symbol != 0, "unknown X11 key name");
                        Ok(self.mapped(symbol)?.0)
                    })
                    .collect::<Result<Vec<_>>>()?;
                for code in &codes {
                    self.key(*code, true)?;
                }
                for code in codes.iter().rev() {
                    self.key(*code, false)?;
                }
            }
            _ => bail!("unresolved click target"),
        }
        Ok(())
    }
    fn release(&mut self) {
        if let Some(t) = &self.test {
            unsafe {
                for key in self.held.drain(..).rev() {
                    (t.XTestFakeKeyEvent)(self.display, key, 0, 0);
                }
                for button in self.buttons.drain(..) {
                    (t.XTestFakeButtonEvent)(self.display, button, 0, 0);
                }
                (self.x.XSync)(self.display, 0);
            }
        }
    }
}
impl Drop for X11 {
    fn drop(&mut self) {
        self.release();
        unsafe {
            (self.x.XCloseDisplay)(self.display);
        };
        OWN.with(|v| v.set(ptr::null_mut()));
    }
}

#[cfg(test)]
mod native_tests {
    use super::*;
    use std::io::BufRead;
    #[test]
    #[ignore = "opens a disposable X11 window and injects input; run explicitly on a local desktop"]
    fn native_fixture_round_trip() {
        let directory = std::env::temp_dir().join(format!("koma-native-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let cleanup = scopeguard::guard(directory, |p| {
            let _ = std::fs::remove_dir_all(p);
        });
        let source = cleanup.join("fixture.c");
        let binary = cleanup.join("fixture");
        let state = cleanup.join("state");
        std::fs::write(
            &source,
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../docs/testing/computer-fixture.c"
            )),
        )
        .unwrap();
        assert!(std::process::Command::new("cc")
            .arg(&source)
            .args(["-lX11", "-o"])
            .arg(&binary)
            .status()
            .unwrap()
            .success());
        let child = std::process::Command::new(&binary)
            .arg(&state)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut child = scopeguard::guard(child, |mut child| {
            let _ = child.kill();
            let _ = child.wait();
        });
        let mut line = String::new();
        std::io::BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let id = line.trim().to_string();
        assert!(!id.is_empty());
        let token = Arc::new(AtomicBool::new(false));
        let mut desktop = X11::open(token.clone()).unwrap();
        for _ in 0..100 {
            if desktop.windows().unwrap().iter().any(|w| w.id == id) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let mut request = Request {
            id: "select".into(),
            session: "fixture".into(),
            generation: "g".into(),
            operation: Operation::Select {
                window: id,
                generation: "g".into(),
            },
            observation: None,
        };
        let first =
            crate::app::runtime::computer::executor::execute(&mut desktop, &request, &token);
        assert!(first.error.is_none(), "{:?}", first.error);
        assert!(!first.png.is_empty());
        request.observation = first.observation;
        request.id = "click".into();
        request.operation = Operation::Act {
            observation: request.observation.as_ref().unwrap().id.clone(),
            actions: vec![Action::Click {
                x: Some(40.0),
                y: Some(40.0),
                element: None,
                button: Button::Left,
            }],
            observe: true,
        };
        let clicked =
            crate::app::runtime::computer::executor::execute(&mut desktop, &request, &token);
        assert!(clicked.error.is_none(), "{:?}", clicked.error);
        assert_eq!(clicked.completed, 1);
        request.observation = clicked.observation;
        request.id = "type".into();
        request.operation = Operation::Act {
            observation: request.observation.as_ref().unwrap().id.clone(),
            actions: vec![Action::Type {
                text: "Koma42".into(),
            }],
            observe: false,
        };
        let typed =
            crate::app::runtime::computer::executor::execute(&mut desktop, &request, &token);
        assert!(typed.error.is_none(), "{:?}", typed.error);
        assert_eq!(typed.completed, 1);
        assert!(typed.png.is_empty());
        for _ in 0..100 {
            if std::fs::read_to_string(&state).unwrap_or_default() == "1\nKoma42" {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(std::fs::read_to_string(&state).unwrap(), "1\nKoma42");
        request.id = "observe".into();
        request.operation = Operation::Observe { crop: None };
        let observed =
            crate::app::runtime::computer::executor::execute(&mut desktop, &request, &token);
        assert!(observed.error.is_none(), "{:?}", observed.error);
        assert_ne!(first.png, observed.png);
        assert!(desktop.held.is_empty());
        assert!(desktop.buttons.is_empty());
    }
}
