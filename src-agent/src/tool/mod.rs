//! Tool foundation for the agentic loop.
//!
//! A [`Tool`] is a callable shaped for OpenRouter function-calling: it exposes a
//! name, a description, and a JSON-Schema `parameters` object, and runs against a
//! shared [`ToolCtx`] (the session's workspace root + the background file cache).
//! [`all_tools`] returns the built-in set; [`resolve`] sandboxes every path so a
//! tool can never touch anything outside the workspace.
//!
//! The trait, the registry, the tool structs, and [`resolve`] are driven by the
//! agentic loop: `service::openrouter::stream_complete` advertises a caller-chosen
//! subset of the tool set to the model (the main loop uses [`main_tool_names`],
//! which hides agent-only tools; each sub-agent advertises only its allow-list),
//! and `app::runtime::stream::run_tool` dispatches the model's requested calls
//! back through [`Tool::run`].

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use anyhow::{bail, Result};
use serde_json::Value;

pub mod cd;
pub mod dircache;
pub mod fs;
pub mod git_cred;
pub mod git_operator;
pub mod git_worktree;
pub mod internet;
pub mod memory;
pub mod plan;
pub mod pong;
pub mod search;
pub mod seqthink;
pub mod shell;
pub mod shell_filter;
pub mod task;
pub mod todo;

pub use dircache::DirCache;

/// True for built-in tools that mutate the workspace, run arbitrary shell
/// commands, mutate git state (local or remote, e.g. `git_operator` push /
/// reset), or fetch from the network and write the result to disk
/// (`web_download`) — and therefore require approval in Normal mode.
/// Deterministic, name-based — no classifier / network call. Canonical
/// single-source definition used by both the interactive approval gate and the
/// sub-agent engine. NOTE: `git_worktree remove` is gated separately, inside its
/// interception in `process_tools` (it never reaches this generic gate).
pub(crate) fn tool_is_risky(name: &str) -> bool {
    matches!(name, "write" | "delete" | "edit" | "bash" | "git_operator" | "web_download")
}

/// Tools reachable while [`crate::app::state::AgentMode::Plan`] is active — the
/// read-only / reasoning / delegation surface a planning turn is allowed to use.
/// Consumed by: the main advertise fold (`app::runtime::stream::run`), the
/// sub-agent allow-list fold (`app::subagent::spawn`), and the defense-in-depth
/// deny gate inside `process_tools` (`app::runtime::stream::tools::approval`) —
/// so a fabricated/stale call name can never mutate anything while Plan is
/// active, even if it slipped past the advertise fold. `git_operator` is allowed
/// here at the tool-name level only; per-subcommand read-only filtering is a
/// separate check, [`plan_git_subcommand_allowed`], since Plan may run READ git
/// but not `commit`/`push`/`checkout`/etc. `checklist` is allowed so the model can
/// maintain the plan checklist; while Plan is active it is FULLY INTERCEPTED (see
/// `process_tools`) to write the session-scoped `plan_todos.md` + auto-managed
/// rails, never the per-directory workspace `TODO.md`.
pub(crate) fn tool_allowed_in_plan(name: &str) -> bool {
    matches!(name,
        "read" | "grep" | "glob" | "dir_list" | "dir_cache_update" | "recall"
        | "web_search" | "web_fetch" | "pong" | "cd" | "git_cred"
        | "task" | "task_output" | "task_kill" | "bash_output" | "bash_kill"
        | "git_operator" | "seqthink" | "plan_ready" | "plan_enter" | "checklist")
}

/// True when `subcmd` (the first element of a `git_operator` call's `args`
/// array) is a read-only git subcommand — the only ones Plan mode may run
/// through `git_operator` (which [`tool_allowed_in_plan`] otherwise lets
/// through at the tool-name level). Anything else (`commit`, `push`,
/// `checkout`, `reset`, …) is denied so Plan mode's git access stays genuinely
/// read-only.
pub(crate) fn plan_git_subcommand_allowed(subcmd: &str) -> bool {
    matches!(subcmd,
        "status" | "log" | "diff" | "show" | "blame" | "branch" | "remote"
        | "rev-parse" | "describe" | "shortlog" | "ls-files" | "ls-remote")
}

