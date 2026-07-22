//! Shared private helpers used by the fs submodules.

use crate::tool::ToolCtx;
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::Path;

/// Pull a required string argument out of the decoded JSON args.
pub(super) fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .with_context(|| format!("missing required string argument '{key}'"))
}

/// Max directory entries to list in a not-found error before summarising the rest.
const NOT_FOUND_MAX_ENTRIES: usize = 30;

/// Build an ACTIONABLE "does not exist" error for a path that resolved cleanly
/// (inside the workspace) but points at a missing file.
///
/// Instead of a dead-end "X does not exist", we walk up from the requested path
/// to the nearest EXISTING ancestor directory (never escaping the workspace —
/// `abs` is already proven to be inside it by [`super::super::resolve`]) and list that
/// directory's entries so the model can retry with a correct path in the SAME
/// turn. Listing reuses the cache-backed [`super::super::dircache::DirCache::children`]
/// (gitignore-aware, sorted, folders suffixed with '/'); if the ancestor is not
/// in the index we fall back to a direct `read_dir` so output is still useful.
/// Entries are capped at [`NOT_FOUND_MAX_ENTRIES`] with a "… (N more)" note, and
/// we always point at `glob` as the escape hatch.
pub(super) fn not_found_help(ctx: &ToolCtx, abs: &Path, rel: &str) -> String {
    // Parse workspace index from the path prefix.
    let (ws_idx, _bare) = super::super::parse_ws_prefix(rel);
    // Find which workspace contains the path, and use it as the floor.
    let ws = super::super::find_workspace(&ctx.workspaces, abs)
        .or_else(|| ctx.workspaces.get(ws_idx).cloned())
        .or_else(|| ctx.workspaces.first().cloned())
        .map(|p| p.canonicalize().unwrap_or(p))
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    // Walk up `abs` to the nearest ancestor that exists AND is a directory,
    // stopping at the workspace root.
    let mut ancestor = abs.parent();
    while let Some(p) = ancestor {
        if p.is_dir() {
            break;
        }
        if p == ws {
            break;
        }
        ancestor = p.parent();
    }
    // Fall back to the workspace root if the walk produced nothing usable.
    let dir_abs = match ancestor {
        Some(p) if p.is_dir() => p,
        _ => ws.as_path(),
    };

    // Workspace-relative label for the ancestor ("" / "." -> the root itself).
    let dir_rel = dir_abs
        .strip_prefix(&ws)
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let dir_label = if dir_rel.is_empty() {
        ".".to_string()
    } else {
        format!("{dir_rel}/")
    };

    // List the ancestor's entries. Prefer the gitignore-aware cache; if it has
    // nothing for this dir (e.g. not yet indexed / ignored), read the dir live.
    let mut entries: Vec<String> = ctx
        .dir_cache
        .read()
        .map(|c| c.children(&dir_rel, ws_idx))
        .unwrap_or_default();
    if entries.is_empty() {
        if let Ok(rd) = std::fs::read_dir(dir_abs) {
            for ent in rd.flatten() {
                let mut name = ent.file_name().to_string_lossy().into_owned();
                if ent.path().is_dir() {
                    name.push('/');
                }
                entries.push(name);
            }
            entries.sort();
        }
    }

    let total = entries.len();
    let shown = total.min(NOT_FOUND_MAX_ENTRIES);

    let mut msg = format!("'{rel}' does not exist.\n");
    if total == 0 {
        msg.push_str(&format!(
            "Nearest existing directory '{dir_label}' is empty."
        ));
    } else {
        msg.push_str(&format!(
            "Nearest existing directory '{dir_label}' contains:\n"
        ));
        for e in entries.iter().take(shown) {
            msg.push_str("  ");
            msg.push_str(e);
            msg.push('\n');
        }
        if total > shown {
            msg.push_str(&format!("  … ({} more)\n", total - shown));
        }
    }
    msg.push_str(
        "Pick a correct path from these (descend into subdirectories shown with '/'), \
         or use the `glob` tool to locate the file by name.",
    );
    msg
}
