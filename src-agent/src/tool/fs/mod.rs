//! Sandboxed filesystem tools: list / read / write / delete.
//!
//! Every path argument is resolved through [`super::resolve`], which pins it
//! inside the session workspace — a tool can never read or write outside it.
//! These structs implement [`Tool`] and are advertised to the model via
//! [`super::all_tools`]; the agentic loop dispatches the model's requested calls
//! through [`Tool::run`].

mod helpers;
pub mod dirlist;
pub mod read;
pub mod write;
pub mod edit;
pub mod delete;

pub use dirlist::DirList;
pub use read::Read;
pub use write::Write;
pub use edit::Edit;
pub use delete::Delete;

use std::path::Path;
use super::ToolCtx;

/// Record a workspace file mutation into the session's cumulative file-change log
/// (#24): `path` is the resolved absolute path just written/edited/deleted, and
/// `status` is `"added"` / `"modified"` / `"deleted"`. Dedup is by the DISPLAYED
/// path (latest status wins), so we store the workspace-relative form when the
/// path is under a workspace root (nicer for the GUI panel + a stable dedup key),
/// falling back to the absolute path otherwise.
///
/// Best-effort + a no-op when no session is active (headless/test constructions
/// have `session_dir == None`): a DB hiccup must never fail the fs op, which has
/// already succeeded by the time we're called.
pub(crate) fn record_change(ctx: &ToolCtx, path: &Path, status: &str) {
    let Some(session_dir) = ctx.session_dir.as_ref() else {
        return;
    };
    // Prefer the shortest workspace-relative rendering across all configured roots
    // so the same file always dedups to the same key regardless of which `[N]` root
    // spelled it. Fall back to the absolute path when it's under none of them.
    let display = ctx
        .workspaces
        .iter()
        .filter_map(|root| path.strip_prefix(root).ok())
        .map(|rel| rel.to_string_lossy().into_owned())
        .min_by_key(|s| s.len())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let _ = crate::model::msglog::record_file_change(session_dir, &display, status);
}
