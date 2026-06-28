//! Turn lifecycle: finish a stream, advance through tool rounds.

use std::sync::Arc;

use crate::app::state::{AppState, AppStateRest};
use crate::dto::chat::Role;
use crate::service::openrouter::OpenRouterClient;

use super::final_answer;

/// Post koma's friendly "this model can't read images" notice into the chat
/// (assistant message + msglog + save). Shared by the submit-time capability
/// guard and the stream-error interception so the wording lives in one place.
pub(crate) fn push_image_unsupported_notice(rest: &mut AppStateRest) {
    let notice = "Sorry, I can't see images on this model. Switch to a vision-capable model, or send your message without the image.".to_string();
    if let Some(sess) = rest.fg_mut().session.as_mut() {
        let _ = crate::model::msglog::append(&sess.path, Role::Assistant, &notice, None);
        sess.conversation.push_assistant(notice, None);
        let _ = sess.save();
    }
}

/// True when a provider error indicates the model/endpoint cannot accept image
/// input, so we can show the friendly notice instead of a raw error toast.
///
/// Only matches definitive rejection patterns (HTTP 400 — permanent, model-
/// level incapability). Transient provider failures (502/503) on vision-
/// capable models — often containing "multimodal" — are NOT matched so the
/// actual error surfaces and the user can retry.
fn is_image_input_error(e: &str) -> bool {
    let e = e.to_lowercase();
    // Definitive model-capability rejections (HTTP 400 — permanent).
    e.contains("image input")
        || e.contains("support image")
        || (e.contains("no endpoints") && e.contains("image"))
        || (e.contains("does not support") && e.contains("image"))
        || (e.contains("cannot") && e.contains("image") && e.contains("input"))
        // OpenRouter typed image errors (HTTP 400 — bad image data, not bad model).
        || e.contains("invalid_image")
        || e.contains("unsupported_image_format")
        || e.contains("image_too_large")
        || e.contains("image_too_small")
        || e.contains("image_not_found")
        || e.contains("image_download_failed")
}

/// Finalize a finished stream: commit any buffered assistant text, clear the
/// waiting flag + task handle, set the status line. `error` is Some on stream
/// failure; a save error is surfaced only if the stream itself succeeded.
pub(crate) fn finish_stream(rest: &mut AppStateRest, sess_idx: usize, error: Option<String>) {
    // Bind session `sess_idx`'s runtime once: the per-session fields (session,
    // streaming buffers, waiting, task handle, and now the cumulative token/cost
    // totals) all live here, while `config` stays on `rest` as a disjoint field.
    // Borrowing `rest.sessions[sess_idx]` directly (not via `fg_mut()`, a `&mut
    // self` method that would lock all of `rest`) keeps those disjoint borrows legal.
    let rt = &mut rest.sessions[sess_idx];
    // Take the in-flight usage unconditionally so it can never leak into the
    // next turn, even when the buffer is empty or there's no session to commit.
    let usage = rt.pending_usage.take();
    // Reasoning taken unconditionally so it can't leak; may be promoted to
    // content below when the model streamed its entire answer through that channel.
    let reasoning = rt.take_reasoning();
    let buf = rt.take_stream().unwrap_or_default();
    let (content, msg_reasoning) = final_answer(buf, reasoning);
    let mut save_err = None;
    if !content.is_empty() {
        let mut committed = false;
        if let Some(sess) = rt.session.as_mut() {
            let _ = crate::model::msglog::append(
                &sess.path,
                crate::dto::chat::Role::Assistant,
                &content,
                usage,
            );
            sess.conversation.push_assistant(content, msg_reasoning);
            if let Err(e) = sess.save() {
                save_err = Some(e.to_string());
            }
            committed = true;
        }
        // tokens_in = current context size (latest prompt), not cumulative.
        // tokens_out and cost are cumulative (each turn adds new spend). Written
        // to THIS session's own counters (the `sess` borrow above has ended).
        if committed {
            if let Some((pt, ct, cost)) = usage {
                rt.tokens_in = pt;        // current context size, not a sum
                rt.tokens_out += ct;
                rt.cost += cost;
            }
        }
        // Record into the global usage ledger (best-effort telemetry, non-fatal).
        if let Some((pt, ct, cost)) = usage {
            if let Some(sess) = rt.session.as_ref() {
                let model_id = crate::app::resolve::resolve_role(
                    &rest.config,
                    &sess.settings,
                    crate::model::app_config::ModelRole::Main,
                )
                .map(|r| r.model_id)
                .unwrap_or_default();
                crate::model::usage::record_usage(
                    &model_id,
                    "main",
                    &sess.id,
                    &sess.pwd_hash,
                    pt,
                    rt.tokens_cached,
                    ct,
                    cost,
                );
            }
        }
    }
    rt.waiting = false;
    rt.current_task = None;
    match error.or(save_err) {
        Some(e) => {
            // If the provider rejected the request because the model can't take
            // images (e.g. "No endpoints found that support image input") and the
            // last user message actually carried image attachments, swap the raw
            // error toast for koma's friendly in-chat notice.
            let last_user_had_image = rest.sessions[sess_idx].session.as_ref().is_some_and(|s| {
                s.conversation
                    .history()
                    .iter()
                    .rev()
                    .find(|m| m.role == Role::User)
                    .is_some_and(|m| !m.attachments.is_empty())
            });
            if last_user_had_image && is_image_input_error(&e) {
                push_image_unsupported_notice(rest);
                rest.status = "ready".into();
            } else {
                rest.set_toast(e.clone());
                rest.status = format!("error: {e}");
            }
        }
        None => rest.status = "ready".into(),
    }
}

