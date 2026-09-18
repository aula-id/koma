//! Phase-2 fold: merge newly-archived messages into the rolling summary.
//!
//! ## Bleed guard (critical)
//!
//! The fold is a SECOND LLM call, and its output is PERSISTED and replayed into
//! every future send-payload. A leaked chain-of-thought here is therefore strictly
//! worse than a transient bleed — it poisons the conversation permanently. The
//! request runs with reasoning explicitly OFF (see
//! [`OpenRouterClient::summarize_fold`]) and the fold prompt instructs the model
//! to emit ONLY the summary. We only ever read already-clean message `content`
//! from sqlite (the reasoning channel is never stored there), so nothing on this
//! path can introduce model thinking into the archive.
//!
//! Everything is best-effort and append-only: we read messages/blobs and upsert
//! the single summary row via the Phase-1 helpers; the `messages` table is never
//! mutated.

use std::path::Path;

use anyhow::Result;

use crate::app::resolve::Resolved;
use crate::model::msglog;
use crate::resources;
use crate::service::openrouter::OpenRouterClient;

/// After a fold, the verbatim tail is shrunk down to roughly this % of `usable`.
pub(super) const TAIL_FLOOR_PCT: u64 = 5;
/// Once the verbatim tail grows past this % of `usable`, refold (advance the
/// watermark). Below it the fold is a no-op — the hysteresis dead-zone that
/// avoids a summarizer call every single turn (unless the message-count path
/// fires — see `tail_n`).
pub(super) const TAIL_HI_PCT: u64 = 15;

/// Upper bound on how many delta messages to pull in one fold. Large enough that
/// a normal session's un-summarised backlog fits in a single pass; the verbatim
/// tail is excluded separately by the `fold_up_to` cap.
const DELTA_LIMIT: i64 = 10_000;

/// Max characters kept from any single message's content when building the fold
/// payload. Bounds the secondary call so one giant message can't blow its
/// context; heavy content is referenced via blobs anyway, not pasted in full.
const PER_MESSAGE_CAP: usize = 6000;

/// Take at most `cap` chars from `s` (char-boundary safe).
fn cap_chars(s: &str, cap: usize) -> String {
    s.chars().take(cap).collect()
}

/// Fold newly-archived messages into the session's rolling summary, advancing the
/// summary watermark by a TOKEN BAND and/or a MESSAGE COUNT rather than a fixed
/// message count alone.
///
/// `usable = context_window - BASE_OVERHEAD` is the budget the percentages are
/// taken against. The verbatim tail (messages after the current watermark) is
/// allowed to grow up to [`TAIL_HI_PCT`] of `usable`; only when it crosses that
/// high-water mark (OR exceeds `tail_n` messages) do we fold, and we fold just
/// enough that the REMAINING tail drops back to ~[`TAIL_FLOOR_PCT`] **and** ≤
/// `tail_n` messages. This hysteresis dead-zone means we do NOT pay for a
/// summarizer call on every turn — only when the tail has genuinely grown past
/// the band or the settings-driven message cap.
///
/// `tail_n` is `settings.short_send_tail_n` (≥ 1): the max verbatim body messages
/// the wire keeps when engaged. Count path does not need a correct window.
///
/// Returns `Ok(true)` when a fold happened (a new summary was written) and
/// `Ok(false)` when there was nothing to fold (tail still within band, or no
/// valid completed-exchange boundary) — so the caller can skip the write
/// entirely. Errors propagate from the secondary model call; the sqlite helpers
/// are best-effort and degrade to empty rather than erroring.
pub async fn update_summary(
    session_dir: &Path,
    client: &OpenRouterClient,
    route: &Resolved,
    usable: u64,
    tail_n: usize,
) -> Result<bool> {
    update_summary_inner(session_dir, client, route, usable, tail_n, false).await
}

/// Like [`update_summary`], but when `force` is true skip the band no-op so an
/// objective transition can refresh the continuity log once (still no-ops when
/// there is nothing foldable past the watermark).
pub async fn update_summary_forced(
    session_dir: &Path,
    client: &OpenRouterClient,
    route: &Resolved,
    usable: u64,
    tail_n: usize,
) -> Result<bool> {
    update_summary_inner(session_dir, client, route, usable, tail_n, true).await
}

