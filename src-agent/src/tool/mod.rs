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
mod computer;
pub mod dircache;
pub mod fs;
pub mod git_cred;
pub mod git_operator;
pub mod git_worktree;
#[cfg(feature = "linker")]
pub mod graph;
pub mod history;
pub mod internet;
pub mod memory;
pub mod output;
pub mod plan;
mod plan_policy;
pub mod pong;
pub mod recovery;
pub mod scope;
mod screen_browser;
pub mod sdlc;
pub mod search;
pub mod seqthink;
pub mod shell;
pub mod shell_filter;
pub mod skill;
pub mod task;
pub mod todo;
pub mod verdict;

pub use dircache::DirCache;
pub use output::CallTrack;
pub(crate) use plan_policy::{delegated_tool_allowed_in_plan, plan_tool_call_allowed};

/// True for built-in tools that mutate the workspace, run arbitrary shell
/// commands, mutate git state (local or remote, e.g. `git_operator` push /
/// `reset`), or fetch from the network and write the result to disk
/// (`web_download`) — and therefore require approval in Normal mode.
/// Deterministic, name-based — no classifier / network call. Canonical
/// single-source definition used by both the interactive approval gate and the
/// sub-agent engine. NOTE: `git_worktree remove` is gated separately, inside
/// its interception in `process_tools` (it never reaches this generic gate).
/// Native computer tools use explicit GUI enablement as consent instead of TAC
/// or per-action approval. Their controller validates that consent at dispatch.
pub(crate) fn tool_is_risky(name: &str) -> bool {
    matches!(
        name,
        "write"
            | "delete"
            | "edit"
            | "bash"
            | "git_operator"
            | "web_download"
            | "browser_tabs"
            | "browser_interact"
            | "browser_evaluate"
    )
}

/// Tools reachable while [`crate::app::state::AgentMode::Plan`] is active — the
/// read-only / reasoning / delegation surface a planning turn is allowed to use.
/// Explicitly enabled native computer control has its own consent lifecycle,
/// independent of the workspace mode (including Plan and SDLC assess).
pub(crate) fn tool_allowed_in_plan(name: &str) -> bool {
    matches!(
        name,
        "computer_windows"
            | "computer_observe"
            | "computer_select_window"
            | "computer_act"
            | "screen_browser_tabs"
            | "screen_browser_observe"
            | "screen_browser_act"
            | "screen_browser_read"
            | "screen_browser_eval"
            | "screen_browser_wait_human"
            | "read"
            | "grep"
            | "glob"
            | "dir_list"
            | "dir_cache_update"
            | "recall"
            | "message_find"
            | "message_load"
            | "skill"
            | "web_search"
            | "web_fetch"
            | "web_page"
            | "web_search_full"
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
            | "search_screenshots"
            | "load_screenshot"
            | "load_image"
            | "describe_screenshot"
            | "browser_tabs"
            | "browser_inspect"
            | "show_image"
            // Security: Plan may browse the catalog only (no run/fs/runtime mutate).
            | "sec_catalog"
    )
}

