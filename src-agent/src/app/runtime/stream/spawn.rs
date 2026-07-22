//! Sub-agent spawning and tool context construction.

use std::sync::Arc;

use crate::app::state::AppState;
use crate::service::openrouter::OpenRouterClient;

/// Build a [`crate::tool::ToolCtx`] from session `sess_idx` + its dir cache.
///
/// Centralises the workspace/workspaces/memory_dir construction so that both
/// `run_tool` (inline tool calls) and the `/task` spawner (sub-agent launch)
/// use the EXACT same paths and dir-cache reference. Reads the session at
/// `sess_idx` (NOT the foreground) so a tool dispatched on a background session
/// runs against that session's own workspace + dir cache.
pub(crate) fn build_tool_ctx(state: &AppState, sess_idx: usize) -> crate::tool::ToolCtx {
    let rt = &state.rest.sessions[sess_idx];
    let session_ref = rt.session.as_ref();
    // The session's EFFECTIVE cwd: the live `cd` override when set, else the
    // configured workdir. This drives `bash` (its `current_dir`) and the dir
    // cache root, so both follow `cd`. The configured `workdirs()` below stay the
    // allow-list / `[N]` multi-root set — cd repoints only the cwd, never the
    // allow-list (use `/adddir` to widen that).
    let workspace = rt.effective_cwd();
    let workspaces = session_ref
        .as_ref()
        .map(|s| s.workdirs())
        .unwrap_or_else(|| vec![workspace.clone()]);
    // Per-PROJECT memory dir (shared by every session in this working dir), not
    // the old per-session `<session_dir>/memory`. Falls back to the per-session
    // path if the bucket dir can't be resolved (it always should).
    let memory_dir = session_ref
        .as_ref()
        .map(|s| {
            crate::model::store::memory_dir(&s.pwd_hash)
                .unwrap_or_else(|_| s.path.join("memory"))
        });
    // The shadow worktree dir (`<pwd_bucket_dir>/worktrees/`) for this session's
    // pwd bucket, mirrored from `memory_dir`. `git_worktree` create/remove resolve
    // `<worktrees_dir>/<name>` so worktrees live OUTSIDE the repo. `None` when the
    // bucket dir can't be resolved (no session).
    let worktrees_dir = session_ref
        .as_ref()
        .and_then(|s| crate::model::store::worktrees_dir(&s.pwd_hash).ok());
    // The media download directory for web_download files: `<media>/downloads/`.
    // Created on access so the tool can write into it without a separate
    // create_dir_all. The MEDIA_WORKDIR: sentinel points at the parent `media/`
    // dir so @-autocomplete shows the `downloads/` subdirectory.
    let download_dir = session_ref
        .as_ref()
        .and_then(|s| {
            crate::model::store::session_media_dir(&s.pwd_hash)
                .ok()
                .map(|m| m.join("downloads"))
        });

    // Ensure the downloads dir exists so web_download can write into it.
    if let Some(ref dir) = download_dir {
        let _ = std::fs::create_dir_all(dir);
    }
    // The active internet tier drives `web_fetch`'s backend choice (Full →
    // scrapion browser, else raw HTTP). No session ⇒ default Simple.
    let internet_mode = session_ref
        .as_ref()
        .map(|s| s.settings.internet_mode)
        .unwrap_or_default();
    // The SSH identity key selected for this session (bare filename, never path
    // or contents). Populated from settings so git_operator can inject it.
    let ssh_key = session_ref
        .as_ref()
        .and_then(|s| s.settings.git_ssh_key.clone());
    // The GLOBAL MCP manager (built once at startup, shared across sessions). Cloned
    // into every ToolCtx so `mcp__*` tool calls can dispatch to their server. `None`
    // before startup builds it (and harmless when there are no MCP servers).
    let mcp_manager = state.rest.mcp_manager.clone();
    // The GLOBAL security daemon manager (built once at startup, shared across
    // sessions). Cloned into every ToolCtx so `sec_*` tool calls can dispatch to
    // the daemon. `None` before startup builds it (and inert when not installed).
    let sec_manager = state.rest.sec_manager.clone();
    // Whether `bash`/`git_operator` run their "saving" output path (filtering +
    // tee-to-disk). No session ⇒ default true.
    let bash_saving = session_ref
        .as_ref()
        .map(|s| s.settings.bash_saving)
        .unwrap_or(true);
    // Tee log directory: `<session_dir>/opt/`. `None` with no active session.
    let bash_log_dir = session_ref.as_ref().map(|s| s.path.join("opt"));
    // The active session's own directory — drives `resolve_read`'s session
    // exemption so the model can read its own `plan.md`/`plan_todos.md` (and
    // nothing from any OTHER session). `None` with no active session.
    let session_dir = session_ref.as_ref().map(|s| s.path.clone());
    crate::tool::ToolCtx {
        workspace,
        workspaces,
        dir_cache: rt.dir_cache.clone(),
        memory_dir,
        worktrees_dir,
        download_dir,
        internet_mode,
        ssh_key,
        mcp_manager,
        sec_manager,
        bash_saving,
        bash_log_dir,
        session_dir,
    }
}