/// Shared context handed to every tool invocation.
pub struct ToolCtx {
    /// Absolute workspace root (the session's primary workdir). All tool paths
    /// are resolved against this and may not escape it.
    pub workspace: PathBuf,
    /// All configured workspace roots (may be >1 when the user lists multiple
    /// workdirs in settings). Indexed as [0], [1], etc. in `@`-prefixed paths.
    pub workspaces: Vec<PathBuf>,
    pub dir_cache: Arc<RwLock<DirCache>>,
    /// The per-PROJECT memory directory (`<pwd_bucket_dir>/memory/`), shared across
    /// every session opened from the same working dir. `None` when no session is active.
    pub memory_dir: Option<PathBuf>,
    /// The shadow worktree dir for this session's pwd bucket
    /// (`~/.koma/sessions/<pwd_hash>/worktrees/`). `None` when no session is active.
    pub worktrees_dir: Option<std::path::PathBuf>,
    /// The per-session media download directory (`<pwd_bucket_dir>/media/`),
    /// where `web_download` saves files. `None` when no session is active.
    pub download_dir: Option<PathBuf>,
    /// The session's active internet tier. `web_fetch` reads this to decide
    /// between the simple raw-HTTP path and the Full browser backend (scrapion);
    /// defaults to `Simple` when no session is available.
    pub internet_mode: crate::model::settings::InternetMode,
    /// The bare filename of the SSH identity key selected for this session
    /// (e.g. `"id_ed25519"`). Set by `git_cred action="select"`; read by
    /// `git_operator` to inject `-i ~/.ssh/<key>` into git commands. `None`
    /// means no key has been selected yet.
    pub ssh_key: Option<String>,
    /// The GLOBAL MCP client manager, shared across every session. `Some` once
    /// startup builds it (it lives in `AppStateRest`); used by [`execute_tool`] to
    /// dispatch `mcp__*` tool calls to their owning server. `None` where no manager
    /// is available (e.g. headless/test constructions) — MCP calls then fall through
    /// to the "unknown tool" error, exactly as before MCP existed.
    pub mcp_manager: Option<Arc<crate::app::mcp::McpManager>>,
    /// The GLOBAL security daemon client manager, shared across every session.
    /// `Some` once startup builds it (it lives in `AppStateRest`); used by
    /// [`execute_tool`] to dispatch `sec_*` tool calls to the daemon. `None` where
    /// no manager is available (e.g. headless/test constructions) — sec calls then
    /// fall through to the "unknown tool" error.
    pub sec_manager: Option<Arc<crate::app::sec::SecDaemonManager>>,
    /// Whether `bash`/`git_operator` should run their "saving" output path
    /// (noise filtering + tee-to-disk on truncation/non-zero-exit). Defaults to
    /// true everywhere until a settings toggle exists to turn it off per-session.
    pub bash_saving: bool,
    /// Directory tee'd full command logs are written to when `bash_saving` is
    /// true (`<session_dir>/opt/`). `None` when no session is active (or for
    /// headless/test constructions), in which case tee is simply skipped.
    pub bash_log_dir: Option<PathBuf>,
    /// The active session's own directory (`~/.koma/sessions/<pwd_hash>/<uuid>/`),
    /// holding `plan.md`, `plan_todos.md`, `settings.json`, etc. Used ONLY by
    /// [`resolve_read`]'s sessions exemption so the model can read files inside
    /// its OWN session dir via an absolute path — never any other session's.
    /// `None` when no session is active (or for headless/test constructions), in
    /// which case the exemption is simply skipped.
    pub session_dir: Option<PathBuf>,
}

