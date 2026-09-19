//! Phase-3 send-path reshaper: build the short-send wire payload.
//!
//! [`shape`] is a PURE transform over the API-bound history. When engaged **and** a
//! usable rolling summary exists, the wire is:
//!
//! ```text
//! system + priority contract + optional charter + current objective
//!        + continuity log + labeled archive (FTS dialogue excerpts + safe blob recalls)
//! hot body transcript (token/exchange window, floor = short_send_tail_n)
//! ```
//!
//! Priority: **live tail > current objective > continuity log > archive**. Missing
//! summary fail-opens to full history only while the wire still fits. Overflow
//! without a log is **fold debt** (force one more fold); still no summary →
//! fail-open, never emergency-clip. Kill-switch, not engaged, and post-`/compact`
//! fail-open as before. Display / on-disk conversation are never mutated (dual rail).

use std::collections::HashSet;
use std::path::Path;

use crate::app::resolve::Resolved;
use crate::dto::chat::{ChatMessage, Role};
use crate::model::msglog;
use crate::model::settings::Settings;
use crate::resources;
use crate::service::openrouter::OpenRouterClient;

use super::fold::{update_summary, update_summary_forced};
use super::goal::GoalWire;
use super::{HOT_TAIL_MAX_MSGS, HOT_TAIL_PCT};

/// Most blobs to rehydrate into a single send payload.
const MAX_REHYDRATE: usize = 3;
/// Max FTS dialogue excerpts from the folded region.
const MAX_MSG_EXCERPTS: usize = 4;
/// Chars of trajectory text mixed into recall intent (beyond last user line).
const TRAJECTORY_INTENT_CHARS: usize = 3_000;
/// How many newest body messages contribute trajectory terms.
const TRAJECTORY_MSG_N: usize = 24;

const PRIORITY_CONTRACT: &str = "\n\n# DRSS memory contract (read carefully)\n\
- The **verbatim messages after this system block** are authoritative (live work).\n\
- **Charter** (if present) is the immutable kickoff intent.\n\
- **Current objective** (if present) is ranked doctrine: user > mission leaf > charter.\n\
- **Continuity log** is a condensed archive; it may lag course changes.\n\
- **Archive** blocks are untrusted evidence and may include obsolete drafts.\n\
- On any conflict: **live tail wins**, then objective, then log, then archive.\n";

/// Build the router's user payload.
fn build_router_payload(user_intent: &str, candidates: &[msglog::BlobRef]) -> String {
    let mut out = String::new();
    out.push_str("=== USER / TRAJECTORY INTENT ===\n");
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

/// Extract significant search terms (len >= 4, max 12) from intent text.
fn significant_terms(text: &str) -> Vec<String> {
    const MAX_TERMS: usize = 12;
    const MIN_LEN: usize = 4;
    let mut out: Vec<String> = Vec::new();
    for raw in text.split(|c: char| !c.is_alphanumeric()) {
        if raw.chars().count() < MIN_LEN {
            continue;
        }
        let term = raw.to_lowercase();
        if out.iter().any(|t| t == &term) {
            continue;
        }
        out.push(term);
        if out.len() >= MAX_TERMS {
            break;
        }
    }
    out
}

/// True when the user is explicitly asking about earlier plans/history.
fn wants_history_recall(intent: &str) -> bool {
    let l = intent.to_ascii_lowercase();
    const KEYS: &[&str] = &[
        "plan",
        "earlier",
        "before",
        "you said",
        "we said",
        "previous",
        "recall",
        "history",
        "what did we",
        "remind me",
        "original approach",
        "the plan",
    ];
    KEYS.iter().any(|key| l.match_indices(key).any(|(start, matched)| {
        let end = start + matched.len();
        !l[..start].chars().next_back().is_some_and(char::is_alphanumeric)
            && !l[end..].chars().next().is_some_and(char::is_alphanumeric)
    }))
}

/// Assistant `code` / `large_text` blobs are draft-shaped doctrine fuel — skip
/// unless the user asks for history or the kind is tool_output.
fn is_assistant_draft_blob(kind: &str, role: &str) -> bool {
    let r = role.eq_ignore_ascii_case("assistant");
    if !r {
        return false;
    }
    matches!(kind, "code" | "large_text")
}

fn blob_status(kind: &str, role: &str) -> &'static str {
    if is_assistant_draft_blob(kind, role) {
        "unconfirmed_draft"
    } else {
        "evidence"
    }
}