/// THE WORKSPACE-MUTATING PRIMITIVE (Phase 8): repoint session `sess_idx`'s live
/// working directory to `new_cwd` and refresh everything derived from the cwd.
///
/// Most tools are read-only against a [`crate::tool::ToolCtx`]; `cd` is the
/// exception. Both the model-callable `cd` tool (allow-list-checked, intercepted
/// in `process_tools`) and the user `/cd` command (unrestricted) funnel their
/// already-resolved + canonicalised target through HERE so the side effects can
/// never diverge:
///
/// 1. set the session's [`active_cwd`](crate::app::state::SessionRuntime::active_cwd)
///    override (so `effective_cwd()` — and thus `build_tool_ctx`'s
///    `ToolCtx::workspace` + the harness workspace check — now point at `new_cwd`);
/// 2. REBUILD the session's dir cache against the new cwd (so `@`-autocomplete and
///    `dir_list` reflect it). Indexed as a SINGLE root — bare relative paths — to
///    match the shell-cd mental model; the async reindex never blocks the UI;
/// 3. recompute the project-awareness summary for the new cwd, IF awareness is
///    enabled + routable. This mirrors the post-`/compact` recompute
///    (`event_loop::drains`) and, like it, is SPAWNED off the event-loop thread
///    (`spawn_awareness_recompute`) rather than `block_on`, so a slow/hung network
///    call never freezes the UI; when awareness is off (the common case) there is
///    no network at all. The result lands asynchronously, keyed by session id, via
///    the `awareness_rx` drain in `service_global` — never written synchronously
///    here — so a background-session cd's result still lands on the RIGHT session.
///
/// The session's persisted `settings.workdir` list (the allow-list / `[N]` roots)
/// is deliberately UNTOUCHED — cd moves only the cwd. `memory_dir` is also left
/// as-is on purpose (a cd does NOT re-point memory; kept simple).
pub(crate) fn apply_workspace_change(
    state: &mut AppState,
    sess_idx: usize,
    new_cwd: std::path::PathBuf,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) {
    // 1. Repoint the live cwd.
    state.rest.sessions[sess_idx].active_cwd = Some(new_cwd.clone());

    // 2. Rebuild the dir cache against the new cwd (single root → bare paths). The
    //    reindex runs on a background thread and replaces the cache when done.
    crate::tool::dircache::reindex(
        vec![new_cwd.clone()],
        state.rest.sessions[sess_idx].dir_cache.clone(),
    );

    // 3. Recompute awareness for the new cwd when enabled + routable. Snapshot the
    //    inputs (cloning the settings + config) so no session borrow is held
    //    across the spawn. The recompute runs OFF the event-loop thread (see
    //    `spawn_awareness_recompute`) and lands its result asynchronously via
    //    `awareness_rx`, drained in `service_global`; `summarize` returns `None`
    //    on no-docs / disabled / failure, which simply clears the summary —
    //    best-effort, never fatal.
    let aware_inputs = match (
        client.as_ref(),
        state.rest.sessions[sess_idx].session.as_ref(),
    ) {
        (Some(c), Some(sess)) if sess.settings.awareness_enabled => Some((
            Arc::clone(c),
            state.rest.config.clone(),
            sess.settings.clone(),
        )),
        _ => None,
    };
    if let Some((c, config, settings)) = aware_inputs {
        if let Some(route) = crate::app::resolve::resolve_role_dispatch(
            &config,
            &settings,
            crate::model::app_config::ModelRole::Awareness,
        )
        .filter(|r| r.is_routable())
        {
            // Also resolve the Main route as a fallback for when the Awareness
            // model call itself errors (e.g. bad/typo'd model name). Cheap — no I/O.
            let main_route = crate::app::resolve::resolve_role_dispatch(
                &config,
                &settings,
                crate::model::app_config::ModelRole::Main,
            );
            let session_id = state.rest.sessions[sess_idx].id.clone();
            crate::app::runtime::spawn_awareness_recompute(
                state,
                handle,
                session_id,
                c,
                settings,
                new_cwd.clone(),
                route,
                main_route,
            );
        }
    }
}

