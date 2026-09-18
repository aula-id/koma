//! Phase-3 send-path reshaper: build the short-send wire payload.
//!
//! [`shape`] is a PURE transform over the API-bound history that drops the older
//! turns in favour of the rolling summary + a verbatim tail hard-capped at
//! `settings.short_send_tail_n`, rehydrating only the archived blobs a strict-JSON
//! router (reasoning OFF) judges relevant to the current question. It reads sqlite
//! and builds a NEW `Vec<ChatMessage>` — it never touches the live `Conversation`,
//! `messages.json`, or the rendered transcript (dual rail: only the wire payload is
//! compressed). Compression only applies when a usable rolling summary already
//! exists — missing/empty summary fail-opens to the full history so the agent is
//! never starved mid-task while the fold is still catching up. The send path
//! applies it inside the spawned stream task, just before the request is POSTed.

use std::path::Path;

use crate::app::resolve::Resolved;
use crate::dto::chat::{ChatMessage, Role};
use crate::model::msglog;
use crate::model::settings::Settings;
use crate::resources;
use crate::service::openrouter::OpenRouterClient;

use super::fold::update_summary;

/// Most blobs to rehydrate into a single send payload. A hard cap so the router
/// can't undo the compression by recalling the whole archive — the summary
/// already carries the gist; recalls are for the few items the question needs.
const MAX_REHYDRATE: usize = 3;

/// Build the router's user payload: the user's latest message followed by the
/// candidate blob list, one per line as `#<id> [<kind>] <snippet>`. The router
/// reads this against [`shortsend_router_prompt`] and returns the ids whose full
/// content the answer needs.
fn build_router_payload(user_intent: &str, candidates: &[msglog::BlobRef]) -> String {
    let mut out = String::new();
    out.push_str("=== USER MESSAGE ===\n");
    out.push_str(user_intent.trim());
    out.push_str("\n\n=== AVAILABLE BLOBS ===\n");
    for b in candidates {
        out.push('#');
        out.push_str(&b.id.to_string());
        out.push_str(" [");
        out.push_str(&b.kind);
        out.push_str("] ");
        out.push_str(b.snippet.trim());
        out.push('\n');
    }
    out
}

/// Extract significant search terms from the user's latest message for the
/// content-recall query: split on every non-alphanumeric boundary, lowercase,
/// keep words of length >= 4 (drops stop-word-ish noise and punctuation), dedup
/// preserving first-seen order, and cap at 8 (enough signal; keeps the LIKE OR
/// chain small). Returns an empty vec when nothing qualifies — the caller then
/// skips the content-search path entirely.
fn significant_terms(user_intent: &str) -> Vec<String> {
    const MAX_TERMS: usize = 8;
    const MIN_LEN: usize = 4;
    let mut out: Vec<String> = Vec::new();
    for raw in user_intent.split(|c: char| !c.is_alphanumeric()) {
        if raw.chars().count() < MIN_LEN {
            continue;
        }
        let term = raw.to_lowercase();
        if out.iter().any(|t| t == &term) {
            continue; // dedup, first-seen order preserved
        }
        out.push(term);
        if out.len() >= MAX_TERMS {
            break;
        }
    }
    out
}

/// Resolve the settings-driven hard tail length (≥ 1).
fn tail_n_cap(settings: &Settings) -> usize {
    (settings.short_send_tail_n.max(1) as usize).max(1)
}

/// Split `history` into system + last `keep` body messages. `keep` is already
/// clamped by the caller. Returns `None` only when history is empty/system-only
/// (caller should fall back).
fn clip_body(history: &[ChatMessage], keep: usize) -> Option<(ChatMessage, Vec<ChatMessage>)> {
    if history.is_empty() {
        return None;
    }
    let body = &history[1..];
    if body.is_empty() {
        return None;
    }
    let keep = keep.min(body.len()).max(1);
    let tail = body[body.len() - keep..].to_vec();
    Some((history[0].clone(), tail))
}