/// Format a labeled archive blob block for system inject.
fn format_blob_block(id: i64, msg_id: i64, kind: &str, role: &str, content: &str) -> String {
    let status = blob_status(kind, role);
    format!(
        "\n\n[archive blob #{id} | role={role} | msg_id={msg_id} | kind={kind} | status={status}]\n{content}"
    )
}

/// Format a labeled FTS dialogue excerpt.
fn format_msg_excerpt(id: i64, role: &str, excerpt: &str) -> String {
    let status = if role.eq_ignore_ascii_case("assistant") {
        "unconfirmed_draft"
    } else {
        "evidence"
    };
    format!(
        "\n\n[archive msg #{id} | role={role} | status={status}]\n{excerpt}"
    )
}

fn tail_n_cap(settings: &Settings) -> usize {
    (settings.short_send_tail_n.max(1) as usize).max(1)
}

fn msg_tok_est(m: &ChatMessage) -> u64 {
    let base = m.content.chars().count() as u64 / 4;
    let args: u64 = m
        .tool_calls
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .map(|tc| tc.function.arguments.chars().count() as u64 / 4)
        .sum();
    base + args
}

/// Split `history` into system + last `keep` body messages.
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

/// Prefer `tail_floor` messages within the hot token budget, but always retain
/// the entire span newer than the persisted summary. Token/count preferences
/// may trim already summarized messages; they cannot create a coverage gap.
fn hot_keep_n(body: &[ChatMessage], after_wm: usize, tail_floor: usize, usable: u64) -> usize {
    if body.is_empty() {
        return 1;
    }
    let n = body.len();
    let floor = tail_floor.max(1).min(n);
    let wm_keep = after_wm.max(1).min(n);
    // Preference only — token budget may shrink below this.
    let mut keep = floor.max(wm_keep).min(n);

    let budget = (HOT_TAIL_PCT.saturating_mul(usable) / 100).max(1);
    let max_msgs = HOT_TAIL_MAX_MSGS.min(n);

    // Grow from `keep` toward max_msgs while under token budget.
    while keep < max_msgs {
        let start = n - (keep + 1);
        let slice = &body[start..];
        let toks: u64 = slice.iter().map(msg_tok_est).sum();
        if toks > budget {
            break;
        }
        keep += 1;
    }

    // A failed/unavailable fold leaves a larger uncovered span. Keep it intact
    // even when it exceeds the budget; only a successful fold may replace it.
    while keep > wm_keep {
        let start = n - keep;
        let toks: u64 = body[start..].iter().map(msg_tok_est).sum();
        if toks <= budget {
            break;
        }
        keep -= 1;
    }

    keep = keep.max(1).min(n);
    snap_keep_to_round(body, keep)
}

/// Expand the window back to the opening assistant when the cut lands inside
/// a tool round. Never discard part of the round to satisfy a token preference;
/// this also preserves the assistant's tool arguments and replay metadata.
fn snap_keep_to_round(body: &[ChatMessage], keep: usize) -> usize {
    let n = body.len();
    if n == 0 {
        return 1;
    }
    let mut start = n.saturating_sub(keep.max(1));
    while start > 0 && matches!(body[start].role, Role::Tool | Role::System) {
        start -= 1;
    }
    n - start
}

/// ~1000 tokens at 4 chars/token. One fat `cat` / tool dump on the hot tail.
const WIRE_STUB_TOKENS: u64 = 1_000;

/// Replace an over-budget tool/assistant body with a pointer + snippet.
/// User messages are never stubbed (the live question stays verbatim).
/// Dual rail: full text remains on disk / in the conversation.
fn stub_one_message(m: &mut ChatMessage, blob: Option<&msglog::BlobRef>) {
    if m.role == Role::User {
        return;
    }
    if msg_tok_est(m) < WIRE_STUB_TOKENS {
        return;
    }
    // Without an indexed source there is no reliable way to recover the body.
    let Some(blob) = blob else { return };
    let snippet: String = match blob.snippet.trim() {
        s if !s.is_empty() => s.to_string(),
        _ => m.content.chars().take(250).collect(),
    };
    let head = format!(
        "[wire stub | blob #{} | msg_id={} | kind={} | status=evidence]\n",
        blob.id, blob.msg_id, blob.kind
    );
    m.content = format!(
        "{head}{snippet}\n(full body on stored rail; read with \
         message_find({{\"message_id\":{},\"offset\":0,\"limit\":3000}}), \
         then follow next_offset until null)", blob.msg_id
    );
}

