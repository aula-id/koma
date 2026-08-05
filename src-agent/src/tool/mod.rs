//! Tool foundation for the agentic loop.
//!
//! A [`Tool`] is a callable shaped for OpenRouter function-calling: it exposes a
//! name, a description, and a JSON-Schema `parameters` object, and runs against
//! a shared [`ToolCtx`] (the session's workspace root + the background file
//! cache). [`all_tools`] returns the built-in set; [`resolve`] sandboxes every
//! path so a tool can never touch anything outside the workspace.
//!
//! The trait, the registry, the tool structs, and [`resolve`] are driven by the
//! agentic loop: `service::openrouter::stream_complete` advertises a
//! caller-chosen subset of the tool set to the model (the main loop uses
//! [`main_tool_names`], which hides agent-only tools; each sub-agent advertises
//! only its allow-list), and `app::runtime::stream::run_tool` dispatches the
//! model's requested calls back through [`Tool::run`].

use anyhow::{bail, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

pub mod cd;
pub mod dircache;
pub mod fs;
pub mod git_cred;
pub mod graph;
pub mod git_operator;
pub mod git_worktree;
pub mod history;
pub mod internet;
pub mod memory;
pub mod plan;
pub mod pong;
pub mod search;
pub mod sdlc;
pub mod seqthink;
pub mod shell;
pub mod shell_filter;
pub mod skill;
pub mod task;
pub mod todo;

pub use dircache::DirCache;

/// True for built-in tools that mutate the workspace, run arbitrary shell
/// commands, mutate git state (local or remote, e.g. `git_operator` push /
/// `reset`), or fetch from the network and write the result to disk
/// (`web_download`) — and therefore require approval in Normal mode.
/// Deterministic, name-based — no classifier / network call. Canonical
/// single-source definition used by both the interactive approval gate and the
/// sub-agent engine. NOTE: `git_worktree remove` is gated separately, inside
/// its interception in `process_tools` (it never reaches this generic gate).
pub(crate) fn tool_is_risky(name: &str) -> bool {
    matches!(
        name,
        "write" | "delete" | "edit" | "bash" | "git_operator" | "web_download"
    )
}

/// Tools reachable while [`crate::app::state::AgentMode::Plan`] is active — the
/// read-only / reasoning / delegation surface a planning turn is allowed to use.
pub(crate) fn tool_allowed_in_plan(name: &str) -> bool {
    matches!(
        name,
        "read"
            | "grep"
            | "glob"
            | "dir_list"
            | "dir_cache_update"
            | "recall"
            | "message_find"
            | "skill"
            | "web_search"
            | "web_fetch"
            | "pong"
            | "cd"
            | "git_cred"
            | "task"
            | "task_output"
            | "task_kill"
            | "task_send"
            | "bash_output"
            | "bash_kill"
            | "git_operator"
            | "seqthink"
            | "plan_ready"
            | "plan_enter"
            | "checklist"
            | "graph_query"
    )
}

/// True when `subcmd` (the first element of a `git_operator` call's `args`
/// array) is a read-only git subcommand — the only ones Plan mode may run
/// through `git_operator`.
pub(crate) fn plan_git_subcommand_allowed(subcmd: &str) -> bool {
    matches!(
        subcmd,
        "status"
            | "log"
            | "diff"
            | "show"
            | "blame"
            | "branch"
            | "remote"
            | "rev-parse"
            | "describe"
            | "shortlog"
            | "ls-files"
            | "ls-remote"
    )
}

/// Shared context handed to every tool invocation.
pub struct ToolCtx {
    /// Absolute workspace root (the session's primary workdir).
    pub workspace: PathBuf,
    /// All configured workspace roots (may be >1).
    pub workspaces: Vec<PathBuf>,
    pub dir_cache: Arc<RwLock<DirCache>>,
    /// The per-PROJECT memory directory.
    pub memory_dir: Option<PathBuf>,
    /// The shadow worktree dir for this session's pwd bucket.
    pub worktrees_dir: Option<std::path::PathBuf>,
    /// The per-session media download directory.
    pub download_dir: Option<PathBuf>,
    /// The session's active internet tier.
    pub internet_mode: crate::model::settings::InternetMode,
    /// The bare filename of the SSH identity key selected for this session.
    pub ssh_key: Option<String>,
    /// Skill catalogue snapshot from the session.
    pub skill_registry: Option<crate::model::skill::SkillRegistry>,
    /// Names of currently active (loaded) skills, for the `list` action.
    pub active_skill_names: Option<Vec<String>>,
    /// The GLOBAL MCP client manager.
    pub mcp_manager: Option<Arc<crate::app::mcp::McpManager>>,
    /// The GLOBAL security daemon client manager.
    pub sec_manager: Option<Arc<crate::app::sec::SecDaemonManager>>,
    /// Whether `bash`/`git_operator` should run their "saving" output path.
    pub bash_saving: bool,
    /// Directory tee'd full command logs.
    pub bash_log_dir: Option<PathBuf>,
    /// The active session's own directory.
    pub session_dir: Option<PathBuf>,
    /// Absolute paths of currently loaded dir-form skill directories.
    pub active_skill_dirs: Vec<PathBuf>,
}

/// Parse a `[N]` workspace-index prefix from the start of a path string.
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

/// Format a path for model/tool results.
pub fn model_display_path(workspaces: &[PathBuf], abs: &Path) -> String {
    if workspaces.len() <= 1 {
        if let Some(ws) = workspaces.first() {
            if let Ok(rel) = abs.strip_prefix(ws) {
                return rel.to_string_lossy().replace('\\', "/");
            }
        }
        return abs.to_string_lossy().replace('\\', "/");
    }
    abs.to_string_lossy().replace('\\', "/")
}

/// Convert a stored DirCache entry to a model-facing path.
pub fn model_path_from_entry(workspaces: &[PathBuf], entry: &str) -> String {
    let (ws_idx, bare) = parse_ws_prefix(entry);
    if workspaces.len() <= 1 {
        return bare.replace('\\', "/");
    }
    if let Some(ws) = workspaces.get(ws_idx) {
        ws.join(bare).to_string_lossy().replace('\\', "/")
    } else {
        bare.replace('\\', "/")
    }
}

/// A callable tool, shaped for OpenRouter function-calling.
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
        Box::new(graph::GraphQuery),
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
        Box::new(skill::Skill),
        Box::new(history::MessageFind),
        Box::new(task::Task),
        Box::new(task::TaskOutput),
        Box::new(task::TaskKill),
        Box::new(task::TaskSend),
        Box::new(todo::Checklist),
        Box::new(internet::WebFetch),
        Box::new(internet::WebSearch),
        Box::new(internet::WebDownload),
        Box::new(git_cred::GitCred),
        Box::new(git_operator::GitOperator),
        Box::new(git_worktree::GitWorktree),
        Box::new(plan::PlanEnter),
        Box::new(plan::PlanReady),
        Box::new(sdlc::MissionReady),
        Box::new(sdlc::MissionVerify),
        Box::new(sdlc::MissionIntegrate),
        Box::new(seqthink::SeqThink),
    ]
}