/// Tools reachable during SDLC **assess** (pre-approval): same read/search/
/// reasoning/delegation surface as Plan, plus `mission_ready` so the contract
/// can be parked. Filesystem-mutating workspace tools (`write`/`edit`/`delete`/
/// `bash`/…) are denied at the runtime gate — not merely by prompt guidance.
pub(crate) fn tool_allowed_in_sdlc_assess(name: &str) -> bool {
    matches!(name, "mission_ready" | "mission_draft" | "mission_clear")
        || tool_allowed_in_plan(name)
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

/// SDLC assess is stricter than Plan's subcommand allow-list for mutating remote
/// forms, but allows local branch create + checkout/switch **without** force/discard
/// so assess can prep the mission branch. Still rejects commit/merge/rebase/push/…
///
/// `args` is the full `git_operator` argv (subcommand first).
pub(crate) fn sdlc_assess_git_args_allowed(args: &[&str]) -> Result<(), String> {
    let subcmd = args.first().copied().unwrap_or("");

    // checkout / switch: allow without force/discard flags (assess branch prep).
    if matches!(subcmd, "checkout" | "switch") {
        if sdlc_assess_checkout_force_denied(args) {
            return Err(format!(
                "git {subcmd} with force/discard is not allowed in assess"
            ));
        }
        return Ok(());
    }

    if !plan_git_subcommand_allowed(subcmd) {
        return Err(format!(
            "git {subcmd} is not allowed (read-only git only until mission approval)"
        ));
    }
    match subcmd {
        "branch" => {
            if sdlc_assess_branch_is_mutating(args) {
                // Positional create is allowed; only delete/rename/force/upstream denied.
                if sdlc_assess_branch_is_create_only(args) {
                    return Ok(());
                }
                return Err("git branch mutating form is not allowed in assess \
                     (no delete/rename/force/upstream; create/list/show only)"
                    .into());
            }
        }
        "remote" if sdlc_assess_remote_is_mutating(args) => {
            return Err("git remote mutating form is not allowed in assess \
                 (no add/remove/set-url/rename; list/show/get-url only)"
                .into());
        }
        _ => {}
    }
    Ok(())
}

/// True when checkout/switch argv includes force or discard-changes.
fn sdlc_assess_checkout_force_denied(args: &[&str]) -> bool {
    args.iter().skip(1).any(|a| {
        *a == "-f"
            || *a == "--force"
            || *a == "--discard-changes"
            || (a.starts_with('-') && !a.starts_with("--") && a.contains('f'))
    })
}

/// True when `git branch …` is a simple positional create (no delete/rename/force/upstream).
fn sdlc_assess_branch_is_create_only(args: &[&str]) -> bool {
    // Reject if any mutating flag present; allow single positional name.
    let mut i = 1;
    let mut positionals = 0usize;
    while i < args.len() {
        let a = args[i];
        if a == "--" {
            positionals += args.len().saturating_sub(i + 1);
            break;
        }
        if a.starts_with('-') {
            if a.starts_with("--") {
                let base = a.split_once('=').map(|(b, _)| b).unwrap_or(a);
                match base {
                    "--delete" | "--move" | "--copy" | "--force" | "--set-upstream"
                    | "--set-upstream-to" | "--track" | "--unset-upstream"
                    | "--edit-description" => return false,
                    "--list" | "--show-current" | "--all" | "--remotes" => return false,
                    _ => return false,
                }
            } else {
                for ch in a.chars().skip(1) {
                    match ch {
                        'd' | 'D' | 'm' | 'M' | 'c' | 'C' | 'f' | 'u' => return false,
                        'l' | 'a' | 'r' => return false, // list forms
                        'v' | 'q' | 'i' => {}
                        _ => return false,
                    }
                }
            }
        } else {
            positionals += 1;
        }
        i += 1;
    }
    positionals == 1
}

/// Detect force-push / delete-push forms for ANY SDLC phase.
/// Returns a short reason when denied.
pub(crate) fn sdlc_git_force_push_denied(args: &[&str]) -> Option<&'static str> {
    let subcmd = args.first().copied().unwrap_or("");
    if subcmd != "push" {
        return None;
    }
    let rest = &args[1..];
    let has_flag = |needle: &str| rest.contains(&needle);
    let has_prefix = |prefix: &str| rest.iter().any(|a| a.starts_with(prefix));
    let has_bundle_char = |ch: char| {
        rest.iter()
            .any(|a| a.starts_with('-') && !a.starts_with("--") && a.contains(ch))
    };
    if has_flag("--force")
        || has_flag("-f")
        || has_bundle_char('f')
        || has_prefix("--force-with-lease")
        || has_flag("--delete")
        || has_flag("-d")
    {
        return Some("push --force / --force-with-lease / --delete");
    }
    if rest.iter().any(|a| a.starts_with(':')) {
        return Some("push with colon-prefixed refspec deletion");
    }
    None
}

/// True when a `git branch …` argv would create/delete/rename/force/set upstream
/// rather than merely list or show branches.
fn sdlc_assess_branch_is_mutating(args: &[&str]) -> bool {
    // args[0] == "branch"
    let mut i = 1;
    // Explicit list/show intent absorbs subsequent pattern operands.
    let mut list_mode = false;
    let mut saw_positional = false;
    while i < args.len() {
        let a = args[i];
        if a == "--" {
            // Operands after `--`: create unless we are listing.
            return if list_mode {
                false
            } else {
                args.get(i + 1).is_some()
            };
        }
        if a.starts_with('-') {
            if a.starts_with("--") {
                let (base, has_eq) = match a.split_once('=') {
                    Some((b, _)) => (b, true),
                    None => (a, false),
                };
                match base {
                    "--delete" | "--move" | "--copy" | "--force" | "--set-upstream"
                    | "--set-upstream-to" | "--track" | "--unset-upstream"
                    | "--edit-description" => return true,
                    "--list" => list_mode = true,
                    "--show-current" | "--verbose" | "--no-verbose" | "--quiet"
                    | "--ignore-case" | "--color" | "--no-color" | "--no-column" => {}
                    "--all" | "--remotes" => list_mode = true,
                    "--contains" | "--no-contains" | "--merged" | "--no-merged" | "--points-at"
                    | "--format" | "--sort" | "--column" => {
                        list_mode = true;
                        if !has_eq {
                            if let Some(n) = args.get(i + 1) {
                                if !n.starts_with('-') {
                                    i += 1;
                                }
                            }
                        }
                    }
                    _ => return true, // unknown long option → fail closed
                }
            } else {
                // Short cluster, e.g. -vv, -av, -d, -D, -m, -M, -c, -C, -f, -u, -l.
                let chars: Vec<char> = a.chars().skip(1).collect();
                for ch in &chars {
                    match ch {
                        'd' | 'D' | 'm' | 'M' | 'c' | 'C' | 'f' | 'u' => return true,
                        'l' => list_mode = true, // --list
                        // -a/--all and -r/--remotes imply list form when a
                        // pattern follows (`git branch -a feat/*`).
                        'a' | 'r' => list_mode = true,
                        'v' | 'q' | 'i' => {}
                        _ => return true, // unknown short → fail closed
                    }
                }
            }
            i += 1;
            continue;
        }
        // Positional branch name / pattern.
        saw_positional = true;
        i += 1;
    }
    // Positional without list-mode is branch creation: `git branch foo`.
    saw_positional && !list_mode
}