/// Stub fat tool/assistant bodies on the hot tail so one file dump cannot
/// eat `HOT_TAIL_PCT`. Matches sqlite blobs by exact content when present.
fn stub_heavy_wire_tail(tail: &mut [ChatMessage], session_dir: &Path) {
    let blobs = msglog::list_blobs(session_dir);
    let max_id = msglog::max_message_id(session_dir);
    let archived = if max_id > 0 {
        msglog::fetch_messages_since(
            session_dir,
            max_id.saturating_sub(tail.len() as i64 + 32),
            tail.len() as i64 + 64,
        )
    } else {
        Vec::new()
    };
    for t in tail.iter_mut() {
        let role = match t.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };
        let blob = archived
            .iter()
            .rev()
            .find(|a| a.content == t.content && a.role == role)
            .and_then(|a| blobs.iter().find(|b| b.msg_id == a.id));
        stub_one_message(t, blob);
    }
}

/// Build recall intent: last user line + recent body trajectory (truncated).
pub fn build_recall_intent(history: &[ChatMessage], last_user: &str) -> String {
    let mut out = String::new();
    out.push_str(last_user.trim());
    let body = if history.len() > 1 {
        &history[1..]
    } else {
        return out;
    };
    if body.is_empty() {
        return out;
    }
    out.push_str("\n\n--- recent trajectory ---\n");
    let mut budget = TRAJECTORY_INTENT_CHARS;
    let mut lines = Vec::new();
    // Allocate from the newest work backward, then render chronologically.
    // Otherwise several large older messages consume the entire budget.
    for m in body.iter().rev().filter(|m| m.role != Role::System).take(TRAJECTORY_MSG_N) {
        if budget == 0 {
            break;
        }
        let role = match m.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
            Role::System => "system",
        };
        let mut line = format!("{role}: ");
        let framing = line.chars().count() + 1;
        if budget < framing {
            break;
        }
        // Reserve the separator before truncation so the earliest partial line
        // cannot run into the next message when chronological order is restored.
        let content: String = m.content.chars().take((budget - framing).min(800)).collect();
        line.push_str(&content);
        line.push('\n');
        budget = budget.saturating_sub(line.chars().count());
        lines.push(line);
    }
    for line in lines.iter().rev() {
        out.push_str(line);
    }
    out
}

