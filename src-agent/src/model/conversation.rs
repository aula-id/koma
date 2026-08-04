//! In-memory chat history and conversation compaction.
//!
//! `Conversation` is a thin wrapper around `Vec<ChatMessage>` that enforces
//! the invariant that `messages[0]` is always a `Role::System` message (once
//! `set_system` or `rebuild_system` has been called). All other messages are
//! user/assistant turns in chronological order.
//!
//! **Compaction** shrinks the history when it grows too large. The flow is:
//! 1. The controller calls `split_for_compaction(preserve_n)` to carve the
//!    history into two parts: an older slice to summarise and a recent tail to
//!    keep verbatim.
//! 2. The older slice is sent to the model for summarisation.
//! 3. The controller calls `apply_compaction(summary, kept_tail)` to rebuild
//!    the conversation as `[system, Assistant(summary), kept_tail…]`.
//!
//! Data flow in the broader app:
//! ```
//! keystroke -> Action -> state mutation (push_user / push_assistant)
//!          -> render (Conversation::messages())
//!          -> Session::save() -> messages.json
//! ```

use crate::dto::chat::{Attachment, ChatMessage, Role, ToolCall};

/// In-memory chat history for one session.
///
/// The first element of the internal vec is always a `System` message after
/// `set_system` (or `rebuild_system`) has been called. Pushing user/assistant
/// messages always appends to the end.
pub struct Conversation {
    messages: Vec<ChatMessage>,
}

impl Conversation {
    /// Start a fresh conversation with an initial system prompt.
    #[allow(dead_code)]
    pub fn new(system_prompt: impl Into<String>) -> Self {
        Self {
            messages: vec![ChatMessage::new(Role::System, system_prompt)],
        }
    }

    /// Wrap an existing vec verbatim (used on resume from disk). May be empty;
    /// the caller (`Session::load`) calls `rebuild_system()` immediately after,
    /// which seeds the system message via `set_system`.
    pub fn from_messages(messages: Vec<ChatMessage>) -> Self {
        Self { messages }
    }

    /// Append a user turn.
    pub fn push_user(&mut self, content: impl Into<String>) {
        self.messages.push(ChatMessage::new(Role::User, content));
    }

    /// Append a user turn carrying image attachments (path-paste / `@`-picker).
    /// `attachments` may be empty, in which case this is identical to
    /// [`Self::push_user`] (the field serialises away entirely). Each attachment
    /// links to an on-disk image under `<session>/images/` and matches an
    /// `[Image #N]` marker in `content`.
    pub fn push_user_with_attachments(
        &mut self,
        content: impl Into<String>,
        attachments: Vec<Attachment>,
    ) {
        self.messages
            .push(ChatMessage::new(Role::User, content).with_attachments(attachments));
    }

    /// Append an assistant turn (used for both streamed and non-streamed replies).
    ///
    /// `reasoning` is the display-only thinking block streamed for this turn
    /// (`None` when the model didn't think). It is attached BEFORE the message
    /// enters the list so the transcript cache captures it on first render. It
    /// is persisted to `messages.json` (for session resume / transcript rehydrate)
    /// but never sent on the wire (`to_wire` builds request bodies from fields
    /// and omits this).
    pub fn push_assistant(
        &mut self,
        content: impl Into<String>,
        reasoning: Option<String>,
        promoted: bool,
    ) {
        self.messages.push(
            ChatMessage::new(Role::Assistant, content)
                .with_reasoning(reasoning)
                .with_reasoning_promoted(promoted),
        );
    }

    /// Append an assistant turn that requested tool calls. `content` is the
    /// assistant text accompanying the calls (often empty). `reasoning` is the
    /// display-only thinking block (the model may think before emitting tool
    /// calls); attached before the push and persisted to `messages.json` for
    /// session resume — see [`Self::push_assistant`]. `reasoning_details` is the
    /// structured OpenRouter chain-of-thought (typed + signed) for this turn,
    /// echoed back on the tool-continuation request so the model keeps its
    /// reasoning across tool calls; never persisted to disk.
    pub fn push_assistant_with_tools(
        &mut self,
        content: String,
        tool_calls: Vec<ToolCall>,
        reasoning: Option<String>,
        reasoning_details: Option<Vec<crate::dto::chat::ReasoningDetail>>,
    ) {
        self.messages.push(
            ChatMessage::assistant_with_tools(content, tool_calls)
                .with_reasoning(reasoning)
                .with_reasoning_details(reasoning_details),
        );
    }

    /// Append a `tool`-role result message answering `tool_call_id`.
    pub fn push_tool(&mut self, tool_call_id: String, content: String) {
        self.messages
            .push(ChatMessage::tool_result(tool_call_id, content));
    }

