//! Wave-5 (#25): persist + restore the per-session background-bash job and
//! sub-agent RECORDS so the GUI Explore Bash/Agents lists survive close/reopen.
//!
//! The live processes — bg-bash worker threads, sub-agent tokio tasks — die with
//! the daemon and CANNOT be resurrected. So we persist only the RECORD (id +
//! command/name + last-known status) into the per-session `messages.sqlite` (via
//! [`crate::model::msglog`]), and on session load restore them as INERT records:
//! a [`BashJob`] with no worker thread (mirrors the client-side shadow) and a
//! [`SubAgent`] with an inert abort-handle + never-drained receiver (mirrors
//! `client_shadow::shadow_subagent`). A record that was still `Running` at close
//! restores as settled-stale (`Killed` — the neutral "no color signal" render),
//! NEVER as running, since there is no live worker to reattach.
//!
//! Persistence is a REPLACE-ALL of the live vec, fired at the two lifecycle
//! transitions each kind has (spawn + terminal), so the table always mirrors the
//! in-memory list. All writes are best-effort.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::app::bgbash::{BashJob, BashJobShared, BashJobStatus};
use crate::app::state::SessionRuntime;
use crate::app::subagent::{SubAgent, SubAgentStatus};
use crate::model::msglog::{BashJobRecord, SubAgentRecord};

/// Map a live [`SubAgentStatus`] to its wire/persist string — identical to the
/// snapshot projection (`ipc::snapshot::projection::core::subagent_snapshot`), so
/// a restored record renders the same status glyph the live one did.
fn subagent_status_str(s: &SubAgentStatus) -> String {
    match s {
        SubAgentStatus::Running => "running".to_string(),
        SubAgentStatus::Done(_) => "done".to_string(),
        SubAgentStatus::Killed => "killed".to_string(),
        SubAgentStatus::Error(e) => format!("error: {e}"),
    }
}

/// Persist the session's background-bash jobs (REPLACE-ALL). Best-effort; a
/// session with no on-disk dir (e.g. a headless/shadow runtime) is skipped.
pub(crate) fn persist_bash_jobs(rt: &SessionRuntime) {
    let Some(dir) = rt.session.as_ref().map(|s| s.path.clone()) else {
        return;
    };
    let records: Vec<BashJobRecord> = rt
        .bash_jobs
        .iter()
        .map(|j| BashJobRecord {
            id: j.id as i64,
            command: j.command.clone(),
            // Round-trip the full status faithfully via serde (keeps Done(code) /
            // Error(msg) detail). A serialise failure degrades to a neutral Killed.
            status: serde_json::to_string(&j.snapshot_status())
                .unwrap_or_else(|_| "\"Killed\"".to_string()),
        })
        .collect();
    let _ = crate::model::msglog::write_bash_jobs(&dir, &records);
}

/// Persist the session's sub-agents (REPLACE-ALL). Best-effort.
pub(crate) fn persist_subagents(rt: &SessionRuntime) {
    let Some(dir) = rt.session.as_ref().map(|s| s.path.clone()) else {
        return;
    };
    let records: Vec<SubAgentRecord> = rt
        .subagents
        .iter()
        .map(|a| SubAgentRecord {
            id: a.id as i64,
            name: a.agent_name.clone(),
            label: a.label.clone(),
            status: subagent_status_str(&a.status),
        })
        .collect();
    let _ = crate::model::msglog::write_subagents(&dir, &records);
}