/// True when a `git remote …` argv would add/remove/set-url/rename rather than
/// list/show/get-url.
fn sdlc_assess_remote_is_mutating(args: &[&str]) -> bool {
    // args[0] == "remote"
    let mut i = 1;
    // Skip global-ish flags before the remote subcommand (-v/--verbose).
    while i < args.len() {
        let a = args[i];
        if a == "-v" || a == "--verbose" {
            i += 1;
            continue;
        }
        break;
    }
    let Some(action) = args.get(i).copied() else {
        // bare `git remote` / `git remote -v` → list (safe)
        return false;
    };
    if action.starts_with('-') {
        // Unknown flag form — fail closed.
        return true;
    }
    match action {
        "show" | "get-url" => false,
        "add" | "remove" | "rm" | "set-url" | "rename" | "set-head" | "set-branches" | "prune"
        | "update" => true,
        // A bare remote name is not a standard read form we allow.
        _ => true,
    }
}

/// Git subcommands that change the checked-out branch/HEAD — blocked during
/// SDLC execute/integrate so the frozen mission binding cannot be escaped.
pub(crate) fn sdlc_execute_git_branch_changing(subcmd: &str) -> bool {
    matches!(subcmd, "checkout" | "switch" | "worktree")
}

/// Validate a `git_operator` invocation for SDLC execute/integrate:
/// - no arbitrary `cwd` override (must run in the bound worktree via session cwd)
/// - no branch-changing subcommands
/// - binding must be live/valid (caller supplies the already-checked flag + detail)
/// - plain `push` only the mission branch (never bare push / wrong refspec)
pub(crate) fn sdlc_execute_git_args_allowed(
    args: &[&str],
    cwd_override: Option<&str>,
    binding_live: bool,
    binding_detail: &str,
    mission_branch: Option<&str>,
) -> Result<(), String> {
    if !binding_live {
        return Err(format!(
            "git_operator blocked: mission binding is not live/valid ({binding_detail})"
        ));
    }
    if let Some(cwd) = cwd_override {
        if !cwd.trim().is_empty() {
            return Err(
                "git_operator cwd override is not allowed during SDLC execute/integrate — \
                 git runs only inside the frozen bound mission worktree"
                    .into(),
            );
        }
    }
    let subcmd = args.first().copied().unwrap_or("");
    if subcmd.is_empty() {
        return Err("git_operator requires a git subcommand".into());
    }
    if sdlc_execute_git_branch_changing(subcmd) {
        return Err(format!(
            "git {subcmd} is not allowed during SDLC execute/integrate — \
             branch/HEAD is frozen to the mission binding (no checkout/switch/…)"
        ));
    }
    if let Some(reason) = sdlc_git_force_push_denied(args) {
        return Err(format!("Never force-push in SDLC ({reason})"));
    }
    if subcmd == "push" {
        let Some(mb) = mission_branch.map(str::trim).filter(|s| !s.is_empty()) else {
            return Err(
                "git push denied: mission has no bound branch — cannot push during SDLC".into(),
            );
        };
        // Collect positionals after "push", skipping common flags.
        let mut positionals: Vec<&str> = Vec::new();
        let mut i = 1usize;
        while i < args.len() {
            let a = args[i];
            if a == "--" {
                i += 1;
                while i < args.len() {
                    positionals.push(args[i]);
                    i += 1;
                }
                break;
            }
            if a.starts_with('-') {
                // Flags that take a value (not covered by force-push deny).
                if matches!(
                    a,
                    "--repo" | "--receive-pack" | "--exec" | "--push-option" | "-o" | "--signed"
                ) {
                    i += 2;
                    continue;
                }
                i += 1;
                continue;
            }
            positionals.push(a);
            i += 1;
        }
        // Need at least remote + one refspec.
        if positionals.len() < 2 {
            return Err(format!(
                "git push denied: bare push is not allowed — use `push <remote> {mb}`"
            ));
        }
        // positionals[0] is remote; remaining are refspecs.
        for spec in positionals.iter().skip(1) {
            if !sdlc_push_refspec_is_mission_branch(spec, mb) {
                return Err(format!(
                    "git push denied: refspec '{spec}' is not the mission branch '{mb}' — \
                     push only `{mb}` (or refs/heads/{mb})"
                ));
            }
        }
    }
    Ok(())
}

/// Destination side of a push refspec must equal the mission branch
/// (strip `refs/heads/`; for `src:dst` use dst, else whole token).
fn sdlc_push_refspec_is_mission_branch(spec: &str, mission_branch: &str) -> bool {
    let dest = match spec.rsplit_once(':') {
        Some((_src, dst)) if !dst.is_empty() => dst,
        _ => spec,
    };
    let dest = dest.strip_prefix('+').unwrap_or(dest);
    let dest = dest.strip_prefix("refs/heads/").unwrap_or(dest);
    dest == mission_branch
}