async fn update_summary_inner(
    session_dir: &Path,
    client: &OpenRouterClient,
    route: &Resolved,
    usable: u64,
    tail_n: usize,
    force: bool,
) -> Result<bool> {
    let tail_n = tail_n.max(1);

    // Existing summary state. Absent row (first ever fold) → empty text, covers 0.
    let cur = msglog::read_summary(session_dir);
    let existing_text = cur.as_ref().map(|s| s.text.as_str()).unwrap_or("");
    let covers_up_to = cur.as_ref().map(|s| s.covers_up_to).unwrap_or(0);

    // The verbatim tail = every message after the current watermark. Measure its
    // token cost (~4 chars/token over content) AND message count to decide whether
    // it has grown out of band. (fetch returns id ASC, id > covers_up_to.)
    let tail: Vec<msglog::ArchivedMsg> =
        msglog::fetch_messages_since(session_dir, covers_up_to, DELTA_LIMIT);
    if tail.is_empty() {
        return Ok(false);
    }
    let tok = |s: &str| s.chars().count() as u64 / 4;
    let tail_tokens: u64 = tail.iter().map(|m| tok(&m.content)).sum();
    let tail_msgs = tail.len();

    // Hysteresis dead-zone: the tail is still within BOTH token band and message
    // cap → no fold this turn (unless force_fold on objective transition).
    let tail_hi = TAIL_HI_PCT * usable / 100;
    let over_tokens = tail_tokens > tail_hi;
    let over_count = tail_msgs > tail_n;
    if !force && !over_tokens && !over_count {
        return Ok(false);
    }

    // Pick the cut so the REMAINING verbatim tail is ~TAIL_FLOOR_PCT of usable
    // AND ≤ tail_n messages. Walk the tail NEWEST→oldest accumulating tokens and
    // counting kept messages; stop when BOTH floors are satisfied (or we run out).
    let tail_floor = (TAIL_FLOOR_PCT * usable / 100).max(1);
    let mut kept_tok = 0u64;
    let mut kept_n = 0usize;
    // Default the cut to "fold the whole tail" (cut at the newest id); the loop
    // below raises it to the boundary where kept-newest hits the floors.
    let mut cut_id = tail.last().map(|m| m.id).unwrap_or(covers_up_to);
    for m in tail.iter().rev() {
        kept_tok += tok(&m.content);
        kept_n += 1;
        // Need enough token budget AND message budget before we stop keeping.
        // When only the count path fired (over_count, tokens still small), the
        // token floor may never be reached on a short-message tail — so also
        // accept once kept_n has reached tail_n (message target met).
        let tok_ok = kept_tok >= tail_floor;
        let n_ok = kept_n >= tail_n;
        if (tok_ok && n_ok) || (over_count && !over_tokens && n_ok) || (over_tokens && !over_count && tok_ok) {
            // `m` is the oldest message we KEEP; fold everything strictly before it.
            cut_id = m.id - 1;
            break;
        }
    }
    // If the loop exhausted without breaking, cut_id stayed at newest — fold all
    // but leave nothing? Prefer leaving at least the last tail_n messages.
    if cut_id == tail.last().map(|m| m.id).unwrap_or(covers_up_to) && tail_msgs > tail_n {
        // Keep the newest tail_n; fold the rest.
        let keep_from = tail_msgs.saturating_sub(tail_n);
        if let Some(m) = tail.get(keep_from) {
            cut_id = m.id - 1;
        }
    }

    // Snap the cut DOWN to a COMPLETED-exchange edge so a tool-call chain is never
    // cut and the current, in-progress exchange is never summarised. An exchange
    // runs from one `user` message up to (but not including) the next; the most
    // recent `user` message begins the live exchange, which must stay verbatim.
    // Valid boundaries are `(user_id) - 1`: pick the LARGEST that is <= the walked
    // cut, strictly > covers_up_to (so we actually advance), and < the last user
    // message id (so the live exchange is never folded).
    let user_ids = msglog::user_message_ids(session_dir); // ascending
    let last_user = user_ids.last().copied().unwrap_or(i64::MAX);
    let fold_up_to = match user_ids
        .iter()
        .copied()
        .filter(|&u| u < last_user) // never fold the in-progress (last) exchange
        .map(|u| u - 1)
        .filter(|&b| b <= cut_id && b > covers_up_to)
        .max()
    {
        Some(b) => b,
        None => return Ok(false), // no completed-exchange boundary in range → don't fold
    };

    // Pull everything after the last-covered id, then trim to the fold ceiling so
    // the verbatim tail stays out. (fetch returns id ASC, id > covers_up_to.)
    let delta: Vec<msglog::ArchivedMsg> =
        msglog::fetch_messages_since(session_dir, covers_up_to, DELTA_LIMIT)
            .into_iter()
            .filter(|m| m.id <= fold_up_to)
            .collect();
    if delta.is_empty() {
        return Ok(false);
    }

    // Blobs whose owning message falls in the delta range (covers_up_to, fold_up_to].
    // These are the heavy items the new messages may reference; older blobs were
    // already folded into the existing summary, newer ones belong to the tail.
    let blobs: Vec<msglog::BlobRef> = msglog::list_blobs(session_dir)
        .into_iter()
        .filter(|b| b.msg_id > covers_up_to && b.msg_id <= fold_up_to)
        .collect();

    let user_payload = build_payload(existing_text, &delta, &blobs);

    // Short-send rides the resolved Awareness route (its connection + model +
    // upstream-route slug), resolved by the caller. `summarize_fold` treats an
    // empty provider as default routing.
    let new_text = client
        .summarize_fold(
            route.conn(),
            &route.model_id,
            Some(route.provider()),
            resources::shortsend_summary_prompt(),
            &user_payload,
        )
        .await?;

    // Belt-and-suspenders: strip ANSI escape codes and tool-call tags that the
    // model may have echoed back before persisting. A dirty summary would poison
    // every future send-payload and every subsequent fold input.
    let new_text = crate::dto::chat::strip_tool_call_tags(&crate::dto::chat::strip_ansi(&new_text));

    // Persist: the new summary now covers through `fold_up_to`; the live-send
    // start id is the first message past it.
    msglog::write_summary(session_dir, &new_text, fold_up_to, fold_up_to + 1)?;
    Ok(true)
}

