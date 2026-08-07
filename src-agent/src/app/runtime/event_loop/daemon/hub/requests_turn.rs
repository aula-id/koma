//! Turn/input-mutation arm bodies for [`super::core::DaemonHub`] — split out of
//! `requests.rs` for file size (pure code motion, no behaviour change). Every
//! method here is called from `requests.rs`'s `handle_controller_mutation` match,
//! one method per moved `ClientRequest` variant, taking exactly the parameters the
//! original arm body used.

use std::sync::Arc;

use crate::app::mode::Mode;
use crate::app::state::AppState;
use crate::controller::input::{handle_key, handle_paste, Action};
use crate::dto::chat::Role;
use crate::ipc::proto::{DaemonEvent, KeyWire};
use crate::service::openrouter::OpenRouterClient;

use crate::app::runtime::actions::apply_action;
use crate::app::runtime::commands::compact::handle_compact;

use super::core::DaemonHub;

impl DaemonHub {
    // Submit composed text to the foreground session — identical to the local
    // Enter-on-composer path (`Action::Submit` carries the text directly).
    pub(super) fn submit_input(
        &mut self,
        idx: usize,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
        handle: &tokio::runtime::Handle,
        text: String,
    ) {
        let result = apply_action(Action::Submit(text), state, client, handle);
        self.ack_or_error(idx, result);
    }

    // Run a `!` shell command in the foreground session's cwd, no model
    // round-trip — the same `Action::Shell` the local composer's leading-`!`
    // detection emits, so the shell-entry-append logic is never forked.
    pub(super) fn shell(
        &mut self,
        idx: usize,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
        handle: &tokio::runtime::Handle,
        cmd: String,
    ) {
        let result = apply_action(Action::Shell(cmd), state, client, handle);
        self.ack_or_error(idx, result);
    }

    // Forward a key to the foreground session through the EXACT local input
    // pipeline: KeyWire -> crossterm KeyEvent -> controller::handle_key ->
    // Action -> apply_action. So the daemon reuses the same per-mode key
    // handling (chat / pickers / forms) as the local TUI.
    pub(super) fn send_key(
        &mut self,
        idx: usize,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
        handle: &tokio::runtime::Handle,
        key: KeyWire,
    ) {
        let action = handle_key(state, key.to_key_event());
        let result = apply_action(action, state, client, handle);
        self.ack_or_error(idx, result);
    }

    // Forward a bracketed PASTE through the EXACT local paste pipeline:
    // `controller::input::handle_paste` routes the text to the active field of
    // the current mode (deepest-modal priority), and — in Chat — runs the
    // image-path detection: a pasted image-file PATH is ingested DAEMON-SIDE
    // into the foreground session's `images/` dir as an `[Image #N]`
    // attachment (the daemon owns the session + its images dir), while
    // ordinary text lands in the composer with CRLF normalisation. The
    // resulting `input` marker, `pending_attachments`, and any toast are
    // projected to the client by the normal snapshot/delta. `handle_paste`
    // mutates `state` directly and is infallible, so this always Acks (mirrors
    // the local loop, which just calls it then redraws — no `apply_action`).
    pub(super) fn paste(&mut self, idx: usize, state: &mut AppState, text: String) {
        handle_paste(state, &text);
        self.send_to(idx, DaemonEvent::Ack);
    }

    // Answer the foreground session's pending tool-approval prompt via the
    // local approve/deny handlers.
    //
    // Server-side UI guarantee: a generic ApproveTool must NOT answer a parked
    // `plan_ready` / `mission_ready` call. Those require PlanDecision (y/a/n).
    // `handle_approve_tool` / `handle_deny_tool` reject that case; the error is
    // surfaced via ack_or_error so the park stays intact.
    pub(super) fn approve_tool(
        &mut self,
        idx: usize,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
        handle: &tokio::runtime::Handle,
        approve: bool,
    ) {
        let action = if approve {
            Action::ApproveTool
        } else {
            Action::DenyTool
        };
        let result = apply_action(action, state, client, handle);
        self.ack_or_error(idx, result);
    }

    // Answer a paused `plan_ready` approval via the local plan handlers.
    // An unrecognised decision maps to `DenyPlan` (fail-safe: keep planning).
    pub(super) fn plan_decision(
        &mut self,
        idx: usize,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
        handle: &tokio::runtime::Handle,
        decision: String,
    ) {
        let action = match decision.as_str() {
            "approve" => Action::ApprovePlan,
            "compact" => Action::ApprovePlanCompact,
            _ => Action::DenyPlan,
        };
        let result = apply_action(action, state, client, handle);
        self.ack_or_error(idx, result);
    }