/// Shared context handed to every tool invocation.
pub struct ToolCtx {
    /// Live parent-session Plan restriction, shared with deferred calls and
    /// delegates. Checked immediately before dispatch; it cannot undo I/O that
    /// has already started.
    pub plan_read_only: Arc<std::sync::atomic::AtomicBool>,
    /// Absolute workspace root (the session's primary workdir). Follows a live
    /// `cd`, so this is the run's cwd, not an allow-list.
    pub workspace: PathBuf,
    /// The session's cyber lane, when security mode has one armed. `None` in a
    /// plain coding session. A `sec_` call with no lane is refused.
    pub cyber_lane: Option<crate::app::sec::lane::Lane>,
    /// Why `sec_` calls are refused. Set when [`crate::app::sec::active::effective`]
    /// fails. `None` when a lane is in force.
    pub sec_block: Option<String>,
    /// Tool names the user turned off in the security panel.
    pub sec_inactive: Vec<String>,
    /// Stable session identity. Every dynamic target traces back to this, so two
    /// sessions on one checkout cannot collide on a board, a scope record or a
    /// spilled artifact. Mirrors `SessionRuntime::id`.
    ///
    /// Read by [`scope::ScopeBase::of`].
    pub session_id: String,
    /// All configured workspace roots (may be >1).
    pub workspaces: Vec<PathBuf>,
    pub dir_cache: Arc<RwLock<DirCache>>,
    /// The per-PROJECT memory directory.
    pub memory_dir: Option<PathBuf>,
    /// The shadow worktree dir for this session's pwd bucket.
    pub worktrees_dir: Option<std::path::PathBuf>,
    /// The per-session media download directory.
    pub download_dir: Option<PathBuf>,
    /// Exact per-session scratch directory (`<tmp>/koma/<session-id>`).
    pub scratch_dir: Option<PathBuf>,
    /// The session's active internet tier.
    pub internet_mode: crate::model::settings::InternetMode,
    /// The bare filename of the SSH identity key selected for this session.
    pub ssh_key: Option<String>,
    /// Skill catalogue snapshot from the session.
    pub skill_registry: Option<crate::model::skill::SkillRegistry>,
    /// Names of currently active (loaded) skills. `list` marks them, and
    /// `load` refuses to replace a body that is already in context.
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
    /// When `false`, mutating path resolution (`resolve` / write/edit/delete)
    /// must NOT accept absolute paths merely because they lie under the
    /// process-global scratch root. SDLC execute/integrate sets this false so
    /// writes stay inside the validated bound mission worktree. Non-SDLC keeps
    /// the historical scratch exemption (`true`).
    pub allow_scratch: bool,
    /// When `true`, the session is in SDLC assess — sub-agent allow-lists fold
    /// through the same assess surface as the main agent (no workspace writes).
    pub sdlc_assess: bool,
    /// SDLC path ownership: the graph node id this context is bound to during
    /// execute/integrate. `None` for the main session (which doesn't own nodes);
    /// set on sub-agent ToolCtxs when spawned for a claimed leaf.
    pub sdlc_active_node_id: Option<String>,
    /// The session's preferred search engine URL template (e.g.
    /// `https://html.duckduckgo.com/html/?q={query}`).
    pub search_engine: Option<String>,
    /// Exact tool-call fingerprints seen this session (name + canonical args +
    /// unclipped-body digest). Shared with deferred workers and sub-agents so
    /// a repeated inspection dump can be stubbed.
    pub call_track: Arc<CallTrack>,
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
        Box::new(computer::Computer("computer_windows")),
        Box::new(computer::Computer("computer_select_window")),
        Box::new(computer::Computer("computer_observe")),
        Box::new(computer::Computer("computer_act")),
        Box::new(screen_browser::ScreenBrowser("screen_browser_tabs")),
        Box::new(screen_browser::ScreenBrowser("screen_browser_observe")),
        Box::new(screen_browser::ScreenBrowser("screen_browser_act")),
        Box::new(screen_browser::ScreenBrowser("screen_browser_read")),
        Box::new(screen_browser::ScreenBrowser("screen_browser_eval")),
        Box::new(screen_browser::ScreenBrowser("screen_browser_wait_human")),
        Box::new(fs::Read),
        Box::new(search::Grep),
        Box::new(search::Glob),
        #[cfg(feature = "linker")]
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
        Box::new(history::MessageLoad),
        Box::new(task::Task),
        Box::new(task::TaskOutput),
        Box::new(task::TaskKill),
        Box::new(task::TaskSend),
        Box::new(todo::Checklist),
        Box::new(internet::WebFetch),
        Box::new(internet::WebSearch),
        Box::new(internet::WebDownload),
        Box::new(internet::WebPage),
        Box::new(internet::WebSearchFull),
        Box::new(internet::SearchScreenshots),
        Box::new(internet::DescribeScreenshot),
        Box::new(internet::LoadScreenshot),
        Box::new(internet::LoadImage),
        Box::new(internet::BrowserTabs),
        Box::new(internet::BrowserInspect),
        Box::new(internet::BrowserInteract),
        Box::new(internet::BrowserEvaluate),
        Box::new(internet::ShowImage),
        Box::new(git_cred::GitCred),
        Box::new(git_operator::GitOperator),
        Box::new(git_worktree::GitWorktree),
        Box::new(plan::PlanEnter),
        Box::new(plan::PlanReady),
        Box::new(sdlc::MissionReady),
        Box::new(sdlc::MissionDraft),
        Box::new(sdlc::MissionClear),
        Box::new(sdlc::MissionVerify),
        Box::new(sdlc::MissionPrepare),
        Box::new(sdlc::MissionIntegrate),
        Box::new(seqthink::SeqThink),
    ]
}

/// Tool names the /agents editor's tool picker EXCLUDES from the selectable list.
const AGENT_PICKER_EXCLUDED: &[&str] = &[
    "computer_windows",
    "computer_select_window",
    "computer_observe",
    "computer_act",
    "screen_browser_tabs",
    "screen_browser_observe",
    "screen_browser_act",
    "screen_browser_read",
    "screen_browser_eval",
    "screen_browser_wait_human",
    "task",
    "task_send",
    "pong",
    "dir_cache_update",
    "skill",
    // Sub-agent continuation currently cannot inject synthetic image messages.
    "load_image",
];

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
    "mission_draft",
    "mission_clear",
    "mission_verify",
    "mission_prepare",
    "mission_integrate",
];