/// Reshape the API-bound history into the short-send payload: drop the older
/// turns in favour of the rolling summary + a verbatim tail, rehydrating only the
/// archived blobs the router judges relevant to `user_intent`.
///
/// This is a PURE transform over the wire payload: it reads `messages.sqlite` and
/// builds a brand-new `Vec<ChatMessage>`, and it NEVER mutates stored or displayed
/// state. The caller passes the full API-bound history (system + body) and sends
/// the returned vec instead; the live `Conversation`, `messages.json`, and the
/// rendered transcript are untouched (dual rail — display is unaffected).
///
/// When **engaged** (`summarizing`) **and** a non-empty rolling summary exists:
/// the wire body is hard-capped at `settings.short_send_tail_n` messages with the
/// summary (and optional blob recalls) on the system tail. Missing/empty summary
/// fail-opens to the full history — fold is best-effort next turn; never drop
/// mid-task context for a stub. Post-`/compact` histories also fail-open (no
/// stacked sqlite summary, no hard clip).
///
/// When **not engaged** or kill-switched: returns the original history unchanged.
///
/// `history[0]` (the system message, already carrying any project-files/awareness
/// injection from the caller) is preserved as index 0 of the output. The rolling
/// summary + any rehydrated blobs are APPENDED to its content (after the volatile
/// dir-listing/awareness tail), NOT emitted as a synthetic assistant turn — so the
/// caching wire layer's mark-split still lands on the real system message, and the
/// per-fold summary stays in the uncached tail (it must not bust the cached head).
///
/// `summarizing` is the engage decision, made UPSTREAM in `start_stream_task`
/// (cache-warmth + sticky hysteresis + count gate). When false, the full history is
/// sent verbatim (cheap via prompt caching) and this is a no-op. `usable =
/// context_window - BASE_OVERHEAD` is the token budget the fold's band sizing is
/// taken against.
///
/// `route` is the resolved Awareness route (short-send's fold + snippet-router
/// share the Awareness role), snapshotted by the caller BEFORE the spawn. `None`
/// (an unresolved Awareness role) skips the fold and the snippet-router branch —
/// an existing summary + content-search recalls still apply, nothing is folded or
/// router-rehydrated this turn.
#[allow(clippy::too_many_arguments)]
pub async fn shape(
    history: Vec<ChatMessage>,
    session_dir: &Path,
    client: &OpenRouterClient,
    settings: &Settings,
    route: Option<Resolved>,
    user_intent: &str,
    summarizing: bool,
    usable: u64,
) -> Vec<ChatMessage> {
    // 1. Kill switch: short-send disabled → send the full history unchanged.
    if !settings.short_send_enabled {
        return history;
    }

    // 2. Engage gate. The cache-warmth + sticky-hysteresis + count decision is made
    //    upstream (in `start_stream_task`); here we simply honour it. Not engaged
    //    → send the full history (cheap via prompt caching; full fidelity).
    if !summarizing {
        return history;
    }

    let tail_cap = tail_n_cap(settings);

    // 2b. Too short to be worth compressing: need a system message plus a couple
    //     of body messages, else dropping the old part saves nothing.
    if history.len() <= 3 {
        return history;
    }

    // 2c. Post-compaction guard: if the conversation already carries a compaction
    //     summary turn, stacking our own sqlite summary on top would produce a
    //     broken double-summary payload. Fail-open to full history — /compact
    //     already condensed; do not also hard-clip the live post-compact tail.
    const COMPACTION_MARKER: &str = "[summary of earlier conversation]";
    if history.len() >= 2 {
        if let Some(msg) = history.get(1) {
            if msg.role == Role::Assistant && msg.content.starts_with(COMPACTION_MARKER) {
                return history;
            }
        }
    }

    // 3. Best-effort fold so the summary reflects everything older than the tail.
    //    Token-band OR message-count trigger (see update_summary). Errors / "nothing
    //    to fold" are ignored — we use whatever summary already exists. Skipped when
    //    the Awareness role doesn't resolve (no `route`).
    if let Some(route) = route.as_ref() {
        let _ = update_summary(session_dir, client, route, usable, tail_cap).await;
    }

    // 4. Read summary. Empty/missing → fail-open (full history). Never starve the
    //    agent with a stub clip while the fold is still catching up.
    let sum = msglog::read_summary(session_dir);
    let has_summary = sum
        .as_ref()
        .map(|s| !s.text.trim().is_empty())
        .unwrap_or(false);

    if !has_summary {
        return history;
    }
    let sum = sum.expect("has_summary implies Some");

    // Everything newer than the summary boundary must stay verbatim, but hard-cap
    // at settings.short_send_tail_n so the wire stays bounded even if the fold
    // watermark lags. Clamp to >= 1 so we always keep at least one message.
    let max_id = msglog::max_message_id(session_dir);
    let after_wm = (max_id.saturating_sub(sum.covers_up_to)).max(1) as usize;
    let keep = after_wm.min(tail_cap).max(1);

    // 5. Split into [system, body...].
    let Some((mut system, tail)) = clip_body(&history, keep) else {
        return history;
    };

    // 6. Rehydrate. Candidate blobs are those in the SUMMARISED region
    //    (msg_id <= covers_up_to), i.e. NOT in the verbatim tail — the tail still
    //    carries its own heavy content in full.
    //
    //    Two recall paths, content-search FIRST:
    //    6a. CONTENT MATCH (the "db lookup"): pull significant terms from the
    //        user's message and ask sqlite which summarised blobs' OWNING MESSAGE
    //        text matches — independent of the (possibly border-first) snippet. A
    //        literal content hit is a strong signal, so the top up-to-3 are
    //        rehydrated DIRECTLY, no router round-trip.
    //    6b. FALLBACK: only when content search finds NOTHING do we ask the
    //        snippet router (`pick_blobs`) — for semantic queries with no keyword
    //        overlap, where the snippet is still the best (only) signal.
    //    Either way the rehydrated set is capped at MAX_REHYDRATE and formatted as
    //    the existing `[recalled blob #<id>]\n{content}` blocks.
    let mut recalls: Vec<String> = Vec::new();
    let candidates: Vec<msglog::BlobRef> = msglog::list_blobs(session_dir)
        .into_iter()
        .filter(|b| b.msg_id <= sum.covers_up_to)
        .collect();

    // 6a. Content search over message text. `search_blobs` already filters to
    //     msg_id <= covers_up_to and ranks by distinct-term-match count desc.
    let terms = significant_terms(user_intent);
    let content_hits: Vec<msglog::BlobRef> = if terms.is_empty() {
        Vec::new()
    } else {
        msglog::search_blobs(session_dir, &terms, sum.covers_up_to)
    };

    if !content_hits.is_empty() {
        // Direct rehydrate: a literal content match needs no router confirmation.
        for hit in content_hits.iter().take(MAX_REHYDRATE) {
            if let Some(content) = msglog::fetch_blob_content(session_dir, hit.msg_id) {
                recalls.push(format!("\n\n[recalled blob #{}]\n{}", hit.id, content));
            }
        }
    } else if let (false, Some(route)) = (candidates.is_empty(), route.as_ref()) {
        // 6b. Fallback to the snippet router only when content search came up empty
        //     AND the Awareness role resolved (the router rides that route). With no
        //     route we rehydrate nothing via the router — content-search recalls
        //     (6a) above are unaffected.
        let payload = build_router_payload(user_intent, &candidates);
        // Best-effort: `pick_blobs` already returns an empty vec on any error.
        let picked = client
            .pick_blobs(
                route.conn(),
                &route.model_id,
                route.provider(),
                resources::shortsend_router_prompt(),
                &payload,
            )
            .await
            .unwrap_or_default();
        for id in picked {
            if recalls.len() >= MAX_REHYDRATE {
                break; // cap rehydration so the router can't undo the compression
            }
            // The router returns `blobs.id` values (the ids shown in the payload).
            // Map back to the candidate to (a) reject any id we never offered
            // (guards a hallucinated id) and (b) resolve its `msg_id` — full blob
            // content lives in the `messages` row, so `fetch_blob_content` keys on
            // the message id, not the blob id. Skip any whose content is missing.
            let Some(cand) = candidates.iter().find(|c| c.id == id) else {
                continue;
            };
            if let Some(content) = msglog::fetch_blob_content(session_dir, cand.msg_id) {
                recalls.push(format!("\n\n[recalled blob #{id}]\n{content}"));
            }
        }
    }

    // 7. B-PLACEMENT: the summary goes into the SYSTEM message tail, NOT a
    //    synthetic assistant turn. `history[0]` already ends (after
    //    CACHE_SPLIT_MARK) with the volatile dir-listing/awareness block; append
    //    the condensed-history summary, then the rehydrated blob blocks, after it.
    //    Because all of this lands AFTER the mark, it rides in the UNCACHED tail —
    //    correct, since the summary changes per fold and must not bust the cached
    //    head.
    system.content.push_str(&format!(
        "\n\n# Conversation so far (reference — earlier turns, condensed)\n{}",
        sum.text
    ));
    for block in &recalls {
        system.content.push_str(block);
    }

    // 8. Output: [ modified system (index 0), verbatim tail... ]. No synthetic
    //    assistant summary turn — index 0 stays the system message so `to_wire`'s
    //    mark-split still applies.
    let mut out: Vec<ChatMessage> = Vec::with_capacity(1 + tail.len());
    out.push(system);
    out.extend(tail);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::chat::{ChatMessage, Role};
    use crate::model::settings::Settings;

    fn msg(role: Role, content: &str) -> ChatMessage {
        ChatMessage::new(role, content)
    }

    fn long_history(n_body: usize) -> Vec<ChatMessage> {
        let mut h = vec![msg(Role::System, "system")];
        for i in 0..n_body {
            let role = if i % 2 == 0 { Role::User } else { Role::Assistant };
            h.push(msg(role, &format!("body-{i}")));
        }
        h
    }

    #[test]
    fn clip_body_respects_tail_cap() {
        let h = long_history(50);
        let (_, tail) = clip_body(&h, 10).expect("clip");
        assert_eq!(tail.len(), 10);
        assert_eq!(tail[0].content, "body-40");
        assert_eq!(tail[9].content, "body-49");
    }

    #[test]
    fn clip_body_clamps_to_available() {
        let h = long_history(3);
        let (_, tail) = clip_body(&h, 10).expect("clip");
        assert_eq!(tail.len(), 3);
    }

    #[test]
    fn tail_n_cap_clamps_at_one() {
        let mut s = Settings::default();
        s.short_send_tail_n = 0;
        assert_eq!(tail_n_cap(&s), 1);
        s.short_send_tail_n = 40;
        assert_eq!(tail_n_cap(&s), 40);
    }

    #[test]
    fn default_tail_is_generous() {
        let s = Settings::default();
        assert!(tail_n_cap(&s) >= 40, "default tail must not starve agent turns");
    }
}
