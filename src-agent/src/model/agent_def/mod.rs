//! Agent-definition data layer: load, merge, and persist agent definitions.
//!
//! An agent is a YAML-frontmatter + markdown file. The markdown BODY is the
//! agent's system prompt; the YAML frontmatter holds its metadata. The agent's
//! NAME is the filename stem, lowercased (`PINEAPPLE.md` → `pineapple`).
//!
//! ```text
//! ---
//! description: When to use this agent.
//! model: openai/gpt-oss-20b      # optional; None inherits the session model
//! tools: [read, grep, glob]      # allow-list; "task" is never auto-included
//! steps: 15                      # optional iteration cap
//! ---
//! You are a focused subagent. Do X, then report Y.   ← the system prompt
//! ```
//!
//! **Loader tiers** (later overrides earlier, by lowercased name):
//! 1. Built-in agents compiled into the binary ([`registry::builtin_agents`]).
//! 2. Global agents from `~/.koma/agents/*.md`.
//! 3. Session agents from `<session_dir>/agents/*.md`.
//!
//! After merging, `disable: true` removes a name from the registry, and
//! `hidden: true` keeps it but hides it from menus. A single malformed file is
//! logged and skipped — it never panics or blocks the rest of the registry.

// This is the data layer + public API; its consumers (the `/agents` UI and the
// `task` tool) are wired up in a later change. Until then the public surface is
// unreferenced from the binary, which is intentional — silence dead-code (and the
// matching unused re-exports) here rather than littering each item with `#[allow]`.
#![allow(dead_code)]
#![allow(unused_imports)]

mod def;
mod parse;
mod registry;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;

// ---------------------------------------------------------------------------
// Re-exports — keep every external path identical to the original flat file
// ---------------------------------------------------------------------------

pub use def::{AgentDef, AgentSource};
pub use parse::load_agent_file;
// Exposed for the extension-uninstall agent-override sweep, which validates each contributed
// sub-agent name (the delete key) exactly as `save_agent` did when it wrote the file.
pub(crate) use parse::validate_agent_name;
pub use registry::{
    agents_dir, delete_agent, global_agents_dir, load_registry, save_agent, session_agents_dir,
    AgentRegistry, AgentScope,
};