/// Tools that MUST run off the UI/event-loop thread because they do blocking I/O.
pub const DEFERRED_TOOLS: &[&str] = &[
    "computer_windows",
    "computer_select_window",
    "computer_observe",
    "computer_act",
    "screen_browser_tabs",
    "screen_browser_observe",
    "screen_browser_act",
    "screen_browser_read",
    "screen_browser_eval",
    "screen_browser_wait_human",
    "read",
    "write",
    "edit",
    "delete",
    // bash is always intercepted as a BashJob (FG or BG) — never deferred.
    "grep",
    "glob",
    "remember",
    "forget",
    "recall",
    "message_find",
    "message_load",
    "web_fetch",
    "web_search",
    "web_download",
    "web_page",
    "web_search_full",
    "search_screenshots",
    "describe_screenshot",
    "git_operator",
    "graph_query",
    "browser_tabs",
    "browser_inspect",
    "browser_interact",
    "browser_evaluate",
];

/// Tool names advertised to the MAIN chat model. Explicitly deduped by name
/// (last-wins if duplicates exist — a future guard) so the schema generation
/// never sees duplicate tool definitions.
pub fn main_tool_names() -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for t in all_tools() {
        let n = t.name().to_string();
        if !INTERNAL_ONLY.contains(&n.as_str()) && seen.insert(n.clone()) {
            out.push(n);
        }
    }
    out
}

// Native Windows separators preserve verbatim and UNC path prefixes.
fn normalize_tool_path(path: &str) -> String {
    #[cfg(windows)]
    {
        path.replace('/', "\\")
    }
    #[cfg(not(windows))]
    {
        path.replace('\\', "/")
    }
}

/// Resolve a path and enforce containment.
///
/// Absolute paths under the process-global scratch root are accepted when
/// `allow_scratch` is true (historical non-SDLC behaviour). Callers that must
/// confine writes to validated workspace roots (SDLC execute/integrate) pass
/// `false` so scratch is not a bypass.
pub fn resolve(workspaces: &[PathBuf], rel: &str) -> Result<PathBuf> {
    resolve_in(workspaces, rel, true)
}

/// Like [`resolve`], with an explicit scratch-root policy.
pub fn resolve_in(workspaces: &[PathBuf], rel: &str, allow_scratch: bool) -> Result<PathBuf> {
    let normalized = normalize_tool_path(rel);
    let rel = normalized.as_str();
    let as_path = Path::new(rel);
    if as_path.is_absolute() {
        let candidate = partial_canonicalize(as_path);
        // Historical exemption: absolute paths under process-global scratch.
        // SDLC execute/integrate disables this so mission binding cannot be bypassed.
        if allow_scratch {
            let scratch = partial_canonicalize(&crate::model::store::scratch_root());
            if candidate.starts_with(&scratch) {
                return Ok(candidate);
            }
        }
        for ws in workspaces {
            let ws_canon = ws.canonicalize().unwrap_or_else(|_| ws.to_path_buf());
            if candidate.starts_with(&ws_canon) {
                return Ok(candidate);
            }
        }
        if !allow_scratch {
            bail!("path is outside the bound workspace (scratch bypass disabled)");
        }
        // allow_scratch path outside workspaces falls through so the failure
        // shape matches historical resolve() (workspace-join containment error).
    }

    let (ws_idx, bare) = parse_ws_prefix(rel);
    let base = workspaces.get(ws_idx).ok_or_else(|| {
        anyhow::anyhow!(
            "workspace index [{ws_idx}] out of range (have {})",
            workspaces.len()
        )
    })?;
    let ws = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    let bare = if rel.starts_with('[') {
        bare.trim_start_matches(['/', '\\'])
    } else {
        bare
    };
    let joined = ws.join(bare);
    let candidate = partial_canonicalize(&joined);
    if !candidate.starts_with(&ws) {
        bail!("{}", outside_workspace_message(bare, ws_idx));
    }
    Ok(candidate)
}

fn outside_workspace_message(path: &str, ws_idx: usize) -> String {
    let shown = path.replace('\\', "/");
    let lower = shown.to_ascii_lowercase();
    let skill_file = lower.ends_with("/skill.md")
        || lower.contains("/.koma/skills/")
        || lower.contains("/.agents/skills/")
        || lower.contains("/.agent/skills/")
        || lower.contains("/.claude/skills/");
    if skill_file {
        format!(
            "path '{shown}' is outside workspace [{ws_idx}]. Skill files are not written with write. Use skill with action create or modify."
        )
    } else {
        format!("path '{shown}' is outside workspace [{ws_idx}]")
    }
}

