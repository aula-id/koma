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
/// valid completed tool-round boundary) — so the caller can skip the write
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

    // Snap the cut DOWN to a completed tool-round edge. Live work stays
    // verbatim: the open assistant+tool chain (or the trailing assistant / user
    // if no tools yet). Older *finished* assistant+tool rounds in the same
    // kickoff MAY fold — a one-user agentic loop is the normal DRSS case, not
    // a reason to no-op. Never open mid-chain (assistant followed by its tools).
    let fold_up_to = match fold_boundary_id(&tail, cut_id, covers_up_to) {
        Some(b) => b,
        None => return Ok(false), // nothing settled before the live turn
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

/// Start id of the live tool-round / live turn (must stay verbatim).
///
/// - last `tool` → walk back to the assistant that opened the chain
/// - last `assistant` or `user` → that message
pub(super) fn live_turn_start_id(msgs: &[msglog::ArchivedMsg]) -> Option<i64> {
    let last = msgs.last()?;
    if last.role.eq_ignore_ascii_case("tool") {
        for m in msgs.iter().rev() {
            if m.role.eq_ignore_ascii_case("assistant") {
                return Some(m.id);
            }
            if m.role.eq_ignore_ascii_case("user") {
                return Some(m.id);
            }
        }
        return Some(last.id);
    }
    Some(last.id)
}

/// True when `m` closes a settled round relative to `next` (next may be the
/// live assistant — a tool followed by that assistant still closes).
fn closes_round(m: &msglog::ArchivedMsg, next: Option<&msglog::ArchivedMsg>) -> bool {
    let r = m.role.as_str();
    let n = next.map(|x| x.role.as_str());
    match (r, n) {
        // Last tool of a chain, next thought / next user request.
        ("tool", Some("assistant") | Some("user")) => true,
        // Text-only assistant turn (no pending tools).
        ("assistant", Some("user") | Some("assistant")) => true,
        // User is never a close by itself — folding "just the question"
        // eats cognition. A later completed tool-round may still cover it.
        _ => false,
    }
}

/// Largest id ≤ `cut_id` that closes a completed tool-round / utterance and
/// is strictly before the live turn. `None` if the live turn is the whole tail.
pub(super) fn fold_boundary_id(
    msgs: &[msglog::ArchivedMsg],
    cut_id: i64,
    covers_up_to: i64,
) -> Option<i64> {
    let live_start = live_turn_start_id(msgs)?;
    msgs.iter()
        .enumerate()
        .filter_map(|(i, m)| {
            if m.id <= covers_up_to || m.id > cut_id || m.id >= live_start {
                return None;
            }
            if closes_round(m, msgs.get(i + 1)) {
                Some(m.id)
            } else {
                None
            }
        })
        .max()
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

    fn am(id: i64, role: &str) -> msglog::ArchivedMsg {
        msglog::ArchivedMsg {
            id,
            role: role.into(),
            content: String::new(),
            reasoning: None,
        }
    }

    /// One-user CyberGym loop: U, A1, T1, A2, T2. Live = A2+T2. Fold through T1.
    #[test]
    fn intra_exchange_folds_completed_tool_round() {
        let msgs = vec![
            am(1, "user"),
            am(2, "assistant"),
            am(3, "tool"),
            am(4, "assistant"),
            am(5, "tool"),
        ];
        assert_eq!(live_turn_start_id(&msgs), Some(4));
        assert_eq!(fold_boundary_id(&msgs, 5, 0), Some(3));
    }

    /// Multi-tool chain: do not cut between T1a and T1b.
    #[test]
    fn intra_exchange_does_not_split_tool_chain() {
        let msgs = vec![
            am(1, "user"),
            am(2, "assistant"),
            am(3, "tool"),
            am(4, "tool"),
            am(5, "assistant"),
            am(6, "tool"),
        ];
        assert_eq!(live_turn_start_id(&msgs), Some(5));
        assert_eq!(fold_boundary_id(&msgs, 6, 0), Some(4));
    }

    /// Kickoff + live assistant, no settled tool-round yet — don't fold the question.
    #[test]
    fn no_boundary_when_only_live_turn() {
        let msgs = vec![am(1, "user"), am(2, "assistant")];
        assert_eq!(live_turn_start_id(&msgs), Some(2));
        assert_eq!(fold_boundary_id(&msgs, 2, 0), None);
    }

    /// Live is the user (nothing after it) — do not fold that user.
    #[test]
    fn no_boundary_when_user_is_live() {
        let msgs = vec![
            am(1, "user"),
            am(2, "assistant"),
            am(3, "tool"),
            am(4, "user"),
        ];
        assert_eq!(live_turn_start_id(&msgs), Some(4));
        assert_eq!(fold_boundary_id(&msgs, 4, 0), Some(3));
    }

    /// Two user exchanges: previous exchange still foldable; live tool-round kept.
    #[test]
    fn prior_user_exchange_still_folds() {
        let msgs = vec![
            am(1, "user"),
            am(2, "assistant"),
            am(3, "tool"),
            am(4, "user"),
            am(5, "assistant"),
            am(6, "tool"),
        ];
        assert_eq!(live_turn_start_id(&msgs), Some(5));
        // Close at T1 (id 3), not at U2 — current question stays verbatim.
        assert_eq!(fold_boundary_id(&msgs, 6, 0), Some(3));
    }

    /// cut_id below the only closed round → none.
    #[test]
    fn boundary_respects_cut_and_watermark() {
        let msgs = vec![
            am(1, "user"),
            am(2, "assistant"),
            am(3, "tool"),
            am(4, "assistant"),
            am(5, "tool"),
        ];
        assert_eq!(fold_boundary_id(&msgs, 2, 0), None);
        assert_eq!(fold_boundary_id(&msgs, 5, 3), None);
    }
}
