//! # TUI SCREEN PROTOCOL (v1) — server-driven UI for extension TUI screens
//!
//! An extension that declares `contributes.tui_screens[]` in its manifest can drive a
//! full-screen view that koma's terminal UI renders on its behalf. The WHOLE exchange
//! reuses the existing `panel.msg` invoke + `panel.push` notify verbs VERBATIM (the
//! screen id rides as `panelId`), so it is wire-legal with zero protocol change — a GUI
//! panel and a TUI screen are the same bytes on the socket, distinguished only by the
//! `kind` tag in the opaque payload.
//!
//! ## Host → ext  (koma invokes `panel.msg`, `panelId` = the tui screen id)
//! | verb   | payload                                             | when                    |
//! |--------|-----------------------------------------------------|-------------------------|
//! | open   | `{ "kind": "tui-open" }`                            | the screen is opened    |
//! | select | `{ "kind": "tui-select", "item": "<menu item id>" }`| Enter on a menu row     |
//! | close  | `{ "kind": "tui-close" }`                           | Esc / exit (best-effort — reply ignored) |
//!
//! ## Ext → host reply  (the `Result` value of the `panel.msg` invoke)
//! - `{ "screen": <Screen> }` — render this screen.
//! - `{ "close": true }` — pop back to the extension detail view.
//!
//! ## Ext → host push  (the extension sends a `panel.push` notify, `panel_id` = the screen id)
//! - payload `{ "kind": "tui-screen", "screen": <Screen> }` — folded LIVE into the open screen.
//!
//! ## Screen model
//! ```text
//! Screen = { "title": String?, "body": [Node], "footer": String? }
//! Node   = { "t": "text",    "text": String }
//!        | { "t": "kv",      "k": String, "v": String }
//!        | { "t": "divider" }
//!        | { "t": "menu",    "items": [ { "id": String, "label": String } ] }
//! ```
//! Unknown node types are SKIPPED (forward-compat). Menu navigation (Up/Down over the
//! UNION of every menu's items, in body order) is HOST-side; Enter sends `tui-select` with
//! the highlighted item's `id`. See [`crate::app::mode::ext_screen`] for the menu-cursor
//! state and [`crate::view::extscreen`] for the renderer.
//!
//! ## Async / non-blocking
//! Every `tui-open` / `tui-select` runs on `spawn_blocking` (both
//! [`ExtHostManager::ensure_started`] and [`ExtHostManager::invoke_with_timeout`] block on a
//! sync→async bridge), delivering its outcome back on an [`ExtScreenReply`] through the
//! `AppStateRest::ext_screen_rx` receiver that `drains::drain_ext_screen` folds per tick —
//! the exact `sec_health_rx` shape. The event loop is NEVER blocked.

use std::sync::Arc;
use std::time::Duration;

use tokio::runtime::Handle;

use crate::app::ext::ExtHostManager;
use crate::app::state::AppStateRest;
use crate::model::app_config::InstalledExtension;
use crate::model::store;

/// Round-trip budget for one `tui-open` / `tui-select` invoke before it times out (shorter
/// than the 120s default `CALL_TIMEOUT` — a screen redraw should be prompt, and a hung
/// extension must not leave the TUI spinning for two minutes).
const EXT_SCREEN_TIMEOUT: Duration = Duration::from_secs(30);

/// The delivered outcome of one async extension-screen invoke, shipped back on
/// `AppStateRest::ext_screen_rx` (the exact `sec_health_rx` shape). `ext_id` + `screen_id`
/// let the drain fold it into the CORRECT open [`crate::app::mode::ExtScreenState`] (single
/// window → one match, but the tag keeps it de-globalization-safe).
#[derive(Debug)]
pub struct ExtScreenReply {
    /// The extension the invoke targeted.
    pub ext_id: String,
    /// The tui-screen the invoke targeted.
    pub screen_id: String,
    /// The invoke `Result` — the ext's reply value (`{ screen }` / `{ close }`) or an error.
    pub result: Result<serde_json::Value, String>,
}

/// The auto-start decision for an extension-screen `panel.msg` — the EXACT mirror of the GUI
/// panel bridge's `requests_ext::panel_start_decision` (a screen open, like a panel open,
/// implies user intent, so a not-running ENABLED daemon is auto-started; a disabled / oneshot
/// / missing extension is "not available"). Kept a PURE function over `running` +
/// `record` so it is unit-testable without a live manager. Returns:
///   - `Ok(true)`  → already running → invoke straight away.
///   - `Ok(false)` → a daemon-kind, ENABLED, not-yet-running extension → `ensure_started` first.
///   - `Err(msg)`  → not serviceable (MISSING / DISABLED / ONESHOT) → surfaced as the reply error.
fn screen_start_decision(
    running: bool,
    record: Option<&InstalledExtension>,
) -> Result<bool, String> {
    if running {
        return Ok(true);
    }
    match record {
        // Disabled (any kind) → not available (its auto-start is intentionally off).
        Some(ext) if !ext.enabled => Err("extension not available".to_string()),
        // Enabled daemon, not yet running → auto-start it.
        Some(ext) if ext.kind == "daemon" => Ok(false),
        // Enabled but not a daemon (oneshot) → no persistent backend to talk to.
        Some(_) => Err("extension not available".to_string()),
        // Not installed.
        None => Err("extension not available".to_string()),
    }
}

