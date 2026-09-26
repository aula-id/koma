//! Watch notifications are hints; fingerprint checks remain authoritative.
use super::WorkspaceRef;
use anyhow::Result;
use notify::{RecursiveMode, Watcher};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    path::Path,
    sync::{mpsc, Mutex, OnceLock},
    time::Duration,
};
static WATCHERS: OnceLock<Mutex<VecDeque<(WorkspaceRef, notify::RecommendedWatcher)>>> =
    OnceLock::new();
pub(super) fn watch(workspace: &WorkspaceRef, root: &Path) -> Result<Value> {
    let mut watchers = WATCHERS.get_or_init(Default::default).lock().unwrap();
    if watchers.iter().any(|(key, _)| key == workspace) {
        return Ok(Value::Null);
    }
    anyhow::ensure!(
        !super::SHUTTING_DOWN.load(std::sync::atomic::Ordering::Acquire),
        "Coding service is shutting down"
    );
    let (tx, rx) = mpsc::sync_channel(1);
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        // Access events caused by our own reads must not trigger a feedback loop.
        if event
            .as_ref()
            .is_ok_and(|e| matches!(e.kind, notify::EventKind::Access(_)))
        {
            return;
        }
        let _ = tx.try_send(());
    })?;
    watcher.watch(root, RecursiveMode::Recursive)?;
    let owner = workspace.clone();
    std::thread::Builder::new()
        .name("coding-watch".into())
        .spawn(move || {
            while rx.recv().is_ok() {
                std::thread::sleep(Duration::from_millis(250));
                while rx.try_recv().is_ok() {}
                super::event(
                    json!({"k":"CodingEvent","workspace":owner,"event":{"k":"FileSystemChanged"}}),
                );
            }
        })?;
    if watchers.len() >= 32 {
        watchers.pop_front();
    }
    watchers.push_back((workspace.clone(), watcher));
    Ok(Value::Null)
}
pub(super) fn shutdown() {
    if let Some(watchers) = WATCHERS.get() {
        if let Ok(mut watchers) = watchers.lock() {
            watchers.clear();
        }
    }
}
