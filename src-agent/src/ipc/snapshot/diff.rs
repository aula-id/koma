//! Pure render-state DIFF for the daemon stage-4 streaming layer.
//!
//! [`diff`] compares a freshly-built snapshot against the previously-sent one and
//! yields the minimal set of [`StateDelta`]s for the high-frequency per-tick
//! changes, OR signals (`needs_full`) that a STRUCTURAL change happened that is
//! not worth diffing incrementally — in which case the caller resends a full
//! [`StateSnapshot`] instead. Correctness-first (daemon stage 4): when in doubt,
//! ask for a full snapshot; a full snapshot is ALWAYS a valid update.

use crate::ipc::proto::{StateDelta, StateSnapshot};

/// The outcome of diffing the current snapshot against the previously-sent one.
///
/// Either a set of fine-grained [`StateDelta`]s to fan out (each becomes one
/// seq-tagged `Delta` frame), or `needs_full` — a STRUCTURAL change the daemon
/// answers with a fresh full `Snapshot` frame instead (and the `deltas` are then
/// ignored). The two are mutually exclusive by construction: the moment a
/// structural change is detected, diffing stops and `needs_full` is set.
#[derive(Debug, Default, PartialEq)]
pub struct DiffResult {
    /// When true, the incremental `deltas` are INSUFFICIENT (a session was
    /// added/removed, the id set changed, or a hard-to-diff field moved); the
    /// caller must resend a full [`StateSnapshot`] instead.
    pub needs_full: bool,
    /// Fine-grained updates to fan out, in emission order. Empty + `!needs_full`
    /// means nothing changed this tick (no frame is emitted).
    pub deltas: Vec<StateDelta>,
}

impl DiffResult {
    /// The "resend a full snapshot" outcome (deltas are irrelevant then).
    fn full() -> Self {
        Self {
            needs_full: true,
            deltas: Vec::new(),
        }
    }
}