    // GUI stop button: interrupt the foreground session's in-flight turn via the
    // SAME `Action::Interrupt` the TUI's Esc runs (abort the stream, commit the
    // partial with `[interrupted]`, halt the agentic loop + kill running sub-agents).
    // Unconditional cut: stop must always cut, busy or not (mirrors the TUI Esc's
    // right to interrupt unconditionally) — `handle_interrupt` itself no longer
    // gates on `is_ui_busy()`. Set `force_resync` so the NEXT `stream_deltas` pass
    // (later this same tick) resends every attached client a full `Snapshot`
    // regardless of what the differ concludes — a guaranteed resync for a client
    // whose shadow drifted (e.g. the fixed `Some("")` stuck-streaming case), not
    // dependent on the differ recognizing the change.
    pub(super) fn interrupt(
        &mut self,
        idx: usize,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
        handle: &tokio::runtime::Handle,
    ) {
        let result = apply_action(Action::Interrupt, state, client, handle);
        self.force_resync = true;
        self.ack_or_error(idx, result);
    }

    // GUI Ctrl+R composer parity: resend the last user turn via the SAME
    // `Action::Resend` the TUI's Ctrl+R runs (pop trailing assistant
    // messages + re-stream). `handle_resend` has its own busy/no-session/
    // nothing-to-resend guards and reports a no-op via the status line.
    pub(super) fn resend(
        &mut self,
        idx: usize,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
        handle: &tokio::runtime::Handle,
    ) {
        let result = apply_action(Action::Resend, state, client, handle);
        self.ack_or_error(idx, result);
    }

    // GUI composer queued-list clear button: cancel every pending mid-turn
    // steer via the SAME `Action::CancelSteers` the TUI's Ctrl+X-with-
    // pending-steers runs (clears `pending_steer` + a status line); a
    // no-op when the queue is already empty.
    pub(super) fn cancel_steers(
        &mut self,
        idx: usize,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
        handle: &tokio::runtime::Handle,
    ) {
        let result = apply_action(Action::CancelSteers, state, client, handle);
        self.ack_or_error(idx, result);
    }

    // GUI hover-edit pencil on a USER chat bubble: rewind the foreground
    // session to JUST BEFORE the message at `index` — the non-key equivalent
    // of the TUI's double-Esc `Mode::MessageRewind` + Enter. Reuses the exact
    // `Action::RewindToMessage` core: abort any in-flight turn, truncate the
    // live conversation + sqlite archive to before `index`, and refill the
    // composer with that message's text (projected back via
    // `GlobalSnapshot.input` / the `InputChanged` delta — NOT auto-sent). The
    // core guards a non-user / out-of-range `index` as a clean no-op.
    pub(super) fn rewind_to(
        &mut self,
        idx: usize,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
        handle: &tokio::runtime::Handle,
        index: usize,
    ) {
        // `index` is the GUI's DISPLAY index — the position in the pushed
        // `messages` array, which FILTERS OUT System + Tool rows (render.rs's
        // projection). `Action::RewindToMessage` indexes the RAW
        // `Conversation::messages()` vec (System at [0], Tool interspersed), so
        // the display index must be remapped to its vec position — skipping the
        // SAME System + Tool rows — or it lands on a non-user row and no-ops
        // (no truncation). Resolve the vec index off the foreground conversation.
        let vec_index = state.rest.fg().session.as_ref().and_then(|s| {
            s.conversation
                .messages()
                .iter()
                .enumerate()
                .filter(|(_, m)| !matches!(m.role, Role::System | Role::Tool))
                .nth(index)
                .map(|(vi, _)| vi)
        });
        if let Some(vi) = vec_index {
            let result = apply_action(Action::RewindToMessage(vi), state, client, handle);
            self.ack_or_error(idx, result);
        } else {
            // Out-of-range / no session — nothing to rewind to; ack cleanly.
            self.send_to(idx, DaemonEvent::Ack);
        }
    }

    // GUI composer mode selector: set the GLOBAL agent mode via the SAME
    // `set_agent_mode` choke-point Shift+Tab / `/mode` use (so Plan enter/leave +
    // the plan-boundary system-prompt swap stay correct — never assign `agent_mode`
    // directly). `"yolo"` is gated on `yolo_armed` exactly like `/mode yolo`; an
    // unknown token is a no-op. The mode change re-projects into the snapshot, so
    // every attached client (incl. this GUI) reflects it live.
    pub(super) fn set_mode(&mut self, idx: usize, state: &mut AppState, mode: String) {
        use crate::app::state::AgentMode;
        // Active SDLC blocks hops to Auto/Plan/Normal/Yolo; allow sdlc no-op and exit.
        if state.rest.agent_mode() == AgentMode::Sdlc {
            match mode.as_str() {
                "sdlc" => {
                    self.send_to(idx, DaemonEvent::Ack);
                    return;
                }
                "exit" => {
                    let ret = state.rest.fg().sdlc_return_mode.unwrap_or(AgentMode::Auto);
                    state.rest.set_agent_mode(ret);
                    self.send_to(idx, DaemonEvent::Ack);
                    return;
                }
                "auto" | "normal" | "plan" | "yolo" => {
                    // Refuse hop; ack so client doesn't hang.
                    self.send_to(idx, DaemonEvent::Ack);
                    return;
                }
                _ => {
                    self.send_to(idx, DaemonEvent::Ack);
                    return;
                }
            }
        }
        let target = match mode.as_str() {
            "auto" => Some(AgentMode::Auto),
            "normal" => Some(AgentMode::Normal),
            "plan" => Some(AgentMode::Plan),
            "sdlc" => Some(AgentMode::Sdlc),
            // Layer-2 gate: an ARMED YOLO only; unarmed → leave the mode untouched.
            "yolo" if state.rest.yolo_armed => Some(AgentMode::Yolo),
            _ => None,
        };
        if let Some(m) = target {
            state.rest.set_agent_mode(m);
        }
        self.send_to(idx, DaemonEvent::Ack);
    }

