//! tui-demo-daemon: the reference sample for `contributes.tui_screens` — koma's
//! TERMINAL UI rendering a full-screen view an extension drives, over the TUI SCREEN
//! PROTOCOL v1 (see `src-agent/src/app/ext/screen.rs`'s module doc for the wire
//! contract this sample mirrors exactly). Run `cargo run -p tui-demo-daemon` to see
//! the demo transcript; install it for real and open the "TUI Demo" row from an
//! extension's `/extension` detail view to see the live screen.
//!
//! It contributes exactly one screen (`id: "demo"`) with a counter, a menu
//! (increment / reset / refresh / close), and a background thread that pushes a
//! fresh screen every 5 seconds so the "live fold" half of the protocol
//! (`panel.push { kind: "tui-screen" }`) has something to show too — the counter can
//! change under a user's cursor even without them pressing a key.
//!
//! # Threading model — narrower than `fleet-board-daemon`'s, and worth contrasting
//!
//! The TUI SCREEN PROTOCOL replies to `tui-open`/`tui-select` SYNCHRONOUSLY, as the
//! `Result` of the `panel.msg` invoke — the exact shape `on_invoke` already returns.
//! Answering it needs nothing from koma: the whole reply is built from this
//! extension's OWN counter state. So, unlike `fleet-board-daemon` (which has to defer
//! every `on_invoke` to a driver thread because it needs to call `koma.call` to
//! actually spawn something), `on_invoke` here never touches a `Koma` handle at all —
//! there is no deadlock risk to route around for the request/reply half of this
//! sample.
//!
//! The one thing that DOES need a live `Koma` handle is the periodic push — that's
//! `Koma::panel_push`, fire-and-forget, sent from the driver thread on a plain
//! `std::thread::sleep` loop. `on_invoke` (mutating state on the host-loop thread) and
//! the driver's ticker (reading state on its own thread) meet at one shared
//! `std::sync::Mutex<State>`, parked in a `std::sync::OnceLock` for the same reason
//! `fleet-board-daemon` parks its `CMD_RX` there: `DaemonDemo::driver` is a bare
//! `fn(&mut Koma)`, a function pointer with no captures, so there is no other way to
//! hand it a reference this file owns.

use koma_extension::{run_daemon, DaemonDemo, Extension, ExtensionManifest, Koma};
use serde_json::{json, Value};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Must match the `id` this extension declares under `contributes.tui_screens` in
/// `manifest.json` — koma passes it back as `panelId` on every `panel.msg` invoke for
/// this screen, and it's the `panel_id` a `panel.push` targets to land in that SAME
/// open screen.
const SCREEN_ID: &str = "demo";

/// How often the driver thread pushes a fresh screen in HOST mode. Demo mode ignores
/// this and pushes exactly once (see `drive` below) so `cargo run` doesn't hang.
const TICK_INTERVAL: Duration = Duration::from_secs(5);

/// The whole of this extension's state: a counter, a label for the last menu action
/// that touched it, and the clock the "uptime" kv row is read off. Shared between
/// `on_invoke` (host-loop thread) and the driver's ticker (its own thread) through
/// [`state`]'s `Mutex`.
struct State {
    counter: i64,
    last_action: String,
    started_at: Instant,
}

/// The single shared `State`, lazily created on first access (by whichever of
/// `on_invoke` or the driver thread gets there first — both go through this same
/// accessor, so there is only ever one `State` for the process's whole life).
fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(State {
        counter: 0,
        last_action: "-".to_string(),
        started_at: Instant::now(),
    }))
}

/// Build the `Screen` model (`{ title, body:[Node], footer }`) for the current
/// `State` — the SAME shape whether it's going out as an `on_invoke` reply
/// (`{"screen": ...}`) or a `panel.push` (`{"kind":"tui-screen","screen": ...}`), so
/// the two paths can never drift into showing different things for the same state.
fn build_screen(st: &State) -> Value {
    let uptime_secs = st.started_at.elapsed().as_secs();
    json!({
        "title": "TUI Demo",
        "body": [
            { "t": "kv", "k": "counter", "v": st.counter.to_string() },
            { "t": "kv", "k": "last action", "v": st.last_action.clone() },
            { "t": "kv", "k": "uptime", "v": format!("{uptime_secs}s") },
            { "t": "divider" },
            { "t": "menu", "items": [
                { "id": "increment", "label": "+1 counter" },
                { "id": "reset", "label": "reset counter" },
                { "id": "refresh", "label": "refresh" },
                { "id": "close", "label": "close" }
            ] }
        ],
        "footer": "server-driven demo screen"
    })
}