/// Look up role string for a message id (best-effort).
fn role_for_msg(session_dir: &Path, msg_id: i64) -> String {
    msglog::fetch_message_role(session_dir, msg_id).unwrap_or_else(|| "unknown".into())
}

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
    force_fold: bool,
    goal_wire: &GoalWire,
) -> Vec<ChatMessage> {
    if !settings.short_send_enabled {
        return history;
    }
    if !summarizing {
        return history;
    }

    let tail_floor = tail_n_cap(settings);

    if history.len() <= 3 {
        return history;
    }

    const COMPACTION_MARKER: &str = "[summary of earlier conversation]";
    if history.len() >= 2 {
        if let Some(msg) = history.get(1) {
            if msg.role == Role::Assistant && msg.content.starts_with(COMPACTION_MARKER) {
                return history;
            }
        }
    }

    // Fold uses floor as message-count trigger (same settings knobs).
    // force_fold: one try after objective transition even if band says no-op.
    if let Some(route) = route.as_ref() {
        if force_fold {
            let _ = update_summary_forced(session_dir, client, route, usable, tail_floor).await;
        } else {
            let _ = update_summary(session_dir, client, route, usable, tail_floor).await;
        }
    }

    let mut sum = msglog::read_summary(session_dir);
    let mut has_summary = sum
        .as_ref()
        .map(|s| !s.text.trim().is_empty())
        .unwrap_or(false);
    // Fold debt: no continuity log AND the full wire would not fit → force
    // one more fold (band no-op / swallowed error). Still no log → fail-open.
    // Never emergency-clip without a summary (that starved cognition before).
    if !has_summary && super::wire_would_overflow(&history, usable) {
        if let Some(route) = route.as_ref() {
            let _ = update_summary_forced(session_dir, client, route, usable, tail_floor).await;
            sum = msglog::read_summary(session_dir);
            has_summary = sum
                .as_ref()
                .map(|s| !s.text.trim().is_empty())
                .unwrap_or(false);
        }
    }
    if !has_summary {
        return history;
    }
    let sum = sum.expect("has_summary");

    let body = &history[1..];
    let max_id = msglog::max_message_id(session_dir);
    let after_wm = (max_id.saturating_sub(sum.covers_up_to)).max(1) as usize;
    let keep = hot_keep_n(body, after_wm, tail_floor, usable);

    let Some((mut system, mut tail)) = clip_body(&history, keep) else {
        return history;
    };
    stub_heavy_wire_tail(&mut tail, session_dir);

    // Only the raw user request grants history recall. Assistant/tool trajectory
    // contributes relevance terms but cannot authorize replaying old drafts.
    let history_ask = wants_history_recall(user_intent);
    let recall_intent = build_recall_intent(&history, user_intent);
    let terms = significant_terms(&recall_intent);

    let mut all_candidates: Vec<msglog::BlobRef> = msglog::list_blobs(session_dir)
        .into_iter()
        .filter(|b| b.msg_id <= sum.covers_up_to)
        .collect();

    let draft_msg_ids: HashSet<i64> = all_candidates.iter()
        .filter(|b| is_assistant_draft_blob(&b.kind, &role_for_msg(session_dir, b.msg_id)))
        .map(|b| b.msg_id)
        .collect();

    // --- archive: FTS dialogue excerpts (folded region only) ---
    let mut archive_blocks: Vec<(i64, String)> = Vec::new();
    let mut excerpt_ids: HashSet<i64> = HashSet::new();

    if !terms.is_empty() {
        let q = terms.join(" ");
        if let Ok(hits) =
            msglog::search_messages_before(session_dir, &q, sum.covers_up_to, MAX_MSG_EXCERPTS as i64)
        {
            for h in hits {
                // Apply the same draft rule to both recall paths.
                if (!history_ask && draft_msg_ids.contains(&h.id)) || excerpt_ids.contains(&h.id) {
                    continue;
                }
                // Prefer user/assistant dialogue over pure tool noise when possible —
                // still allow tool if that's all we get.
                excerpt_ids.insert(h.id);
                archive_blocks.push((h.id, format_msg_excerpt(h.id, &h.role, h.snippet.trim())));
                if archive_blocks.len() >= MAX_MSG_EXCERPTS {
                    break;
                }
            }
        }
    }

    // --- blobs ---
    // Filter draft assistant blobs from auto paths unless history ask.
    let filter_drafts = |cands: Vec<msglog::BlobRef>| -> Vec<msglog::BlobRef> {
        cands
            .into_iter()
            .filter(|b| {
                if history_ask {
                    return true;
                }
                let role = role_for_msg(session_dir, b.msg_id);
                !is_assistant_draft_blob(&b.kind, &role)
            })
            .collect()
    };

    let content_hits: Vec<msglog::BlobRef> = if terms.is_empty() {
        Vec::new()
    } else {
        filter_drafts(msglog::search_blobs(session_dir, &terms, sum.covers_up_to))
    };

    // Excerpts do not count as full recall: successful rehydration upgrades them.
    let mut used_msg_ids: HashSet<i64> = HashSet::new();
    let mut blob_blocks: Vec<String> = Vec::new();
    if !content_hits.is_empty() {
        for hit in &content_hits {
            if blob_blocks.len() >= MAX_REHYDRATE {
                break;
            }
            if used_msg_ids.contains(&hit.msg_id) {
                continue;
            }
            if let Some(content) = msglog::fetch_blob_content(session_dir, hit.msg_id) {
                let role = role_for_msg(session_dir, hit.msg_id);
                used_msg_ids.insert(hit.msg_id);
                blob_blocks.push(format_blob_block(
                    hit.id,
                    hit.msg_id,
                    &hit.kind,
                    &role,
                    &content,
                ));
            }
        }
    } else {
        all_candidates = filter_drafts(all_candidates);
        if let (false, Some(route)) = (all_candidates.is_empty(), route.as_ref()) {
            let payload = build_router_payload(&recall_intent, &all_candidates);
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
                if blob_blocks.len() >= MAX_REHYDRATE {
                    break;
                }
                let Some(cand) = all_candidates.iter().find(|c| c.id == id) else {
                    continue;
                };
                if used_msg_ids.contains(&cand.msg_id) {
                    continue;
                }
                if let Some(content) = msglog::fetch_blob_content(session_dir, cand.msg_id) {
                    let role = role_for_msg(session_dir, cand.msg_id);
                    used_msg_ids.insert(cand.msg_id);
                    blob_blocks.push(format_blob_block(
                        cand.id,
                        cand.msg_id,
                        &cand.kind,
                        &role,
                        &content,
                    ));
                }
            }
        }
    }

    archive_blocks.retain(|(id, _)| !used_msg_ids.contains(id));

    // --- system inject: contract → charter → objective → log → archive ---
    system.content.push_str(PRIORITY_CONTRACT);
    inject_goal_blocks(&mut system.content, goal_wire);

    system.content.push_str(
        "\n\n# Continuity log (condensed archive — live tail wins on conflict)\n",
    );
    system.content.push_str(&sum.text);

    if !archive_blocks.is_empty() || !blob_blocks.is_empty() {
        system.content.push_str(
            "\n\n# Archive (evidence only — may be obsolete; live tail wins)\n",
        );
        for (_, b) in &archive_blocks {
            system.content.push_str(b);
        }
        for b in &blob_blocks {
            system.content.push_str(b);
        }
    }

    let mut out: Vec<ChatMessage> = Vec::with_capacity(1 + tail.len());
    out.push(system);
    out.extend(tail);
    out
}