/// Assemble the plain-text fold payload: three labeled sections the prompt
/// expects — the existing summary, the new messages to merge (each capped), and
/// the available blob references the summary may point at instead of inlining.
pub(super) fn build_payload(
    existing_text: &str,
    delta: &[msglog::ArchivedMsg],
    blobs: &[msglog::BlobRef],
) -> String {
    let mut out = String::new();

    out.push_str("=== EXISTING SUMMARY ===\n");
    let existing = existing_text.trim();
    if existing.is_empty() {
        out.push_str("(none)\n");
    } else {
        out.push_str(existing);
        out.push('\n');
    }

    out.push_str("\n=== NEW MESSAGES ===\n");
    for m in delta {
        // `role: content`, with any single message bounded so the call stays small.
        let content = cap_chars(m.content.trim(), PER_MESSAGE_CAP);
        out.push_str(&m.role);
        out.push_str(": ");
        out.push_str(&content);
        out.push_str("\n\n");
    }

    out.push_str("=== AVAILABLE BLOBS ===\n");
    if blobs.is_empty() {
        out.push_str("(none)\n");
    } else {
        for b in blobs {
            // `#<id> [<kind>] <snippet>` — the id the summary references as
            // `[blob #<id>]` instead of pasting the heavy content.
            out.push('#');
            out.push_str(&b.id.to_string());
            out.push_str(" [");
            out.push_str(&b.kind);
            out.push_str("] ");
            out.push_str(b.snippet.trim());
            out.push('\n');
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_chars_respects_limit() {
        let s = "abcdefghij";
        assert_eq!(cap_chars(s, 4), "abcd");
        assert_eq!(cap_chars(s, 100), s);
    }

    #[test]
    fn build_payload_empty_sections() {
        let p = build_payload("", &[], &[]);
        assert!(p.contains("(none)"));
        assert!(p.contains("=== EXISTING SUMMARY ==="));
        assert!(p.contains("=== NEW MESSAGES ==="));
        assert!(p.contains("=== AVAILABLE BLOBS ==="));
    }
}