/// Parse a `[N]` workspace-index prefix from the start of a path string.
/// If the path starts with `[digits]`, returns `(index, rest)`.
/// Otherwise returns `(0, original)` — a bare path resolves against workspace 0.
pub fn parse_ws_prefix(path: &str) -> (usize, &str) {
    if !path.starts_with('[') {
        return (0, path);
    }
    if let Some(end) = path.find(']') {
        if let Ok(idx) = path[1..end].parse::<usize>() {
            return (idx, &path[end + 1..]);
        }
    }
    (0, path)
}

/// A callable tool, shaped for OpenRouter function-calling: `parameters` is a
/// JSON Schema object; `run` takes the decoded arguments and returns a string
/// result fed back to the model.
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters(&self) -> Value;
    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String>;
}

/// The built-in tool set.
pub fn all_tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(fs::Read),
        Box::new(search::Grep),
        Box::new(search::Glob),
        Box::new(fs::Write),
        Box::new(fs::Edit),
        Box::new(fs::Delete),
        Box::new(shell::Bash),
        Box::new(shell::BashOutput),
        Box::new(shell::BashKill),
        Box::new(cd::Cd),
        Box::new(fs::DirList),
        Box::new(dircache::DirCacheUpdate),
        Box::new(pong::Pong),
        Box::new(memory::Remember),
        Box::new(memory::Forget),
        Box::new(memory::Recall),
        Box::new(task::Task),
        Box::new(task::TaskOutput),
        Box::new(task::TaskKill),
        Box::new(todo::Checklist),
        Box::new(internet::WebFetch),
        Box::new(internet::WebSearch),
        Box::new(internet::WebDownload),
        Box::new(git_cred::GitCred),
        Box::new(git_operator::GitOperator),
        Box::new(git_worktree::GitWorktree),
        Box::new(plan::PlanEnter),
        Box::new(plan::PlanReady),
        Box::new(seqthink::SeqThink),
    ]
}

/// Tool names the /agents editor's tool picker EXCLUDES from the selectable list —
/// internal / infra tools a sub-agent should never be handed: `task` (the recursion
/// guard), `pong` (a health no-op), and `dir_cache_update` (an internal reindex trigger).
const AGENT_PICKER_EXCLUDED: &[&str] = &["task", "pong", "dir_cache_update"];

/// The user-selectable tool names for the /agents editor, in [`all_tools`] SOURCE ORDER
/// (deterministic): every built-in tool name except [`AGENT_PICKER_EXCLUDED`].
///
/// THE SINGLE SOURCE OF TRUTH shared by the TUI tool picker
/// ([`crate::app::mode::agents::ToolPickerState::from_draft`]) and the GUI /agents
/// dashboard's `AgentsValues` envelope, so both offer exactly the same set — the daemon's
/// attached reply, the host's un-attached fallback, and the TUI picker can never drift.
pub fn agent_selectable_tools() -> Vec<String> {
    all_tools()
        .iter()
        .map(|t| t.name().to_string())
        .filter(|n| !AGENT_PICKER_EXCLUDED.contains(&n.as_str()))
        .collect()
}

/// Tools that are NEVER advertised to the main chat model via
/// [`main_tool_names`] — the caller pushes them onto `advertise` explicitly,
/// mode-gated, instead (see `app::runtime::stream::run::start_stream_task`).
/// `plan_enter` (the request to enter Plan mode) is advertised only OUTSIDE Plan
/// mode; `seqthink` (structured reasoning) and `plan_ready` (present a finished
/// plan) are advertised only WHILE Plan mode is active. None of the three can
/// live in the unconditional `main_tool_names` set, since each is gated on
/// `agent_mode`.
const INTERNAL_ONLY: &[&str] = &["seqthink", "plan_enter", "plan_ready"];