/// Diff `prev` -> `next` into the minimal [`StateDelta`]s, or request a full
/// snapshot for a structural change.
///
/// High-frequency per-tick changes are diffed incrementally (streamed token /
/// reasoning appends, working / finished-unseen flips, the global + per-session
/// status line, the foreground switch, the toast). Anything STRUCTURAL or awkward
/// to fold incrementally — the session list changing, a session's committed
/// history / token counters / approval state / sub-agent set moving — short-
/// circuits to `needs_full` so the client rebuilds from a fresh snapshot. This is
/// the correctness-first stance the stage calls for: a full snapshot is always a
/// valid update, so when in doubt we send one rather than risk a wrong shadow.
pub fn diff(prev: &StateSnapshot, next: &StateSnapshot) -> DiffResult {
    // --- structural: the mode VARIANT or its payload changed ---
    // `ModeSnapshot` is now a pure-data projection (not a bare tag), so this `!=`
    // fires on BOTH a variant switch (Chat -> QuitConfirm) AND a payload change
    // within a variant (e.g. QuitConfirm's busy/total counts moving). Neither is
    // carried by any incremental delta, and an idle session has no other structural
    // change to coincidentally trigger a full snapshot, so without this the client
    // shadow stays in the old mode/payload (e.g. never enters QuitConfirm, so its
    // key-interception branch never fires; or shows a stale overlay header). A full
    // snapshot rebuilds the screen and is always a valid update, so force one the
    // instant the mode projection moves. (The per-tick `work_elapsed_ms` is
    // deliberately NOT diffed — see the note where the global fields are compared.)
    if prev.global.mode != next.global.mode {
        return DiffResult::full();
    }

    // --- structural: theme / accent palette changed ---
    // The daemon builds the outer palette via `theme::palette(&state.rest.config)` BEFORE
    // dispatching to any mode renderer, so without this the client stays in the default
    // palette (Dark/green) until the next structural change forces a full resync. A full
    // snapshot ensures the client's `rest.config` palette stays in sync with the daemon's.
    if prev.global.theme != next.global.theme
        || prev.global.accent != next.global.accent
        || prev.global.agent_mode != next.global.agent_mode
        || prev.global.latest_version != next.global.latest_version
    {
        return DiffResult::full();
    }

    // --- structural: the sub-agent viewer / `$` panel state changed ---
    // These global flags (the full-screen viewer's open-index + scroll + follow, and
    // the `$` panel's open-state + selection) are rendered FROM Chat mode, so a change
    // doesn't move `global.mode` and no incremental delta carries them. They flip only
    // on discrete user actions (open/scroll the viewer, open/navigate the panel), so a
    // full snapshot on a change is cheap-correct — without it the client's viewer/panel
    // would lag until the next structural change. (The viewer's CONTENT updates already
    // force a full snapshot via the per-session `subagents` comparison below.)
    // The `@` file-picker + `/` command-picker selection index (`palette_sel`) is in
    // the same boat: it renders from Chat mode, rides no incremental delta, and changes
    // only on discrete Up/Down, so a full snapshot on change is cheap-correct.
    if prev.global.agent_viewer != next.global.agent_viewer
        || prev.global.agent_viewer_scroll != next.global.agent_viewer_scroll
        || prev.global.agent_viewer_follow != next.global.agent_viewer_follow
        || prev.global.subagents_open != next.global.subagents_open
        || prev.global.subagent_sel != next.global.subagent_sel
        || prev.global.palette_sel != next.global.palette_sel
    {
        return DiffResult::full();
    }

    // --- structural: staged composer attachments / `@`-file palette changed ---
    // Neither rides an incremental delta. `pending_attachments` flips only on a
    // discrete attach/submit/clear; `file_palette` changes as an `@token` is typed
    // (the match set narrows) — both infrequent relative to streaming, so a full
    // snapshot on a change is cheap-correct. Without this the client's `[Image #N]`
    // card data lags and (crucially) its `@` dropdown — which renders ONLY from the
    // projected `file_palette` on a thin client — never updates as the user types
    // the partial. (The `[Image #N]` marker TEXT still rides `input` via InputChanged,
    // but the palette + attachment records need the snapshot.)
    if prev.global.pending_attachments != next.global.pending_attachments
        || prev.global.file_palette != next.global.file_palette
    {
        return DiffResult::full();
    }

    // --- structural: the on-demand model catalogue changed ---
    // The omnisearch cache (and the endpoint it was fetched for) feeds the Settings
    // model modal + the KeyInput search dropdowns. It changes only when a fetch
    // lands (infrequent) and no incremental delta carries it, so a change forces a
    // full snapshot — the screen that reads it (Settings/KeyInput) then re-renders
    // with the populated dropdown instead of a stale `searching models…`.
    if prev.global.models_cache != next.global.models_cache
        || prev.global.models_cache_endpoint != next.global.models_cache_endpoint
    {
        return DiffResult::full();
    }

    // --- structural: the session SET (count or id order) changed ---
    // A different length or a reordered/replaced id list can't be expressed by the
    // per-session deltas (which address sessions by id and assume the set is
    // stable), so rebuild wholesale. SessionAdded exists in the vocabulary, but a
    // full snapshot is simpler AND always correct for any list change.
    if prev.sessions.len() != next.sessions.len()
        || prev
            .sessions
            .iter()
            .zip(next.sessions.iter())
            .any(|(a, b)| a.id != b.id)
    {
        return DiffResult::full();
    }

    let mut deltas: Vec<StateDelta> = Vec::new();

    // --- per-session, id-keyed (the set is identical + in the same order here) ---
    for (p, n) in prev.sessions.iter().zip(next.sessions.iter()) {
        // Any of these moving is either hard to fold incrementally or rare enough
        // that a full resync is the honest, cheap-correct answer.
        let structural = p.messages != n.messages
            // Committed reasoning rides the message list; a change (new turn's
            // thinking block committed) has no incremental delta, so resync.
            || p.committed_reasoning != n.committed_reasoning
            || p.tokens_in != n.tokens_in
            || p.tokens_out != n.tokens_out
            || p.tokens_cached != n.tokens_cached
            || p.cost != n.cost
            || p.awaiting_approval != n.awaiting_approval
            || p.approval_reason != n.approval_reason
            // The pending tool-call set / cursor moving changes what the approval
            // overlay draws; no incremental delta carries it, so resync wholesale.
            || p.pending_tool_calls != n.pending_tool_calls
            || p.tool_idx != n.tool_idx
            || p.name != n.name
            // A `cd` (the effective cwd moving) has no incremental delta, so a
            // change forces a full resync — the client rebuilds with the new cwd.
            || p.cwd != n.cwd
            || p.subagents != n.subagents
            || p.pending_subagents != n.pending_subagents
            // A model change (settings override or global catalogue edit) has no
            // incremental delta; resync so the header updates immediately.
            || p.resolved_model_id != n.resolved_model_id;
        if structural {
            return DiffResult::full();
        }

        // Streaming content: only a pure APPEND is expressible as TokenAppended.
        match append_suffix(p.streaming.as_deref(), n.streaming.as_deref()) {
            AppendDiff::Same => {}
            AppendDiff::Appended(text) => deltas.push(StateDelta::TokenAppended {
                session_id: n.id.clone(),
                text,
            }),
            // Buffer reset / diverged / cleared (turn boundary) — not a suffix
            // append; a full snapshot keeps the shadow exact.
            AppendDiff::Reset => return DiffResult::full(),
        }

        // Reasoning content: same pure-append rule on the parallel buffer.
        match append_suffix(
            Some(p.stream_reasoning.as_str()),
            Some(n.stream_reasoning.as_str()),
        ) {
            AppendDiff::Same => {}
            AppendDiff::Appended(text) => deltas.push(StateDelta::ReasoningAppended {
                session_id: n.id.clone(),
                text,
            }),
            AppendDiff::Reset => return DiffResult::full(),
        }

        // Working / finished-unseen flags (the sticky marker rides here).
        if p.working != n.working || p.finished_unseen != n.finished_unseen {
            deltas.push(StateDelta::SessionStatusChanged {
                session_id: n.id.clone(),
                working: n.working,
                finished_unseen: n.finished_unseen,
            });
        }
    }

    // --- global status line ---
    if prev.global.status != next.global.status {
        deltas.push(StateDelta::StatusChanged {
            session_id: None,
            text: next.global.status.clone(),
        });
    }

    // --- transcript scroll + follow (global view state) ---
    // A daemon-side scroll (forwarded PageUp/Home/End, or new content re-pinning
    // follow) moves these every-so-often; carry an incremental delta so a controller
    // client's rendered scroll tracks the daemon between full snapshots instead of
    // freezing until the next structural change. Both fields ride together since
    // they move together (a scroll up clears follow; reaching bottom re-sets it).
    if prev.global.scroll != next.global.scroll || prev.global.follow != next.global.follow {
        deltas.push(StateDelta::ScrollChanged {
            scroll: next.global.scroll,
            follow: next.global.follow,
        });
    }

    // NOTE: `global.work_elapsed_ms` is intentionally NOT diffed. It is the comet's
    // clock and ticks up every render while a session works — diffing it would force
    // a delta (or worse, a full snapshot) on EVERY tick. The client re-anchors its
    // own `work_since` clock from each full snapshot and lets it tick locally in
    // between, so the comet stays smooth without per-tick wire traffic (same stance
    // as the toast TTL `Instant`, which is also not carried).

    // --- shared composer (text + caret) ---
    // The composer is NOT append-only (mid-string insert/delete, arrow-key caret
    // moves), so unlike the streaming buffers it can't be expressed as a suffix
    // append — carry the whole input string. A caret-only move (text unchanged,
    // cursor changed) still emits so the rendered caret tracks the daemon. Without
    // this the controller client's composer stays blank while the user types, until
    // the next structural change forces a full snapshot.
    if prev.global.input != next.global.input || prev.global.cursor != next.global.cursor {
        deltas.push(StateDelta::InputChanged {
            text: next.global.input.clone(),
            cursor: next.global.cursor,
        });
    }

    // --- foreground switch (by stable id) ---
    if prev.foreground_id != next.foreground_id {
        if let Some(id) = next.foreground_id.clone() {
            deltas.push(StateDelta::ForegroundChanged { session_id: id });
        } else {
            // Foreground became "none" — unusual (there is always >=1 session
            // today); resync rather than invent a delta the vocabulary lacks.
            return DiffResult::full();
        }
    }

    // --- toast (kind, text) ---
    // A new or changed toast emits a Toast delta. A toast CLEARING has no dedicated
    // delta in this stage's vocabulary; it is purely cosmetic (the client's own TTL
    // would dismiss it anyway), so a clear is intentionally NOT forced to a full
    // resync — favouring cheap per-tick deltas over a snapshot for a fading toast.
    if prev.global.toast != next.global.toast {
        if let Some((kind, text)) = next.global.toast.clone() {
            deltas.push(StateDelta::Toast { kind, text });
        }
    }

    DiffResult {
        needs_full: false,
        deltas,
    }
}

