//! `checklist` tool: lets the model create and update the session's task list.
//!
//! Writes to `memory/TODO.md` in the session directory, using the markdown
//! checkbox format that the `/todo` overlay parses. The model provides the
//! COMPLETE updated list on each call (not just deltas).

use super::{Tool, ToolCtx};
use anyhow::{Context, Result};
use serde_json::{json, Value};

/// The `checklist` tool: update the session's task list.
pub struct Checklist;

impl Tool for Checklist {
    fn name(&self) -> &'static str {
        "checklist"
    }

    fn description(&self) -> &'static str {
        include_str!("checklist.txt")
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "description": "The complete updated todo list",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": {
                                "type": "string",
                                "description": "Brief description of the task (under 80 chars)"
                            },
                            "status": {
                                "type": "string",
                                "description": "Current status: pending, in_progress, completed, cancelled",
                                "enum": ["pending", "in_progress", "completed", "cancelled"]
                            },
                            "priority": {
                                "type": "string",
                                "description": "Priority level: high, medium, low",
                                "enum": ["high", "medium", "low"]
                            },
                            "parent": {
                                "type": "string",
                                "description": "Optional parent task title."
                            }
                        },
                        "required": ["content", "status", "priority"]
                    }
                }
            },
            "required": ["todos"]
        })
    }

    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let todos = args
            .get("todos")
            .and_then(Value::as_array)
            .context("missing required 'todos' array")?;

        // Determine the write path: memory/TODO.md in the session directory.
        // We use the workspace root + memory/ as the base, matching the memory tool.
        let memory_dir = ctx
            .memory_dir
            .as_ref()
            .context("no session active — cannot write todos")?;

        // Ensure the memory directory exists.
        std::fs::create_dir_all(memory_dir).context("failed to create memory directory")?;

        let todo_path = memory_dir.join("TODO.md");

        // Build the markdown content.
        let mut md = String::from("# Session Tasks\n\n");
        for item in todos {
            let content = item
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or("(no description)");
            let status = item
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("pending");
            let priority = item
                .get("priority")
                .and_then(Value::as_str)
                .unwrap_or("medium");

            let checkbox = match status {
                "in_progress" => "~",
                "completed" => "x",
                "cancelled" => "-",
                _ => " ",
            };

            md.push_str(&format!("- [{checkbox}] {content} ({priority})\n"));
        }

        std::fs::write(&todo_path, &md).context("failed to write TODO.md")?;

        let active = todos
            .iter()
            .filter(|t| {
                t.get("status").and_then(Value::as_str) != Some("completed")
                    && t.get("status").and_then(Value::as_str) != Some("cancelled")
            })
            .count();

        Ok(format!("Updated {} todos ({} active)", todos.len(), active))
    }
}
