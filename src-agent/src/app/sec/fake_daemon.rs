//! A deterministic stand-in for the security daemon.
//!
//! The real daemon shells out to `python3 -m koma_sec_daemon`, which makes every
//! test that exercises it depend on a Python install, a docker probe and real
//! timing. That is fine for integration, but it makes a baseline fixture useless:
//! a byte-comparison oracle must not wobble.
//!
//! So this fake speaks the same frames and does nothing on its own. Responses are
//! SCRIPTED — a call's reply is decided by what the test registered, not by the
//! tool or the args. Anything not registered answers with the same `untyped`
//! result every time, so an unhandled call is loud rather than silently passing.
//!
//! It is `#[cfg(test)]`: nothing in the shipping build sees it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// One frame the fake wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame(pub String);

/// A scripted reply, or the default.
#[derive(Debug, Clone)]
pub enum Reply {
    /// A `result` payload for a matching tool.
    Result(String),
    /// An `error` payload.
    Error(String),
}

/// Script + recorded frames.
#[derive(Debug, Default)]
pub struct Script {
    replies: VecDeque<(String, Reply)>,
    written: Vec<Frame>,
    /// Tool names this fake advertises at handshake.
    tools: Vec<String>,
}

impl Script {
    /// A fake with no tools and no replies.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Advertise `names` at handshake, in order.
    #[must_use]
    pub fn advertising(mut self, names: &[&str]) -> Self {
        self.tools = names.iter().map(|n| (*n).to_string()).collect();
        self
    }

    /// Queue a result for the next call to `tool`.
    #[must_use]
    pub fn then_result(mut self, tool: &str, payload: &str) -> Self {
        self.replies.push_back((tool.to_string(), Reply::Result(payload.to_string())));
        self
    }

    /// Queue an error for the next call to `tool`.
    #[must_use]
    pub fn then_error(mut self, tool: &str, payload: &str) -> Self {
        self.replies.push_back((tool.to_string(), Reply::Error(payload.to_string())));
        self
    }

    /// The advertisement list, as the handshake would carry it.
    #[must_use]
    pub fn advertised(&self) -> &[String] {
        &self.tools
    }
}

/// Shared handle to the fake.
pub type Shared = Arc<Mutex<Script>>;

/// Build a shared fake.
#[must_use]
pub fn shared(script: Script) -> Shared {
    Arc::new(Mutex::new(script))
}

/// Answer one `call` frame body, recording the round trip.
///
/// The reply for a matching tool is consumed, so a scripted sequence is
/// order-dependent: calling a tool twice with one queued reply gives the scripted
/// answer then the default, which is what makes a duplicate-call bug visible.
#[must_use]
pub fn call(fake: &Shared, tool: &str, args: &serde_json::Value) -> String {
    let mut s = fake.lock().unwrap_or_else(|p| p.into_inner());
    s.written.push(Frame(format!("{tool} {args}")));
    let queued = s
        .replies
        .iter()
        .position(|(t, _)| t == tool)
        .map(|i| s.replies.remove(i).expect("index from position"));
    match queued {
        Some((_, Reply::Result(payload))) => payload,
        Some((_, Reply::Error(payload))) => payload,
        // Unscripted calls answer identically every time, so a test that forgot to
        // script one still runs deterministically.
        None => r#"{"verdict":"untyped","excerpt":"fake: no scripted reply"}"#.into(),
    }
}

/// Concatenated frames written so far — the comparison surface for a fixture.
#[must_use]
pub fn recorded(fake: &Shared) -> String {
    let s = fake.lock().unwrap_or_else(|p| p.into_inner());
    s.written
        .iter()
        .map(|f| f.0.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
#[path = "daemon_test.rs"]
mod daemon_test;