struct TuiDemo;

impl Extension for TuiDemo {
    fn manifest(&self) -> ExtensionManifest {
        serde_json::from_str(include_str!("../manifest.json")).expect("manifest.json is valid")
    }

    /// koma invokes this for every screen message: `tui-open` when the screen is
    /// opened, `tui-select` on Enter over a menu row, `tui-close` on Esc/exit — all
    /// three riding the same `panel.msg` verb `{ "panelId": "demo", "payload": {
    /// "kind": ... } }` a GUI panel would use (see the module doc comment above and
    /// `src-agent/src/app/ext/screen.rs`). Every branch here only ever touches the
    /// shared `State` mutex — never `Koma` — so there is nothing to defer to a
    /// driver thread.
    fn on_invoke(&mut self, method: &str, params: Value) -> Value {
        if method != "panel.msg" {
            return json!({ "error": format!("unknown method: {method}") });
        }
        let payload = params.get("payload");
        let kind = payload.and_then(|p| p.get("kind")).and_then(|k| k.as_str()).unwrap_or("");
        match kind {
            "tui-open" => {
                // Just show the current state — opening the screen isn't itself a
                // menu action, so `last_action` is left untouched.
                let st = state().lock().expect("tui-demo state mutex poisoned");
                json!({ "screen": build_screen(&st) })
            }
            "tui-select" => {
                let item = payload.and_then(|p| p.get("item")).and_then(|i| i.as_str()).unwrap_or("");
                match item {
                    "increment" => {
                        let mut st = state().lock().expect("tui-demo state mutex poisoned");
                        st.counter += 1;
                        st.last_action = "increment".to_string();
                        json!({ "screen": build_screen(&st) })
                    }
                    "reset" => {
                        let mut st = state().lock().expect("tui-demo state mutex poisoned");
                        st.counter = 0;
                        st.last_action = "reset".to_string();
                        json!({ "screen": build_screen(&st) })
                    }
                    "refresh" => {
                        let mut st = state().lock().expect("tui-demo state mutex poisoned");
                        st.last_action = "refresh".to_string();
                        json!({ "screen": build_screen(&st) })
                    }
                    // Tell koma to pop back to the extension's detail view — the
                    // menu's own "close" row, distinct from an `Esc`-driven
                    // `tui-close` below (same destination, different trigger).
                    "close" => json!({ "close": true }),
                    other => json!({ "error": format!("unknown menu item: {other}") }),
                }
            }
            // Best-effort courtesy notice that the screen closed (Esc/exit); koma
            // ignores this reply either way, so an empty object is enough.
            "tui-close" => json!({}),
            other => json!({ "error": format!("unknown panel.msg kind: {other}") }),
        }
    }
}

/// Runs on its own thread with a live `Koma` handle (host mode) or a demo stub (demo
/// mode) — see `koma_extension::sdk::run_daemon`. This is the ONLY function in this
/// sample that calls `Koma::panel_push`, and the only reader of `State` off the
/// host-loop thread.
fn drive(koma: &mut Koma) {
    // Host mode: keep ticking for as long as the daemon runs. Demo mode has no real
    // clients to fold a live update into and nothing else scripted happens after
    // this, so push once immediately and return instead of sleeping 5s for no
    // audience (mirrors `fleet-board-daemon`'s demo-vs-host split on the same env
    // var check).
    if std::env::var_os("KOMA_EXT_SOCKET").is_some() {
        loop {
            std::thread::sleep(TICK_INTERVAL);
            push_tick(koma);
        }
    } else {
        push_tick(koma);
    }
}

/// Read the current `State` and push it as a live screen update — the
/// `{ "kind": "tui-screen", "screen": <Screen> }` payload `panel.push` carries per the
/// TUI SCREEN PROTOCOL, folded into every open "demo" screen client-side.
fn push_tick(koma: &mut Koma) {
    let screen = {
        let st = state().lock().expect("tui-demo state mutex poisoned");
        build_screen(&st)
    };
    koma.panel_push(SCREEN_ID, json!({ "kind": "tui-screen", "screen": screen }));
}

fn main() {
    run_daemon(
        TuiDemo,
        DaemonDemo {
            // Simulates koma opening the screen: a real `/extension` detail view
            // selecting this screen's row sends exactly this `panel.msg` shape.
            invoke: Some((
                "panel.msg".to_string(),
                json!({ "panelId": SCREEN_ID, "payload": { "kind": "tui-open" } }),
            )),
            driver: Some(drive),
        },
    );
}
