//! Local X11 desktop adapter; no shell commands or polling capture loop.
use super::*;
use crate::app::runtime::computer::executor::Desktop;
use anyhow::{bail, ensure, Result};
use std::{
    ffi::{CStr, CString},
    os::raw::{c_int, c_short, c_ulong},
    ptr,
};
use x11_dl::{xfixes, xlib, xrandr, xtest};

// Xlib's default error handler exits the process for stale XIDs. Preserve the
// host's handler for other displays and catch errors on this worker's display.
type ErrorHandler = unsafe extern "C" fn(*mut xlib::Display, *mut xlib::XErrorEvent) -> c_int;
static PREVIOUS: std::sync::OnceLock<Option<ErrorHandler>> = std::sync::OnceLock::new();
thread_local! { static OWN: std::cell::Cell<*mut xlib::Display> = const {std::cell::Cell::new(ptr::null_mut())}; static ERROR: std::cell::Cell<bool> = const {std::cell::Cell::new(false)}; }
fn arrow_points() -> [xlib::XPoint; 8] {
    [
        (6, 6),
        (6, 84),
        (27, 63),
        (42, 96),
        (57, 87),
        (39, 57),
        (66, 57),
        (6, 6),
    ]
    .map(|(x, y)| xlib::XPoint {
        x: x as c_short,
        y: y as c_short,
    })
}
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
                "X11: a screen share includes overlapping windows. An application share is that window only; input brings it forward and the real pointer glides to the point. An enlarged arrow is drawn over that pointer during the glide and is unmapped before the screenshot. Unicode typing uses a temporary keycode and restores it. The preview cannot be excluded from live X11 frames; it is hidden for model observations and input. AT-SPI metadata is available only for application observations; use screenshot coordinates.".into()
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
    input_started: bool,
    target: Option<Window>,
    temporary_key: Option<(u8, Vec<c_ulong>, c_ulong)>,
    arrow: c_ulong,
    arrow_gc: xlib::GC,
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
            input_started: false,
            target: None,
            temporary_key: None,
            arrow: 0,
            arrow_gc: ptr::null_mut(),
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
    fn pointer_pos(&self) -> Result<(f64, f64)> {
        let (mut root, mut child, mut root_x, mut root_y, mut win_x, mut win_y, mut mask) =
            (0, 0, 0, 0, 0, 0, 0);
        let ok = unsafe {
            (self.x.XQueryPointer)(
                self.display,
                self.root,
                &mut root,
                &mut child,
                &mut root_x,
                &mut root_y,
                &mut win_x,
                &mut win_y,
                &mut mask,
            )
        };
        ensure!(ok != 0, "X11 pointer unavailable");
        Ok((f64::from(root_x), f64::from(root_y)))
    }
    fn motion(&mut self, x: f64, y: f64) -> Result<()> {
        let t = self
            .test
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("XTEST unavailable"))?;
        unsafe {
            (t.XTestFakeMotionEvent)(self.display, -1, x.round() as i32, y.round() as i32, 0);
        }
        self.sync()
    }
    /// Click-through enlarged arrow. The bounding shape is the arrow; the input
    /// shape is empty, so the click falls through to the window underneath.
    /// Unmapped before `input` returns, which is before the model capture.
    fn ensure_arrow(&mut self) -> bool {
        if self.arrow != 0 {
            return true;
        }
        let Some(fixes) = xfixes::Xlib::open().ok() else {
            return false;
        };
        const SIZE: u32 = 108;
        const SHAPE_BOUNDING: c_int = 0;
        const SHAPE_INPUT: c_int = 2;
        let screen = unsafe { (self.x.XDefaultScreen)(self.display) };
        let mut attr: xlib::XSetWindowAttributes = unsafe { std::mem::zeroed() };
        attr.override_redirect = 1;
        attr.background_pixel = unsafe { (self.x.XWhitePixel)(self.display, screen) };
        attr.border_pixel = 0;
        attr.backing_store = xlib::Always;
        let window = unsafe {
            (self.x.XCreateWindow)(
                self.display,
                self.root,
                -(SIZE as c_int),
                -(SIZE as c_int),
                SIZE,
                SIZE,
                0,
                xlib::CopyFromParent,
                xlib::InputOutput as u32,
                ptr::null_mut(),
                xlib::CWOverrideRedirect
                    | xlib::CWBackPixel
                    | xlib::CWBorderPixel
                    | xlib::CWBackingStore,
                &mut attr,
            )
        };
        if window == 0 {
            return false;
        }
        let mask = unsafe { (self.x.XCreatePixmap)(self.display, self.root, SIZE, SIZE, 1) };
        let mask_gc = unsafe { (self.x.XCreateGC)(self.display, mask, 0, ptr::null_mut()) };
        let mut points = arrow_points();
        let shaped = !mask_gc.is_null() && mask != 0;
        if shaped {
            unsafe {
                (self.x.XSetForeground)(self.display, mask_gc, 0);
                (self.x.XFillRectangle)(self.display, mask, mask_gc, 0, 0, SIZE, SIZE);
                (self.x.XSetForeground)(self.display, mask_gc, 1);
                (self.x.XFillPolygon)(
                    self.display,
                    mask,
                    mask_gc,
                    points.as_mut_ptr(),
                    points.len() as c_int,
                    xlib::Nonconvex,
                    xlib::CoordModeOrigin,
                );
                let region = (fixes.XFixesCreateRegionFromBitmap)(self.display, mask);
                (fixes.XFixesSetWindowShapeRegion)(
                    self.display,
                    window,
                    SHAPE_BOUNDING,
                    0,
                    0,
                    region,
                );
                (fixes.XFixesDestroyRegion)(self.display, region);
                let empty = (fixes.XFixesCreateRegion)(self.display, ptr::null_mut(), 0);
                (fixes.XFixesSetWindowShapeRegion)(self.display, window, SHAPE_INPUT, 0, 0, empty);
                (fixes.XFixesDestroyRegion)(self.display, empty);
                (self.x.XFreeGC)(self.display, mask_gc);
                (self.x.XFreePixmap)(self.display, mask);
            }
        }
        let gc = unsafe { (self.x.XCreateGC)(self.display, window, 0, ptr::null_mut()) };
        if gc.is_null() || !shaped {
            unsafe {
                if !shaped {
                    if !mask_gc.is_null() {
                        (self.x.XFreeGC)(self.display, mask_gc);
                    }
                    if mask != 0 {
                        (self.x.XFreePixmap)(self.display, mask);
                    }
                }
                if !gc.is_null() {
                    (self.x.XFreeGC)(self.display, gc);
                }
                (self.x.XDestroyWindow)(self.display, window);
            }
            return false;
        }
        unsafe {
            (self.x.XSetForeground)(self.display, gc, (self.x.XWhitePixel)(self.display, screen));
            (self.x.XFillPolygon)(
                self.display,
                window,
                gc,
                points.as_mut_ptr(),
                points.len() as c_int,
                xlib::Nonconvex,
                xlib::CoordModeOrigin,
            );
            (self.x.XSetForeground)(self.display, gc, (self.x.XBlackPixel)(self.display, screen));
            (self.x.XSetLineAttributes)(
                self.display,
                gc,
                4,
                xlib::LineSolid,
                xlib::CapButt,
                xlib::JoinMiter,
            );
            (self.x.XDrawLines)(
                self.display,
                window,
                gc,
                points.as_mut_ptr(),
                points.len() as c_int,
                xlib::CoordModeOrigin,
            );
        }
        self.arrow = window;
        self.arrow_gc = gc;
        true
    }
    fn place_arrow(&mut self, x: f64, y: f64) {
        if !self.ensure_arrow() {
            return;
        }
        unsafe {
            (self.x.XMoveWindow)(
                self.display,
                self.arrow,
                x.round() as c_int - 6,
                y.round() as c_int - 6,
            );
            (self.x.XMapRaised)(self.display, self.arrow);
        }
    }
    fn hide_arrow(&mut self) {
        if self.arrow != 0 {
            unsafe {
                (self.x.XUnmapWindow)(self.display, self.arrow);
            }
        }
    }
    fn glide(&mut self, to_x: f64, to_y: f64) -> Result<()> {
        let (from_x, from_y) = self.pointer_pos().unwrap_or((to_x, to_y));
        let glide = super::super::cursor_glide::samples(from_x, from_y, to_x, to_y);
        self.input_started = true;
        let mut previous = (from_x, from_y);
        for (index, step) in glide.steps.iter().enumerate() {
            ensure!(!self.cancelled.load(Ordering::SeqCst), "cancelled");
            self.motion(step[0], step[1])?;
            self.place_arrow(step[0], step[1]);
            if let Ok((px, py)) = self.pointer_pos() {
                let dx = px - step[0];
                let dy = py - step[1];
                let lx = px - previous.0;
                let ly = py - previous.1;
                // A sample that has not been applied yet still sits on the
                // previous point. Only a position away from both is the hand
                // taking the mouse.
                if dx * dx + dy * dy > 144.0 && lx * lx + ly * ly > 144.0 {
                    self.motion(to_x, to_y)?;
                    self.place_arrow(to_x, to_y);
                    break;
                }
            }
            previous = (step[0], step[1]);
            if index + 1 != glide.steps.len() {
                for _ in 0..2 {
                    ensure!(!self.cancelled.load(Ordering::SeqCst), "cancelled");
                    std::thread::sleep(std::time::Duration::from_millis(8));
                }
            }
        }
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
    fn desktop_focus(&self) -> Result<String> {
        let window = self
            .property(self.root, "_NET_ACTIVE_WINDOW")?
            .first()
            .copied()
            .unwrap_or(0);
        Ok(format!("{window}:{:?}", self.geometry(window).ok()))
    }
    fn displays(&self) -> Result<Vec<Window>> {
        let mut sources = vec![];
        if let Ok(randr) = xrandr::Xrandr::open() {
            let (mut major, mut minor) = (0, 0);
            let ok = unsafe { (randr.XRRQueryVersion)(self.display, &mut major, &mut minor) };
            if ok != 0 && (major, minor) >= (1, 5) {
                let mut count = 0;
                let monitors =
                    unsafe { (randr.XRRGetMonitors)(self.display, self.root, 1, &mut count) };
                if !monitors.is_null() {
                    let _free =
                        scopeguard::guard(monitors, |m| unsafe { (randr.XRRFreeMonitors)(m) });
                    ensure!((0..=256).contains(&count), "Invalid monitor count");
                    for m in unsafe { std::slice::from_raw_parts(monitors, count as usize) } {
                        let geometry = Rect {
                            x: m.x as f64,
                            y: m.y as f64,
                            width: m.width as f64,
                            height: m.height as f64,
                        };
                        if !geometry.valid() {
                            continue;
                        }
                        sources.push(Window {
                            id: format!("display:{}:{}", self.root, m.name),
                            application: "Desktop".into(),
                            title: format!(
                                "Display {}{}",
                                sources.len() + 1,
                                if m.primary != 0 { " · Main" } else { "" }
                            ),
                            geometry,
                            focused: true,
                            focus: Some(self.desktop_focus()?),
                        });
                    }
                }
            }
        }
        if sources.is_empty() {
            sources.push(Window {
                id: format!("display:{}:root", self.root),
                application: "Desktop".into(),
                title: "Entire X11 desktop (all monitors)".into(),
                geometry: self.geometry(self.root)?,
                focused: true,
                focus: Some(self.desktop_focus()?),
            });
        }
        self.sync()?;
        Ok(sources)
    }
    fn display_source(&self, id: &str) -> Result<Window> {
        self.displays()?
            .into_iter()
            .find(|w| w.id == id)
            .ok_or_else(|| anyhow::anyhow!("Shared display disconnected; select a display again"))
    }
    fn guard_keyboard(&self) -> Result<()> {
        let target = self
            .target
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no input target"))?;
        if !target.id.starts_with("display:") {
            return Ok(());
        }
        // Title, bounds, and window identity are not a batch gate. A predicted
        // chord or type keeps going when focus moves to another window on this
        // display. macOS and Windows use the same center check.
        let window = self
            .property(self.root, "_NET_ACTIVE_WINDOW")?
            .first()
            .copied()
            .unwrap_or(self.root);
        let r = self.geometry(window)?;
        ensure!(target.geometry.contains(r.x + r.width / 2.0, r.y + r.height / 2.0),
            "Keyboard focus is outside the shared display; click a visible window and observe again");
        Ok(())
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
                    "The application window is still covered; observe again"
                );
            }
        }
        Ok(())
    }
    fn guard_input(&self) -> Result<()> {
        ensure!(!self.cancelled.load(Ordering::SeqCst), "cancelled");
        self.wait_input_idle()?;
        let target = self
            .target
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no input target"))?;
        if target.id.starts_with("display:") {
            // Still connected. Position and size are taken from the live
            // transform, so a move or resize does not cancel the batch.
            self.display_source(&target.id)?;
        } else {
            let xid = self.xid(&target.id)?;
            self.raise_window(xid)?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(300);
            loop {
                if self.property(self.root, "_NET_ACTIVE_WINDOW")?.first() == Some(&xid) {
                    break;
                }
                ensure!(
                    std::time::Instant::now() < deadline,
                    "Could not bring the application window forward; observe again"
                );
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let live = self.geometry(xid)?;
            self.unobstructed(xid, live)?;
        }
        Ok(())
    }
    /// Bring one window forward so a real click lands in it. The pointer then
    /// glides to the programmed point, with an enlarged arrow drawn over it.
    fn raise_window(&self, xid: c_ulong) -> Result<()> {
        unsafe {
            (self.x.XRaiseWindow)(self.display, xid);
        }
        let mut event: xlib::XEvent = unsafe { std::mem::zeroed() };
        unsafe {
            event.client_message.type_ = xlib::ClientMessage;
            event.client_message.display = self.display;
            event.client_message.window = xid;
            event.client_message.message_type = self.atom("_NET_ACTIVE_WINDOW")?;
            event.client_message.format = 32;
            event.client_message.data.set_long(0, 1);
            event
                .client_message
                .data
                .set_long(1, xlib::CurrentTime as i64);
            (self.x.XSendEvent)(
                self.display,
                self.root,
                0,
                xlib::SubstructureRedirectMask | xlib::SubstructureNotifyMask,
                &mut event,
            );
            (self.x.XSetInputFocus)(self.display, xid, xlib::RevertToParent, xlib::CurrentTime);
            (self.x.XFlush)(self.display);
        }
        self.sync()?;
        Ok(())
    }
    fn wait_input_idle(&self) -> Result<()> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
        loop {
            ensure!(!self.cancelled.load(Ordering::SeqCst), "cancelled");
            let mut keys = [0 as std::os::raw::c_char; 32];
            unsafe {
                (self.x.XQueryKeymap)(self.display, keys.as_mut_ptr());
            }
            let keys_idle = (0..256u32).all(|key| {
                keys[key as usize / 8] as u8 & (1 << (key % 8)) == 0 || self.held.contains(&key)
            });
            let mut state: xlib::XkbStateRec = unsafe { std::mem::zeroed() };
            let status = unsafe { (self.x.XkbGetState)(self.display, 0x0100, &mut state) };
            ensure!(status == 0, "Cannot query keyboard state");
            if keys_idle && state.ptr_buttons == 0 {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(InputBusy {
                    input_started: self.input_started,
                }
                .into());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    /// Character typing uses group-0 keycodes. Another XKB group, Caps Lock, or a
    /// latched modifier would change the character. Key chords, pointer moves,
    /// clicks, and scrolls do not consult this. macOS and Windows inject text
    /// as Unicode and do not have this gate. The message must not contain
    /// "observe again" or the model treats a layout mismatch as a desktop change.
    fn guard_layout(&self) -> Result<()> {
        let mut state: xlib::XkbStateRec = unsafe { std::mem::zeroed() };
        let status = unsafe { (self.x.XkbGetState)(self.display, 0x0100, &mut state) };
        ensure!(status == 0, "Cannot query keyboard state");
        ensure!(
            state.group == 0
                && state.latched_mods == 0
                && u32::from(state.locked_mods) & !xlib::Mod2Mask == 0,
            "Release locked/sticky modifiers and use the primary keyboard group before typing"
        );
        Ok(())
    }
    fn key(&mut self, code: u32, down: bool) -> Result<()> {
        let t = self
            .test
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("XTEST unavailable"))?;
        if down {
            self.guard_input()?;
            self.guard_keyboard()?;
            ensure!(!self.cancelled.load(Ordering::SeqCst), "cancelled");
            self.held.push(code);
        }
        unsafe {
            self.input_started = true;
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
        self.input_started = true;
        unsafe {
            (t.XTestFakeButtonEvent)(self.display, button, 1, 0);
            (t.XTestFakeButtonEvent)(self.display, button, 0, 0);
        }
        self.sync()?;
        self.buttons.clear();
        Ok(())
    }
    fn chord_codes(&self, keys: &[String]) -> Result<Vec<u32>> {
        let mut codes = Vec::with_capacity(keys.len());
        for k in keys {
            let mut chars = k.chars();
            if let (Some(c), None) = (chars.next(), chars.next()) {
                if c.is_ascii_graphic() && !c.is_ascii_alphanumeric() && c != ' ' {
                    let (code, shifted) = self.mapped(c as c_ulong)?;
                    if shifted {
                        let shift = self.mapped(0xffe1)?.0;
                        if !codes.contains(&shift) {
                            codes.push(shift);
                        }
                    }
                    codes.push(code);
                    continue;
                }
            }
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
                "PageUp" => "Prior",
                "PageDown" => "Next",
                "ArrowLeft" => "Left",
                "ArrowRight" => "Right",
                "ArrowUp" => "Up",
                "ArrowDown" => "Down",
                other => other,
            })?;
            let symbol = unsafe { (self.x.XStringToKeysym)(name.as_ptr()) };
            ensure!(symbol != 0, "unknown X11 key name");
            codes.push(self.mapped(symbol)?.0);
        }
        Ok(codes)
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
    fn reserve_unicode_key(&mut self) -> Result<()> {
        if self.temporary_key.is_some() {
            return Ok(());
        }
        self.guard_input()?;
        let (mut first, mut last, mut per_key) = (0, 0, 0);
        unsafe {
            (self.x.XDisplayKeycodes)(self.display, &mut first, &mut last);
        }
        let raw = unsafe {
            (self.x.XGetKeyboardMapping)(self.display, first as u8, last - first + 1, &mut per_key)
        };
        ensure!(
            !raw.is_null() && per_key > 0,
            "Cannot inspect X11 keyboard map"
        );
        let map = scopeguard::guard(raw, |p| unsafe {
            (self.x.XFree)(p.cast());
        });
        let entries =
            unsafe { std::slice::from_raw_parts(*map, ((last - first + 1) * per_key) as usize) };
        for key in (first..=last).rev() {
            let offset = ((key - first) * per_key) as usize;
            let symbols = &entries[offset..offset + per_key as usize];
            if symbols.iter().all(|v| *v == 0) {
                self.temporary_key = Some((key as u8, symbols.to_vec(), 0));
                return Ok(());
            }
        }
        bail!("No unused X11 keycode is available for Unicode typing");
    }
    fn unicode_key(&mut self, symbol: c_ulong) -> Result<u32> {
        self.reserve_unicode_key()?;
        self.guard_input()?;
        let (key, original, current) = self
            .temporary_key
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Unicode key allocation failed"))?;
        let mut replacement = vec![symbol; original.len()];
        unsafe {
            (self.x.XChangeKeyboardMapping)(
                self.display,
                i32::from(*key),
                replacement.len() as i32,
                replacement.as_mut_ptr(),
                1,
            );
        }
        *current = symbol;
        let code = u32::from(*key);
        self.sync()?;
        // Let the target process MappingNotify before delivering the key event.
        std::thread::sleep(std::time::Duration::from_millis(20));
        Ok(code)
    }
}
impl X11 {
    pub fn capture_source(
        &mut self,
        window: &Window,
        region: Option<Rect>,
        preview: bool,
    ) -> Result<(Transform, Vec<u8>)> {
        let display_source = window.id.starts_with("display:");
        let xid = if display_source {
            ensure!(
                self.display_source(&window.id)?.geometry == window.geometry,
                "Display geometry changed; observe again"
            );
            self.root
        } else {
            let xid = self.xid(&window.id)?;
            if !preview {
                self.raise_window(xid)?;
            }
            self.unobstructed(xid, self.geometry(xid)?)?;
            xid
        };
        let desktop = if let Some(region) = region {
            ensure!(
                display_source
                    && region.valid()
                    && region.x >= window.geometry.x
                    && region.y >= window.geometry.y
                    && region.x + region.width <= window.geometry.x + window.geometry.width
                    && region.y + region.height <= window.geometry.y + window.geometry.height,
                "Region outside shared display"
            );
            Rect {
                x: region.x.floor(),
                y: region.y.floor(),
                width: (region.x + region.width).ceil() - region.x.floor(),
                height: (region.y + region.height).ceil() - region.y.floor(),
            }
        } else {
            window.geometry
        };
        let (left, top) = if display_source {
            (desktop.x as i32, desktop.y as i32)
        } else {
            (0, 0)
        };
        let w = desktop.width as u32;
        let h = desktop.height as u32;
        ensure!(
            u64::from(w) * u64::from(h) <= 67_108_864,
            "Display exceeds the 64-megapixel native capture budget"
        );
        let raw =
            unsafe { (self.x.XGetImage)(self.display, xid, left, top, w, h, !0, xlib::ZPixmap) };
        let guard = scopeguard::guard(raw, |image| unsafe {
            if !image.is_null() {
                (self.x.XDestroyImage)(image);
            }
        });
        self.sync()?;
        ensure!(!raw.is_null(), "capture failed");
        let (width, height) = capture_size(w, h, preview);
        let mut image = image::RgbImage::new(width, height);
        let masks = unsafe { [(*raw).red_mask, (*raw).green_mask, (*raw).blue_mask] };
        ensure!(masks.iter().all(|v| *v != 0), "unsupported X11 visual");
        // Sample the native XImage into a bounded buffer before PNG/OCR/IPC.
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            if x == 0 {
                ensure!(!self.cancelled.load(Ordering::SeqCst), "cancelled");
            }
            let sx = ((f64::from(x) + 0.5) * f64::from(w) / f64::from(width) - 0.5)
                .clamp(0.0, f64::from(w - 1));
            let sy = ((f64::from(y) + 0.5) * f64::from(h) / f64::from(height) - 0.5)
                .clamp(0.0, f64::from(h - 1));
            let (x0, y0) = (sx as u32, sy as u32);
            let mut rgb = [0.0; 3];
            for (px, py, weight) in [
                (x0, y0, (1.0 - sx.fract()) * (1.0 - sy.fract())),
                ((x0 + 1).min(w - 1), y0, sx.fract() * (1.0 - sy.fract())),
                (x0, (y0 + 1).min(h - 1), (1.0 - sx.fract()) * sy.fract()),
                (
                    (x0 + 1).min(w - 1),
                    (y0 + 1).min(h - 1),
                    sx.fract() * sy.fract(),
                ),
            ] {
                let value = unsafe { (self.x.XGetPixel)(*guard, px as i32, py as i32) };
                for (i, mask) in masks.iter().enumerate() {
                    let shift = mask.trailing_zeros();
                    rgb[i] += (((value & mask) >> shift) * 255 / (mask >> shift)) as f64 * weight;
                }
            }
            *pixel = image::Rgb(rgb.map(|v| v.round() as u8));
        }
        let mut png = std::io::Cursor::new(vec![]);
        image.write_to(&mut png, image::ImageFormat::Png)?;
        ensure!(
            png.get_ref().len() <= 20 * 1024 * 1024,
            "captured PNG exceeds the 20 MiB observation limit"
        );
        Ok((
            Transform {
                desktop,
                width,
                height,
            },
            png.into_inner(),
        ))
    }
}
impl Desktop for X11 {
    fn windows(&mut self) -> Result<Vec<Window>> {
        let mut sources = self.displays()?;
        for xid in self
            .property(self.root, "_NET_CLIENT_LIST")?
            .into_iter()
            .take(256)
        {
            if self.cancelled.load(Ordering::SeqCst) {
                bail!("cancelled");
            }
            let Some(pid) = self
                .property(xid, "_NET_WM_PID")
                .ok()
                .and_then(|v| v.first().copied())
            else {
                continue;
            };
            if pid == 0 || pid == u64::from(std::process::id()) as c_ulong {
                continue;
            }
            if let Ok(window) = self.inspect(&format!("{xid}:{pid}")) {
                sources.push(window);
            }
        }
        Ok(sources)
    }
    fn select(&mut self, id: &str) -> Result<Window> {
        self.inspect(id)
    }
    fn inspect(&mut self, id: &str) -> Result<Window> {
        if id.starts_with("display:") {
            let result = self.display_source(id)?;
            self.target = Some(result.clone());
            return Ok(result);
        }
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
            focus: None,
        };
        self.target = Some(result.clone());
        Ok(result)
    }
    fn capture(&mut self, window: &Window) -> Result<(Transform, Vec<u8>)> {
        self.capture_source(window, None, false)
    }
    fn capture_region(&mut self, window: &Window, region: Rect) -> Result<(Transform, Vec<u8>)> {
        self.capture_source(window, Some(region), false)
    }
    fn validate_input(&mut self, action: &Action) -> Result<()> {
        self.input_started = false;
        self.guard_input()?;
        if matches!(action, Action::Type { .. } | Action::Key { .. }) {
            self.guard_keyboard()?;
        }
        if matches!(action, Action::Type { .. }) {
            self.guard_layout()?;
        }
        if let Action::Key { keys } = action {
            self.chord_codes(keys)
                .map_err(|e| anyhow::anyhow!("key is not available: {e}"))?;
        }
        Ok(())
    }
    fn input(&mut self, action: &Action, transform: &Transform) -> Result<()> {
        self.input_started = false;
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
                ensure!(
                    self.target
                        .as_ref()
                        .is_some_and(|w| w.geometry.contains(x, y)),
                    "Input outside shared source"
                );
                let played = (|| -> Result<()> {
                    self.glide(x, y)?;
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
                    Ok(())
                })();
                self.hide_arrow();
                let _ = self.sync();
                played?;
            }
            Action::Type { text } => {
                self.guard_layout()?;
                let symbols = text
                    .chars()
                    .map(|c| match c {
                        '\n' | '\r' => 0xff0d,
                        '\t' => 0xff09,
                        c if (c as u32) < 256 => c as c_ulong,
                        c => 0x01000000 | c as c_ulong,
                    })
                    .collect::<Vec<_>>();
                if symbols.iter().any(|s| self.mapped(*s).is_err()) {
                    self.reserve_unicode_key()?;
                }
                let shift = self.mapped(0xffe1)?.0;
                for symbol in symbols {
                    let (key, shifted, temporary) = match self.mapped(symbol) {
                        Ok((key, shifted))
                            if self
                                .temporary_key
                                .as_ref()
                                .is_none_or(|(reserved, _, _)| key != u32::from(*reserved)) =>
                        {
                            (key, shifted, false)
                        }
                        // Xlib's keysym cache may still describe the previous
                        // temporary mapping. Always remap that code and allow
                        // the target to consume its event before restoring it.
                        _ => (self.unicode_key(symbol)?, false, true),
                    };
                    if shifted {
                        self.key(shift, true)?;
                    }
                    self.key(key, true)?;
                    self.key(key, false)?;
                    if shifted {
                        self.key(shift, false)?;
                    }
                    if temporary {
                        std::thread::sleep(std::time::Duration::from_millis(30));
                    }
                }
            }
            Action::Key { keys } => {
                let codes = self
                    .chord_codes(keys)
                    .map_err(|e| anyhow::anyhow!("key is not available: {e}"))?;
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
        self.hide_arrow();
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
        if let Some((key, mut original, symbol)) = self.temporary_key.take() {
            // Do not overwrite a layout change made by the user during control.
            let mut count = 0;
            let current = unsafe { (self.x.XGetKeyboardMapping)(self.display, key, 1, &mut count) };
            let ours = !current.is_null() && count > 0 && unsafe { *current == symbol };
            if !current.is_null() {
                unsafe {
                    (self.x.XFree)(current.cast());
                }
            }
            if ours {
                unsafe {
                    (self.x.XChangeKeyboardMapping)(
                        self.display,
                        i32::from(key),
                        original.len() as i32,
                        original.as_mut_ptr(),
                        1,
                    );
                    (self.x.XSync)(self.display, 0);
                }
            }
        }
    }
}
impl Drop for X11 {
    fn drop(&mut self) {
        self.release();
        unsafe {
            if !self.arrow_gc.is_null() {
                (self.x.XFreeGC)(self.display, self.arrow_gc);
            }
            if self.arrow != 0 {
                (self.x.XDestroyWindow)(self.display, self.arrow);
            }
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
        let app_click =
            crate::app::runtime::computer::executor::execute(&mut desktop, &request, &token);
        assert!(
            !app_click.requires_screen,
            "an application window accepts the click: {:?}",
            app_click.error
        );
        let app_clicks = u32::from(app_click.error.is_none() && app_click.completed > 0);
        let application = request.observation.as_ref().unwrap().window.geometry;
        let screen = desktop
            .windows()
            .unwrap()
            .into_iter()
            .find(|w| {
                is_screen(&w.id)
                    && w.geometry
                        .contains(application.x + 40.0, application.y + 40.0)
            })
            .unwrap();
        request.operation = Operation::Select {
            window: screen.id,
            generation: "g".into(),
        };
        let shared =
            crate::app::runtime::computer::executor::execute(&mut desktop, &request, &token);
        assert!(shared.error.is_none(), "{:?}", shared.error);
        request.observation = shared.observation;
        let obs = request.observation.as_ref().unwrap();
        request.operation = Operation::Act {
            observation: obs.id.clone(),
            actions: vec![Action::Click {
                x: Some(
                    (application.x + 40.0 - obs.transform.desktop.x)
                        * f64::from(obs.transform.width)
                        / obs.transform.desktop.width,
                ),
                y: Some(
                    (application.y + 40.0 - obs.transform.desktop.y)
                        * f64::from(obs.transform.height)
                        / obs.transform.desktop.height,
                ),
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
        assert_eq!(
            std::fs::read_to_string(&state).unwrap(),
            format!("{}\nKoma42", 1 + app_clicks)
        );
        request.id = "observe".into();
        request.operation = Operation::Observe {
            crop: None,
            region: None,
        };
        let observed =
            crate::app::runtime::computer::executor::execute(&mut desktop, &request, &token);
        assert!(observed.error.is_none(), "{:?}", observed.error);
        assert_ne!(first.png, observed.png);
        assert!(desktop.held.is_empty());
        assert!(desktop.buttons.is_empty());
    }
}