/// Tool names the /agents editor's tool picker EXCLUDES from the selectable list.
const AGENT_PICKER_EXCLUDED: &[&str] = &["task", "task_send", "pong", "dir_cache_update", "skill"];

/// The user-selectable tool names for the /agents editor.
pub fn agent_selectable_tools() -> Vec<String> {
    all_tools()
        .iter()
        .map(|t| t.name().to_string())
        .filter(|n| !AGENT_PICKER_EXCLUDED.contains(&n.as_str()))
        .collect()
}

/// Tools that are NEVER advertised to the main chat model via
/// [`main_tool_names`] — the caller pushes them onto `advertise` explicitly,
/// mode-gated, instead.
const INTERNAL_ONLY: &[&str] = &[
    "seqthink",
    "plan_enter",
    "plan_ready",
    "mission_ready",
    "mission_verify",
    "mission_integrate",
];

/// Tools that MUST run off the UI/event-loop thread because they do blocking I/O.
pub const DEFERRED_TOOLS: &[&str] = &[
    "read",
    "write",
    "edit",
    "delete",
    "bash",
    "grep",
    "glob",
    "remember",
    "forget",
    "recall",
    "message_find",
    "web_fetch",
    "web_search",
    "web_download",
    "git_operator",
    "graph_query",
];

