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
    crate::tool::ToolCtx {
        workspace,
        workspaces,
        dir_cache: rt.dir_cache.clone(),
        memory_dir,
        internet_mode,
        ssh_key,
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
///    (`event_loop::drains`) and, like it, runs via `block_on` on the event-loop
///    thread; when awareness is off (the common case) there is no network at all.
///    The summary is recomputed for the session at `sess_idx` directly (not
///    `fg_mut()`), so a background-session cd updates the RIGHT session.
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
    //    across the `block_on`. `summarize` returns `None` on no-docs / disabled /
    //    failure, which simply clears the summary — best-effort, never fatal.
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
        if let Some(route) = crate::app::resolve::resolve_role(
            &config,
            &settings,
            crate::model::app_config::ModelRole::Awareness,
        )
        .filter(|r| r.is_routable())
        {
            let summary = handle.block_on(crate::app::awareness::summarize(
                &c,
                &settings,
                route.conn(),
                &route.model_id,
                route.provider(),
                &new_cwd,
            ));
            state.rest.sessions[sess_idx].awareness_summary = summary;
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
) -> Option<usize> {
    if client.is_none() || state.rest.sessions[sess_idx].session.is_none() {
        return None;
    }
    // Snapshot inputs before borrowing state mutably below — identical to the
    // `/task` command's construction so the two paths can never diverge. All
    // per-session inputs (workspace, session dir, settings, awareness, memory)
    // are baked from session `sess_idx`, so a sub-agent keeps ITS PARENT
    // session's context regardless of which session is foreground.
    let ctx = build_tool_ctx(state, sess_idx);
    let (session_dir, config, settings, awareness, memory_md) = {
        let rt = &state.rest.sessions[sess_idx];
        let sess = rt.session.as_ref().unwrap();
        let session_dir = sess.path.clone();
        let config = state.rest.config.clone();
        let settings = sess.settings.clone();
        let awareness = rt.awareness_summary.clone().unwrap_or_default();
        // Sub-agents receive the per-PROJECT memory INDEX (pointers only), the
        // same text injected into the main system prompt. Empty when absent.
        let memory_md = crate::model::store::memory_dir(&sess.pwd_hash)
            .ok()
            .and_then(|d| crate::model::memory::load_memory_index(&d))
            .unwrap_or_default();
        (session_dir, config, settings, awareness, memory_md)
    };

    let registry = crate::model::agent_def::AgentRegistry::load(Some(&session_dir));
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
    )?;
    state.rest.sessions[sess_idx].subagents.push(sub);
    Some(id)
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
pub(crate) fn spawn_task(
    state: &mut AppState,
    sess_idx: usize,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
    agent_name: &str,
    task_text: &str,
    tool_call_id: Option<String>,
) -> Option<usize> {
    let id = state.rest.sessions[sess_idx].next_subagent_id;
    let spawned =
        spawn_task_with_id(state, sess_idx, client, handle, id, agent_name, task_text, tool_call_id)?;
    // Only consume the id on a successful spawn (a failed spawn leaves it free).
    state.rest.sessions[sess_idx].next_subagent_id += 1;
    Some(spawned)
}

/// Outcome of [`spawn_or_queue`]: the delegation was started immediately, parked
/// in the pending queue, or rejected outright.
pub(crate) enum SpawnOutcome {
    /// A slot was free — the sub-agent started now under this id.
    Spawned(usize),
    /// All slots were busy — the delegation was queued under this id and will
    /// start when a slot frees.
    Queued(usize),
    /// No client/session, or (for the immediate-spawn branch) the named agent
    /// doesn't exist. Nothing was started or queued.
    Failed,
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
pub(crate) fn spawn_or_queue(
    state: &mut AppState,
    sess_idx: usize,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
    agent_name: &str,
    task_text: &str,
    tool_call_id: Option<String>,
) -> SpawnOutcome {
    if running_subagents(state, sess_idx) < crate::app::subagent::MAX_SUBAGENTS {
        match spawn_task(state, sess_idx, client, handle, agent_name, task_text, tool_call_id) {
            Some(id) => SpawnOutcome::Spawned(id),
            None => SpawnOutcome::Failed,
        }
    } else {
        // Over cap: enqueue (unlimited). Needs a client+session so the queued
        // delegation can eventually run and (for a task-tool call) unpark the turn.
        if client.is_none() || state.rest.sessions[sess_idx].session.is_none() {
            return SpawnOutcome::Failed;
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
        );
        if started.is_none() {
            // The agent no longer resolves. Drop the entry; for a task-tool
            // delegation, free its parked call so the round can't hang.
            if let Some(call_id) = pending.tool_call_id {
                if state.rest.sessions[sess_idx].pending_subagent_calls.contains(&call_id) {
                    state.rest.sessions[sess_idx]
                        .pending_subagent_calls
                        .retain(|c| c != &call_id);
                    state.rest.sessions[sess_idx].tool_results.push((
                        call_id,
                        format!("error: unknown agent '{}'", pending.agent_name),
                    ));
                }
            }
            // Try the next queued entry within the same free slot.
            continue;
        }
    }
}
