//! Session-lifecycle methods (sub-agent teardown, tombstone-close, turn-halt
//! interrupt, busy predicates, effective cwd), split out of
//! [`super::SessionRuntime`] for file size. Same `impl SessionRuntime` type as
//! `mod.rs`; every method here was already `pub fn`, so this is a pure lift —
//! no visibility bumps, no behaviour change.

use std::path::PathBuf;

use super::SessionRuntime;

impl SessionRuntime {
    /// Kill running sub-agents that belong to THIS session, drop model-delegated
    /// queued sub-agents, but PRESERVE user-initiated /task jobs
    /// (tool_call_id == None).
    ///
    /// `include_detached` controls whether detached background sub-agents
    /// (`sub.detached == true`) are killed along with blocking ones:
    ///
    /// - `false` (turn-halt / Esc / deny): SKIP detached agents — they are
    ///   background jobs the user explicitly asked to run independently, and they
    ///   must survive an Esc/interrupt just as bg-bash jobs do (bg-bash: `bash_jobs`
    ///   are never touched by `interrupt()`, only dropped with the session on
    ///   `close()`). Only blocking (non-detached) Running sub-agents are killed.
    ///   `pending_subagent_nudges` is left INTACT because that buffer is exclusively
    ///   fed by detached agents; clearing it would drop a completion notification
    ///   that arrived just before the interrupt.
    ///
    /// - `true` (session close / tombstone): kill ALL Running sub-agents including
    ///   detached ones — the session is going away entirely, nothing survives.
    ///   `pending_subagent_nudges` is cleared (the session can no longer fire them).
    ///
    /// - Running (non-detached) sub-agents: `abort.abort()` kills the tokio task;
    ///   status is flipped to `Killed` immediately so the `$` panel reflects it
    ///   without waiting for a terminal event that will never arrive.
    /// - Model-delegated queued sub-agents (tool_call_id == Some): dropped to
    ///   halt the interrupted turn's work (always, regardless of `include_detached`).
    /// - User-initiated /task entries (tool_call_id == None): retained so the
    ///   user's independent pending commands survive the turn halt.
    /// - `pending_subagent_calls` / `awaiting_subagents`: cleared here so the
    ///   caller does NOT need to do it separately (keeps the halt paths consistent).
    ///
    /// This method ONLY touches the session it is called on — it is always
    /// invoked via `state.rest.fg_mut()` (or a named session slot), so other
    /// sessions are not affected.
    pub fn abort_running_subagents(&mut self, include_detached: bool) {
        for sub in &mut self.subagents {
            if matches!(sub.status, crate::app::subagent::SubAgentStatus::Running) {
                // When NOT including detached agents, skip detached ones so they
                // keep running independently (mirrors bg-bash surviving Esc).
                if !include_detached && sub.detached {
                    continue;
                }
                sub.abort.abort();
                sub.status = crate::app::subagent::SubAgentStatus::Killed;
                // Suppress the detached-completion nudge for an agent killed here:
                // the user stopped this turn (or the session closed), so the session
                // must NOT auto-wake to announce "your background agent finished".
                // Latching `nudged` stops the next-tick terminal-fold in
                // `drain_subagents` from buffering a nudge for it.
                sub.nudged = true;
            }
        }
        self.pending_subagents.retain(|p| p.tool_call_id.is_none());
        self.pending_subagent_calls.clear();
        self.awaiting_subagents = false;
        // `pending_subagent_nudges` is exclusively fed by DETACHED agents. When
        // `include_detached == false` (turn-halt/Esc), preserved detached agents
        // may have already buffered a nudge that arrived just before the interrupt —
        // keep it so the session auto-wakes to announce completion when next idle.
        // When `include_detached == true` (session close), clear it: the session is
        // going away and can no longer fire the nudge.
        if include_detached {
            self.pending_subagent_nudges.clear();
        }
    }