/// Tool names advertised to the MAIN chat model.
pub fn main_tool_names() -> Vec<String> {
    all_tools()
        .iter()
        .map(|t| t.name().to_string())
        .filter(|n| !INTERNAL_ONLY.contains(&n.as_str()))
        .collect()
}

/// Resolve a path and enforce containment.
pub fn resolve(workspaces: &[PathBuf], rel: &str) -> Result<PathBuf> {
    let as_path = Path::new(rel);
    if as_path.is_absolute() {
        let scratch = crate::model::store::scratch_root();
        let candidate = match as_path.canonicalize() {
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
                let mut base = existing
                    .canonicalize()
                    .unwrap_or_else(|_| existing.to_path_buf());
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

    if as_path.is_absolute() {
        let candidate = match as_path.canonicalize() {
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
                let mut base = existing
                    .canonicalize()
                    .unwrap_or_else(|_| existing.to_path_buf());
                for seg in tail.iter().rev() {
                    base.push(seg);
                }
                base
            }
        };
        for ws in workspaces {
            let ws_canon = ws.canonicalize().unwrap_or_else(|_| ws.to_path_buf());
            if candidate.starts_with(&ws_canon) {
                return Ok(candidate);
            }
        }
    }

    let (ws_idx, bare) = parse_ws_prefix(rel);
    let base = workspaces.get(ws_idx).ok_or_else(|| {
        anyhow::anyhow!(
            "workspace index [{ws_idx}] out of range (have {})",
            workspaces.len()
        )
    })?;
    let ws = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    let joined = ws.join(bare);
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
            let mut base = existing
                .canonicalize()
                .unwrap_or_else(|_| existing.to_path_buf());
            for seg in tail.iter().rev() {
                base.push(seg);
            }
            base
        }
    };
    if !candidate.starts_with(&ws) {
        bail!("path '{bare}' is outside workspace [{ws_idx}]");
    }
    Ok(candidate)
}

/// Resolve a path for READ-ONLY tools, forgiving a dropped [N] prefix.
pub fn resolve_read(
    workspaces: &[PathBuf],
    rel: &str,
    session_dir: Option<&Path>,
    active_skill_dirs: &[PathBuf],
) -> Result<PathBuf> {
    let as_path = Path::new(rel);
    if as_path.is_absolute() {
        let scratch = crate::model::store::scratch_root();
        let candidate = as_path
            .canonicalize()
            .unwrap_or_else(|_| as_path.to_path_buf());
        if candidate.starts_with(&scratch) {
            return resolve(workspaces, rel);
        }
        if let Some(session_dir) = session_dir {
            let normalized = partial_canonicalize(as_path);
            let session_canon = canonicalize_or_verbatim(session_dir);
            if normalized.starts_with(&session_canon) {
                return Ok(normalized);
            }
        }
        for skill_dir in active_skill_dirs {
            let normalized = partial_canonicalize(as_path);
            let skill_canon = canonicalize_or_verbatim(skill_dir);
            if normalized.starts_with(&skill_canon) {
                return Ok(normalized);
            }
        }
        let candidate = match as_path.canonicalize() {
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
                let mut base = existing
                    .canonicalize()
                    .unwrap_or_else(|_| existing.to_path_buf());
                for seg in tail.iter().rev() {
                    base.push(seg);
                }
                base
            }
        };
        for ws in workspaces {
            let ws_canon = ws.canonicalize().unwrap_or_else(|_| ws.to_path_buf());
            if candidate.starts_with(&ws_canon) {
                return Ok(candidate);
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

/// Canonicalize a path, falling back to verbatim if the OS can't resolve it.
fn canonicalize_or_verbatim(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// Partial-canonicalize a path.
fn partial_canonicalize(as_path: &Path) -> PathBuf {
    match as_path.canonicalize() {
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
            let mut base = existing
                .canonicalize()
                .unwrap_or_else(|_| existing.to_path_buf());
            for seg in tail.iter().rev() {
                base.push(seg);
            }
            base
        }
    }
}

/// Pure tool dispatcher: given a ready [`ToolCtx`] and a [`ToolCall`].
pub fn execute_tool(ctx: &ToolCtx, call: &crate::dto::chat::ToolCall) -> String {
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