/// Tools that MUST run off the UI/event-loop thread because they do blocking
/// I/O — running them inline in `process_tools` (which executes on the main
/// event-loop thread) would freeze the TUI for the whole call (no redraw, no
/// input), so the status-bar comet appears to stall. This covers:
/// - the network tools: `web_search` and the simple-tier `web_fetch` do a
///   blocking `reqwest` GET (via [`internet::http_get_blocking`]); in Full mode
///   `web_fetch` instead drives a scrapion subprocess;
/// - the filesystem tools: `read` / `write` / `edit` / `delete` block on
///   `fs::read_to_string` / `fs::write` / `fs::remove_file` (a huge file write
///   can take long enough to drop frames);
/// - `bash`, which spawns a subprocess and waits for it to finish;
/// - `grep` / `glob`, which walk the workspace tree;
/// - the memory tools `remember` / `forget` / `recall`, which read + rewrite
///   the per-project memory index (`MEMORY.md`) and its `<slug>.md` files.
///
/// `process_tools` intercepts any call whose name is in this list (AFTER the
/// approval/classifier gate has cleared it) and runs it on a plain `std::thread`,
/// then PARKS the tool round until the result lands — and crucially runs the
/// round's deferred tools ONE AT A TIME (park-after-each, resume continues at the
/// next call), so two writes/edits to the same file in one round can't race.
///
/// Truly-instant tools are deliberately NOT listed (they'd pay a needless park
/// round for nothing): `pong` returns a constant, `dir_list` reads the in-memory
/// dir cache, `dir_cache_update` returns immediately after kicking off a
/// background reindex, and `task` is intercepted by `process_tools` before this
/// check (it delegates to a sub-agent on its own lane).
pub const DEFERRED_TOOLS: &[&str] = &[
    "read", "write", "edit", "delete", "bash", "grep", "glob",
    "remember", "forget", "recall",
    "web_fetch", "web_search", "web_download",
    "git_operator",
];

/// Tool names advertised to the MAIN chat model (everything except agent-only
/// tools). Used by the interactive loop's `stream_complete` call so the main
/// model never sees [`INTERNAL_ONLY`] tools.
pub fn main_tool_names() -> Vec<String> {
    all_tools()
        .iter()
        .map(|t| t.name().to_string())
        .filter(|n| !INTERNAL_ONLY.contains(&n.as_str()))
        .collect()
}

/// Resolve a path (optionally with `[N]` workspace-index prefix) and enforce
/// containment. A bare path like `src/main.rs` resolves against workspace 0.
/// A prefixed path like `[2]src/main.rs` resolves against workspace 2.
///
/// SCRATCH BYPASS: if `rel` is an absolute path that starts with the koma
/// scratch root (`<temp>/koma`), it is returned as-is (no workspace required).
/// Only absolute paths get this bypass; relative paths still resolve against
/// the workspace as normal.
pub fn resolve(workspaces: &[PathBuf], rel: &str) -> Result<PathBuf> {
    // Scratch bypass: absolute paths under the scratch root skip workspace
    // containment entirely. The scratch root itself exists once a session is
    // active, so canonicalize succeeds; for deeper paths that don't yet exist
    // we accept the literal path (same partial-canonicalize logic as below).
    let as_path = Path::new(rel);
    if as_path.is_absolute() {
        let scratch = crate::model::store::scratch_root();
        // Normalize the candidate the same way we do for workspace paths, so
        // `..` tricks inside the scratch tree can't escape it.
        let candidate = match as_path.canonicalize() {
            Ok(p) => p,
            Err(_) => {
                // Partial-canonicalize: walk up to the longest existing prefix,
                // re-append the non-existent tail.
                let mut existing = as_path;
                let mut tail: Vec<std::ffi::OsString> = Vec::new();
                while !existing.exists() {
                    match existing.file_name() {
                        Some(n) => tail.push(n.to_os_string()),
                        None => break,
                    }
                    match existing.parent() {
                        Some(p) => existing = p,
                        None => break,
                    }
                }
                let mut base = existing.canonicalize().unwrap_or_else(|_| existing.to_path_buf());
                for seg in tail.iter().rev() {
                    base.push(seg);
                }
                base
            }
        };
        if candidate.starts_with(&scratch) {
            return Ok(candidate);
        }
    }

    let (ws_idx, bare) = parse_ws_prefix(rel);
    let base = workspaces.get(ws_idx)
        .ok_or_else(|| anyhow::anyhow!("workspace index [{ws_idx}] out of range (have {})", workspaces.len()))?;
    let ws = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    let joined = ws.join(bare);
    // Canonicalize as far as exists, then re-append the non-existent tail, so
    // `..` tricks are normalised out before the containment check.
    let candidate = match joined.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            let mut existing = joined.as_path();
            let mut tail: Vec<std::ffi::OsString> = Vec::new();
            while !existing.exists() {
                match existing.file_name() {
                    Some(n) => tail.push(n.to_os_string()),
                    None => break,
                }
                match existing.parent() {
                    Some(p) => existing = p,
                    None => break,
                }
            }
            let mut base = existing.canonicalize().unwrap_or_else(|_| existing.to_path_buf());
            for seg in tail.iter().rev() {
                base.push(seg);
            }
            base
        }
    };
    // Containment: must be inside the resolved workspace.
    if !candidate.starts_with(&ws) {
        bail!("path '{bare}' is outside workspace [{ws_idx}]");
    }
    Ok(candidate)
}

