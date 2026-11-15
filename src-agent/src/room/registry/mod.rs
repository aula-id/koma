//! Hot-reloadable registry of the room's prompts, tool definitions, role files
//! and skills.
//!
//! Three layers, lowest first: the files embedded from `src-misc/room` (always
//! valid, the fallback), an optional template directory (`<koma data dir>/room/`)
//! read once when the handle is built, and an optional per-session overlay
//! directory (`<session dir>/room/`). Template and overlay use the same file
//! format and the same narrowing rules, applied in that order.
//! [`RegistryHandle::reload`] re-reads the overlay, layers it over the stored
//! template, and atomically swaps in a new immutable [`Registry`]. A turn takes
//! a snapshot at its start via [`RegistryHandle::current`] and keeps it, so a reload never
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

pub use source::Tree;
// Reads a template or overlay directory into a tree for `layer_revision` and `validate_layers`.
#[allow(unused_imports)]
pub use source::read_overlay;

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

/// The template layer shared by every room: `<koma data dir>/room/`. `None`
/// when the data directory cannot be resolved.
pub fn template_dir() -> Option<PathBuf> {
    crate::model::store::base_dir().ok().map(|base| base.join("room"))
}

/// The start of every warning that comes from the template layer, so a
/// template problem is never read as one in the session overlay.
pub const TEMPLATE_WARNING_PREFIX: &str = "template: ";

/// The revision of one layer's files: the first 16 hex chars of sha256 over
/// each relative path and its bytes, in path order, or `"absent"` for a layer
/// with no files.
pub fn layer_revision(tree: &Tree) -> String {
    use sha2::{Digest, Sha256};
    if tree.is_empty() {
        return "absent".to_string();
    }
    let mut hasher = Sha256::new();
    for (path, text) in tree {
        // Lengths first, so no two different trees hash the same bytes.
        hasher.update((path.len() as u64).to_le_bytes());
        hasher.update(path.as_bytes());
        hasher.update((text.len() as u64).to_le_bytes());
        hasher.update(text.as_bytes());
    }
    let hex = format!("{:x}", hasher.finalize());
    hex[..16].to_string()
}

/// The registry that candidate `template` and `session` trees would give over
/// the embedded files, with every warning the loader reports for them.
/// Template warnings start with [`TEMPLATE_WARNING_PREFIX`]. Nothing is read
/// from disk and no handle is touched.
pub(crate) fn validate_layers(template: &Tree, session: Option<&Tree>) -> (Registry, Vec<String>) {
    let base = match load::build_base(&source::embedded_tree()) {
        Ok(base) => base,
        Err(e) => return (Registry::default(), vec![e]),
    };
    let mut warnings = Vec::new();
    let registry = layered(&base, Some(template), &[], session, &mut warnings);
    (registry, warnings)
}

/// `base`, then the template tree (its read warnings `template_read` first),
/// then the session tree, under the narrowing rules.
fn layered(
    base: &Registry,
    template: Option<&Tree>,
    template_read: &[String],
    session: Option<&Tree>,
    warnings: &mut Vec<String>,
) -> Registry {
    let mut next = base.clone();
    if let Some(tree) = template {
        let mut layer = template_read.to_vec();
        let parsed = load::parse(tree, &mut layer);
        load::apply_overlay(&mut next, parsed, &mut layer);
        warnings.extend(layer.into_iter().map(|w| format!("{TEMPLATE_WARNING_PREFIX}{w}")));
    }
    if let Some(tree) = session {
        let parsed = load::parse(tree, warnings);
        load::apply_overlay(&mut next, parsed, warnings);
    }
    next
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

/// The template layer as it was read when the handle was built.
struct Template {
    dir: PathBuf,
    tree: Tree,
    /// Warnings from reading the directory, without the layer prefix.
    read_warnings: Vec<String>,
    revision: String,
}

/// Owner of the current registry snapshot.
pub struct RegistryHandle {
    base: Registry,
    template: Option<Template>,
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
            template: None,
            overlay: None,
            report: Mutex::new(LoadReport::default()),
        })
    }

    /// The embedded files plus the overlay at `dir`, read once now. A missing
    /// directory is fine; it simply contributes nothing.
    pub fn with_overlay(dir: PathBuf) -> Result<Self, String> {
        Self::with_layers(None, dir)
    }

    /// The embedded files, then the template at `template`, then the overlay
    /// at `overlay`. The template is read now and never again: later reloads
    /// re-read only the overlay. A missing directory contributes nothing.
    pub fn with_layers(template: Option<PathBuf>, overlay: PathBuf) -> Result<Self, String> {
        let mut handle = Self::embedded()?;
        handle.template = template.map(|dir| {
            let mut read_warnings = Vec::new();
            let tree = source::read_overlay(&dir, &mut read_warnings);
            let revision = layer_revision(&tree);
            Template { dir, tree, read_warnings, revision }
        });
        handle.overlay = Some(overlay);
        handle.reload();
        Ok(handle)
    }

    pub fn overlay_dir(&self) -> Option<&Path> {
        self.overlay.as_deref()
    }

    /// The template directory this handle was built with, if any.
    pub fn template_dir(&self) -> Option<&Path> {
        self.template.as_ref().map(|t| t.dir.as_path())
    }

    /// The [`layer_revision`] of the template as it was read when the handle
    /// was built, or `None` when it was built without a template directory.
    pub fn template_revision(&self) -> Option<String> {
        self.template.as_ref().map(|t| t.revision.clone())
    }

    /// The embedded files with the stored template layered on top.
    fn below_overlay(&self, warnings: &mut Vec<String>) -> Registry {
        let (template, template_read) = match &self.template {
            Some(t) => (Some(&t.tree), t.read_warnings.as_slice()),
            None => (None, &[][..]),
        };
        layered(&self.base, template, template_read, None, warnings)
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
    /// The stored template layer sits below the overlay in both loads.
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
        let load = |tree: &Tree| {
            let mut warnings = Vec::new();
            let mut next = self.below_overlay(&mut warnings);
            warnings.extend(read_warnings.iter().cloned());
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

    /// Re-read the overlay, layer it over the stored template, and swap in the
    /// result. Returns whether the content changed; the version only advances
    /// when it did.
    pub fn reload(&self) -> bool {
        let mut warnings = Vec::new();
        let mut next = self.below_overlay(&mut warnings);
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
