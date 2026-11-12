//! Hot-reloadable registry of the room's prompts, tool definitions, role files
//! and skills.
//!
//! Two tiers: the files embedded from `src-misc/room` (always valid, the
//! fallback) and an optional per-session overlay directory
//! (`<session dir>/room/`). [`RegistryHandle::reload`] re-reads the overlay and
//! atomically swaps in a new immutable [`Registry`]. A turn takes a snapshot at
//! its start via [`RegistryHandle::current`] and keeps it, so a reload never
//! changes a turn that is already running. If an overlay file is broken or
//! breaks the narrowing rule, that file is skipped with a warning and the
//! embedded version stays in force.

mod load;
mod model;
mod source;
mod template;
mod watch;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

// Public surface for the lane runners and the daemon, which land in later steps.
#[allow(unused_imports)]
pub use model::{
    is_host_handler, Avatar, LaneKind, LaneModel, Registry, RenderError, Role, Skill, ToolDef,
    HOST_HANDLERS, KNOWN_HANDLERS, SHELF_TOOLS, SINGLE_ROLES,
};
#[allow(unused_imports)]
pub use watch::{spawn_watcher, ReloadOutcome, WatchGuard};

/// Where a session keeps its overlay: `<session dir>/room/`.
pub fn overlay_dir_for(session_dir: &Path) -> PathBuf {
    session_dir.join("room")
}

/// Warnings from the most recent load. Empty means every file was accepted.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LoadReport {
    pub warnings: Vec<String>,
}

/// Owner of the current registry snapshot.
pub struct RegistryHandle {
    base: Registry,
    overlay: Option<PathBuf>,
    current: RwLock<Arc<Registry>>,
    report: Mutex<LoadReport>,
}

impl RegistryHandle {
    /// A registry made of the embedded files only. Fails only if the embedded
    /// files are invalid, which the unit tests rule out.
    pub fn embedded() -> Result<Self, String> {
        let base = load::build_base(&source::embedded_tree())?;
        Ok(Self {
            current: RwLock::new(Arc::new(base.clone())),
            base,
            overlay: None,
            report: Mutex::new(LoadReport::default()),
        })
    }

    /// The embedded files plus the overlay at `dir`, read once now. A missing
    /// directory is fine; it simply contributes nothing.
    pub fn with_overlay(dir: PathBuf) -> Result<Self, String> {
        let mut handle = Self::embedded()?;
        handle.overlay = Some(dir);
        handle.reload();
        Ok(handle)
    }

    pub fn overlay_dir(&self) -> Option<&Path> {
        self.overlay.as_deref()
    }

    /// The snapshot a new turn should use.
    pub fn current(&self) -> Arc<Registry> {
        Arc::clone(&self.current.read().unwrap_or_else(|e| e.into_inner()))
    }

    /// Warnings from the latest load.
    pub fn report(&self) -> LoadReport {
        self.report
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Re-read the overlay and swap in the result. Returns whether the content
    /// changed; the version only advances when it did.
    pub fn reload(&self) -> bool {
        let mut warnings = Vec::new();
        let mut next = self.base.clone();
        if let Some(dir) = &self.overlay {
            let tree = source::read_overlay(dir, &mut warnings);
            let parsed = load::parse(&tree, &mut warnings);
            load::apply_overlay(&mut next, parsed, &mut warnings);
        }
        let changed = {
            let mut slot = self.current.write().unwrap_or_else(|e| e.into_inner());
            next.version = slot.version;
            let changed = next != **slot;
            if changed {
                next.version = slot.version + 1;
                *slot = Arc::new(next);
            }
            changed
        };
        *self.report.lock().unwrap_or_else(|e| e.into_inner()) = LoadReport { warnings };
        changed
    }
}
