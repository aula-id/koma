// ─── StreamEvent wire mirror ─────────────────────────────────────────────────

use serde::{Deserialize, Serialize};

use crate::dto::chat::ChatMessage;
use crate::service::StreamEvent;

/// A serde mirror of the cross-session-relevant `StreamEvent` variants.
///
/// `StreamEvent` is `Clone` but not serde-cleanly transferable (the endpoint /
/// catalogue variants carry `ModelEndpoint`, which is `Deserialize`-only). Those
/// variants are CLIENT-LOCAL UI concerns (the model modal + catalogue fetch live
/// on `AppStateRest` directly, not per-session) and never cross the daemon
/// boundary, so they are deliberately omitted: [`From<&StreamEvent>`] yields
/// `None` for them. The eight variants that actually drive a session's turn are
/// mirrored faithfully.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)] // wired in daemon stage 2+ (no callers in stage 1)
pub enum StreamEventWire {
    Token(String),
    Reasoning(String),
    Usage {
        prompt_tokens: u64,
        completion_tokens: u64,
        cached_tokens: u64,
        cost: f64,
    },
    ToolCalls(Vec<crate::dto::chat::ToolCall>),
    Done,
    Error(String),
    Compacted {
        summary: String,
        kept_tail: Vec<ChatMessage>,
    },
    HarnessVerdict {
        allow: bool,
        reason: String,
    },
}

impl StreamEventWire {
    /// Project a live [`StreamEvent`] into its wire mirror.
    ///
    /// Returns `None` for the client-local UI events (endpoint list + catalogue)
    /// that never cross the daemon boundary — see the type docs.
    #[allow(dead_code)] // wired in daemon stage 2+ (no callers in stage 1)
    pub fn from_event(ev: &StreamEvent) -> Option<Self> {
        Some(match ev {
            StreamEvent::Token(s) => StreamEventWire::Token(s.clone()),
            StreamEvent::Reasoning(s) => StreamEventWire::Reasoning(s.clone()),
            StreamEvent::Usage {
                prompt_tokens,
                completion_tokens,
                cached_tokens,
                cost,
            } => StreamEventWire::Usage {
                prompt_tokens: *prompt_tokens,
                completion_tokens: *completion_tokens,
                cached_tokens: *cached_tokens,
                cost: *cost,
            },
            StreamEvent::ToolCalls(calls) => StreamEventWire::ToolCalls(calls.clone()),
            StreamEvent::Done => StreamEventWire::Done,
            StreamEvent::Error(s) => StreamEventWire::Error(s.clone()),
            StreamEvent::Compacted { summary, kept_tail } => StreamEventWire::Compacted {
                summary: summary.clone(),
                kept_tail: kept_tail.clone(),
            },
            StreamEvent::HarnessVerdict { allow, reason } => StreamEventWire::HarnessVerdict {
                allow: *allow,
                reason: reason.clone(),
            },
            // Client-local UI events — never sent over the wire.
            StreamEvent::EndpointsLoaded { .. } | StreamEvent::EndpointsError { .. } => return None,
        })
    }
}

impl From<&StreamEvent> for Option<StreamEventWire> {
    fn from(ev: &StreamEvent) -> Self {
        StreamEventWire::from_event(ev)
    }
}