    /// Overwrite the stored `arguments` JSON of the tool call identified by
    /// `call_id`, searching assistant turns from the tail (most recent first).
    /// Used by the `plan_ready` interception to swap the model's raw `highlights`
    /// for the composed user-facing plan digest, so the transcript renders the
    /// digest with no view-layer changes. No-op if the id isn't found.
    pub fn set_tool_call_args(&mut self, call_id: &str, arguments: String) {
        for msg in self.messages.iter_mut().rev() {
            let Some(tcs) = msg.tool_calls.as_mut() else {
                continue;
            };
            if let Some(tc) = tcs.iter_mut().find(|c| c.id == call_id) {
                tc.function.arguments = arguments;
                return;
            }
        }
    }

    /// Borrow the full message list (system + turns). Passed directly to the
    /// wire-format `ChatRequest` without copying.
    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    /// Clone the message list, sanitized for API consumption.
    ///
    /// OpenRouter (and the OpenAI-compatible API spec) require that every
    /// assistant message containing `tool_calls` is immediately followed by a
    /// `Tool` message for **each** call id in that group. If the agentic loop
    /// is interrupted mid-turn the stored history can violate this invariant,
    /// causing a 400 error on the next request.
    ///
    /// This method strips dangling tool-call groups before returning, so the
    /// caller can always forward the result to the API safely. The raw
    /// `messages()` slice is left untouched (used by the TUI for display).
    pub fn history(&self) -> Vec<ChatMessage> {
        use std::collections::HashSet;
        let msgs = &self.messages;

        // Pass 1: collect ids of tool_calls that are fully answered.
        // An assistant tool-call group is fully answered only when EVERY one
        // of its ids has a corresponding Tool message in the contiguous run
        // of Tool messages immediately following it.
        let mut valid_ids: HashSet<String> = HashSet::new();
        for (i, m) in msgs.iter().enumerate() {
            if m.role == Role::Assistant {
                if let Some(tcs) = &m.tool_calls {
                    let mut responded: HashSet<&str> = HashSet::new();
                    for later in &msgs[i + 1..] {
                        if later.role == Role::Tool {
                            if let Some(id) = &later.tool_call_id {
                                responded.insert(id.as_str());
                            }
                        } else {
                            break; // tool responses are contiguous right after the assistant
                        }
                    }
                    if tcs.iter().all(|c| responded.contains(c.id.as_str())) {
                        for c in tcs {
                            valid_ids.insert(c.id.clone());
                        }
                    }
                }
            }
        }

        // Pass 2: emit a valid sequence.
        let mut out: Vec<ChatMessage> = Vec::with_capacity(msgs.len());
        for m in msgs {
            match m.role {
                Role::Assistant => {
                    if let Some(tcs) = m.tool_calls.as_ref() {
                        if tcs.iter().all(|c| valid_ids.contains(&c.id)) {
                            out.push(m.clone()); // complete tool-call group → keep as-is
                        } else if !m.content.trim().is_empty() {
                            // dangling tool-call → drop tool_calls, keep any text content
                            let mut m2 = m.clone();
                            m2.tool_calls = None;
                            out.push(m2);
                        }
                        // else: empty dangling assistant → drop entirely
                    } else if m.reasoning_promoted {
                        // Excluded from the wire only: this turn's content was PROMOTED
                        // from raw reasoning/chain-of-thought by `final_answer` (empty
                        // `content` + non-empty `reasoning`). Replaying that prose
                        // few-shot-conditions other models into skipping tool use, so it
                        // is dropped from the sent history — storage/display (the stored
                        // `Conversation` + msglog) are untouched. Promoted turns never
                        // carry `tool_calls` (that's handled by the arm above), so
                        // dropping one here can never orphan a Tool result.
                    } else {
                        out.push(m.clone());
                    }
                }
                Role::Tool => {
                    // keep tool results only when their call was fully answered
                    if m.tool_call_id
                        .as_deref()
                        .is_some_and(|id| valid_ids.contains(id))
                    {
                        out.push(m.clone());
                    }
                }
                _ => out.push(m.clone()),
            }
        }
        out
    }

    /// Insert or replace the system message at index 0.
    ///
    /// "Absent" means the vec is empty or `messages[0].role != System`. In
    /// both cases a new `System` message is inserted at position 0. When a
    /// system message already exists its `content` is replaced in-place.
    /// This never appends a second system message.
    pub fn set_system(&mut self, content: impl Into<String>) {
        let content = content.into();
        if self
            .messages
            .first()
            .map(|m| m.role == Role::System)
            .unwrap_or(false)
        {
            // Fast path: system message is already at [0], just update it.
            self.messages[0].content = content;
        } else {
            // No system message present — prepend one.
            self.messages
                .insert(0, ChatMessage::new(Role::System, content));
        }
    }