    /// TOMBSTONE this session (daemon stage 10): tear down ALL of its in-flight work
    /// and latch [`closed`](Self::closed) so the per-session servicer skips it from
    /// now on, WITHOUT removing it from the sessions Vec (a `Vec::remove` would shift
    /// every later index and cross-wire index-routed async — see `ipc::proto`
    /// critique #2). After this returns the slot is inert: `is_working()` is false,
    /// no receiver is live, no lock is held.
    ///
    /// Steps (a superset of `abort_current` + `abort_running_subagents`, applied to
    /// THIS session rather than the foreground):
    /// - abort the in-flight stream task + drop its receiver (late events vanish),
    /// - drop the advisory prompt-classifier channel,
    /// - abort every running sub-agent + drop queued model delegations,
    /// - clear `waiting` and the parked-lane flags so nothing reads as busy,
    /// - RELEASE this session's on-disk `session.lock` (so a closed session frees its
    ///   lock immediately, not only at daemon teardown — another process may reopen
    ///   it); the path is unlinked here and dropped from `held_lock`.
    ///
    /// Idempotent: closing an already-closed session is a harmless no-op (everything
    /// is already torn down). Does NOT touch foreground — the caller repoints
    /// foreground off a tombstone (only it knows the session set).
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        // In-flight stream task: abort + drop the receiver so late events vanish.
        if let Some(h) = self.current_task.take() {
            h.abort();
        }
        self.active_rx = None;
        self.harness_rx = None;
        self.waiting = false;
        self.awaiting_approval = false;
        self.approval_reason = None;
        // Drop any detached-park timer (daemon stage 11) — a tombstone is never
        // awaiting, and the loop only inspects non-closed sessions, so this just
        // keeps the inert slot fully clean.
        self.park_started_at = None;
        self.awaiting_tool_tasks = false;
        // Drop any TAC-classify park too (a tombstone is never awaiting); a late
        // verdict to this inert slot is discarded because the servicer skips a
        // closed session's drain.
        self.awaiting_classify = false;
        self.pending_classify_verdict = None;
        // A `!` shell may be draining off-thread; clear the park flag so a late
        // delivery to this tombstone is discarded by the gated drain (the OS child
        // finishes on its own — we never block close() on it).
        self.awaiting_shell = false;
        // Sub-agents: kill running, drop model-delegated queued work, clear the
        // parked-delegation bookkeeping. (Unlike a turn-halt, a CLOSE also drops
        // user-initiated /task entries — the session is going away entirely.)
        // include_detached = true: session is gone, all background agents die with it.
        self.abort_running_subagents(true);
        self.pending_subagents.clear();
        // Release this session's on-disk lock right away (unlink + forget the path).
        if let Some(path) = self.held_lock.take() {
            crate::model::store::remove_lock(&path);
        }
        self.closed = true;
    }

    /// Stop this session's in-flight turn WITHOUT tombstoning it: abort the
    /// stream task + sub-agents, drop all parked agentic state, and commit any
    /// partial assistant buffer with an `[interrupted]` marker. Idempotent and
    /// safe on an idle session (nothing in flight → no-op commit). Used both by
    /// the foreground Esc-interrupt and by the session hub's Ctrl+X "stop".
    ///
    /// This is the per-session half of the old `handle_interrupt`: every step here
    /// operated on `fg_mut()` before, so it works on ANY session now. The partial
    /// buffer is committed to THIS session's own `session` (path/conversation/log),
    /// and only THIS session's counters are touched. The rest-GLOBAL compaction
    /// cleanup + status line stay with the caller (`actions::chat::handle_interrupt`).
    pub fn interrupt(&mut self) {
        // Abort the in-flight stream task + stop listening to it (the per-session
        // part of `abort_current`): abort the handle, drop the active receiver so
        // any late events from the aborted task vanish, and clear `waiting`.
        if let Some(h) = self.current_task.take() {
            h.abort();
        }
        self.active_rx = None;
        self.waiting = false;
        // Halt the agentic loop: drop any stashed tool calls, reset the step
        // counter, and clear the approval machine so a halt mid-approval doesn't
        // leave the turn wedged.
        self.pending_tool_calls.clear();
        self.agent_steps = 0;
        self.awaiting_approval = false;
        self.approval_reason = None;
        self.tool_idx = 0;
        self.tool_results.clear();
        // Kill every BLOCKING running sub-agent spawned by this turn and drop the
        // pending queue. `abort_running_subagents` also clears
        // `pending_subagent_calls` and `awaiting_subagents`, so the halt path is
        // complete. Detached (background) sub-agents are PRESERVED — they are the
        // user's independent background jobs and survive Esc exactly as bg-bash
        // jobs do (bash_jobs is never touched by interrupt()).
        // include_detached = false: Esc/turn-halt, preserve background agents.
        self.abort_running_subagents(false);
        // Abandon any round parked on a deferred tool task. The off-thread worker
        // keeps running but its result lands with no matching pending id, so the
        // next-turn machine reset discards it; it can't resume a turn that was
        // killed. The channel itself is left intact for reuse by later deferred
        // tools. We deliberately do NOT join the worker here.
        self.pending_tool_tasks.clear();
        self.awaiting_tool_tasks = false;
        // Abandon any round parked on the TAC classifier the same way: the
        // off-thread classify task keeps running but its verdict lands with no
        // matching parked id (pending_tool_calls is cleared above), so the drain
        // drops it. Channel ends are left intact for reuse (mirrors the tool-task
        // lane); a stale staged verdict is cleared so it can't be mis-consumed.
        self.awaiting_classify = false;
        self.pending_classify_verdict = None;
        // Take any captured usage unconditionally so a partial turn's usage can't
        // leak into the next response.
        let usage = self.pending_usage.take();
        // Likewise drain the reasoning buffer unconditionally so a half-streamed
        // thinking block can't bleed into the next turn; it's folded onto the
        // interrupted message (display-only).
        let reasoning = self.take_reasoning();
        self.stream_reasoning_details.clear();
        let buf = self.take_stream();
        if let Some(b) = buf {
            if !b.is_empty() {
                let mut committed = false;
                if let Some(sess) = self.session.as_mut() {
                    // Decode any echoed-back escaped reasoning tag BEFORE persisting,
                    // so both the msglog append and push_assistant store the REAL
                    // `<think>` (mirrors turn.rs's tool-call-turn decode; this path
                    // bypasses `final_answer` too).
                    let b = crate::dto::chat::unescape_reasoning_tags(&b).into_owned();
                    let content = format!("{b}  [interrupted]");
                    let _ = crate::model::msglog::append(
                        &sess.path,
                        crate::dto::chat::Role::Assistant,
                        &content,
                        usage,
                    );
                    sess.conversation.push_assistant(content, reasoning);
                    let _ = sess.save();
                    committed = true;
                }
                // Update THIS session's own counters once the `sess` borrow above
                // has ended (mirrors the foreground-interrupt accounting).
                if committed {
                    if let Some((pt, ct, cost)) = usage {
                        self.tokens_in = pt; // current context size, not a sum
                        self.tokens_out += ct;
                        self.cost += cost;
                    }
                }
            }
        }
    }

    /// True when this session has work in flight: a turn waiting / streaming, a
    /// paused approval, a parked deferred lane (tool tasks or sub-agent
    /// delegations), or any still-running sub-agent. Used by the session hub's
    /// cooking pane to flag busy sessions, by the foreground status line, and by
    /// the background-finish nudge.
    ///
    /// A CLOSED (tombstoned) session is NEVER working: `close()` already tore down
    /// every lane, but short-circuit here so a stray flag can't keep a tombstone
    /// reading as busy (the self-exit grace timer treats `!is_working()` as quiesced).
    pub fn is_working(&self) -> bool {
        if self.closed {
            return false;
        }
        self.waiting
            || self.streaming.is_some()
            || self.awaiting_approval
            || self.awaiting_tool_tasks
            || self.awaiting_classify
            || self.awaiting_shell
            || self.awaiting_subagents
            || self
                .subagents
                .iter()
                .any(|s| matches!(s.status, crate::app::subagent::SubAgentStatus::Running))
    }

    /// UI-activity twin of [`is_working`](Self::is_working): identical busy-flag set,
    /// except a DETACHED (backgrounded) sub-agent does NOT count. A detached agent
    /// runs off to the side while the main turn is idle, so the UI must read 'ready'.
    ///
    /// The split exists because `is_working` serves two masters that need different
    /// answers for detached agents:
    /// - LIVENESS (daemon quiescence / self-exit grace / hub-kill abort-vs-close /
    ///   quit warning / completion-nudge gating) — a detached agent MUST count, else
    ///   the daemon could self-exit and kill the running background agent. That is
    ///   `is_working`; leave those callers on it.
    /// - UI ACTIVITY (projected `working` → client `waiting` → comet shimmer + fast
    ///   redraw cadence, the session-hub ●/○ marker, the foreground status line) — a
    ///   detached agent must NOT count. Those callers read this.
    pub fn is_ui_busy(&self) -> bool {
        if self.closed {
            return false;
        }
        self.waiting
            || self.streaming.is_some()
            || self.awaiting_approval
            || self.awaiting_tool_tasks
            || self.awaiting_classify
            || self.awaiting_shell
            || self.awaiting_subagents
            || self.subagents.iter().any(|s| {
                matches!(s.status, crate::app::subagent::SubAgentStatus::Running) && !s.detached
            })
    }

    /// True once this session has been tombstoned via [`close()`](Self::close) —
    /// its slot stays in `sessions` (so no index shifts) but it is inert. Read by
    /// the session-hub cooking builder (a closed session must not reappear) and by
    /// the kill handler's foreground reassignment (never repoint onto a tombstone).
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// This session's EFFECTIVE working directory: the live `cd` override
    /// ([`active_cwd`](Self::active_cwd)) when set, else the session's configured
    /// workdir (`Session::workdir()` — the first `settings.workdir` entry), else
    /// the process cwd when there is no session at all.
    ///
    /// The single source of truth for "where this session is right now". Read by
    /// `build_tool_ctx` (→ `ToolCtx::workspace`, so `bash` + the dir cache follow
    /// `cd`), by the harness workspace check (so a `cd` outside every allowed root
    /// blocks the next MODEL tool turn), and by the IPC snapshot. The configured
    /// allow-list / `[N]` roots in `Session::workdirs()` are deliberately NOT
    /// affected — cd moves only the cwd.
    pub fn effective_cwd(&self) -> PathBuf {
        if let Some(cwd) = self.active_cwd.as_ref() {
            return cwd.clone();
        }
        self.session
            .as_ref()
            .map(|s| s.workdir())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }
}
