use super::*;
use zbus::{
    blocking::{connection::Builder, Connection, Proxy},
    zvariant::OwnedObjectPath,
};
type Object = (String, OwnedObjectPath);
fn proxy<'a>(conn: &'a Connection, obj: &'a Object, interface: &'a str) -> Result<Proxy<'a>> {
    Ok(Proxy::new(conn, obj.0.as_str(), obj.1.as_str(), interface)?)
}
fn accessible<'a>(conn: &'a Connection, obj: &'a Object) -> Result<Proxy<'a>> {
    proxy(conn, obj, "org.a11y.atspi.Accessible")
}
fn extents(conn: &Connection, obj: &Object) -> Result<Rect> {
    let (x, y, w, h): (i32, i32, i32, i32) =
        proxy(conn, obj, "org.a11y.atspi.Component")?.call("GetExtents", &(0u32,))?;
    Ok(Rect {
        x: x as f64,
        y: y as f64,
        width: w as f64,
        height: h as f64,
    })
}
fn children(
    conn: &Connection,
    obj: &Object,
    limit: usize,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> Result<Vec<Object>> {
    let p = accessible(conn, obj)?;
    let count: i32 = p.get_property("ChildCount")?;
    let mut result = Vec::new();
    for index in 0..(count.max(0) as usize).min(limit) {
        ensure!(
            Instant::now() < deadline && !cancelled.load(Ordering::SeqCst),
            "accessibility deadline exceeded"
        );
        result.push(p.call("GetChildAtIndex", &(index as i32,))?);
    }
    Ok(result)
}
pub fn extract(obs: &Observation, cancelled: &AtomicBool) -> Result<Vec<Element>> {
    let deadline = Instant::now() + Duration::from_millis(750);
    let session = Builder::session()?
        .method_timeout(Duration::from_millis(100))
        .build()?;
    let bus = Proxy::new(&session, "org.a11y.Bus", "/org/a11y/bus", "org.a11y.Bus")?;
    let address: String = bus.call("GetAddress", &())?;
    let conn = Builder::address(address.as_str())?
        .method_timeout(Duration::from_millis(100))
        .build()?;
    let root = (
        "org.a11y.atspi.Registry".into(),
        OwnedObjectPath::try_from("/org/a11y/atspi/accessible/root")?,
    );
    let apps = children(&conn, &root, 128, deadline, cancelled)?;
    let expected: u32 = obs
        .window
        .id
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("missing window process identity"))?
        .1
        .parse()?;
    ensure!(expected != 0, "window process identity unavailable");
    let dbus = Proxy::new(
        &conn,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )?;
    let mut candidates = vec![];
    for app in apps.into_iter().take(128) {
        ensure!(
            Instant::now() < deadline && !cancelled.load(Ordering::SeqCst),
            "accessibility deadline exceeded"
        );
        let pid: u32 = dbus.call("GetConnectionUnixProcessID", &(app.0.as_str(),))?;
        if pid != expected {
            continue;
        }
        let children = children(&conn, &app, 64, deadline, cancelled)?;
        for window in children.into_iter().take(64) {
            ensure!(Instant::now() < deadline, "accessibility deadline exceeded");
            let p = accessible(&conn, &window)?;
            let name: String = p.get_property("Name")?;
            if name == obs.window.title {
                if let Ok(bounds) = extents(&conn, &window) {
                    let w = obs.window.geometry;
                    if bounds.contains(w.x + w.width / 2.0, w.y + w.height / 2.0) {
                        candidates.push(window.clone());
                    }
                }
            }
        }
    }
    ensure!(
        candidates.len() == 1,
        "selected window cannot be uniquely matched in accessibility tree"
    );
    let mut queue: std::collections::VecDeque<_> = candidates.into_iter().map(|o| (o, 0)).collect();
    let mut output = vec![];
    let mut visited = std::collections::HashSet::new();
    while let Some((obj, depth)) = queue.pop_front() {
        ensure!(
            Instant::now() < deadline && !cancelled.load(Ordering::SeqCst),
            "accessibility deadline exceeded"
        );
        if !visited.insert(obj.clone()) || visited.len() > 256 || depth > 12 {
            continue;
        }
        let p = accessible(&conn, &obj)?;
        let role: String = p.call("GetRoleName", &())?;
        // Never query protected text/value interfaces, or traverse their children.
        if role.to_ascii_lowercase().contains("password") {
            continue;
        }
        let states: Vec<u32> = p.call("GetState", &())?;
        let has = |bit: u32| {
            states
                .get((bit / 32) as usize)
                .is_some_and(|v| v & (1 << (bit % 32)) != 0)
        };
        if !has(25) || !has(30) {
            continue;
        }
        if let Ok(screen) = extents(&conn, &obj) {
            let t = &obs.transform;
            let bounds = Rect {
                x: (screen.x - t.desktop.x) * t.width as f64 / t.desktop.width,
                y: (screen.y - t.desktop.y) * t.height as f64 / t.desktop.height,
                width: screen.width * t.width as f64 / t.desktop.width,
                height: screen.height * t.height as f64 / t.desktop.height,
            };
            if bounds.valid()
                && bounds.x >= 0.0
                && bounds.y >= 0.0
                && bounds.x + bounds.width <= t.width as f64
                && bounds.y + bounds.height <= t.height as f64
            {
                let label: String = p.get_property("Name")?;
                output.push(Element {
                    id: format!("{}:ax:{}", obs.id, output.len()),
                    source: "accessibility".into(),
                    label: label.chars().take(256).collect(),
                    role: role.chars().take(80).collect(),
                    bounds,
                    enabled: has(8) && has(24),
                    selected: has(23),
                    focused: has(12),
                    confidence: None,
                });
            }
        }
        let children = children(
            &conn,
            &obj,
            256usize.saturating_sub(visited.len() + queue.len()),
            deadline,
            cancelled,
        )?;
        for child in children
            .into_iter()
            .take(256usize.saturating_sub(visited.len() + queue.len()))
        {
            queue.push_back((child, depth + 1));
        }
    }
    Ok(output)
}