    /// Strip dangling tool-call groups from a message slice so it can be sent to a
    /// model without a "no tool output for function call" 400. Mirrors the
    /// tool-pairing logic in [`Conversation::history`] (pass-1 valid_ids + pass-2
    /// keep/strip/drop) — used by compaction, which slices raw messages and would
    /// otherwise cut a tool round in half. Keeps behavior identical to history()'s
    /// tool handling: an assistant keeps its tool_calls only if EVERY id has a
    /// matching contiguous Role::Tool result within the slice; a partial/dangling
    /// group is stripped (content kept if non-empty, else message dropped); a
    /// Role::Tool result is kept only if its call is answered within the slice.
    fn strip_dangling_tool_calls(msgs: &[ChatMessage]) -> Vec<ChatMessage> {
        use std::collections::HashSet;

        // Pass 1: collect ids of tool_calls that are fully answered within msgs.
        let mut valid_ids: HashSet<String> = HashSet::new();
        for (i, m) in msgs.iter().enumerate() {
            if m.role == Role::Assistant {
                if let Some(tcs) = &m.tool_calls {
                    let mut responded: HashSet<&str> = HashSet::new();
                    for later in &msgs[i + 1..] {
                        if later.role == Role::Tool {
                            if let Some(id) = &later.tool_call_id {
                                responded.insert(id.as_str());
                            }
                        } else {
                            break; // tool responses are contiguous right after the assistant
                        }
                    }
                    if tcs.iter().all(|c| responded.contains(c.id.as_str())) {
                        for c in tcs {
                            valid_ids.insert(c.id.clone());
                        }
                    }
                }
            }
        }

        // Pass 2: emit a valid sequence.
        let mut out: Vec<ChatMessage> = Vec::with_capacity(msgs.len());
        for m in msgs {
            match m.role {
                Role::Assistant => {
                    if let Some(tcs) = m.tool_calls.as_ref() {
                        if tcs.iter().all(|c| valid_ids.contains(&c.id)) {
                            out.push(m.clone()); // complete tool-call group → keep as-is
                        } else if !m.content.trim().is_empty() {
                            // dangling tool-call → drop tool_calls, keep any text content
                            let mut m2 = m.clone();
                            m2.tool_calls = None;
                            out.push(m2);
                        }
                        // else: empty dangling assistant → drop entirely
                    } else {
                        out.push(m.clone());
                    }
                }
                Role::Tool => {
                    // keep tool results only when their call was fully answered
                    if m.tool_call_id
                        .as_deref()
                        .is_some_and(|id| valid_ids.contains(id))
                    {
                        out.push(m.clone());
                    }
                }
                _ => out.push(m.clone()),
            }
        }
        out
    }

    /// Split the conversation into two parts for compaction, skipping the
    /// system message.
    ///
    /// Given `messages = [system, m1, m2, … mN]` and `preserve_n`:
    ///
    /// - `body = messages[1..]` (all non-system messages, length `N`)
    /// - If `N <= preserve_n` there is nothing old enough to summarise:
    ///   returns `([], body)`.
    /// - Otherwise `split_at = N - preserve_n`:
    ///   - `to_summarize = body[..split_at]`  ← sent to the model as context
    ///   - `kept_tail    = body[split_at..]`  ← kept verbatim after compaction
    ///
    /// The system message is excluded from both halves; `apply_compaction`
    /// re-prepends it.
    pub fn split_for_compaction(&self, preserve_n: usize) -> (Vec<ChatMessage>, Vec<ChatMessage>) {
        if self.messages.is_empty() {
            return (vec![], vec![]);
        }
        // Skip messages[0] (system prompt) — it is not subject to compaction.
        let body = &self.messages[1..];
        if body.len() <= preserve_n {
            // Not enough history to compact; return everything as kept_tail.
            return (vec![], body.to_vec());
        }
        let split_at = body.len() - preserve_n;
        let to_summarize = Self::strip_dangling_tool_calls(&body[..split_at]);
        let kept_tail = body[split_at..].to_vec();
        (to_summarize, kept_tail)
    }

    /// Rebuild the conversation from a compaction snapshot.
    ///
    /// After this call `messages` is exactly:
    /// ```text
    /// [ system, Assistant("[summary of earlier conversation]\n<summary>"),
    ///   kept_tail[0], kept_tail[1], … ]
    /// ```
    ///
    /// The `kept_tail` is supplied by the caller (it came from
    /// `split_for_compaction`) and is NOT re-derived here. The system message
    /// is taken from `self.messages[0]`; if no system message exists yet a
    /// blank one is inserted first via `set_system`.
    pub fn apply_compaction(&mut self, summary: String, kept_tail: Vec<ChatMessage>) {
        // Guard: ensure a System message exists at [0] before we clone it.
        if !self
            .messages
            .first()
            .map(|m| m.role == Role::System)
            .unwrap_or(false)
        {
            self.set_system(String::new());
        }
        let system = self.messages[0].clone();
        let mut rebuilt = vec![
            system,
            // The summary is injected as an Assistant turn so models that
            // enforce strict user/assistant alternation don't choke on it.
            ChatMessage::new(
                Role::Assistant,
                format!("[summary of earlier conversation]\n{summary}"),
            ),
        ];
        rebuilt.extend(kept_tail);
        self.messages = rebuilt;
    }