/// Resolve a path for read-only tools without guessing a different workspace.
pub fn resolve_read(
    workspaces: &[PathBuf],
    rel: &str,
    session_dir: Option<&Path>,
    active_skill_dirs: &[PathBuf],
) -> Result<PathBuf> {
    let normalized = normalize_tool_path(rel);
    let rel = normalized.as_str();
    let as_path = Path::new(rel);
    if as_path.is_absolute() {
        let scratch = partial_canonicalize(&crate::model::store::scratch_root());
        let candidate = partial_canonicalize(as_path);
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

    resolve(workspaces, rel)
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
    let mut args: serde_json::Value =
        serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));
    if ctx
        .plan_read_only
        .load(std::sync::atomic::Ordering::Acquire)
    {
        if let Err(reason) = plan_tool_call_allowed(&call.function.name, &args) {
            return output::finish_tool_output(
                ctx,
                &call.function.name,
                &args,
                format!("blocked: {reason}"),
            );
        }
    }
    for tool in all_tools() {
        if tool.name() == call.function.name {
            let raw = match tool.run(ctx, &args) {
                Ok(s) => s,
                Err(e) => format!("error: {e}"),
            };
            return output::finish_tool_output(ctx, &call.function.name, &args, raw);
        }
    }
    if call.function.name.starts_with("mcp__") {
        let config = crate::model::app_config::AppConfig::load();
        let selected = ctx
            .session_dir
            .as_ref()
            .and_then(|p| crate::model::settings::Settings::load(&p.join("settings.json")).ok())
            .map(|s| s.active_extensions)
            .unwrap_or_default();
        if !crate::app::mcp::tool_active_in_session(&call.function.name, &config, &selected) {
            return output::finish_tool_output(
                ctx,
                &call.function.name,
                &args,
                "error: extension is inactive in this session; the user can select it with /extension use".into(),
            );
        }
        if let Some(mgr) = ctx.mcp_manager.as_ref() {
            let raw = mgr
                .execute_blocking(&call.function.name, &args)
                .unwrap_or_else(|e| format!("error: {e}"));
            return output::finish_tool_output(ctx, &call.function.name, &args, raw);
        }
    }
    if crate::security::is_sec_tool(&call.function.name) {
        // Fail-closed boundary. `ctx` carries the manager but not the lane flag
        // or the advertised set, so ask the manager for its own view: an
        // unadvertised/forged `sec_` name must not run even when the daemon is
        // live and armed.
        if let Some(mgr) = ctx.sec_manager.as_ref() {
            // Meta control plane: catalog / runtime / fs / run.
            if crate::app::sec::meta::is_meta_tool(&call.function.name) {
                return dispatch_sec_meta(ctx, mgr, &call.function.name, &mut args);
            }
            let lane_refusal = scope::lane_refusal(ctx, &call.function.name, &mut args);
            if let Some(reason) = lane_refusal {
                let denied = verdict::TypedResult {
                    verdict: verdict::Verdict::Blocked,
                    excerpt: reason,
                    handle: None,
                };
                return output::finish_tool_output(
                    ctx,
                    &call.function.name,
                    &args,
                    denied.to_json(),
                );
            }
            if let Some(reason) = mgr.refusal(&call.function.name, ctx.cyber_lane.is_some()) {
                // A refusal is the tool's own `error` verdict, not a transport
                // failure: it becomes typed so the model sees the same shape it
                // sees from every other outcome.
                let refusal = verdict::TypedResult {
                    verdict: verdict::Verdict::Error,
                    excerpt: reason,
                    handle: None,
                };
                return output::finish_tool_output(
                    ctx,
                    &call.function.name,
                    &args,
                    refusal.to_json(),
                );
            }
            // The lane gate already ran. Scope admission is the remaining check.
            let lane_refusal = scope::admit_from_call(ctx, &call.function.name, &args);
            // Admit before anything else: a call whose target the scope does
            // not cover must not reach the container at all.
            if let Some(reason) = lane_refusal {
                let denied = verdict::TypedResult {
                    verdict: verdict::Verdict::Blocked,
                    excerpt: reason,
                    handle: None,
                };
                return output::finish_tool_output(
                    ctx,
                    &call.function.name,
                    &args,
                    denied.to_json(),
                );
            }

            // Parse before the output caps run. The verdict is the result; the
            // excerpt is only its body, and `finish_tool_output` trims bodies.
            // Untyped results (no verdict) go through the unchanged path below.
            let typed = match mgr.execute_blocking(&call.function.name, &args) {
                // Hold an oversized payload aside BEFORE it reaches the model: a
                // 200-row scan is a turn's context otherwise. Reuses the same tmp
                // spill the other tools use, so cleanup is already handled.
                Ok(raw) => verdict::enforce_shape(verdict::parse(&raw), |body| {
                    output::spill_foreign(ctx.session_dir.as_deref(), &call.function.name, body)
                }),
                Err(e) => verdict::TypedResult {
                    verdict: verdict::Verdict::Error,
                    excerpt: e,
                    handle: None,
                },
            };
            // A verdict clears or fails the binding: only a confirmation keeps
            // an addressable target. A `not_found` or a crash leaves nothing to
            // revisit, so the record goes rather than lingering as a dead link
            // that a later call would happily re-admit.
            scope::settle(ctx, &call.function.name, &args, typed.verdict);
            // Frame the body at the ONE place every sec_ result reaches the model:
            // a target controls this text, so the model is told where the trusted
            // metadata ends. The verdict and handle ride above the fence, so a body
            // forging either cannot win.
            let framed = crate::app::sec::untrusted::frame_result(
                &typed.excerpt,
                typed.verdict.as_str(),
                typed.handle.as_deref(),
                crate::app::sec::untrusted::Origin::ToolOutput,
            );
            let framed = verdict::TypedResult {
                verdict: typed.verdict,
                excerpt: framed,
                handle: typed.handle,
            };
            return output::finish_tool_output(ctx, &call.function.name, &args, framed.to_json());
        }
    }
    output::finish_tool_output(
        ctx,
        &call.function.name,
        &args,
        format!("error: unknown tool '{}'", call.function.name),
    )
}

