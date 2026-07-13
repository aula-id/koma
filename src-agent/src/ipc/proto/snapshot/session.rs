// ─── per-session state projections (pure data) ───────────────────────────────

use serde::{Deserialize, Serialize};

use crate::dto::chat::ChatMessage;

use super::global::GlobalSnapshot;

/// A complete, frozen projection of the daemon's renderable state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct StateSnapshot {
    pub foreground_id: Option<String>,
    pub sessions: Vec<SessionSnapshot>,
    pub global: GlobalSnapshot,
}

/// A per-session projection of everything the client needs to render ONE session tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct SessionSnapshot {
    pub id: String,
    pub name: String,
    pub cwd: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub committed_reasoning: Vec<Option<String>>,
    pub streaming: Option<String>,
    pub stream_reasoning: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost: f64,
    pub tokens_cached: u64,
    pub waiting: bool,
    pub awaiting_approval: bool,
    pub approval_reason: Option<String>,
    pub pending_tool_calls: Vec<crate::dto::chat::ToolCall>,
    pub tool_idx: usize,
    pub working: bool,
    pub finished_unseen: bool,
    pub subagents: Vec<SubAgentSnapshot>,
    pub pending_subagents: Vec<PendingSubagentSnapshot>,
    pub resolved_model_id: String,
    /// Truncated previews of the foreground session's queued mid-turn steer messages
    /// (full text lives daemon-side). Drives the pending panel between transcript +
    /// composer. Empty = no panel.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_steer: Vec<String>,
    /// The session's background-bash jobs (list + RAW status only — no output/elapsed).
    /// Carried so the native-React GUI's Explore sidepanel `bash[]` shows jobs — INCLUDING
    /// finished / failed ones — which it never did before (the shadow's `bash_jobs` had no
    /// wire source and stayed empty). The client shadow rebuilds an INERT
    /// [`crate::app::bgbash::BashJob`] from each. `#[serde(default, skip_serializing_if)]`
    /// keeps the no-jobs case + version-skewed peers wire-free. The TUI client ignores it
    /// (its `/bash` panel sources the separate `BashSnapshot`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bash_jobs: Vec<BashJobSnapshot>,
    /// The session's CUMULATIVE file-change log (#24): every workspace file the
    /// `write`/`edit`/`delete` tools touched this session, with its latest status
    /// (added/modified/deleted, dedup by path). Persisted per-session (survives
    /// `/compact` + close/reopen) and projected here so the native-React GUI's
    /// Explore "File changed" panel renders it. `#[serde(default, skip_serializing_if)]`
    /// keeps the no-changes case + version-skewed peers wire-free. The TUI ignores it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_changes: Vec<FileChangeSnapshot>,
    /// The session's user-facing Plan-mode todo checklist (Plan mode only; empty
    /// outside it or when no plan is in progress). The two locked workflow rails
    /// now ride the wire too (flagged via `PlanTodoSnapshot::locked`) so the GUI
    /// shows the TUI-parity rails right after `plan_enter`, before the model's
    /// first `checklist` lands. Projected so the native-React GUI's Explore
    /// "PLAN" section renders live; the TUI ignores it (its `/todo` overlay
    /// reads `plan_todos.md` directly). `#[serde(default, skip_serializing_if)]`
    /// keeps the no-plan case + version-skewed peers wire-free.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plan_todos: Vec<PlanTodoSnapshot>,
}

/// A serde-safe projection of ONE Plan-mode todo entry for the GUI Explore "PLAN"
/// section: the step text, its status, and whether it's one of the two locked
/// workflow rails (internal bookkeeping, not model-authored content — the GUI
/// dims these and excludes them from the done/total count).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct PlanTodoSnapshot {
    pub content: String,
    pub status: crate::app::mode::todo::TodoStatus,
    /// `#[serde(default)]` so an older daemon's wire payload (pre-locked-field)
    /// decodes cleanly as `false`.
    #[serde(default)]
    pub locked: bool,
}

/// A serde-safe projection of ONE cumulative file-change entry for the GUI Explore
/// "File changed" panel: the (workspace-relative when possible) path + its latest
/// status string (`"added"` / `"modified"` / `"deleted"`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct FileChangeSnapshot {
    pub path: String,
    pub status: String,
}

/// A serde-safe projection of ONE background bash job for the native-React GUI's
/// per-session sidepanel.
///
/// Unlike [`BashJobView`] (the pre-rendered `/bash` full-screen panel row, which carries
/// elapsed time + an output tail), this carries only identity + the RAW
/// [`crate::app::bgbash::BashJobStatus`] so the client shadow can rebuild an inert
/// [`crate::app::bgbash::BashJob`] whose `snapshot_status()` matches the daemon exactly
/// (running / done / killed / error, incl. failed-at-spawn jobs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct BashJobSnapshot {
    pub id: usize,
    pub command: String,
    pub status: crate::app::bgbash::BashJobStatus,
    /// Captured OUTPUT TAIL — populated ONLY for the job a client is currently
    /// streaming into an Explore stream tab (`ClientRequest::SetStreamView`), during
    /// THAT client's per-client snapshot build (see the hub's `stream_deltas`), so the
    /// live `Arc<Mutex<String>>` output can cross the wire for the viewed job alone.
    /// `None` for every non-viewed job (the common case), keeping the wire lean + the
    /// per-session diff quiet for un-viewed jobs. A change to a VIEWED job's tail forces
    /// a full resync for that client only (it rides `SessionSnapshot.bash_jobs`'s
    /// structural diff). `#[serde(default, skip_serializing_if)]` keeps the None case +
    /// version-skewed peers wire-free; the client shadow rebuilds the inert job's output
    /// buffer from it (`client_shadow::shadow_bash_job`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tail: Option<String>,
}

/// A plain-data projection of one SubAgent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct SubAgentSnapshot {
    pub id: usize,
    pub name: String,
    pub label: String,
    pub status: String,
    /// Whether this sub-agent is backgrounded (detached). Projected so the
    /// client's `!detached` render/cadence checks work — without it the client
    /// defaults every agent to attached and keeps animating a backgrounded one.
    #[serde(default)]
    pub detached: bool,
    pub steps: usize,
    pub transcript: Vec<String>,
    pub messages: Vec<ChatMessage>,
    /// Live in-progress assistant report text for the current (not-yet-committed)
    /// turn, mirrored from `SubAgent::live_text`. Lets the full-screen viewer render
    /// the streaming report on a thin client as it arrives, instead of only after the
    /// turn commits into `messages`. `#[serde(default)]` keeps version-skewed peers
    /// safe (empty when absent).
    #[serde(default)]
    pub live_text: String,
    /// Out-of-band, index-aligned reasoning for `messages` (mirrors
    /// `SessionSnapshot::committed_reasoning`). `ChatMessage::reasoning` is
    /// `#[serde(skip)]`, so the viewer's thinking blocks must ride this side
    /// channel to survive IPC. Empty vec = no reasoning on any message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub committed_reasoning: Vec<Option<String>>,
}

/// A plain-data projection of one queued PendingSubagent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct PendingSubagentSnapshot {
    pub id: usize,
    pub agent_name: String,
    pub prompt: String,
}