/// Count the sub-agents currently in [`crate::app::subagent::SubAgentStatus::Running`].
///
/// This is the live concurrency figure both spawn paths check against
/// [`crate::app::subagent::MAX_SUBAGENTS`] before launching: terminated
/// sub-agents are pruned each tick, so a `Running` count is exactly the number
/// of occupied slots. `pub(crate)` so the `/task` command handler can share it.
///
/// Counts session `sess_idx`'s own sub-agents, so the [`crate::app::subagent::MAX_SUBAGENTS`]
/// cap is PER-SESSION (each session gets its own slots), not global.
pub(crate) fn running_subagents(state: &AppState, sess_idx: usize) -> usize {
    state.rest.sessions[sess_idx]
        .subagents
        .iter()
        .filter(|s| matches!(s.status, crate::app::subagent::SubAgentStatus::Running))
        .count()
}

/// Spawn a background sub-agent for `agent_name` running `task_text` under a
/// CALLER-SUPPLIED `id`, wiring it into app state. The core spawn step shared by
/// the live `spawn_task` path (which allocates a fresh id) and `try_start_pending`
/// (which reuses the queued entry's pre-allocated id). Builds the EXACT same
/// `ToolCtx` / registry / awareness / memory inputs in every case.
///
/// On success: pushes the [`crate::app::subagent::SubAgent`] (carrying `id`) into
/// `state.rest.subagents` and returns `Some(id)`. Returns `None` when there is no
/// client/session or the named agent doesn't resolve. Does NOT touch
/// `next_subagent_id` (the caller owns id allocation). Does NOT await the
/// sub-agent; the `$` panel is NOT auto-opened.
///
/// `overrides` steers the SPAWNED agent's route only (see
/// [`crate::app::subagent::spawn_subagent`]'s doc); `None` (every caller except
/// `agents.spawn`) resolves exactly as before.
// Wide by nature: it bakes the full per-session sub-agent context (id, agent,
// task, deferred-call id) on top of `state`/`sess_idx`/client/handle. Splitting
// it into a struct would only obscure the call sites.
#[allow(clippy::too_many_arguments)]
fn spawn_task_with_id(
    state: &mut AppState,
    sess_idx: usize,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
    id: usize,
    agent_name: &str,
    task_text: &str,
    tool_call_id: Option<String>,
    detached: bool,
    ext_owned: bool,
    overrides: Option<crate::app::subagent::SpawnOverrides>,
    initial_injects: Vec<String>,
) -> Result<usize, SpawnFailReason> {
    if client.is_none() || state.rest.sessions[sess_idx].session.is_none() {
        return Err(SpawnFailReason::Unresolved);
    }
    // Refresh global provider/model config at delegation time. Daemons can live
    // longer than config.json edits made by another window; a stale in-memory
    // catalogue would make agent `model_uuid` resolution miss and silently inherit
    // Main. Keep the refreshed copy in rest so this daemon is warmed for the next
    // settings-dependent operation too.
    state.rest.config = crate::model::app_config::AppConfig::load();

    // Snapshot inputs before borrowing state mutably below — identical to the
    // `/task` command's construction so the two paths can never diverge. All
    // per-session inputs (workspace, session dir, settings, awareness, memory)
    // are baked from session `sess_idx`, so a sub-agent keeps ITS PARENT
    // session's context regardless of which session is foreground.
    let mut ctx = build_tool_ctx(state, sess_idx);
    // A caller-supplied `workspace` override (extension `agents.spawn`/
    // `sessions.spawn_into` only — every other spawn path passes `None`) narrows
    // `ctx` to a single confined root INSTEAD of the whole session tree. This is a
    // sandbox trust boundary: on any failure the spawn is rejected outright, never
    // silently widened back to the session workspace.
    narrow_ctx_to_workspace(&mut ctx, overrides.as_ref())?;
    let (session_dir, config, settings, awareness, memory_md) = {
        let rt = &state.rest.sessions[sess_idx];
        let sess = rt.session.as_ref().unwrap();
        let session_dir = sess.path.clone();
        let config = state.rest.config.clone();
        // Use sess.settings as-is: session_models is per-session and authoritative
        // in-memory (this daemon owns it — every mutator edits it in place and then
        // sess.save() persists it to the per-session settings.json). The old
        // re-overlay here re-read session_models from the shared per-dir bucket to
        // work around its former #[serde(skip)]; that field now round-trips in the
        // per-session file, so no refresh is needed and re-reading the (no-longer-
        // written) shared bucket would only reintroduce cross-session drift.
        let settings = sess.settings.clone();
        let pwd_hash = sess.pwd_hash.clone();
        let awareness = rt.awareness_summary.clone().unwrap_or_default();
        // Sub-agents receive the per-PROJECT memory INDEX (pointers only), the
        // same text injected into the main system prompt. Empty when absent.
        let memory_md = crate::model::store::memory_dir(&pwd_hash)
            .ok()
            .and_then(|d| crate::model::memory::load_memory_index(&d))
            .unwrap_or_default();
        (session_dir, config, settings, awareness, memory_md)
    };

    let registry = crate::model::agent_def::AgentRegistry::load(Some(&session_dir));

    // Warn when the agent declared its own model but it failed to resolve against
    // the session's in-memory session_models + global catalogue. The agent will run
    // on Main — surface this so the user isn't left wondering why their chosen model
    // wasn't used. An `overrides.model`, when set, REPLACES the agent's own model
    // reference for this check (applied to a throwaway clone) — so a bad override
    // slug warns too, even when the agent's own declared model (if any) would have
    // resolved fine.
    if let Some(agent) = registry.get(agent_name) {
        let mut check_agent = agent.clone();
        if let Some(model) = overrides.as_ref().and_then(|o| o.model.as_ref()) {
            check_agent.model = Some(model.clone());
            check_agent.model_uuid = None;
            check_agent.provider_uuid = None;
        }
        if crate::app::resolve::agent_declares_model(&check_agent)
            && !crate::app::resolve::agent_model_resolves(&config, &settings, &check_agent)
        {
            state.rest.sessions[sess_idx]
                .set_toast(format!("agent '{}' model unresolved — using main", agent_name));
        }
    }
    let client_arc = Arc::clone(client.as_ref().unwrap());

    let sub = crate::app::subagent::spawn_subagent(
        &client_arc,
        handle,
        &registry,
        &config,
        &settings,
        ctx,
        &awareness,
        &memory_md,
        id,
        agent_name,
        task_text,
        tool_call_id,
        detached,
        ext_owned,
        state.rest.agent_mode,
        overrides,
        initial_injects,
    )
    .ok_or(SpawnFailReason::Unresolved)?;
    state.rest.sessions[sess_idx].subagents.push(sub);
    // Persist the new sub-agent record so it survives close/reopen (#25). Covers
    // every spawn path (model `task`, `/task`, and queued→running promotion via
    // `try_start_pending`, all of which route through here).
    crate::app::runtime::bg_persist::persist_subagents(&state.rest.sessions[sess_idx]);
    Ok(id)
}

