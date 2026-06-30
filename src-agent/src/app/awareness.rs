//! Project self-awareness (Phase 2): summarise the project's docs once per
//! session so the agent knows what it's working on without burning a full chat
//! round on it.
//!
//! On startup (and after `/compact`) the runtime reads the depth-1 project
//! docs (AGENT.md / AGENTS.md / README.md / CLAUDE.md) and asks a cheap
//! secondary model — via [`OpenRouterClient::complete_with`] — for a few-
//! sentence summary. That summary is stashed in `AppStateRest::awareness_summary`
//! and appended to the first System message on every request (see
//! `runtime::stream::start_stream_task`), so it survives compaction the same way
//! the top-level file listing does.
//!
//! Everything here is best-effort: a disabled flag, missing docs, no API key, or
//! a failed network call all degrade to `None`. The summary is never persisted —
//! it is recomputed per session so it always reflects the current docs.

use std::path::Path;

use crate::dto::chat::{ChatMessage, Role};
use crate::model::settings::Settings;
use crate::service::openrouter::{Conn, OpenRouterClient};

/// Project doc filenames probed at depth 1 of the workspace, in priority order.
/// Case-sensitive (matched verbatim) — the same names the rest of the app uses.
const DOC_FILES: &[&str] = &["AGENT.md", "AGENTS.md", "README.md", "CLAUDE.md"];

/// Max characters kept from any single doc file.
const PER_FILE_CAP: usize = 6000;

/// Max characters of combined doc corpus sent to the model.
const CORPUS_CAP: usize = 16000;

/// System instruction for the summary call. Kept terse and concrete so a small
/// model stays on task and returns prose the agent can use directly.
const SUMMARY_SYSTEM: &str = "You summarize a software project for an AI coding assistant. In 4-6 sentences, state what the project is, its stack, structure, and any conventions the agent must follow. Be concrete. No preamble.";

/// Take at most `cap` chars from `s` (char-boundary safe).
fn cap_chars(s: &str, cap: usize) -> String {
    s.chars().take(cap).collect()
}

/// Read depth-1 project docs and summarize them via the secondary model on the
/// resolved Awareness route (`conn` = endpoint + key; `model` + `provider` = the
/// upstream-route slug, `""` = default routing).
///
/// The caller resolves the Awareness role (`resolve_role(config, settings,
/// Awareness)`) and passes its connection in, so an awareness model on a
/// different provider/key than the chat path is reached without a client rebuild.
///
/// Returns `None` if awareness is disabled, no docs are found, or the model
/// call fails — the caller treats `None` as "no summary" and carries on.
pub async fn summarize(
    client: &OpenRouterClient,
    settings: &Settings,
    conn: Conn<'_>,
    model: &str,
    provider: &str,
    workdir: &Path,
) -> Option<String> {
    if !settings.awareness_enabled {
        return None;
    }

    // Gather the docs that exist at depth 1, labelling each with its filename so
    // the model can tell them apart. Each file is capped, and the running corpus
    // is bounded so a giant README can't blow the secondary call's context.
    let mut corpus = String::new();
    for name in DOC_FILES {
        if corpus.len() >= CORPUS_CAP {
            break;
        }
        let path = workdir.join(name);
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue; // missing / unreadable / non-UTF8 → skip
        };
        let body = cap_chars(raw.trim(), PER_FILE_CAP);
        if body.trim().is_empty() {
            continue;
        }
        let section = format!("## {name}\n{body}\n\n");
        // Trim the final section to fit the corpus cap rather than overshoot.
        let remaining = CORPUS_CAP - corpus.len();
        corpus.push_str(&cap_chars(&section, remaining));
    }

    let corpus = corpus.trim();
    if corpus.is_empty() {
        return None; // no docs to summarise
    }

    let messages = vec![
        ChatMessage::new(Role::System, SUMMARY_SYSTEM),
        ChatMessage::new(Role::User, corpus),
    ];

    // The model/provider/connection come from the caller's resolved Awareness
    // route (session override > config > legacy inherit/dedicated fields).
    // `complete_with` treats an empty provider as default routing.
    // Best-effort: any error (no key, network, bad provider) → no summary.
    match client.complete_with(conn, model, provider, messages).await {
        Ok(s) => {
            let s = s.trim();
            if s.is_empty() {
                None
            } else {
                Some(s.to_string())
            }
        }
        Err(_) => None,
    }
}

/// Like [`summarize`] but retries once with a fallback connection/model/provider
/// when the primary call fails (i.e. the model call itself errored, not a disabled
/// flag or missing docs). Only retries when the fallback differs from the primary
/// (`fallback_model != model` OR `fallback_conn.endpoint != conn.endpoint`).
///
/// All early-return `None` cases (disabled, no docs) are preserved unchanged.
/// The fallback is only attempted when the primary network/parse call actually
/// errored, so a no-docs `None` is never retried.
#[allow(clippy::too_many_arguments)]
pub async fn summarize_with_fallback(
    client: &OpenRouterClient,
    settings: &Settings,
    conn: Conn<'_>,
    model: &str,
    provider: &str,
    workdir: &std::path::Path,
    fallback_conn: Conn<'_>,
    fallback_model: &str,
    fallback_provider: &str,
) -> Option<String> {
    if !settings.awareness_enabled {
        return None;
    }

    // Gather the docs corpus (same as `summarize`).
    let mut corpus = String::new();
    for name in DOC_FILES {
        if corpus.len() >= CORPUS_CAP {
            break;
        }
        let path = workdir.join(name);
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let body = cap_chars(raw.trim(), PER_FILE_CAP);
        if body.trim().is_empty() {
            continue;
        }
        let section = format!("## {name}\n{body}\n\n");
        let remaining = CORPUS_CAP - corpus.len();
        corpus.push_str(&cap_chars(&section, remaining));
    }

    let corpus = corpus.trim();
    if corpus.is_empty() {
        return None; // no docs — nothing to retry
    }

    let messages = vec![
        ChatMessage::new(Role::System, SUMMARY_SYSTEM),
        ChatMessage::new(Role::User, corpus),
    ];

    // Primary attempt.
    match client.complete_with(conn, model, provider, messages.clone()).await {
        Ok(s) => {
            let s = s.trim();
            if s.is_empty() {
                None
            } else {
                Some(s.to_string())
            }
        }
        Err(_) => {
            // Primary call failed. Retry with the fallback route when it is
            // meaningfully different (avoids a guaranteed-duplicate failure).
            let same =
                fallback_model == model && fallback_conn.endpoint == conn.endpoint;
            if !same {
                if let Ok(s) = client
                    .complete_with(fallback_conn, fallback_model, fallback_provider, messages)
                    .await
                {
                    let s = s.trim();
                    if !s.is_empty() {
                        return Some(s.to_string());
                    }
                }
            }
            None
        }
    }
}