/// Resolve a path for READ-ONLY tools, forgiving a dropped [N] prefix.
/// An explicit [N] prefix is honoured strictly (same as resolve). A BARE path
/// resolves against workspace 0 first; if that file does not exist on disk, the
/// other workspaces are tried by existence and the first physical match wins.
/// This lets weak models that drop the [N] prefix still READ a file that only
/// lives in another workspace, while writes (which keep using resolve) stay
/// strictly pinned to workspace 0 unless an explicit [N] is given.
///
/// SCRATCH BYPASS: delegates to `resolve` which allows absolute paths under
/// the scratch root through without workspace containment checks.
///
/// SESSION BYPASS (read-only, NOT mirrored into `resolve`): absolute paths
/// under the ACTIVE session's own directory (`session_dir`, e.g.
/// `~/.koma/sessions/<pwd_hash>/<uuid>/`) are also let through directly. This
/// is what lets the model `read` its own `plan.md` (and other session
/// artifacts) after a plain plan approval, whose path names a location outside
/// every configured workspace root. It is intentionally scoped to
/// `read`/`resolve_read` only — `resolve` is shared with `write`/`edit`/
/// `delete`, and those must stay workspace-contained, so this exemption is NOT
/// added there. It is ALSO intentionally scoped to only the caller's OWN
/// session directory, not the whole `sessions/` tree: `resolve_read` also
/// backs `grep`/`glob`, so widening this to every session would let the model
/// read/grep every past session's chat history across all projects.
/// `session_dir` is `None` when no session is active, in which case this
/// exemption is simply skipped (falls through to normal workspace resolution).
pub fn resolve_read(workspaces: &[PathBuf], rel: &str, session_dir: Option<&Path>) -> Result<PathBuf> {
    // Absolute scratch-root paths: let resolve() handle the bypass.
    let as_path = Path::new(rel);
    if as_path.is_absolute() {
        let scratch = crate::model::store::scratch_root();
        // Quick containment check before canonicalize (scratch dir may not
        // exist yet for a brand-new session).
        let candidate = as_path.canonicalize().unwrap_or_else(|_| as_path.to_path_buf());
        if candidate.starts_with(&scratch) {
            return resolve(workspaces, rel);
        }
        // Session bypass: exempt ONLY the active session's own directory.
        // Partial-canonicalize the same way `resolve()` does for the scratch
        // bypass above, so `..` tricks can't walk the candidate back out of
        // the session dir before the containment check.
        if let Some(session_dir) = session_dir {
            let normalized = match as_path.canonicalize() {
                Ok(p) => p,
                Err(_) => {
                    let mut existing = as_path;
                    let mut tail: Vec<std::ffi::OsString> = Vec::new();
                    while !existing.exists() {
                        match existing.file_name() {
                            Some(n) => tail.push(n.to_os_string()),
                            None => break,
                        }
                        match existing.parent() {
                            Some(p) => existing = p,
                            None => break,
                        }
                    }
                    let mut base = existing.canonicalize().unwrap_or_else(|_| existing.to_path_buf());
                    for seg in tail.iter().rev() {
                        base.push(seg);
                    }
                    base
                }
            };
            if normalized.starts_with(session_dir) {
                return Ok(normalized);
            }
        }
    }

    if rel.starts_with('[') {
        return resolve(workspaces, rel);
    }
    let primary = resolve(workspaces, rel)?;
    if primary.exists() {
        return Ok(primary);
    }
    for idx in 1..workspaces.len() {
        if let Ok(p) = resolve(workspaces, &format!("[{idx}]{rel}")) {
            if p.exists() {
                return Ok(p);
            }
        }
    }
    Ok(primary)
}