/// Spawn a background sub-agent for `agent_name` running `task_text`, allocating
/// it a FRESH id from `next_subagent_id`. Shared by the `/task` slash command and
/// the model-callable `task` tool (via [`spawn_or_queue`]) so both build the EXACT
/// same inputs and advance the same bookkeeping.
///
/// On success: increments `next_subagent_id`, pushes the [`crate::app::subagent::SubAgent`]
/// into `state.rest.subagents`, and returns `Some(id)` (the id assigned to the
/// spawned sub-agent). Returns `None` when there is no client/session or the
/// named agent doesn't exist — the caller surfaces that as it sees fit. Does NOT
/// await the sub-agent. The `$` panel is NOT auto-opened; the user opens it manually.
// Wide by nature (mirrors `spawn_task_with_id`): it bakes the full per-session
// sub-agent context on top of `state`/`sess_idx`/client/handle. A struct would
// only obscure the two call sites.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_task(
    state: &mut AppState,
    sess_idx: usize,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
    agent_name: &str,
    task_text: &str,
    tool_call_id: Option<String>,
    detached: bool,
    ext_owned: bool,
    overrides: Option<crate::app::subagent::SpawnOverrides>,
) -> Result<usize, SpawnFailReason> {
    let id = state.rest.sessions[sess_idx].next_subagent_id;
    let spawned = spawn_task_with_id(
        state, sess_idx, client, handle, id, agent_name, task_text, tool_call_id, detached, ext_owned, overrides,
        // Live spawn: nothing was stashed while queued (there was no queue wait).
        Vec::new(),
    )?;
    // Only consume the id on a successful spawn (a failed spawn leaves it free).
    state.rest.sessions[sess_idx].next_subagent_id += 1;
    Ok(spawned)
}