    /// Rewind the conversation to JUST BEFORE the message at `idx`, dropping that
    /// message and every message after it. Keeps `messages[0..idx]`.
    ///
    /// Used by the double-Esc message-rewind picker: `idx` is the vec position of
    /// the selected user message (from `messages()`), so the cut leaves the
    /// history exactly as it was before that turn was sent. The System message at
    /// index 0 is never dropped: `idx == 0` is treated as a no-op (there is no
    /// pre-system state to rewind to), and an out-of-range `idx` (>= len) also
    /// leaves the history untouched.
    pub fn truncate_to_before_index(&mut self, idx: usize) {
        // Never drop the system message; never index past the end.
        if idx == 0 || idx >= self.messages.len() {
            return;
        }
        self.messages.truncate(idx);
    }

    /// Drop every non-system turn. Keeps `messages[0]` when it is System;
    /// otherwise empties the vec entirely (caller should re-seed via
    /// `set_system` / `Session::rebuild_system`). Used by `/clear`.
    pub fn clear_body(&mut self) {
        if self
            .messages
            .first()
            .map(|m| m.role == Role::System)
            .unwrap_or(false)
        {
            self.messages.truncate(1);
        } else {
            self.messages.clear();
        }
    }

    /// Pop all trailing `Assistant` messages (used before a resend so the
    /// model doesn't see its own previous partial reply as context).
    ///
    /// Returns the number of messages removed.
    pub fn pop_trailing_assistants(&mut self) -> usize {
        let mut removed = 0;
        while self
            .messages
            .last()
            .map(|m| m.role == Role::Assistant)
            .unwrap_or(false)
        {
            self.messages.pop();
            removed += 1;
        }
        removed
    }

    /// Return the content of the most-recent `User` message, if any.
    ///
    /// Used by the resend flow to replay the last user input.
    pub fn last_user_content(&self) -> Option<String> {
        self.messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| m.content.clone())
    }

    /// Render the recent conversation tail as a compact transcript for the
    /// tool-call safety classifier (TAC).
    ///
    /// Returns the last `max_msgs` User/Assistant messages (System and Tool
    /// messages are skipped — the classifier cares about human intent and the
    /// agent's stated plan, not raw tool I/O), oldest-to-newest, one per line as
    /// `User: ...` / `Assistant: ...`. Each message's content is trimmed and
    /// truncated to `max_chars` (char-boundary safe, ellipsis appended when cut).
    /// Empty-content messages (e.g. a tool-call-only assistant turn) are skipped
    /// so no blank `Assistant:` lines leak in. Returns an empty string when there
    /// is nothing to show.
    ///
    /// Why a tail and not just the latest user line: in multi-turn chats the most
    /// recent user message is often a terse confirmation ("ok go!", "yes") whose
    /// intent only resolves against the earlier request + the agent's proposal.
    pub fn recent_context(&self, max_msgs: usize, max_chars: usize) -> String {
        let mut picked: Vec<String> = self
            .messages
            .iter()
            .filter(|m| matches!(m.role, Role::User | Role::Assistant))
            .filter(|m| !m.content.trim().is_empty())
            .rev()
            .take(max_msgs)
            .map(|m| {
                let label = match m.role {
                    Role::User => "User",
                    _ => "Assistant",
                };
                let body = m.content.trim();
                let body: String = if body.chars().count() > max_chars {
                    let cut: String = body.chars().take(max_chars).collect();
                    format!("{cut}…")
                } else {
                    body.to_string()
                };
                format!("{label}: {body}")
            })
            .collect();
        picked.reverse();
        picked.join("\n")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod clear_body_tests {
    use super::*;

    #[test]
    fn clear_body_keeps_system_drops_rest() {
        let mut c = Conversation::new("sys");
        c.push_user("hi");
        c.push_assistant("hello", None, false);
        c.clear_body();
        assert_eq!(c.messages().len(), 1);
        assert_eq!(c.messages()[0].role, Role::System);
        assert_eq!(c.messages()[0].content, "sys");
    }

    #[test]
    fn clear_body_empties_when_no_system() {
        let mut c = Conversation::from_messages(vec![]);
        c.push_user("hi");
        c.push_assistant("hello", None, false);
        c.clear_body();
        assert!(c.messages().is_empty());
    }

    #[test]
    fn clear_body_on_empty_is_noop() {
        let mut c = Conversation::from_messages(vec![]);
        c.clear_body();
        assert!(c.messages().is_empty());
    }
}