/// Update the session security working set after a successful catalog/run touch.
fn touch_sec_working_set(ctx: &ToolCtx, op: &str, raw_or_name: &str, extra: &str) {
    let Some(dir) = ctx.session_dir.as_ref() else {
        return;
    };
    let mut ws = crate::app::sec::working_set::load(dir);
    if let Some(lane) = ctx.cyber_lane {
        ws.set_lane(lane.label());
    }
    match op {
        "pin" => {
            ws.pin(raw_or_name, extra, "");
        }
        "run" => {
            ws.pin(raw_or_name, "", "");
            ws.set_last_run(raw_or_name, extra);
        }
        "categories" => {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw_or_name) {
                if let Some(arr) = v.get("categories").and_then(|c| c.as_array()) {
                    let cats: Vec<String> = arr
                        .iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect();
                    ws.remember_categories(&cats);
                }
            }
        }
        "get" => {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw_or_name) {
                if let Some(tool) = v.get("tool") {
                    let name = tool.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let blurb = tool
                        .get("description")
                        .or_else(|| tool.get("blurb"))
                        .and_then(|n| n.as_str())
                        .unwrap_or("");
                    let effect = tool.get("effect").and_then(|n| n.as_str()).unwrap_or("");
                    if !name.is_empty() {
                        ws.pin(name, blurb, effect);
                    }
                }
            }
        }
        "list" => {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw_or_name) {
                if let Some(arr) = v.get("tools").and_then(|c| c.as_array()) {
                    // Pin first few list hits lightly (name only) so a page is not lost entirely.
                    for tool in arr.iter().take(5) {
                        let name = tool.get("name").and_then(|n| n.as_str()).unwrap_or("");
                        let blurb = tool.get("blurb").and_then(|n| n.as_str()).unwrap_or("");
                        let effect = tool.get("effect").and_then(|n| n.as_str()).unwrap_or("");
                        if !name.is_empty() {
                            ws.pin(name, blurb, effect);
                        }
                    }
                }
            }
        }
        _ => {}
    }
    let _ = crate::app::sec::working_set::store(dir, &ws);
}