/// Why a spawn attempt failed outright (see [`SpawnOutcome::Failed`]).
///
/// `Unresolved` is the ORIGINAL (pre-workspace) failure mode: no live
/// client/session, or the named agent doesn't resolve. Every existing caller
/// already has its own agent-name-flavored wording for this case — matching on
/// it preserves that wording byte-for-byte.
///
/// `Workspace` is new: a caller-supplied `workspace` override (see
/// [`crate::app::subagent::SpawnOverrides::workspace`]) failed to canonicalize,
/// or canonicalized outside every one of the session's workspace roots. This is
/// a SANDBOX TRUST BOUNDARY — the carried `String` names the rejected path and
/// is meant to be surfaced to the caller VERBATIM; there is no silent fallback
/// to the wide session workspace.
#[derive(Debug, Clone)]
pub(crate) enum SpawnFailReason {
    Unresolved,
    Workspace(String),
}

/// Outcome of [`spawn_or_queue`]: the delegation was started immediately, parked
/// in the pending queue, or rejected outright.
pub(crate) enum SpawnOutcome {
    /// A slot was free — the sub-agent started now under this id.
    Spawned(usize),
    /// All slots were busy — the delegation was queued under this id and will
    /// start when a slot frees.
    Queued(usize),
    /// No client/session, (for the immediate-spawn branch) the named agent
    /// doesn't exist, or a `workspace` override failed containment — see
    /// [`SpawnFailReason`]. Nothing was started or queued.
    Failed(SpawnFailReason),
}

