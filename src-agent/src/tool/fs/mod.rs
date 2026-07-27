//! Sandboxed filesystem tools: list / read / write / delete.
//!
//! Every path argument is resolved through [`super::resolve`], which pins it
//! inside the session workspace — a tool can never read or write outside it.
//! These structs implement [`Tool`] and are advertised to the model via
//! [`super::all_tools`]; the agentic loop dispatches the model's requested calls
//! through [`Tool::run`].

pub mod delete;
pub mod dirlist;
pub mod edit;
mod helpers;
pub mod read;
pub mod write;

pub use delete::Delete;
pub use dirlist::DirList;
pub use edit::Edit;
pub use read::Read;
pub use write::Write;

use super::ToolCtx;
use std::path::Path;

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
    let display = display_key(ctx, path);
    let _ = crate::model::msglog::record_file_change(session_dir, &display, status);
}

/// The canonical dedup/display key for a workspace file: the shortest
/// workspace-relative rendering across all configured roots so the same file always
/// keys the same regardless of which `[N]` root spelled it, falling back to the
/// absolute path when it's under none of them. Shared by the file-change log AND the
/// baseline store so a `fileChanges` record's path always looks its baseline up
/// directly.
///
/// CAVEAT: the key is derived from `ctx.workspaces` AT CALL TIME — adding/reordering
/// roots mid-session (`/adddir`) can re-key the same absolute file, orphaning an
/// earlier baseline under the old spelling. Accepted drift, inherited from the
/// file-change log's own key scheme.
fn display_key(ctx: &ToolCtx, path: &Path) -> String {
    ctx.workspaces
        .iter()
        .filter_map(|root| path.strip_prefix(root).ok())
        .map(|rel| rel.to_string_lossy().into_owned())
        .min_by_key(|s| s.len())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Baseline pre-image over ~2MiB isn't stored (marker row only) — matches the GUI
/// diff tab's own size cap, past which it refuses to render anyway.
const BASELINE_SIZE_CAP: usize = 2 * 1024 * 1024;

/// Capture a file's "virtual git" BASELINE — its byte-exact pre-image — immediately
/// BEFORE a `write`/`edit`/`delete` mutates it. First touch per session wins (the
/// store is `INSERT OR IGNORE`), so repeated edits keep the session-start snapshot
/// and the GUI diff tab shows cumulative session changes even in a non-git
/// directory. A missing file (the create case) stores an `"empty"` marker; oversize
/// and binary content store `"toolarge"`/`"binary"` markers with no bytes.
///
/// Best-effort + no-op without a session, exactly like [`record_change`]: baseline
/// bookkeeping must never fail (or slow-fail) the fs op the model asked for.
pub(crate) fn capture_baseline(ctx: &ToolCtx, path: &Path) {
    let Some(session_dir) = ctx.session_dir.as_ref() else {
        return;
    };
    let display = display_key(ctx, path);
    let (kind, content): (&str, Option<Vec<u8>>) = match std::fs::read(path) {
        // Not on disk yet — the tool is about to CREATE it: baseline = empty file.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => ("empty", None),
        // Exists but unreadable (permissions, is-a-directory, …): the mutation is
        // about to fail anyway — record NOTHING rather than poison the first-touch
        // slot with a bogus "empty" baseline.
        Err(_) => return,
        Ok(bytes) if bytes.len() > BASELINE_SIZE_CAP => ("toolarge", None),
        // NUL in the first 8KiB or invalid UTF-8 = not diffable text; marker only
        // (mirrors the GUI diff tab's own binary sniff).
        Ok(bytes)
            if bytes[..bytes.len().min(8192)].contains(&0)
                || std::str::from_utf8(&bytes).is_err() =>
        {
            ("binary", None)
        }
        Ok(bytes) => ("text", Some(bytes)),
    };
    let _ =
        crate::model::msglog::record_file_baseline(session_dir, &display, kind, content.as_deref());
}

/// Maximum number of paths shown per side in the L3 neighborhood footer.
const NEIGHBORHOOD_FOOTER_CAP: usize = 8;

/// Best-effort neighborhood footer for L3: queries the linker daemon for the
/// 1-hop import graph of `rel` and appends a `# Related (graph)` section to
/// `out`. If the daemon is not running, the file has no graph edges, or the
/// fetch fails, this is a silent no-op.
pub(crate) fn append_neighborhood_footer(out: &mut String, rel: &str) {
    let Some((imports, imported_by)) = crate::linker::client::fetch_neighborhood(rel) else {
        return;
    };
    if imports.is_empty() && imported_by.is_empty() {
        return;
    }
    out.push_str("\n---\n# Related (graph)\n");
    if !imports.is_empty() {
        let display: Vec<_> = imports.iter().take(NEIGHBORHOOD_FOOTER_CAP).cloned().collect();
        let suffix = if imports.len() > NEIGHBORHOOD_FOOTER_CAP {
            format!(" (and {} more)", imports.len() - NEIGHBORHOOD_FOOTER_CAP)
        } else {
            String::new()
        };
        out.push_str(&format!("imports: {}{}\n", display.join(", "), suffix));
    }
    if !imported_by.is_empty() {
        let display: Vec<_> = imported_by
            .iter()
            .take(NEIGHBORHOOD_FOOTER_CAP)
            .cloned()
            .collect();
        let suffix = if imported_by.len() > NEIGHBORHOOD_FOOTER_CAP {
            format!(" (and {} more)", imported_by.len() - NEIGHBORHOOD_FOOTER_CAP)
        } else {
            String::new()
        };
        out.push_str(&format!("imported-by: {}{}\n", display.join(", "), suffix));
    }
}
