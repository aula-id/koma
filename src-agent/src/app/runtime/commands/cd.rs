//! `/cd` + `/adddir` commands (Phase 8): move / widen the session workspace.
//!
//! `/cd <path>` repoints the session's live working directory. UNLIKE the
//! model-callable `cd` tool, the user path is UNRESTRICTED — there is NO
//! workspace allow-list check (the user is trusted), so `/cd` may land anywhere
//! on disk. Resolution is the same shell-like rule the tool uses (`[N]` root /
//! absolute / relative-to-cwd), shared via `tool::cd::resolve_cd_target`. The
//! repoint goes through the SAME `apply_workspace_change` primitive the tool's
//! interception uses, so the dir cache + awareness refresh identically. If the
//! user cds outside every configured root, that is fine — the next MODEL tool
//! turn there is WC-denied by the harness (the effective-cwd check), which is the
//! intended safety boundary.
//!
//! `/adddir <path>` appends a directory to the session's `settings.workdir` list,
//! widening the allow-list / adding an `[N]` root, then persists + reindexes.

use std::sync::Arc;

use anyhow::Result;

use crate::app::state::AppState;
use crate::service::openrouter::OpenRouterClient;

/// Handle `/cd <path>`: change the foreground session's working directory with NO
/// allow-list restriction. Resolves shell-like against the current cwd, validates
/// the target exists + is a directory, then applies the workspace change.
pub(super) fn handle_cd(
    path: String,
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    if state.rest.fg().session.is_none() {
        state.rest.status = "no active session".into();
        return Ok(());
    }
    let path = path.trim();
    if path.is_empty() {
        state.rest.status = "usage: /cd <path>".into();
        return Ok(());
    }
    let fgi = state.rest.foreground;
    // Resolve against the session's CURRENT cwd + configured roots ([N] support),
    // reusing the tool's shared resolver so user + model cd never diverge.
    let cwd = state.rest.fg().effective_cwd();
    let workspaces = state.rest.fg().session.as_ref().map(|s| s.workdirs()).unwrap_or_default();
    let resolved = match crate::tool::cd::resolve_cd_target(path, &cwd, &workspaces) {
        Ok(p) => p,
        Err(e) => {
            state.rest.status = format!("cd: {e}");
            return Ok(());
        }
    };
    // Must exist + be a directory. (No allow-list check — the user is trusted.)
    let canonical = match crate::tool::cd::canonicalize_existing(&resolved) {
        Ok(p) => p,
        Err(e) => {
            state.rest.status = format!("cd: {e}");
            return Ok(());
        }
    };
    if !canonical.is_dir() {
        state.rest.status = format!("cd: '{}' is not a directory", canonical.display());
        return Ok(());
    }
    // Apply via the shared primitive (sets cwd, reindexes, recomputes awareness).
    super::super::stream::apply_workspace_change(state, fgi, canonical.clone(), client, handle);
    state.rest.status = format!("cwd: {}", canonical.display());
    Ok(())
}

/// Handle `/adddir <path>`: append a directory to the session's workspace roots.
///
/// Resolves the path relative to the current cwd (absolute paths used as-is),
/// requires it to exist + be a directory, appends it to `settings.workdir` (when
/// not already present), persists `settings.json`, and reindexes the dir cache so
/// the new root's files appear in `@`-autocomplete. Widening the allow-list lets
/// subsequent MODEL `cd`/tool runs reach the new root.
pub(super) fn handle_adddir(path: String, state: &mut AppState) -> Result<()> {
    if state.rest.fg().session.is_none() {
        state.rest.status = "no active session".into();
        return Ok(());
    }
    let path = path.trim();
    if path.is_empty() {
        state.rest.status = "usage: /adddir <path>".into();
        return Ok(());
    }
    // Resolve relative to the current cwd (shell-like); absolute paths as-is.
    let cwd = state.rest.fg().effective_cwd();
    let candidate = {
        let p = std::path::Path::new(path);
        if p.is_absolute() { p.to_path_buf() } else { cwd.join(path) }
    };
    let canonical = match crate::tool::cd::canonicalize_existing(&candidate) {
        Ok(p) => p,
        Err(e) => {
            state.rest.status = format!("adddir: {e}");
            return Ok(());
        }
    };
    if !canonical.is_dir() {
        state.rest.status = format!("adddir: '{}' is not a directory", canonical.display());
        return Ok(());
    }
    let canonical_str = canonical.display().to_string();
    // Append to the session's workdir list (dedup by canonicalised equality) +
    // persist. Skip when the directory is already a root.
    let already = state.rest.fg().session.as_ref().is_some_and(|s| {
        s.settings.workdir.iter().any(|w| {
            std::path::Path::new(w.trim())
                .canonicalize()
                .map(|c| c == canonical)
                .unwrap_or(false)
        })
    });
    if already {
        state.rest.status = format!("adddir: already a root: {canonical_str}");
        return Ok(());
    }
    if let Some(sess) = state.rest.fg_mut().session.as_mut() {
        sess.settings.workdir.push(canonical_str.clone());
        if let Err(e) = sess.save() {
            state.rest.status = format!("adddir: save failed: {e}");
            return Ok(());
        }
    }
    // Reindex against the widened root set so `@`/dir_list pick up the new root.
    let roots = state.rest.fg().session.as_ref().map(|s| s.workdirs());
    let dir_cache = state.rest.fg().dir_cache.clone();
    if let Some(r) = roots {
        crate::tool::dircache::reindex(r, dir_cache);
    }
    state.rest.status = format!("added workspace root: {canonical_str}");
    Ok(())
}
