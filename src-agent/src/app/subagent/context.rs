//! Seed-conversation builder for a sub-agent.
//!
//! A sub-agent runs against its OWN isolated [`Conversation`] — it never reads or
//! references the main session's history. [`build_seed`] assembles that fresh
//! conversation from the agent's persona prompt plus optional project context,
//! then seeds the task as the first user turn.

// Inert in Stage 1: used only by the (also-inert) spawn path until a later stage
// wires the sub-agent runtime into the binary.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crate::model::agent_def::AgentDef;
use crate::model::conversation::Conversation;
use crate::tool::DirCache;

/// Build the sub-agent's isolated seed conversation.
///
/// The system message is composed, in order:
/// 1. the agent's persona (`agent.prompt`, the markdown body of its definition);
/// 2. the project's `MEMORY.md` text, under a `# Project memory` header — only
///    when non-empty;
/// 3. the awareness blurb, under a `# Project context` header — only when
///    non-empty;
/// 4. a workspace idx→path mapping table (multi-workspace only) so the model can
///    resolve `@N` workspace references without guessing;
/// 5. the project files listing (top-level children from the dir cache) so the
///    sub-agent has the same project layout awareness as the main agent.
///
/// The `task` is then pushed as the first (and only) user turn. The returned
/// conversation is fully self-contained: it shares nothing with the main session
/// `Conversation`, so a sub-agent can never see — or pollute — the interactive
/// chat history.
pub fn build_seed(
    agent: &AgentDef,
    awareness: &str,
    memory_md: &str,
    workspaces: &[PathBuf],
    dir_cache: &Arc<RwLock<DirCache>>,
    task: &str,
) -> Conversation {
    let mut system = agent.prompt.trim_end().to_string();

    // Project memory (MEMORY.md) — verbatim, under its own header. Skipped when
    // empty so a memory-less project doesn't seed a dangling header.
    let memory_md = memory_md.trim();
    if !memory_md.is_empty() {
        if !system.is_empty() {
            system.push_str("\n\n");
        }
        system.push_str("# Project memory\n");
        system.push_str(memory_md);
    }

    // Awareness blurb — the project-context digest, under its own header. Also
    // skipped when empty.
    let awareness = awareness.trim();
    if !awareness.is_empty() {
        if !system.is_empty() {
            system.push_str("\n\n");
        }
        system.push_str("# Project context\n");
        system.push_str(awareness);
    }

    // Project files listing + workspace index table: mirror the main agent's
    // volatile tail so the sub-agent has the same project layout awareness.
    if let Ok(cache) = dir_cache.read() {
        let mut listing = cache.children(".", 0);
        let is_multi = cache.is_multi();
        if is_multi {
            for i in 1.. {
                let more = cache.children(".", i);
                if more.is_empty() {
                    break;
                }
                listing.extend(more);
            }
        }
        if !listing.is_empty() {
            if !system.is_empty() {
                system.push_str("\n\n");
            }
            system.push_str("# Project files (top level)\n");
            // Multi-workspace idx→path table so @N references are unambiguous.
            if is_multi && !workspaces.is_empty() {
                system.push_str("|idx|path|\n|---|---|\n");
                for (i, ws) in workspaces.iter().enumerate() {
                    system.push_str(&format!("|{i}|{}|\n", ws.display()));
                }
                system.push('\n');
            }
            let model_listing: Vec<_> = listing
                .iter()
                .map(|e| crate::tool::model_path_from_entry(workspaces, e))
                .collect();
            system.push_str(&model_listing.join("\n"));
        }
    }

    system.push_str(
        "\n\n# Reporting back\n         Your final message IS your report to the main agent (delivered as a tool result), not a chat reply.          Keep it CONCISE, structured, and self-contained so it survives the context window: lead with the          answer/outcome, then the key findings (paths + line numbers; include only load-bearing snippets),          then any blockers. Do NOT paste whole files or narrate your search/steps — reference by path:line.          There is a hard ~50k-character ceiling on delivery; stay well under it. If you found a lot, prioritise          the most decision-relevant facts and summarise the rest in a line or two.",
    );

    let mut convo = Conversation::from_messages(Vec::new());
    convo.set_system(system);
    convo.push_user(task);
    convo
}