/// Accept a delegation: spawn it NOW if a slot is free, else ENQUEUE it.
///
/// The single decision point shared by both spawn sites (the `task`-tool
/// interception and the `/task` command). When [`running_subagents`] is below
/// [`crate::app::subagent::MAX_SUBAGENTS`] it spawns immediately (returning
/// [`SpawnOutcome::Spawned`] / [`SpawnOutcome::Failed`] exactly as [`spawn_task`]
/// would); otherwise it enqueues a [`crate::app::subagent::PendingSubagent`] with
/// a freshly-allocated id and returns [`SpawnOutcome::Queued`]. Enqueue still
/// requires a client + session (so a parked task-tool turn can actually resume);
/// without them it returns [`SpawnOutcome::Failed`] and the caller answers the
/// call with an error instead of parking forever.
///
/// This does NOT touch `pending_subagent_calls`: the blocking `task`-tool caller
/// records the call id there itself (for BOTH spawned and queued outcomes) so the
/// parked main turn waits for the delegation whether it ran now or later.
// Wide by nature: threads the full sub-agent context (agent, task, deferred-call
// id, detached flag) through to `spawn_task` / the queue. A struct would only
// obscure the two call sites (`task`-tool interception + `/task` command).
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_or_queue(
    state: &mut AppState,
    sess_idx: usize,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
    agent_name: &str,
    task_text: &str,
    tool_call_id: Option<String>,
    detached: bool,
    ext_owned: bool,
    overrides: Option<crate::app::subagent::SpawnOverrides>,
) -> SpawnOutcome {
    if running_subagents(state, sess_idx) < crate::app::subagent::MAX_SUBAGENTS {
        match spawn_task(state, sess_idx, client, handle, agent_name, task_text, tool_call_id, detached, ext_owned, overrides) {
            Ok(id) => SpawnOutcome::Spawned(id),
            Err(reason) => SpawnOutcome::Failed(reason),
        }
    } else {
        // Over cap: enqueue (unlimited). Needs a client+session so the queued
        // delegation can eventually run and (for a task-tool call) unpark the turn.
        if client.is_none() || state.rest.sessions[sess_idx].session.is_none() {
            return SpawnOutcome::Failed(SpawnFailReason::Unresolved);
        }
        let id = state.rest.sessions[sess_idx].next_subagent_id;
        state.rest.sessions[sess_idx].next_subagent_id += 1;
        state.rest.sessions[sess_idx]
            .pending_subagents
            .push_back(crate::app::subagent::PendingSubagent {
                id,
                agent_name: agent_name.to_string(),
                prompt: task_text.to_string(),
                tool_call_id,
                detached,
                ext_owned,
                overrides,
                // No follow-ups yet; `agents.send`/`task_send` may stash some here
                // while this delegation waits for a slot, delivered at promotion.
                pending_injects: Vec::new(),
            });
        SpawnOutcome::Queued(id)
    }
}

/// Start queued delegations while slots are free.
///
/// Called from the event-loop sub-agent drain after a handle reaches a terminal
/// state (a slot just freed). While [`running_subagents`] is below
/// [`crate::app::subagent::MAX_SUBAGENTS`] and the queue is non-empty, it pops the
/// FRONT [`crate::app::subagent::PendingSubagent`] and spawns it via the SAME
/// spawn path used live ([`spawn_task_with_id`], reusing the queued entry's
/// pre-allocated id).
///
/// A queued `task`-tool delegation's call id is ALREADY in `pending_subagent_calls`
/// (recorded at enqueue time), so a successful start needs no bookkeeping there —
/// the id simply stays until the now-running agent finishes and the drain delivers
/// its result. If the named agent no longer resolves (or client/session vanished),
/// the entry is DROPPED; for a `task`-tool entry we also deliver an error result
/// for its call id (and remove it from `pending_subagent_calls`) so the parked
/// round can't hang on a delegation that will never run.
///
/// Early-returns (leaving the queue intact) when there is no client/session, so a
/// transient gap doesn't drain + fail the whole queue.
pub(crate) fn try_start_pending(
    state: &mut AppState,
    sess_idx: usize,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) {
    if client.is_none() || state.rest.sessions[sess_idx].session.is_none() {
        return;
    }
    while running_subagents(state, sess_idx) < crate::app::subagent::MAX_SUBAGENTS {
        let Some(pending) = state.rest.sessions[sess_idx].pending_subagents.pop_front() else {
            break;
        };
        let started = spawn_task_with_id(
            state,
            sess_idx,
            client,
            handle,
            pending.id,
            &pending.agent_name,
            &pending.prompt,
            pending.tool_call_id.clone(),
            pending.detached,
            pending.ext_owned,
            pending.overrides.clone(),
            // Deliver any follow-ups stashed while this delegation was queued as
            // its first injected messages (right after the task prompt).
            pending.pending_injects.clone(),
        );
        if let Err(reason) = started {
            // The agent no longer resolves, or (for an `agents.spawn` delegation
            // queued with a `workspace` override) that override lost containment
            // between enqueue and promotion. Drop the entry; for a task-tool
            // delegation, free its parked call so the round can't hang.
            if let Some(call_id) = pending.tool_call_id {
                if state.rest.sessions[sess_idx].pending_subagent_calls.contains(&call_id) {
                    state.rest.sessions[sess_idx]
                        .pending_subagent_calls
                        .retain(|c| c != &call_id);
                    let msg = match reason {
                        SpawnFailReason::Unresolved => {
                            format!("error: unknown agent '{}'", pending.agent_name)
                        }
                        SpawnFailReason::Workspace(m) => format!("error: {m}"),
                    };
                    state.rest.sessions[sess_idx].tool_results.push((call_id, msg));
                }
            }
            // Try the next queued entry within the same free slot.
            continue;
        }
    }
}