/// Apply the [`screen_start_decision`], (maybe) auto-start the extension, then
/// `invoke_with_timeout("panel.msg", { panelId, payload })` and return the ext's reply value
/// or a human-readable error. Runs on `spawn_blocking` (both `ensure_started` and
/// `invoke_with_timeout` block on a sync→async bridge). Every failure logs via
/// `append_global_error_log` (never `eprintln!`). Mirrors `requests_ext::run_panel_msg`'s
/// start-then-invoke logic; returned as a `Result` (not a wire reply) so the TUI screen path
/// can fold it into the open mode.
pub(crate) fn start_and_invoke_panel_msg(
    mgr: &Arc<ExtHostManager>,
    ext_id: &str,
    panel_id: &str,
    payload: serde_json::Value,
    record: Option<&InstalledExtension>,
    timeout: Duration,
) -> Result<serde_json::Value, String> {
    match screen_start_decision(mgr.is_running(ext_id), record) {
        Ok(true) => {}
        Ok(false) => {
            // `Ok(false)` is only returned for a Some(daemon, enabled) record, so this is set.
            let ext = record.ok_or_else(|| "extension not available".to_string())?;
            mgr.ensure_started(ext).map_err(|e| {
                store::append_global_error_log(
                    "ext screen",
                    &format!("auto-start {ext_id} for tui screen failed: {e:#}"),
                );
                format!("extension failed to start: {e:#}")
            })?;
        }
        Err(msg) => return Err(msg),
    }

    mgr.invoke_with_timeout(
        ext_id,
        "panel.msg",
        serde_json::json!({ "panelId": panel_id, "payload": payload }),
        timeout,
    )
    .map_err(|e| {
        store::append_global_error_log(
            "ext screen",
            &format!("panel.msg invoke for {ext_id} tui screen failed: {e:#}"),
        );
        format!("{e:#}")
    })
}

/// Kick off an async `tui-open` / `tui-select` invoke for an extension screen WITHOUT
/// blocking the event loop: snapshot the manager `Arc` + the registry record (for the
/// auto-start decision), open a fresh `ext_screen_rx`, and run
/// [`start_and_invoke_panel_msg`] on `spawn_blocking`, shipping the outcome back as an
/// [`ExtScreenReply`] the per-tick drain folds. Returns `Ok(())` once the invoke is spawned
/// (the caller flips `waiting = true`), or `Err(msg)` when there is no live ext manager (no
/// session runtime) — the caller shows that as the screen's error line.
///
/// Opening a fresh receiver DROPS any previous in-flight invoke's receiver (its spawned task
/// still completes but its `send` no-ops) — the desired "latest invoke wins" (a rapid
/// tui-select supersedes a pending tui-open), mirroring the endpoints/oauth stale-cancel.
pub(crate) fn kick_off_ext_screen_msg(
    rest: &mut AppStateRest,
    handle: &Handle,
    ext_id: String,
    screen_id: String,
    payload: serde_json::Value,
) -> Result<(), String> {
    let Some(mgr) = rest.ext_manager.clone() else {
        return Err("extension not available".to_string());
    };
    // Snapshot the registry record up front — the closure runs off the event loop and must
    // not borrow `rest`. `None` = not installed (→ the decision's error path).
    let record = rest
        .config
        .installed_extensions
        .iter()
        .find(|e| e.id == ext_id)
        .cloned();

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<ExtScreenReply>();
    rest.ext_screen_rx = Some(rx);

    handle.spawn_blocking(move || {
        let result = start_and_invoke_panel_msg(
            &mgr,
            &ext_id,
            &screen_id,
            payload,
            record.as_ref(),
            EXT_SCREEN_TIMEOUT,
        );
        // A dropped receiver (a superseding invoke replaced it) makes this a no-op.
        let _ = tx.send(ExtScreenReply {
            ext_id,
            screen_id,
            result,
        });
    });
    Ok(())
}

/// Fire a best-effort `tui-close` at the extension and DISCARD the reply — the courtesy
/// "screen closed" signal on Esc/exit. Only pokes a LIVE extension (never auto-starts one
/// just to say goodbye); a missing manager / stopped extension is a silent no-op. Runs on
/// `spawn_blocking` (the invoke blocks) so the input path returns immediately.
pub(crate) fn fire_tui_close(
    rest: &AppStateRest,
    handle: &Handle,
    ext_id: String,
    screen_id: String,
) {
    let Some(mgr) = rest.ext_manager.clone() else {
        return;
    };
    handle.spawn_blocking(move || {
        if mgr.is_running(&ext_id) {
            let _ = mgr.invoke_with_timeout(
                &ext_id,
                "panel.msg",
                serde_json::json!({ "panelId": screen_id, "payload": { "kind": "tui-close" } }),
                Duration::from_secs(5),
            );
        }
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn ext_record(kind: &str, enabled: bool) -> InstalledExtension {
        InstalledExtension {
            id: "run.koma.test".to_string(),
            version: "0.0.1".to_string(),
            tier: "free".to_string(),
            granted: Vec::new(),
            enabled,
            kind: kind.to_string(),
            exec: "bin/x".to_string(),
        }
    }

    /// The screen auto-start decision mirrors the GUI panel bridge's over every input:
    /// running → invoke; not-running daemon+enabled → start; oneshot / disabled / missing →
    /// "extension not available".
    #[test]
    fn screen_start_decision_covers_all_cases() {
        assert_eq!(screen_start_decision(true, None), Ok(true));
        assert_eq!(
            screen_start_decision(true, Some(&ext_record("daemon", false))),
            Ok(true)
        );
        assert_eq!(
            screen_start_decision(false, Some(&ext_record("daemon", true))),
            Ok(false)
        );
        assert_eq!(
            screen_start_decision(false, Some(&ext_record("oneshot", true))),
            Err("extension not available".to_string())
        );
        assert_eq!(
            screen_start_decision(false, Some(&ext_record("daemon", false))),
            Err("extension not available".to_string())
        );
        assert_eq!(
            screen_start_decision(false, None),
            Err("extension not available".to_string())
        );
    }
}