/// Restore the persisted bg-bash + sub-agent records into a freshly built
/// [`SessionRuntime`] as INERT records, and bump `next_bash_job_id` /
/// `next_subagent_id` above the highest restored id so a new job/agent never
/// collides with a restored one. Called once from `install_daemon_session` right
/// after the session is installed. `handle` is the daemon's tokio runtime handle,
/// used to mint the inert sub-agent abort-handle without an ambient runtime.
pub(crate) fn restore_bg_records(
    rt: &mut SessionRuntime,
    session_dir: &Path,
    handle: &tokio::runtime::Handle,
) {
    // --- bg-bash jobs ---
    let mut max_bash_id: usize = 0;
    for rec in crate::model::msglog::read_bash_jobs(session_dir) {
        let id = rec.id.max(0) as usize;
        max_bash_id = max_bash_id.max(id);
        // Decode the persisted status; coerce a still-Running record to Killed —
        // the worker died with the previous daemon, so it is stale, never running.
        let status = match serde_json::from_str::<BashJobStatus>(&rec.status) {
            Ok(BashJobStatus::Running) | Err(_) => BashJobStatus::Killed,
            Ok(s) => s,
        };
        // Restored records are always terminal (Running is coerced to Killed
        // above). Stamp ended_at = started_at so `/bash` elapsed freezes at 0s
        // instead of climbing from Instant::now() forever.
        let started = Instant::now();
        rt.bash_jobs.push(BashJob {
            id,
            command: rec.command,
            started_at: started,
            // Restored jobs are never mid-park of a live turn.
            tool_call_id: None,
            suppress_completion_nudge: false,
            // No worker thread — the shared state is pre-baked from the record, so
            // `snapshot_status()` returns the restored (terminal) status. Mirrors
            // `client_shadow::session::shadow_bash_job`.
            shared: Arc::new(BashJobShared {
                output: Mutex::new(String::new()),
                status: Mutex::new(status),
                pid: Mutex::new(None),
                ended_at: Mutex::new(Some(started)),
                tee_path: Mutex::new(None),
                deadline: Mutex::new(None),
            }),
        });
    }
    if max_bash_id >= rt.next_bash_job_id {
        rt.next_bash_job_id = max_bash_id + 1;
    }

    // --- sub-agents ---
    let mut max_sub_id: usize = 0;
    for rec in crate::model::msglog::read_subagents(session_dir) {
        let id = rec.id.max(0) as usize;
        max_sub_id = max_sub_id.max(id);
        // Coerce a still-"running" record to Killed (settled-stale) — no live task
        // to reattach. Terminal statuses keep their kind (error detail preserved).
        let status = match rec.status.as_str() {
            "done" => SubAgentStatus::Done(String::new()),
            "killed" | "running" => SubAgentStatus::Killed,
            s if s.starts_with("error:") => {
                SubAgentStatus::Error(s.trim_start_matches("error:").trim().to_string())
            }
            _ => SubAgentStatus::Killed,
        };
        // Computed before `status` moves into the `SubAgent` literal below.
        let is_terminal = !matches!(status, SubAgentStatus::Running);
        // Inert abort handle + never-drained receiver (mirrors
        // `client_shadow::session::shadow_subagent`): a task that completes at once,
        // whose handle is never used to abort, and a fresh channel nothing writes to.
        let abort = handle.spawn(std::future::ready(())).abort_handle();
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        // Inert injection sender: a restored record is always terminal (no live
        // loop drains it), so `inject_into_subagent` reports `Terminal` and never
        // sends — this channel is only here to satisfy the field's type.
        let (inject_tx, _inject_rx) = tokio::sync::mpsc::unbounded_channel();
        rt.subagents.push(SubAgent {
            id,
            agent_name: rec.name,
            label: rec.label,
            model_id: String::new(),
            status,
            abort,
            rx,
            inject_tx,
            // The live transcript/report is NOT persisted (record + status only), so a
            // restored agent shows an empty history — honest for a dead worker.
            transcript: Vec::new(),
            messages: Vec::new(),
            live_text: String::new(),
            tool_call_id: None,
            detached: false,
            // Pre-latch terminal records so the /task-path completion-nudge scan
            // (`drain_subagents`, gated on `!sa.nudged`) can never re-enter them
            // and re-fold "[sub-agent #N ...] finished:" into chat every tick — a
            // defense-in-depth backstop even if a future match arm forgets its own
            // latch. The status match above always yields a terminal variant
            // (Done/Killed/Error; a still-"running" record is coerced to Killed,
            // since there is no live task to reattach), so this is currently
            // always `true`; written as a derived check rather than a literal so a
            // future non-terminal restore path fails safe (`nudged: false`, i.e.
            // still eligible for a nudge) instead of silently swallowing one.
            nudged: is_terminal,
            // Restored records are dead AND already latched (`nudged` above), so the
            // /task terminal fold and the `agents.done` Running->terminal edge can
            // never re-fire on them — the origin flag is therefore INERT here and is
            // deliberately NOT persisted (the `SubAgentRecord` carries no `ext_owned`
            // column). `false` is the safe, behavior-neutral value.
            ext_owned: false,
            usage_tokens_in: 0,
            usage_tokens_out: 0,
            usage_cost: 0.0,
            // Restored records are inert — the node_id claim is stale and
            // must not drive handoff application on a restored session.
            sdlc_active_node_id: None,
        });
    }
    if max_sub_id >= rt.next_subagent_id {
        rt.next_subagent_id = max_sub_id + 1;
    }
}