/// Append charter / objective sections. When both equal, one labeled block.
fn inject_goal_blocks(out: &mut String, wire: &GoalWire) {
    let charter = wire.charter.trim();
    let objective = wire.objective.trim();
    let source = wire.source.trim();

    if charter.is_empty() && objective.is_empty() {
        return;
    }

    if !charter.is_empty() && !objective.is_empty() && charter == objective {
        if source == "charter" || source.is_empty() {
            out.push_str("\n\n# Charter / current objective (kickoff)\n");
        } else {
            out.push_str(&format!("\n\n# Current objective (source={source})\n"));
        }
        out.push_str(objective);
        out.push('\n');
        return;
    }

    if !charter.is_empty() {
        out.push_str("\n\n# Charter (kickoff)\n");
        out.push_str(charter);
        out.push('\n');
    }
    if !objective.is_empty() {
        let src = if source.is_empty() { "none" } else { source };
        out.push_str(&format!("\n\n# Current objective (source={src})\n"));
        out.push_str(objective);
        out.push('\n');
    }
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
            let role = if i % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            };
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
        assert!(tail_n_cap(&s) >= 40);
    }

    #[test]
    fn priority_contract_mentions_tail_wins() {
        assert!(PRIORITY_CONTRACT.contains("live tail wins") || PRIORITY_CONTRACT.contains("Live tail wins") || PRIORITY_CONTRACT.contains("live tail"));
        assert!(PRIORITY_CONTRACT.contains("authoritative"));
    }

    #[test]
    fn format_blob_marks_assistant_draft() {
        let b = format_blob_block(1, 9, "code", "assistant", "fn main() {}");
        assert!(b.contains("unconfirmed_draft"));
        assert!(b.contains("msg_id=9"));
        let t = format_blob_block(2, 3, "tool_output", "tool", "ok");
        assert!(t.contains("status=evidence"));
    }

    #[test]
    fn format_msg_excerpt_labeled() {
        let e = format_msg_excerpt(7, "user", "please fix amnesia");
        assert!(e.contains("archive msg #7"));
        assert!(e.contains("role=user"));
    }

    #[test]
    fn hot_keep_grows_past_floor_for_tiny_msgs() {
        // 80 tiny body msgs; floor 10; huge usable → keep grows toward HOT_TAIL_MAX_MSGS
        let body: Vec<_> = (0..80)
            .map(|i| {
                let role = if i % 2 == 0 {
                    Role::User
                } else {
                    Role::Assistant
                };
                msg(role, "x")
            })
            .collect();
        let k = hot_keep_n(&body, 5, 10, 100_000);
        assert!(k >= 10, "floor");
        assert!(k > 10, "should grow under token budget, got {k}");
        assert!(k <= HOT_TAIL_MAX_MSGS);
    }

    #[test]
    fn hot_keep_clamps_huge_messages() {
        let big = "y".repeat(50_000);
        let body = vec![
            msg(Role::User, &big),
            msg(Role::Assistant, &big),
            msg(Role::User, &big),
            msg(Role::Assistant, &big),
        ];
        let k = hot_keep_n(&body, 1, 4, 8_000);
        assert!(k >= 1);
        // Token budget binds: four 12k-token msgs cannot all stay.
        assert!(k <= 2, "budget must shrink below floor, got {k}");
    }

    #[test]
    fn hot_keep_floor_does_not_win_over_budget() {
        // 40 × ~500-token msgs, floor 40, usable 8k → budget 2k. Must shrink.
        let body: Vec<_> = (0..40)
            .map(|i| {
                let role = if i % 2 == 0 {
                    Role::User
                } else {
                    Role::Assistant
                };
                msg(role, &"z".repeat(2_000))
            })
            .collect();
        // Only one message is newer than the summary. The other 39 are optional.
        let k = hot_keep_n(&body, 1, 40, 8_000);
        assert!(k < 40, "floor must yield to token budget, got {k}");
        assert!(k >= 1);
    }

    #[test]
    fn stub_skips_user_and_small_tool() {
        let mut u = msg(Role::User, &"u".repeat(8_000));
        stub_one_message(&mut u, None);
        assert!(u.content.starts_with("u"), "user must stay verbatim");

        let mut t = msg(Role::Tool, "ok");
        stub_one_message(&mut t, None);
        assert_eq!(t.content, "ok");
    }

    #[test]
    fn stub_rewrites_fat_tool() {
        let mut t = msg(Role::Tool, &"dump".repeat(2_000));
        let blob = msglog::BlobRef {
            id: 7, msg_id: 7, kind: "tool_output".into(),
            token_est: 2000, snippet: "dump preview".into(),
        };
        stub_one_message(&mut t, Some(&blob));
        assert!(t.content.contains("wire stub"));
        assert!(t.content.contains("stored rail"));
        assert!(t.content.len() < 1_000);
    }

    #[test]
    fn snap_does_not_start_on_tool() {
        let body = vec![
            msg(Role::User, "u0"),
            msg(Role::Assistant, "a0"),
            msg(Role::Tool, "t0"),
            msg(Role::Assistant, "a1"),
            msg(Role::User, "u1"),
            msg(Role::Assistant, "a2"),
        ];
        // keep=4 starts at t0; retain its opening assistant a0 as well.
        let k = snap_keep_to_round(&body, 4);
        assert_eq!(k, 5);
        assert_eq!(body[body.len() - k].role, Role::Assistant);
    }

    #[test]
    fn snap_one_user_agentic_opens_on_assistant() {
        let body = vec![
            msg(Role::User, "kickoff"),
            msg(Role::Assistant, "a0"),
            msg(Role::Tool, "t0"),
            msg(Role::Assistant, "a1"),
            msg(Role::Tool, "t1"),
        ];
        // keep=3 starts at t0; expand to a0 without dropping either round.
        let k = snap_keep_to_round(&body, 3);
        assert_eq!(k, 4);
        assert_eq!(body[body.len() - k].role, Role::Assistant);
    }

    #[test]
    fn wants_history_on_plan() {
        assert!(wants_history_recall("what was the plan?"));
        assert!(!wants_history_recall("continue"));
    }

    #[test]
    fn significant_terms_from_trajectory() {
        let t = significant_terms("fix amnesia DRSS hot window continue");
        assert!(t.iter().any(|x| x == "amnesia"));
        assert!(t.iter().any(|x| x == "window"));
    }

    #[test]
    fn build_recall_intent_includes_trajectory() {
        let h = long_history(6);
        let s = build_recall_intent(&h, "continue please");
        assert!(s.contains("continue please"));
        assert!(s.contains("recent trajectory"));
        assert!(s.contains("body-"));
    }
}

#[cfg(test)]
#[path = "recall_regression_tests.rs"]
mod regression_tests;