    // GUI bash-row kill: terminate the foreground session's bg-bash job by id via
    // the SAME `Action::BashKillJob` the `/bash` panel's Ctrl+X runs (SIGTERM +
    // flip status→Killed). A no-op when the id is already gone.
    pub(super) fn bash_kill(
        &mut self,
        idx: usize,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
        handle: &tokio::runtime::Handle,
        id: usize,
    ) {
        let result = apply_action(Action::BashKillJob(id), state, client, handle);
        self.ack_or_error(idx, result);
    }

    // GUI agent-row kill: kill ONE sub-agent of the foreground session by id,
    // mirroring the model-callable `task_kill` primitive — abort the tokio task +
    // flip a still-Running status to Killed (a terminal status is left untouched).
    // No pre-existing Action kills a sub-agent BY ID (the TUI's Ctrl+X targets by
    // selection index), so this resolves + mutates inline. A no-op when the id is
    // absent.
    pub(super) fn kill_subagent(&mut self, idx: usize, state: &mut AppState, id: usize) {
        use crate::app::subagent::SubAgentStatus;
        if let Some(sa) = state
            .rest
            .fg_mut()
            .subagents
            .iter_mut()
            .find(|s| s.id == id)
        {
            sa.abort.abort();
            if matches!(sa.status, SubAgentStatus::Running) {
                sa.status = SubAgentStatus::Killed;
            }
        }
        self.send_to(idx, DaemonEvent::Ack);
    }

    // GUI agent-row background button: flip ONE running sub-agent to detached via
    // the SAME `Action::BackgroundSubagent` the TUI's Ctrl+B-on-selection runs.
    // `handle_background_subagent` re-checks eligibility itself (Running, not
    // already detached, has a `tool_call_id`) — a stale/ineligible id is a no-op.
    pub(super) fn background_subagent(
        &mut self,
        idx: usize,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
        handle: &tokio::runtime::Handle,
        id: usize,
    ) {
        let result = apply_action(Action::BackgroundSubagent(id), state, client, handle);
        self.ack_or_error(idx, result);
    }

    // GUI global Ctrl+B: background EVERY eligible sub-agent via the SAME
    // `Action::BackgroundAllSubagents` the TUI's composer Ctrl+B runs.
    // `handle_background_all_subagents` is a no-op when nothing is eligible.
    pub(super) fn background_all_subagents(
        &mut self,
        idx: usize,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
        handle: &tokio::runtime::Handle,
    ) {
        let result = apply_action(Action::BackgroundAllSubagents, state, client, handle);
        self.ack_or_error(idx, result);
    }

    // The client reports the on-screen editor wrap width so the daemon's
    // TextEditorState can navigate soft-wrapped rows with the same visual
    // width the client renders. Only meaningful when the daemon is in the
    // agents full-screen editor; a no-op Ack otherwise.
    pub(super) fn editor_wrap_w(&mut self, idx: usize, state: &mut AppState, n: usize) {
        if let Mode::Agents(ref a) = state.mode() {
            if let Some((_, ref ed)) = a.editor {
                ed.wrap_w.set(n);
            }
        }
        self.send_to(idx, DaemonEvent::Ack);
    }

    // GUI status-footer Compact action: summarise + trim the foreground
    // session's history via the SAME `handle_compact` entry point the TUI's
    // `/compact` command calls (`preserve_n_override: None` — use the
    // session's configured `compaction.preserve_n`). Busy / no-session is a
    // no-op reported via the session's `status` line, exactly like `/compact`;
    // any real error surfaces as `DaemonEvent::Error`.
    pub(super) fn compact(
        &mut self,
        idx: usize,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
        handle: &tokio::runtime::Handle,
    ) {
        let result = handle_compact(state, client, handle, None);
        self.ack_or_error(idx, result);
    }
}