fn dispatch_sec_meta(
    ctx: &ToolCtx,
    mgr: &std::sync::Arc<crate::app::sec::SecDaemonManager>,
    name: &str,
    args: &mut serde_json::Value,
) -> String {
    // Meta tools still need a cyber lane selected (security mode on).
    // Plan mode is gated earlier via tool_allowed_in_plan (sec_catalog only).
    if let Some(reason) = scope::lane_refusal(ctx, name, args) {
        let denied = verdict::TypedResult {
            verdict: verdict::Verdict::Blocked,
            excerpt: reason,
            handle: None,
        };
        return output::finish_tool_output(ctx, name, args, denied.to_json());
    }
    if let Some(reason) = mgr.refusal(name, ctx.cyber_lane.is_some()) {
        let refusal = verdict::TypedResult {
            verdict: verdict::Verdict::Error,
            excerpt: reason,
            handle: None,
        };
        return output::finish_tool_output(ctx, name, args, refusal.to_json());
    }

    let typed = match name {
        "sec_catalog" => {
            let op = args.get("op").and_then(|v| v.as_str()).unwrap_or("");
            let mut payload = args.clone();
            if let Some(obj) = payload.as_object_mut() {
                obj.remove("op");
                if !obj.contains_key("lane") {
                    if let Some(lane) = ctx.cyber_lane {
                        obj.insert(
                            "lane".into(),
                            serde_json::Value::String(lane.label().to_string()),
                        );
                    }
                }
                // Default list filter=ready so the agent does not drown in not-ready rows.
                if op == "list" && !obj.contains_key("filter") {
                    obj.insert("filter".into(), serde_json::Value::String("ready".into()));
                }
            }
            match mgr.catalog_op(op, &payload) {
                Ok(raw) => {
                    match op {
                        "get" => touch_sec_working_set(ctx, "get", &raw, ""),
                        "list" => touch_sec_working_set(ctx, "list", &raw, ""),
                        "categories" => touch_sec_working_set(ctx, "categories", &raw, ""),
                        _ => {}
                    }
                    verdict::TypedResult {
                        verdict: verdict::Verdict::Untyped,
                        excerpt: raw,
                        handle: None,
                    }
                }
                Err(e) => verdict::TypedResult {
                    verdict: verdict::Verdict::Error,
                    excerpt: e,
                    handle: None,
                },
            }
        }
        "sec_runtime" => {
            let op = args.get("op").and_then(|v| v.as_str()).unwrap_or("");
            let mut payload = serde_json::json!({});
            if let Some(obj) = args.as_object() {
                let mut map = serde_json::Map::new();
                for (k, v) in obj {
                    if k != "op" {
                        map.insert(k.clone(), v.clone());
                    }
                }
                payload = serde_json::Value::Object(map);
            }
            // Lane net policy: Research never carries egress allowlist; Red may use
            // scope targets only when provided (empty up = deny-all offline guest).
            if let Some(obj) = payload.as_object_mut() {
                use crate::app::sec::lane::Lane;
                match ctx.cyber_lane {
                    Some(Lane::Research) => {
                        obj.insert("targets".into(), serde_json::json!([]));
                        obj.insert("exclusions".into(), serde_json::json!([]));
                        if op == "net" {
                            obj.insert("mode".into(), serde_json::Value::String("off".into()));
                        }
                    }
                    Some(Lane::Red) if op == "net" => {
                        let mode = obj.get("mode").and_then(|v| v.as_str()).unwrap_or("off");
                        let targets_empty = obj
                            .get("targets")
                            .and_then(|v| v.as_array())
                            .map(|a| a.is_empty())
                            .unwrap_or(true);
                        if mode == "scope" && targets_empty {
                            return output::finish_tool_output(
                                ctx,
                                name,
                                args,
                                verdict::TypedResult {
                                    verdict: verdict::Verdict::Blocked,
                                    excerpt: "red lane: sec_runtime net mode=scope requires targets (host/IP allowlist from confirmed scope)"
                                        .into(),
                                    handle: None,
                                }
                                .to_json(),
                            );
                        }
                    }
                    _ => {}
                }
            }
            match mgr.runtime_op(op, &payload) {
                Ok(raw) => verdict::TypedResult {
                    verdict: verdict::Verdict::Untyped,
                    excerpt: raw,
                    handle: None,
                },
                Err(e) => verdict::TypedResult {
                    verdict: verdict::Verdict::Error,
                    excerpt: e,
                    handle: None,
                },
            }
        }
        "sec_fs" => {
            let op = args.get("op").and_then(|v| v.as_str()).unwrap_or("");
            let mut payload = args.clone();
            if let Some(obj) = payload.as_object_mut() {
                obj.remove("op");
                // Workspace roots become allow_roots for copy_in.
                if op == "copy_in" && !obj.contains_key("allow_roots") {
                    let roots: Vec<String> = ctx
                        .workspaces
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect();
                    obj.insert("allow_roots".into(), serde_json::json!(roots));
                }
            }
            match mgr.fs_op(op, &payload) {
                Ok(raw) => verdict::TypedResult {
                    verdict: verdict::Verdict::Untyped,
                    excerpt: raw,
                    handle: None,
                },
                Err(e) => verdict::TypedResult {
                    verdict: verdict::Verdict::Error,
                    excerpt: e,
                    handle: None,
                },
            }
        }
        "sec_run" => {
            let inner = args
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if inner.is_empty() || crate::app::sec::meta::is_meta_tool(&inner) {
                verdict::TypedResult {
                    verdict: verdict::Verdict::Error,
                    excerpt: "sec_run requires a concrete tool name (see sec_catalog)".into(),
                    handle: None,
                }
            } else {
                let mut inner_args = args
                    .get("args")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                if !inner_args.is_object() {
                    inner_args = serde_json::json!({});
                }
                // Admit path on the INNER tool name — meta must not bypass lane/scope.
                if let Some(reason) = scope::lane_refusal(ctx, &inner, &mut inner_args) {
                    verdict::TypedResult {
                        verdict: verdict::Verdict::Blocked,
                        excerpt: reason,
                        handle: None,
                    }
                } else if let Some(reason) = mgr.refusal(&inner, ctx.cyber_lane.is_some()) {
                    verdict::TypedResult {
                        verdict: verdict::Verdict::Error,
                        excerpt: reason,
                        handle: None,
                    }
                } else if let Some(reason) = scope::admit_from_call(ctx, &inner, &inner_args) {
                    verdict::TypedResult {
                        verdict: verdict::Verdict::Blocked,
                        excerpt: reason,
                        handle: None,
                    }
                } else {
                    let lane_s = ctx.cyber_lane.map(|l| l.label().to_string());
                    let typed = match mgr.execute_blocking_in_lane(
                        &inner,
                        &inner_args,
                        lane_s.as_deref(),
                    ) {
                        Ok(raw) => verdict::enforce_shape(verdict::parse(&raw), |body| {
                            output::spill_foreign(ctx.session_dir.as_deref(), &inner, body)
                        }),
                        Err(e) => verdict::TypedResult {
                            verdict: verdict::Verdict::Error,
                            excerpt: e,
                            handle: None,
                        },
                    };
                    scope::settle(ctx, &inner, &inner_args, typed.verdict);
                    touch_sec_working_set(ctx, "run", &inner, typed.verdict.as_str());
                    let framed = crate::app::sec::untrusted::frame_result(
                        &typed.excerpt,
                        typed.verdict.as_str(),
                        typed.handle.as_deref(),
                        crate::app::sec::untrusted::Origin::ToolOutput,
                    );
                    return output::finish_tool_output(
                        ctx,
                        name,
                        args,
                        verdict::TypedResult {
                            verdict: typed.verdict,
                            excerpt: framed,
                            handle: typed.handle,
                        }
                        .to_json(),
                    );
                }
            }
        }
        other => verdict::TypedResult {
            verdict: verdict::Verdict::Error,
            excerpt: format!("unknown sec meta tool: {other}"),
            handle: None,
        },
    };

    // Catalog/runtime/fs results are control-plane JSON (not target-controlled
    // bodies); still frame lightly for consistency when Ok from a run path.
    let excerpt = typed.excerpt;
    let framed = if name == "sec_run" {
        crate::app::sec::untrusted::frame_result(
            &excerpt,
            typed.verdict.as_str(),
            typed.handle.as_deref(),
            crate::app::sec::untrusted::Origin::ToolOutput,
        )
    } else {
        excerpt
    };
    output::finish_tool_output(
        ctx,
        name,
        args,
        verdict::TypedResult {
            verdict: typed.verdict,
            excerpt: framed,
            handle: typed.handle,
        }
        .to_json(),
    )
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
// Shared minimal `ToolCtx` for tests. Re-exported rather than declared a second
// time: the file is already a `#[path]` module under `seqthink`, and a second
// `mod` declaration makes clippy warn about a doubly-loaded file.
#[cfg(test)]
pub(crate) use seqthink::test_ctx;