/// Pure tool dispatcher: given a ready [`ToolCtx`] and a [`ToolCall`], loop
/// all built-in tools, parse arguments, dispatch to the matching tool's
/// [`Tool::run`], and return the result string. Does NOT touch any app state.
///
/// Error strings match exactly what `run_tool` produced before the refactor:
/// - `"error: <msg>"` on a tool execution failure
/// - `"error: unknown tool '<name>'"` when no tool matches
pub fn execute_tool(ctx: &ToolCtx, call: &crate::dto::chat::ToolCall) -> String {
    // OpenAI/OpenRouter send `arguments` as a JSON-encoded string; an empty or
    // malformed payload degrades to `{}` so the tool sees no arguments. Sanitize
    // first: a non-delta provider may have produced a duplicated `{...}{...}`
    // string (valid JSON document + trailing copy) that `from_str` would reject
    // outright — collapsing it to one clean value here recovers the real arguments
    // (e.g. the bash `command`) instead of silently degrading to `{}`. A single
    // clean value is unchanged, so the normal path is unaffected.
    let sanitized = crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
    let args: serde_json::Value =
        serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));
    for tool in all_tools() {
        if tool.name() == call.function.name {
            return match tool.run(ctx, &args) {
                Ok(s) => s,
                Err(e) => format!("error: {e}"),
            };
        }
    }
    // No built-in tool matched. An `mcp__*` name is a remote MCP tool: dispatch it
    // through the manager's sync→async bridge. Only attempted when a manager is
    // present (it always is in the live app), so a build/test ToolCtx without one
    // falls through to the unknown-tool error unchanged.
    if call.function.name.starts_with("mcp__") {
        if let Some(mgr) = ctx.mcp_manager.as_ref() {
            return mgr
                .execute_blocking(&call.function.name, &args)
                .unwrap_or_else(|e| format!("error: {e}"));
        }
    }
    if call.function.name.starts_with("sec_") {
        if let Some(mgr) = ctx.sec_manager.as_ref() {
            return mgr
                .execute_blocking(&call.function.name, &args)
                .unwrap_or_else(|e| format!("error: {e}"));
        }
    }
    format!("error: unknown tool '{}'", call.function.name)
}

/// Find which workspace contains the given absolute path.
/// Returns the canonicalized workspace root if found.
pub fn find_workspace(workspaces: &[PathBuf], abs: &Path) -> Option<PathBuf> {
    for ws in workspaces {
        let ws = ws.canonicalize().unwrap_or_else(|_| ws.clone());
        if abs.starts_with(&ws) {
            return Some(ws);
        }
    }
    None
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;
