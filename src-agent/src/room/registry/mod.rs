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

/// Whether a load warning stops a write to `avatar`'s entry in `avatars.toml`:
/// a failure of that file as a whole does, and so does a warning about that
/// entry. A warning about another entry, or about another file, does not, since
/// the loader still applies the rest of the file.
pub fn blocks_avatar_write(warning: &str, avatar: &str) -> bool {
    match load::warned_entry(warning) {
        Some(id) => id == avatar,
        None => warning.starts_with("avatars.toml: "),
    }
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

    /// The warnings that stop a candidate `text` for the overlay file `name`
    /// from taking effect: the overlay is read, then loaded on top of the
    /// embedded files once as it is and once with `text` in place of `name`.
    /// Nothing is written and the current snapshot is left alone. Returned are
    /// the warnings the candidate brings that the file on disk does not, and,
    /// even when the file on disk has them too, a failure that drops the
    /// candidate file as a whole and every warning about one of its entries,
    /// because those parts would not take effect. When the whole overlay is
    /// ignored, its read warning is returned as `"<name>: <warning>"`.
    pub fn check_overlay_file(&self, name: &str, text: &str) -> Vec<String> {
        let mut read_warnings = Vec::new();
        let tree = match &self.overlay {
            Some(dir) => source::read_overlay(dir, &mut read_warnings),
            None => source::Tree::new(),
        };
        let dropped: Vec<String> = read_warnings
            .iter()
            .filter(|w| self.drops_whole_overlay(w))
            .map(|w| format!("{name}: {w}"))
            .collect();
        if !dropped.is_empty() {
            return dropped;
        }
        let load = |tree: &source::Tree| {
            let mut warnings = read_warnings.clone();
            let mut next = self.base.clone();
            let parsed = load::parse(tree, &mut warnings);
            load::apply_overlay(&mut next, parsed, &mut warnings);
            warnings
        };
        let before = load(&tree);
        let mut candidate = tree;
        candidate.insert(name.to_string(), text.to_string());
        let whole_file = format!("{name}: ");
        load(&candidate)
            .into_iter()
            .filter(|w| !before.contains(w) || w.starts_with(&whole_file))
            .collect()
    }

    /// Whether a load warning means the overlay file `name` was not read: the
    /// whole overlay was ignored, or that one file was.
    pub fn drops_overlay_file(&self, warning: &str, name: &str) -> bool {
        let Some(dir) = &self.overlay else {
            return false;
        };
        self.drops_whole_overlay(warning)
            || warning.starts_with(&format!("{}: ", dir.join(name).display()))
    }

    /// Whether a read warning means the overlay directory was ignored as a
    /// whole: it is a symlink, has the wrong owner, or cannot be listed.
    fn drops_whole_overlay(&self, warning: &str) -> bool {
        let Some(dir) = &self.overlay else {
            return false;
        };
        warning.starts_with("overlay ")
            || warning == format!("cannot read directory {}", dir.display())
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