/// Advance a turn after a stream finished cleanly (`StreamEvent::Done`).
///
/// A single user turn may span several model calls when the model requests
/// tools. This commits the just-finished assistant message, then EITHER:
/// - ends the turn (no tool calls → the model gave its final answer), or
/// - runs the requested tools, appends their results, and starts the next
///   model call to continue the turn (`waiting` stays true throughout).
///
/// Mirrors the usage/counter bookkeeping of [`finish_stream`]: `tokens_in` is
/// the latest prompt size (current context), `tokens_out` / `cost` accumulate.
pub(crate) fn advance_turn(
    state: &mut AppState,
    sess_idx: usize,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) {
    // 1. Take the stashed tool calls + the streamed text + the in-flight usage
    //    out of state up front so nothing leaks into the next model call.
    let mut pending = state.rest.sessions[sess_idx].pending_tool_calls.clone();
    let mut buf = state.rest.sessions[sess_idx].take_stream();
    let usage = state.rest.sessions[sess_idx].pending_usage.take();
    // Display-only reasoning streamed this round. Taken unconditionally (so it
    // can never leak into the next round) and folded onto the committed message
    // below; never logged to disk or sent to the API.
    let reasoning = state.rest.sessions[sess_idx].take_reasoning();

    // 1b. Text-format tool-call fallback. Some models (Hermes/Qwen/ChatML on
    //     budget / gpt-oss / GLM routes) emit a tool call as `<tool_call>…JSON…
    //     </tool_call>` TEXT inside content instead of via the native
    //     `tool_calls` field. When the native path produced NO pending calls but
    //     the model did stream text, try to harvest such calls and feed them
    //     through the IDENTICAL path as native ones: the cleaned content (markup
    //     stripped) becomes the committed/persisted/displayed message, the
    //     synthesized calls become `pending`, AND they are written back into
    //     `state.rest.pending_tool_calls` so any other reader of rest state sees
    //     them too. Zero behaviour change when native calls already exist or when
    //     no parseable block is present (cleaned == original, pending stays empty).
    if pending.is_empty() {
        if let Some(text) = buf.as_deref() {
            if !text.is_empty() {
                let (cleaned, synthesized) =
                    crate::dto::chat::extract_text_tool_calls(text);
                if !synthesized.is_empty() {
                    buf = Some(cleaned);
                    pending = synthesized.clone();
                    state.rest.sessions[sess_idx].pending_tool_calls = synthesized;
                }
            }
        }
    }

    // 2. Commit the assistant message (and log + count it). The assistant text
    //    may be empty on a tool-call turn — we still record the row so usage
    //    accounting stays correct across rounds.
    let mut save_err = None;
    {
        // Bind session `sess_idx`'s runtime directly (not via `fg_mut()`, a
        // `&mut self` method that would lock all of `rest`) so the per-session
        // `session` and this session's own `tokens_*` totals stay independently
        // borrowable; `state.rest.config` remains a disjoint field of `rest`.
        let rt = &mut state.rest.sessions[sess_idx];
        let mut committed = false;
        if let Some(sess) = rt.session.as_mut() {
            if !pending.is_empty() {
                let content = buf.clone().unwrap_or_default();
                let _ = crate::model::msglog::append(&sess.path, Role::Assistant, &content, usage);
                sess.conversation
                    .push_assistant_with_tools(content, pending.clone(), reasoning);
                if let Err(e) = sess.save() {
                    save_err = Some(e.to_string());
                }
            } else {
                let (content, msg_reasoning) =
                    final_answer(buf.clone().unwrap_or_default(), reasoning);
                if !content.is_empty() {
                    let _ = crate::model::msglog::append(&sess.path, Role::Assistant, &content, usage);
                    sess.conversation.push_assistant(content, msg_reasoning);
                    if let Err(e) = sess.save() {
                        save_err = Some(e.to_string());
                    }
                }
            }
            committed = true;
        }
        // Counter update on THIS session's own totals, after the `sess` borrow
        // above ends so the disjoint-field borrows don't overlap.
        if committed {
            if let Some((pt, ct, cost)) = usage {
                rt.tokens_in = pt; // current context size, not a sum
                rt.tokens_out += ct;
                rt.cost += cost;
            }
        }
        // Record into the global usage ledger (best-effort telemetry, non-fatal).
        if let Some((pt, ct, cost)) = usage {
            if let Some(sess) = rt.session.as_ref() {
                let model_id = crate::app::resolve::resolve_role(
                    &state.rest.config,
                    &sess.settings,
                    crate::model::app_config::ModelRole::Main,
                )
                .map(|r| r.model_id)
                .unwrap_or_default();
                crate::model::usage::record_usage(
                    &model_id,
                    "main",
                    &sess.id,
                    &sess.pwd_hash,
                    pt,
                    rt.tokens_cached,
                    ct,
                    cost,
                );
            }
        }
    }

    // 3. No tool calls → the model produced its final answer; the turn is done.
    if pending.is_empty() {
        state.rest.sessions[sess_idx].waiting = false;
        state.rest.sessions[sess_idx].current_task = None;
        state.rest.sessions[sess_idx].agent_steps = 0;
        state.rest.status = match save_err {
            Some(e) => {
                state.rest.set_toast(e.clone());
                format!("error: {e}")
            }
            None => "ready".into(),
        };
        return;
    }

    state.rest.sessions[sess_idx].agent_steps += 1;

    // 4b. Workspace check (WC): the deterministic harness gate. When the harness
    //     is enabled and the session cwd is NOT within an allowed folder (at or
    //     under the launch dir or an allow-list root), refuse to run ANY tool this
    //     turn. Every pending call is answered with a refusal (so the conversation
    //     stays API-valid — no dangling tool_call ids) and the turn is stopped.
    //     When the harness is disabled this is skipped entirely (zero behaviour
    //     change). The check runs once per round, before the plan gate / tools.
    // Check the session's EFFECTIVE cwd (the live `cd` override when set, else the
    // configured workdir) — NOT the raw configured workdir — so that a `/cd` into a
    // SUBDIR of an allowed root stays allowed (containment, not equality), while a
    // `/cd` outside every allowed root makes this turn WC-denied (Phase 8).
    let effective_cwd = state.rest.sessions[sess_idx].effective_cwd();
    let wc_blocked = state.rest.sessions[sess_idx]
        .session
        .as_ref()
        .is_some_and(|sess| {
            sess.settings.classifier_enabled
                && !crate::app::harness::workspace_allowed(
                    &sess.settings,
                    &effective_cwd,
                    &state.rest.launch_dir,
                )
        });
    if wc_blocked {
        super::tools::deny_all_pending(state, sess_idx, "workspace not in allowed folders");
        state.rest.set_toast("workspace not in allowed folders".into());
        state.rest.status = "stopped: workspace not allowed".into();
        return;
    }

    // 5b. Hand off to the tool-approval state machine. The pending calls were
    //     already stashed into `state.rest.pending_tool_calls` by the event loop
    //     (`StreamEvent::ToolCalls`); `process_tools` walks them from index 0,
    //     running safe calls inline and — in Normal mode — pausing on the first
    //     risky one for a `y/n`. `pending` (the local copy) is no longer needed.
    drop(pending);
    state.rest.sessions[sess_idx].tool_idx = 0;
    state.rest.sessions[sess_idx].tool_results.clear();
    super::tools::process_tools(state, sess_idx, client, handle);
}
