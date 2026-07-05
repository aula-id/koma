//! The `seqthink` tool: a stateless sequential-thinking scratchpad.
//!
//! Adapted from the upstream MCP `sequentialthinking` tool, in koma's own
//! snake_case parameter convention (the upstream camelCase is NOT copied).
//! Advertised only while [`crate::app::state::AgentMode::Plan`] is active (see
//! `app::runtime::stream::run::start_stream_task`) — it gives the model a place
//! to externalize numbered, revisable reasoning steps while it explores and
//! designs a plan. STATELESS BY DESIGN: no history is kept across calls; `run`
//! only validates the current call's fields and echoes back the bookkeeping
//! trio the model needs to keep track of its own progress. The actual thinking
//! lives in the model's own `thought` argument (and the conversation transcript
//! that carries every call), not in any server-side state.

use anyhow::Result;
use serde_json::{json, Value};

use super::{Tool, ToolCtx};

/// Structured step-by-step reasoning for planning/analysis. See module docs.
pub struct SeqThink;

impl Tool for SeqThink {
    fn name(&self) -> &'static str {
        "seqthink"
    }

    fn description(&self) -> &'static str {
        "Structured step-by-step reasoning for complex planning and analysis. Break a \
         problem into numbered thoughts that can build on, question, or revise earlier \
         ones. Start with an estimate of total_thoughts and adjust as you go: revise a \
         previous thought with is_revision + revises_thought, branch an alternative with \
         branch_from_thought + branch_id, extend past your estimate by raising \
         total_thoughts or setting needs_more_thoughts. Express uncertainty freely; \
         generate a hypothesis, verify it against what you have read, and repeat. Set \
         next_thought_needed=false only when you have a satisfactory conclusion. Use one \
         tool call per thought."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "thought": {
                    "type": "string",
                    "description": "Your current thinking step"
                },
                "next_thought_needed": {
                    "type": "boolean",
                    "description": "Whether another thought step is needed"
                },
                "thought_number": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Current thought number"
                },
                "total_thoughts": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Estimated total thoughts needed"
                },
                "is_revision": {
                    "type": "boolean",
                    "description": "Whether this revises previous thinking"
                },
                "revises_thought": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Which thought is being reconsidered"
                },
                "branch_from_thought": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Branching point thought number"
                },
                "branch_id": {
                    "type": "string",
                    "description": "Branch identifier"
                },
                "needs_more_thoughts": {
                    "type": "boolean",
                    "description": "If more thoughts are needed"
                }
            },
            "required": ["thought", "next_thought_needed", "thought_number", "total_thoughts"]
        })
    }

    fn run(&self, _ctx: &ToolCtx, args: &Value) -> Result<String> {
        if args.get("thought").and_then(Value::as_str).is_none() {
            return Ok("error: missing required string argument 'thought'".into());
        }
        let Some(next_thought_needed) = args.get("next_thought_needed").and_then(Value::as_bool)
        else {
            return Ok("error: missing required boolean argument 'next_thought_needed'".into());
        };
        let thought_number = match args.get("thought_number").and_then(Value::as_u64) {
            Some(n) if n >= 1 => n,
            _ => {
                return Ok(
                    "error: missing or invalid required integer argument 'thought_number' (must be >= 1)"
                        .into(),
                )
            }
        };
        let total_thoughts = match args.get("total_thoughts").and_then(Value::as_u64) {
            Some(n) if n >= 1 => n,
            _ => {
                return Ok(
                    "error: missing or invalid required integer argument 'total_thoughts' (must be >= 1)"
                        .into(),
                )
            }
        };

        // Stateless: nothing is remembered across calls. If the model has already
        // pushed past its own estimate, bump the estimate up to match rather than
        // silently reporting a stale total.
        let total_thoughts = total_thoughts.max(thought_number);

        Ok(json!({
            "thought_number": thought_number,
            "total_thoughts": total_thoughts,
            "next_thought_needed": next_thought_needed
        })
        .to_string())
    }
}

#[cfg(test)]
#[path = "seqthink_test.rs"]
mod tests;
