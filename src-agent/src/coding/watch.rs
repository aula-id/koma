//! Watch notifications are hints; fingerprint checks remain authoritative.
use super::sync::CheckedMutex;
use super::WorkspaceRef;
use anyhow::Result;
use notify::{RecursiveMode, Watcher};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, VecDeque},
    path::Path,
    sync::{mpsc, Mutex, OnceLock},
    time::Duration,
};
static WATCHERS: OnceLock<Mutex<VecDeque<(WorkspaceRef, notify::RecommendedWatcher)>>> =
    OnceLock::new();
pub(super) fn watch(workspace: &WorkspaceRef, root: &Path) -> Result<Value> {
    let mut watchers = WATCHERS.get_or_init(Default::default).checked_lock()?;
    if watchers.iter().any(|(key, _)| key == workspace) {
        return Ok(Value::Null);
    }
    anyhow::ensure!(
        !super::SHUTTING_DOWN.load(std::sync::atomic::Ordering::Acquire),
        "Coding service is shutting down"
    );
    let (tx, rx) = mpsc::sync_channel(1024);
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        // Access events caused by our own reads must not trigger a feedback loop.
        if event
            .as_ref()
            .is_ok_and(|e| matches!(e.kind, notify::EventKind::Access(_)))
        {
            return;
        }
        let _ = tx.try_send(event.ok());
    })?;
    watcher.watch(root, RecursiveMode::Recursive)?;
    let owner = workspace.clone();
    let root = root.to_path_buf();
    std::thread::Builder::new()
        .name("coding-watch".into())
        .spawn(move || {
            while let Ok(first) = rx.recv() {
                std::thread::sleep(Duration::from_millis(250));
                let mut changes = BTreeMap::new();
                for event in std::iter::once(first)
                    .chain(rx.try_iter().take(1023))
                    .flatten()
                {
                    let renamed = matches!(
                        event.kind,
                        notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
                    );
                    for path in event.paths {
                        if !path.starts_with(&root)
                            || path
                                .strip_prefix(&root)
                                .is_ok_and(|p| p.components().any(|c| c.as_os_str() == ".git"))
                        {
                            continue;
                        }
                        let kind = match event.kind {
                            notify::EventKind::Create(_) => 1,
                            notify::EventKind::Remove(_) => 3,
                            _ if renamed => {
                                if path.exists() {
                                    1
                                } else {
                                    3
                                }
                            }
                            _ => 2,
                        };
                        if changes.len() < 4096 || changes.contains_key(&path) {
                            changes.insert(path, kind);
                        }
                    }
                }
                super::language::files_changed(&owner, changes.into_iter().collect());
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