/// Canonicalize `requested` and verify it is contained within (or equal to) one
/// of `roots` — each of which is ALSO canonicalized for the comparison, so a
/// symlinked workspace root still contains its real children. Containment is
/// checked path-COMPONENT-wise via [`std::path::Path::starts_with`], never a raw
/// string prefix, so a sibling whose name merely starts with a root's name
/// (e.g. `/a/bc` against root `/a/b`) is correctly rejected.
///
/// This is a SANDBOX TRUST BOUNDARY: on any failure — `requested` can't be
/// canonicalized (missing / IO error), or it canonicalizes but sits outside
/// every root — this returns `Err` naming the rejected path. There is
/// deliberately no fallback to the wide session workspace; the caller must fail
/// the spawn outright.
fn canonicalize_and_contain(
    requested: &std::path::Path,
    roots: &[std::path::PathBuf],
) -> Result<std::path::PathBuf, String> {
    let canon = requested
        .canonicalize()
        .map_err(|e| format!("workspace '{}' could not be resolved: {e}", requested.display()))?;
    let contained = roots.iter().any(|root| {
        root.canonicalize()
            .map(|canon_root| canon.starts_with(&canon_root))
            .unwrap_or(false)
    });
    if contained {
        Ok(canon)
    } else {
        Err(format!(
            "workspace '{}' is outside the session's allowed workspace root(s)",
            canon.display()
        ))
    }
}

