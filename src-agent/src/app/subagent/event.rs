//! Events emitted by a running sub-agent loop.
//!
//! A sub-agent runs autonomously in a background task and reports its progress
//! down an [`mpsc::UnboundedSender`](tokio::sync::mpsc::UnboundedSender) as a
//! stream of [`AgentEvent`]s. This is the ONLY type that crosses the sub-agent
//! task -> orchestrator boundary; the orchestrator (wired up in a later stage)
//! drains the matching receiver and folds the events into its UI / transcript.
//!
//! The sub-agent channel is intentionally separate from the main chat
//! `StreamEvent` channel: a sub-agent has no `AppState`, never prompts a human,
//! and reports tool activity at a coarser grain (started / done) than the
//! token-level streaming the interactive chat needs.

// This module is the inert sub-agent runtime (Stage 1): its public surface is
// fully defined but not yet wired into the chat loop / `task` tool, so every item
// is legitimately unreferenced from the binary until a later stage. Silence
// dead-code here rather than littering each item with `#[allow]`.
#![allow(dead_code)]

/// A single progress event from a sub-agent's autonomous loop.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// A chunk of assistant text produced this step (the `content` delta).
    Token(String),
    /// A permitted tool call is about to run. `args` is the raw JSON-encoded
    /// arguments string as the model emitted them.
    ToolStarted { name: String, args: String },
    /// A tool call finished; `result` is the string fed back to the model
    /// (an `error: …` / `blocked …` line on failure or refusal).
    ToolDone { name: String, result: String },
    /// The loop entered step `usize` (0-based), i.e. it is about to make the
    /// `usize`-th model call.
    Step(usize),
    /// A follow-up user message was INJECTED into this sub-agent (via the broker
    /// `agents.send` verb or the main-agent `task_send` tool) and folded into the
    /// isolated history at a turn boundary. `String` is the injected text. Emitted
    /// so the orchestrator records it in the flat viewer transcript (the paired
    /// [`Snapshot`](AgentEvent::Snapshot) that follows carries the structured
    /// history), letting a human watching the `$` panel see the steer.
    Injected(String),
    /// A full snapshot of the sub-agent's structured conversation after a turn
    /// was committed. Emitted once per turn (and right before `Done`) so the UI
    /// always holds the sub-agent's complete structured history for later
    /// viewing. Bounded by `max_steps`, so cloning per turn is cheap.
    Snapshot(Vec<crate::dto::chat::ChatMessage>),
    /// The loop finished cleanly; `String` is the final assistant answer (or a
    /// "(stopped: …)" note when the step budget was exhausted).
    Done(String),
    /// The loop aborted on a fatal stream error; `String` is the cause.
    Error(String),
    /// Accumulated token/cost spend across all steps. Emitted once, immediately
    /// before [`Done`](AgentEvent::Done), so the orchestrator can merge it into
    /// the session total and record a ledger row. `model_id` is the resolved
    /// model the loop ran against. Non-fatal: if no Usage chunk was ever received
    /// (e.g. the provider omits them) all three counters are 0/0.0.
    UsageReport {
        model_id: String,
        tokens_in: u64,
        tokens_out: u64,
        cost: f64,
    },
}
