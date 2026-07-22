//! Long-term memory tools: `remember` / `forget` / `recall` operate on the
//! per-PROJECT index store so memories persist across every session opened from
//! the same working dir.
//!
//! The store is an index of pointers plus one file per memory (see
//! [`crate::model::memory`]): `remember` writes `<slug>.md` (frontmatter + body)
//! and refreshes `MEMORY.md` (the index); `forget` deletes a memory by slug; and
//! `recall` returns one memory's full body by slug. Only the index is injected
//! into the system prompt on rebuild; the model pulls a body on demand via
//! `recall`.

use super::{Tool, ToolCtx};
use crate::model::memory::{read_memory, remove_memory, slugify, write_memory};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

/// Pull a required string argument out of the decoded JSON args.
fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .with_context(|| format!("missing required string argument '{key}'"))
}

/// Pull an optional string argument out of the decoded JSON args.
fn opt_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

/// Saves (creates or updates) a memory in the per-project index store.
pub struct Remember;

impl Tool for Remember {
    fn name(&self) -> &'static str {
        "remember"
    }

    fn description(&self) -> &'static str {
        "Save a long-term memory for this project (shared across sessions). \
         Provide `description` (a one-line hook shown in the memory index) and \
         `content` (the full memory body). Optionally set `type` and a `slug` id; \
         if `slug` is omitted one is derived from the description. Saving to an \
         existing slug overwrites it (use this to edit/swap a memory). Returns the \
         slug; use `recall(slug)` later to read the full body, `forget(slug)` to \
         delete it."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "description": {
                    "type": "string",
                    "description": "One-line hook for this memory, shown in the injected memory index."
                },
                "content": {
                    "type": "string",
                    "description": "The full memory body to store."
                },
                "type": {
                    "type": "string",
                    "description": "Optional category: project | preference | reference | fact (default: fact)."
                },
                "slug": {
                    "type": "string",
                    "description": "Optional id/filename for the memory. Omit to derive one from the description. Reusing an existing slug overwrites that memory."
                }
            },
            "required": ["description", "content"]
        })
    }

    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let description = arg_str(args, "description")?;
        let content = arg_str(args, "content")?;
        let kind = opt_str(args, "type").unwrap_or("");
        // An explicit slug takes precedence; otherwise derive it from the
        // description. Either way `write_memory` re-sanitizes it (defence in
        // depth) before it ever becomes a filename.
        let slug_seed = match opt_str(args, "slug").filter(|s| !s.trim().is_empty()) {
            Some(s) => s.to_string(),
            None => slugify(description)
                .with_context(|| "could not derive a slug from the description")?,
        };

        let memory_dir = match ctx.memory_dir.as_ref() {
            Some(d) => d,
            None => bail!("no active session to save memory to"),
        };

        let slug = write_memory(memory_dir, &slug_seed, description, kind, content)
            .with_context(|| format!("saving memory to '{}'", memory_dir.display()))?;

        Ok(format!("Saved memory '{slug}'"))
    }
}

/// Deletes a memory from the per-project index store.
pub struct Forget;

impl Tool for Forget {
    fn name(&self) -> &'static str {
        "forget"
    }

    fn description(&self) -> &'static str {
        "Delete a saved memory by its `slug` (the id shown next to each entry in \
         the injected memory index). Removes the memory file and its index line. \
         No-op if the slug doesn't exist."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "slug": {
                    "type": "string",
                    "description": "The id of the memory to delete (as listed in the memory index)."
                }
            },
            "required": ["slug"]
        })
    }

    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let slug = arg_str(args, "slug")?;

        let memory_dir = match ctx.memory_dir.as_ref() {
            Some(d) => d,
            None => bail!("no active session to delete memory from"),
        };

        let removed = remove_memory(memory_dir, slug)
            .with_context(|| format!("deleting memory from '{}'", memory_dir.display()))?;

        if removed {
            Ok(format!("Forgot memory '{slug}'"))
        } else {
            Ok(format!("No memory '{slug}' to forget"))
        }
    }
}

/// Reads one memory's full body by slug from the per-project index store.
pub struct Recall;

impl Tool for Recall {
    fn name(&self) -> &'static str {
        "recall"
    }

    fn description(&self) -> &'static str {
        "Read a saved memory's full body by its `slug` (the id shown next to each \
         entry in the injected memory index). Only the index of one-line hooks is \
         injected into the prompt, so use this to pull a specific memory's full \
         content on demand."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "slug": {
                    "type": "string",
                    "description": "The id of the memory to read (as listed in the memory index)."
                }
            },
            "required": ["slug"]
        })
    }

    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let slug = arg_str(args, "slug")?;

        let memory_dir = match ctx.memory_dir.as_ref() {
            Some(d) => d,
            None => bail!("no active session to recall memory from"),
        };

        match read_memory(memory_dir, slug) {
            Some(body) => Ok(body),
            None => Ok(format!("No memory '{slug}'")),
        }
    }
}