/// Result of comparing an old vs new append-only string buffer.
enum AppendDiff {
    /// Unchanged.
    Same,
    /// `next` is `prev` plus this non-empty suffix.
    Appended(String),
    /// `next` is NOT an extension of `prev` (shrunk, cleared, or diverged) — the
    /// buffer was reset at a turn boundary; the caller must resync.
    Reset,
}

/// Classify `prev` -> `next` for an APPEND-ONLY buffer (the streaming content /
/// reasoning buffers only ever grow within a turn, then reset between turns).
///
/// `None` and `Some("")` are treated alike (both "no buffer"): a stream that goes
/// `None` -> `Some("")` -> `Some("hi")` yields `Same` then `Appended("hi")`, and a
/// commit that drops `Some("...")` -> `None` yields `Reset` so the shadow re-syncs.
fn append_suffix(prev: Option<&str>, next: Option<&str>) -> AppendDiff {
    let p = prev.unwrap_or("");
    let n = next.unwrap_or("");
    if p == n {
        AppendDiff::Same
    } else if let Some(rest) = n.strip_prefix(p) {
        // Pure extension of the previous buffer (covers the empty-prefix start).
        AppendDiff::Appended(rest.to_string())
    } else {
        // Shrunk or diverged: a turn boundary reset the buffer.
        AppendDiff::Reset
    }
}
