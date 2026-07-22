//! Hidden gem: the `pong` tool.
//!
//! No arguments, no side effects, no approval needed. The model calls it
//! whenever the user says "ping" (the name + description make the gag
//! obvious). Returns a cheerful "pong!" so the conversation stays snappy.

use super::{Tool, ToolCtx};
use anyhow::Result;
use serde_json::{json, Value};

/// Respond to "ping" with "pong!" — a zero-argument easter egg.
pub struct Pong;
impl Tool for Pong {
    fn name(&self) -> &'static str {
        "pong"
    }

    fn description(&self) -> &'static str {
        "Respond to a user saying 'ping' — return 'pong!' with a smile. No arguments, no side effects."
    }

    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }

    fn run(&self, _ctx: &ToolCtx, _args: &Value) -> Result<String> {
        Ok("pong! 🏓".into())
    }
}
