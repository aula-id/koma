//! Knowledge injection — enriches the system prompt with cross-session
//! knowledge graph context before each send.
//!
//! Split into two stages:
//! 1. **Pre-send** (sync, on event loop): embed query → recall facts from
//!    local memory + daemon fallback — fast, no I/O beyond the local DB.
//! 2. **Distill** (async, in spawned task): if `use_awareness`, call the
//!    awareness model to produce a compact note; otherwise use a mechanical
//!    top-N-by-trust listing. The note is injected into the System message's
//!    volatile tail and NEVER rendered in the UI.

use std::path::Path;

use crate::app::resolve::Resolved;
use crate::model::settings::KnowledgeConfig;
use crate::model::surreal::memory::Fact;
use crate::service::openrouter::OpenRouterClient;

/// Facts gathered during pre-send, ready for distillation.
pub struct KnowledgeFacts {
    pub facts: Vec<Fact>,
}

/// Gather relevant facts for the user's query (synchronous, fast).
/// Returns `None` when disabled, query is empty, or no facts found.
pub fn gather(
    session_dir: &Path,
    config: &KnowledgeConfig,
    user_query: &str,
) -> Option<KnowledgeFacts> {
    if !config.enabled || user_query.trim().is_empty() {
        crate::model::store::append_global_error_log(
            "knowledge-gather",
            &format!(
                "skipped: enabled={}, query_empty={}",
                config.enabled,
                user_query.trim().is_empty()
            ),
        );
        return None;
    }
    let limit = (config.max_input_tokens / 500).max(3);
    let qv = crate::model::surreal::core::embed_one(user_query.trim());
    let facts = crate::model::surreal::memory::recall_memory(session_dir, &qv, limit);
    crate::model::store::append_global_error_log(
        "knowledge-gather",
        &format!(
            "session={}, query='{}', limit={}, found={}",
            session_dir.display(),
            &user_query[..user_query.len().min(60)],
            limit,
            facts.len(),
        ),
    );
    if facts.is_empty() {
        return None;
    }
    Some(KnowledgeFacts { facts })
}

/// Distill gathered facts into a compact note string.
///
/// When `use_awareness` is true and an awareness route is available, the
/// awareness model produces a 1-5 sentence summary. Otherwise, the top 3
/// facts by trust are mechanically joined.
///
/// Returns the note ready to append to the System message, e.g.:
/// `"\n\n[Knowledge context: Rust is a systems programming language ...]"`
#[allow(dead_code)]
pub async fn distill(
    facts: &KnowledgeFacts,
    config: &KnowledgeConfig,
    client: &OpenRouterClient,
    awareness_route: Option<&Resolved>,
) -> Option<String> {
    if !config.use_awareness {
        return Some(raw_note(facts));
    }

    // Awareness model distillation.
    let route = awareness_route?;
    let fact_lines: Vec<String> = facts
        .facts
        .iter()
        .map(|f| format!("- {} (trust: {:.2})", f.content, f.trust))
        .collect();
    let fact_text = fact_lines.join("\n");

    let prompt = format!(
        "Summarize these facts into 1-5 concise sentences. \
         Keep only what is relevant. Be factual, no filler.\n\n{fact_text}"
    );

    let messages = vec![
        crate::dto::chat::ChatMessage::new(
            crate::dto::chat::Role::System,
            "You distill knowledge facts into compact context notes. \
             Output 1-5 sentences of pure information. No introductions, \
             no meta-commentary, no bullet points.",
        ),
        crate::dto::chat::ChatMessage::new(crate::dto::chat::Role::User, prompt),
    ];

    let conn = route.conn();
    let result = client
        .complete_with(conn, &route.model_id, route.provider(), messages, false)
        .await
        .ok()?;

    let body = result.trim().to_string();
    if body.is_empty() {
        return Some(raw_note(facts));
    }

    let body = if body.len() > config.max_output_tokens.saturating_mul(4) {
        // Rough char cap: ~4 chars/token. Use floor_char_boundary to
        // avoid panicking on multi-byte UTF-8 (Chinese, emoji, etc).
        let cap = config.max_output_tokens.saturating_mul(4) - 1;
        let safe = body.floor_char_boundary(cap);
        format!("{}…", &body[..safe])
    } else {
        body
    };

    Some(format!("\n\n[Knowledge context: {body}]"))
}

/// Re-export of the quality filter for use by extraction code.
pub(crate) fn is_quality_fact_static(content: &str) -> bool {
    crate::model::surreal::memory::is_quality_fact(content)
}
/// disabled or unavailable). Deduplicates by content similarity.
pub(crate) fn raw_note(facts: &KnowledgeFacts) -> String {
    let mut sorted: Vec<&Fact> = facts.facts.iter().collect();
    sorted.sort_by(|a, b| {
        b.trust
            .partial_cmp(&a.trust)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Dedup: skip facts whose first 40 chars already appeared.
    let mut seen_prefixes: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut unique: Vec<&Fact> = Vec::with_capacity(3);
    for f in &sorted {
        if unique.len() >= 3 {
            break;
        }
        let prefix = f.content[..f.content.len().min(40)].to_lowercase();
        if seen_prefixes.insert(prefix) {
            unique.push(f);
        }
    }

    let lines: Vec<String> = unique
        .iter()
        .map(|f| f.content.trim().to_string())
        .filter(|s: &String| !s.is_empty())
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    let body = lines.join(". ");
    format!("\n\n[Knowledge context: {body}]")
}
