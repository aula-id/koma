//! Shared, MANAGER-INDEPENDENT uninstall helpers used by BOTH the attached daemon
//! ([`crate::app::runtime::event_loop::daemon::hub::requests_ext`]'s `uninstall_extension`)
//! and the detached GUI host ([`crate::app::runtime::client::store_host`]'s
//! `spawn_uninstall`) so the "complete nuke" stays identical across the two paths.
//!
//! Everything here is pure fs/registry work — NO live [`crate::app::mcp::McpManager`] /
//! [`crate::app::ext::ExtHostManager`], NO `AppConfig` mutation (that stays inline on each
//! path so its single `config.save()` is unambiguous) — best-effort, and NEVER panics: a
//! broken manifest or a missing file degrades to a skip + one `~/.koma/error.log` line,
//! never a failed uninstall.

use koma_extension::protocol::ExtensionManifest;

use crate::model::store;

/// The one-shot manifest snapshot an uninstall takes BEFORE it deletes `extensions/<id>/`
/// (step 1 of the nuke) — the two facts later steps still need once the on-disk manifest is
/// gone: the extension's contributed sub-agent names (the agent-override delete key) and its
/// declared `workspace_dir` (the data-dir nuke target).
#[derive(Debug, Clone, Default)]
pub struct UninstallManifestInfo {
    /// `contributes.sub_agents[].name`, lowercased + trimmed with blanks dropped — the exact
    /// keys [`sweep_agent_overrides`] deletes `<name>.md` override files for.
    pub sub_agent_names: Vec<String>,
    /// The manifest's declared `workspace_dir` (trimmed, non-empty) or `None`.
    pub workspace_dir: Option<String>,
}

/// Read `extensions/<id>/manifest.json` ONCE and project the [`UninstallManifestInfo`] the
/// later nuke steps need. A missing/unreadable/unparsable manifest degrades to an EMPTY
/// snapshot (the dependent steps simply find nothing to do) — the parse failure is logged,
/// but a missing/never-installed manifest is a silent no-op, mirroring every other
/// best-effort manifest read in the ext subsystem.
pub fn snapshot_manifest(id: &str) -> UninstallManifestInfo {
    let path = match store::extensions_dir() {
        Ok(dir) => dir.join(id).join("manifest.json"),
        Err(_) => return UninstallManifestInfo::default(),
    };
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        // Not installed on disk / unreadable — nothing to snapshot (no agents, no workspace).
        Err(_) => return UninstallManifestInfo::default(),
    };
    let manifest: ExtensionManifest = match serde_json::from_slice(&bytes) {
        Ok(m) => m,
        Err(e) => {
            store::append_global_error_log(
                "ext-uninstall",
                &format!(
                    "snapshot manifest for {id} failed to parse (agent/workspace nuke skipped): {e}"
                ),
            );
            return UninstallManifestInfo::default();
        }
    };
    let sub_agent_names = manifest
        .contributes
        .sub_agents
        .iter()
        .map(|a| a.name.trim().to_lowercase())
        .filter(|n| !n.is_empty())
        .collect();
    let workspace_dir = manifest
        .workspace_dir
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    UninstallManifestInfo {
        sub_agent_names,
        workspace_dir,
    }
}

/// Delete same-named agent-override files (step 7) — `~/.koma/agents/<name>.md` AND every
/// `~/.koma/sessions/<pwd_hash>/<session_id>/agents/<name>.md` — for each snapshotted
/// sub-agent name. When a user saved an EDITED copy of an extension's sub-agent it persisted
/// as one of these override files (see `actions::agents::handle_save_agent`); left behind
/// after uninstall it would shadow a same-named agent with a now-orphaned definition, so the
/// nuke removes it.
///
/// SAME-NAME CAVEAT (documented + accepted): the delete key is the sub-agent NAME, so an
/// UNRELATED user agent that happens to share a name with an uninstalled extension's
/// sub-agent is swept too — the registry is name-keyed and carries no ownership tag to
/// distinguish them. Each name is first run through [`validate_agent_name`] — the SAME
/// normalisation `save_agent` used to write the file — so a name that could never have been
/// a valid override filename is skipped, and a `..`/slash escape in an adversarial manifest
/// name can never be formed into a path. Best-effort: each removal logs on success and
/// ignores a missing file.
pub fn sweep_agent_overrides(sub_agent_names: &[String]) {
    // Validate + normalise once (rejects anything `save_agent` could never have written, and
    // neutralises any path-escape in an adversarial manifest name).
    let names: Vec<String> = sub_agent_names
        .iter()
        .filter_map(|n| crate::model::agent_def::validate_agent_name(n).ok())
        .collect();
    if names.is_empty() {
        return;
    }

    // Global overrides: ~/.koma/agents/<name>.md
    if let Ok(dir) = crate::model::agent_def::global_agents_dir() {
        for name in &names {
            remove_override_if_present(&dir.join(format!("{name}.md")), name, "global");
        }
    }

    // Session overrides: ~/.koma/sessions/<pwd_hash>/<session_id>/agents/<name>.md — a
    // two-level walk (pwd-hash bucket, then session dir).
    if let Ok(sessions) = store::sessions_dir() {
        for bucket in read_subdirs(&sessions) {
            for session in read_subdirs(&bucket) {
                let agents = session.join("agents");
                for name in &names {
                    remove_override_if_present(&agents.join(format!("{name}.md")), name, "session");
                }
            }
        }
    }
}

/// Remove `path` iff it exists, logging the deletion (step 7 asks each to be logged). A
/// missing file is a silent no-op (idempotent); any other IO error is logged, never fatal.
fn remove_override_if_present(path: &std::path::Path, name: &str, scope: &str) {
    if !path.exists() {
        return;
    }
    match std::fs::remove_file(path) {
        Ok(()) => store::append_global_error_log(
            "ext-uninstall",
            &format!("removed {scope} agent override for '{name}': {}", path.display()),
        ),
        Err(e) => store::append_global_error_log(
            "ext-uninstall",
            &format!("remove {scope} agent override {}: {e}", path.display()),
        ),
    }
}

/// The immediate SUBDIRECTORIES of `dir` (best-effort — an unreadable/missing dir yields an
/// empty list, never an error). Used for the two-level session-override walk above.
fn read_subdirs(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.push(p);
            }
        }
    }
    out
}
