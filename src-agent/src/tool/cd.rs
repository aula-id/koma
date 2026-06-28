//! The `cd` tool: REPOINT the live session's working directory (Phase 8).
//!
//! Most tools are read-only against [`ToolCtx`]; `cd` is the exception — it must
//! MUTATE session state (the active cwd, the dir cache, the awareness summary).
//! It does that the same way the `task` tool does: the runtime
//! (`app::runtime::stream::process_tools`) INTERCEPTS a `cd` call BEFORE the
//! generic dispatch path, resolves + validates the target itself, and applies the
//! change to the live `SessionRuntime`. [`Tool::run`] here only does the pure,
//! side-effect-free part — resolve the requested path against the context and
//! return either a [`CWD_CHANGE_PREFIX`]-tagged canonical target (success) or an
//! `error:`/refusal string — so the runtime can apply the repoint without
//! duplicating the resolution logic. It is never reached through the generic
//! dispatcher.
//!
//! RESOLUTION (shell-like), in priority order:
//! - exactly `[N]` (N an integer) → the session's `workdirs[N]` root. Only valid
//!   when the session has MULTIPLE roots (a single-root session has no `[1]` etc.);
//!   `[0]` is accepted in both cases.
//! - an ABSOLUTE path → used as-is.
//! - anything else → RELATIVE to the session's CURRENT cwd (`ctx.workspace`).
//!
//! MODEL RESTRICTION (enforced HERE, the model-callable path): the resolved
//! canonical target must be UNDER one of the session's allowed roots
//! (`ctx.workspaces`). Outside → REFUSE (the cwd is NOT changed). The user `/cd`
//! command bypasses this check entirely (it does its own resolution — see
//! `app::runtime::commands::cd`).

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::{json, Value};

use super::{Tool, ToolCtx};

/// Sentinel prefix on a successful [`Cd::run`] result. The runtime's `cd`
/// interception recognises this, strips it to recover the canonical target path,
/// and applies the workspace change; the model never sees it (the interception
/// replaces it with a human-readable confirmation). A normal `error:` result has
/// no such prefix and is surfaced to the model verbatim.
pub const CWD_CHANGE_PREFIX: &str = "__CWD_CHANGE__:";

/// Resolve a `cd` target string against a current cwd + the allowed roots, with
/// the shell-like `[N]` / absolute / relative rules described on the module.
///
/// Returns the RESOLVED (not-yet-canonicalised) path on success, or an `Err` with
/// a user-facing message for an out-of-range `[N]`. Shared by the model tool (via
/// [`Cd::run`]) and the user `/cd` command so their resolution can never diverge;
/// the existence/dir + allow-list checks are applied by the respective callers.
pub fn resolve_cd_target(path: &str, cwd: &Path, workspaces: &[PathBuf]) -> Result<PathBuf> {
    let trimmed = path.trim();
    // `[N]` — jump straight to a configured root by index. Only the exact `[N]`
    // form (nothing after the bracket) is a root jump; `[N]sub/dir` is left to the
    // path branches below (it is not a cd-root selector).
    if let Some(idx) = parse_bare_index(trimmed) {
        return workspaces.get(idx).cloned().ok_or_else(|| {
            anyhow::anyhow!(
                "workspace index [{idx}] out of range (have {} root{})",
                workspaces.len(),
                if workspaces.len() == 1 { "" } else { "s" }
            )
        });
    }
    let as_path = Path::new(trimmed);
    if as_path.is_absolute() {
        Ok(as_path.to_path_buf())
    } else {
        // Relative → resolve against the CURRENT cwd (shell semantics). An empty
        // path degrades to the cwd itself (a no-op cd), matching a bare `cd`.
        Ok(cwd.join(trimmed))
    }
}

/// Parse a string that is EXACTLY `[N]` (a non-negative integer in brackets,
/// nothing else) into `N`. Returns `None` for any other shape (including
/// `[N]trailing`), so only a bare root selector is treated as one.
fn parse_bare_index(s: &str) -> Option<usize> {
    let inner = s.strip_prefix('[')?.strip_suffix(']')?;
    inner.parse::<usize>().ok()
}

/// Canonicalise an existing path (resolving symlinks + `..`), returning the
/// canonical form. Errors when the path does not exist. Kept as a tiny helper so
/// the tool and the `/cd` command canonicalise identically before any containment
/// check / before storing the new cwd.
pub fn canonicalize_existing(p: &Path) -> Result<PathBuf> {
    p.canonicalize()
        .map_err(|e| anyhow::anyhow!("cannot resolve '{}': {e}", p.display()))
}

/// Change the session's working directory. The resolved target must be an
/// existing directory UNDER one of the session's allowed roots.
pub struct Cd;
impl Tool for Cd {
    fn name(&self) -> &'static str {
        "cd"
    }

    fn description(&self) -> &'static str {
        "Change your working directory for subsequent shell commands and relative \
         paths (like `cd` in a shell). `path` may be absolute, relative to your \
         current directory, or `[N]` to jump to configured workspace root N (when \
         several roots are configured). The target must exist and be a directory \
         INSIDE one of your allowed workspace roots — a path outside them is \
         refused and your directory is left unchanged."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Directory to change to: absolute, relative to the current directory, or `[N]` for workspace root N."
                }
            },
            "required": ["path"]
        })
    }

    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing required string argument 'path'"))?;

        // Resolve with the shared shell-like rules, against the CURRENT cwd.
        let resolved = match resolve_cd_target(path, &ctx.workspace, &ctx.workspaces) {
            Ok(p) => p,
            Err(e) => return Ok(format!("error: {e}")),
        };
        // Must exist + be a directory (canonicalize first so the containment check
        // and the stored cwd both use the symlink-resolved real path).
        let canonical = match canonicalize_existing(&resolved) {
            Ok(p) => p,
            Err(e) => return Ok(format!("error: {e}")),
        };
        if !canonical.is_dir() {
            return Ok(format!("error: '{}' is not a directory", canonical.display()));
        }
        // MODEL RESTRICTION: the target must be under an allowed root. `/cd`
        // (user) skips this; the model may only roam inside its workspaces.
        if super::find_workspace(&ctx.workspaces, &canonical).is_none() {
            return Ok(format!(
                "error: '{}' is outside your allowed workspace roots; cd refused",
                canonical.display()
            ));
        }
        // Success — hand the canonical target back to the runtime to apply.
        Ok(format!("{CWD_CHANGE_PREFIX}{}", canonical.display()))
    }
}