/// Apply a caller-supplied `workspace` override (see
/// [`crate::app::subagent::SpawnOverrides::workspace`]) to `ctx`, if present.
///
/// `overrides` absent, or present with `workspace: None`, is a no-op — `ctx` is
/// returned exactly as `build_tool_ctx` produced it (every non-extension spawn
/// path, and the common `agents.spawn`/`sessions.spawn_into` call). When a
/// `workspace` IS present, it is canonicalized and checked for containment
/// within one of `ctx.workspaces`' EXISTING roots via [`canonicalize_and_contain`];
/// on success both `ctx.workspace` and `ctx.workspaces` are replaced with the
/// single canonicalized path (the sub-agent then sees ONLY that root). On
/// failure `ctx` is left untouched and `Err` is returned — this is a sandbox
/// trust boundary, so a rejected override never silently falls back to the wide
/// session workspace.
fn narrow_ctx_to_workspace(
    ctx: &mut crate::tool::ToolCtx,
    overrides: Option<&crate::app::subagent::SpawnOverrides>,
) -> Result<(), SpawnFailReason> {
    let Some(requested) = overrides.and_then(|o| o.workspace.as_ref()) else {
        return Ok(());
    };
    match canonicalize_and_contain(requested, &ctx.workspaces) {
        Ok(canon) => {
            ctx.workspace = canon.clone();
            ctx.workspaces = vec![canon];
            Ok(())
        }
        Err(msg) => Err(SpawnFailReason::Workspace(msg)),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::{Arc, RwLock};

    /// A minimal `ToolCtx` for tests, mirroring `tool::seqthink_test::test_ctx` —
    /// the workspace-narrowing logic never touches any field besides
    /// `workspace`/`workspaces`.
    fn test_ctx(workspaces: Vec<std::path::PathBuf>) -> crate::tool::ToolCtx {
        crate::tool::ToolCtx {
            workspace: workspaces.first().cloned().unwrap_or_default(),
            workspaces,
            dir_cache: Arc::new(RwLock::new(crate::tool::DirCache::default())),
            memory_dir: None,
            worktrees_dir: None,
            download_dir: None,
            internet_mode: crate::model::settings::InternetMode::default(),
            ssh_key: None,
            mcp_manager: None,
            sec_manager: None,
            bash_saving: true,
            bash_log_dir: None,
            session_dir: None,
        }
    }

    /// Absent override (either no `SpawnOverrides` at all, or one with
    /// `workspace: None`) leaves `ctx` byte-identical to what `build_tool_ctx`
    /// produced — no canonicalization, no narrowing.
    #[test]
    fn absent_workspace_leaves_ctx_unchanged() {
        let root = std::env::temp_dir();
        let mut ctx = test_ctx(vec![root.clone()]);
        let before = (ctx.workspace.clone(), ctx.workspaces.clone());

        assert!(narrow_ctx_to_workspace(&mut ctx, None).is_ok());
        assert_eq!((ctx.workspace.clone(), ctx.workspaces.clone()), before);

        let overrides = crate::app::subagent::SpawnOverrides::default();
        assert!(narrow_ctx_to_workspace(&mut ctx, Some(&overrides)).is_ok());
        assert_eq!((ctx.workspace, ctx.workspaces), before);
    }

    /// A requested path INSIDE one of the session's existing roots narrows
    /// `ctx.workspace`/`ctx.workspaces` down to that single canonicalized path.
    #[test]
    fn containment_pass_narrows_ctx_to_single_root() {
        let base = std::env::temp_dir().join(format!("koma-spawn-test-pass-{}", std::process::id()));
        let child = base.join("desk-1");
        std::fs::create_dir_all(&child).expect("create nested test dir");

        let mut ctx = test_ctx(vec![base.clone()]);
        let overrides = crate::app::subagent::SpawnOverrides {
            workspace: Some(child.clone()),
            ..Default::default()
        };
        narrow_ctx_to_workspace(&mut ctx, Some(&overrides)).expect("contained path must pass");

        let canon_child = child.canonicalize().unwrap();
        assert_eq!(ctx.workspace, canon_child);
        assert_eq!(ctx.workspaces, vec![canon_child]);

        let _ = std::fs::remove_dir_all(&base);
    }

    /// A requested path OUTSIDE every one of the session's roots is rejected —
    /// `ctx` is left untouched and the spawn fails with a `Workspace` reason
    /// naming the rejected path.
    #[test]
    fn containment_reject_when_outside_every_root() {
        let root = std::env::temp_dir().join(format!("koma-spawn-test-root-{}", std::process::id()));
        let outsider = std::env::temp_dir().join(format!("koma-spawn-test-outsider-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("create root dir");
        std::fs::create_dir_all(&outsider).expect("create outsider dir");

        let mut ctx = test_ctx(vec![root.clone()]);
        let before = (ctx.workspace.clone(), ctx.workspaces.clone());
        let overrides = crate::app::subagent::SpawnOverrides {
            workspace: Some(outsider.clone()),
            ..Default::default()
        };
        let err = narrow_ctx_to_workspace(&mut ctx, Some(&overrides))
            .expect_err("outsider path must be rejected");
        match err {
            SpawnFailReason::Workspace(msg) => assert!(
                msg.contains(&outsider.canonicalize().unwrap().display().to_string()),
                "error must name the rejected path: {msg}"
            ),
            SpawnFailReason::Unresolved => panic!("expected a Workspace failure reason"),
        }
        assert_eq!((ctx.workspace, ctx.workspaces), before, "ctx must be untouched on rejection");

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outsider);
    }

    /// PREFIX TRAP: a sibling directory whose name merely starts with the root's
    /// name must never pass a raw string-prefix check — containment is
    /// component-wise. Build root `<base>/b` and requested `<base>/bc`: `bc` is
    /// NOT a child of `b`, so this must be rejected even though the string
    /// "…/b" is a literal prefix of "…/bc".
    #[test]
    fn containment_rejects_string_prefix_trap() {
        let base = std::env::temp_dir().join(format!("koma-spawn-test-trap-{}", std::process::id()));
        let root = base.join("b");
        let sibling = base.join("bc");
        std::fs::create_dir_all(&root).expect("create root dir");
        std::fs::create_dir_all(&sibling).expect("create sibling dir");

        let mut ctx = test_ctx(vec![root.clone()]);
        let overrides = crate::app::subagent::SpawnOverrides {
            workspace: Some(sibling.clone()),
            ..Default::default()
        };
        let err = narrow_ctx_to_workspace(&mut ctx, Some(&overrides))
            .expect_err("string-prefix sibling must NOT pass containment");
        assert!(matches!(err, SpawnFailReason::Workspace(_)));

        let _ = std::fs::remove_dir_all(&base);
    }
}
