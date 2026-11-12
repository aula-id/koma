//! Watch the overlay directory and reload the registry after edits settle.

use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};

use super::{LoadReport, Registry, RegistryHandle};

/// Quiet period after the last filesystem event before reloading, so an
/// editor's burst of writes triggers one reload.
const DEBOUNCE: Duration = Duration::from_millis(300);

/// What one debounced reload produced.
#[derive(Debug, Clone)]
pub struct ReloadOutcome {
    /// The snapshot now in force.
    pub registry: Arc<Registry>,
    pub report: LoadReport,
    /// False when the files changed but the resulting content did not.
    pub changed: bool,
}

/// Keeps the watcher alive; dropping it stops watching and ends the thread.
pub struct WatchGuard {
    _watcher: notify::RecommendedWatcher,
}

/// Start watching the handle's overlay directory (created if missing).
/// `on_reload` runs on a background thread after each debounced batch.
pub fn spawn_watcher(
    handle: Arc<RegistryHandle>,
    on_reload: impl Fn(ReloadOutcome) + Send + 'static,
) -> Result<WatchGuard, String> {
    let dir = handle
        .overlay_dir()
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| "registry has no overlay directory".to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;

    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let _ = tx.send(event);
    })
    .map_err(|e| e.to_string())?;
    watcher
        .watch(&dir, RecursiveMode::Recursive)
        .map_err(|e| format!("{}: {e}", dir.display()))?;

    std::thread::Builder::new()
        .name("room-registry-watch".to_string())
        .spawn(move || {
            // `recv` fails once the watcher (and its sender) is dropped.
            while rx.recv().is_ok() {
                while rx.recv_timeout(DEBOUNCE).is_ok() {}
                let changed = handle.reload();
                on_reload(ReloadOutcome {
                    registry: handle.current(),
                    report: handle.report(),
                    changed,
                });
            }
        })
        .map_err(|e| e.to_string())?;

    Ok(WatchGuard { _watcher: watcher })
}
