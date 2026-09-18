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
//! Priority: **live tail > current objective > continuity log > archive**. Missing summary,
//! kill-switch, not engaged, and post-`/compact` all fail-open to full history.
//! Display / on-disk conversation are never mutated (dual rail).

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
    KEYS.iter().any(|k| l.contains(k))
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
    format!(
        "\n\n[archive msg #{id} | role={role} | status=evidence]\n{excerpt}"
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

/// Hot-window size: floor `tail_floor`, grow under `HOT_TAIL_PCT` of `usable`,
/// cap `HOT_TAIL_MAX_MSGS`, never below watermark span when smaller, snap trim
/// to a user-exchange start so tool chains stay intact.
fn hot_keep_n(body: &[ChatMessage], after_wm: usize, tail_floor: usize, usable: u64) -> usize {
    if body.is_empty() {
        return 1;
    }
    let n = body.len();
    let floor = tail_floor.max(1).min(n);
    let wm_keep = after_wm.max(1).min(n);
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

    // If still over budget at floor, shrink toward 1 but prefer exchange snap.
    while keep > 1 {
        let start = n - keep;
        let toks: u64 = body[start..].iter().map(msg_tok_est).sum();
        if toks <= budget || keep <= floor {
            break;
        }
        keep -= 1;
    }

    keep = keep.max(1).min(n);
    snap_keep_to_exchange(body, keep)
}

/// Move the cut forward (keep fewer older msgs) to the nearest user message at
/// the hot-window start so we don't open mid tool-call chain. Never increases keep.
fn snap_keep_to_exchange(body: &[ChatMessage], keep: usize) -> usize {
    let n = body.len();
    if keep >= n || keep == 0 {
        return keep.min(n).max(1);
    }
    let start = n - keep;
    // If body[start] is already User, good.
    if body[start].role == Role::User {
        return keep;
    }
    // Walk toward newer messages to find a User (shortens the older side).
    for i in start + 1..n {
        if body[i].role == Role::User {
            return n - i;
        }
    }
    // No user in window — keep as-is (tool-only tail).
    keep
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
    let take = TRAJECTORY_MSG_N.min(body.len());
    if take == 0 {
        return out;
    }
    out.push_str("\n\n--- recent trajectory ---\n");
    let start = body.len() - take;
    let mut budget = TRAJECTORY_INTENT_CHARS;
    for m in &body[start..] {
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
        let content: String = m.content.chars().take(budget.min(800)).collect();
        line.push_str(&content);
        line.push('\n');
        if line.len() > budget {
            out.push_str(&line[..budget]);
            break;
        }
        budget = budget.saturating_sub(line.len());
        out.push_str(&line);
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

    let sum = msglog::read_summary(session_dir);
    let has_summary = sum
        .as_ref()
        .map(|s| !s.text.trim().is_empty())
        .unwrap_or(false);
    if !has_summary {
        return history;
    }
    let sum = sum.expect("has_summary");

    let body = &history[1..];
    let max_id = msglog::max_message_id(session_dir);
    let after_wm = (max_id.saturating_sub(sum.covers_up_to)).max(1) as usize;
    let keep = hot_keep_n(body, after_wm, tail_floor, usable);

    let Some((mut system, tail)) = clip_body(&history, keep) else {
        return history;
    };

    let recall_intent = if user_intent.contains("--- recent trajectory ---") {
        user_intent.to_string()
    } else {
        build_recall_intent(&history, user_intent)
    };
    let history_ask = wants_history_recall(&recall_intent);
    let terms = significant_terms(&recall_intent);

    // --- archive: FTS dialogue excerpts (folded region only) ---
    let mut archive_blocks: Vec<String> = Vec::new();
    let mut used_msg_ids: HashSet<i64> = HashSet::new();

    if !terms.is_empty() {
        let q = terms.join(" ");
        if let Ok(hits) =
            msglog::search_messages_before(session_dir, &q, sum.covers_up_to, MAX_MSG_EXCERPTS as i64)
        {
            for h in hits {
                if used_msg_ids.contains(&h.id) {
                    continue;
                }
                // Prefer user/assistant dialogue over pure tool noise when possible —
                // still allow tool if that's all we get.
                used_msg_ids.insert(h.id);
                archive_blocks.push(format_msg_excerpt(h.id, &h.role, h.snippet.trim()));
                if archive_blocks.len() >= MAX_MSG_EXCERPTS {
                    break;
                }
            }
        }
    }

    // --- blobs ---
    let mut all_candidates: Vec<msglog::BlobRef> = msglog::list_blobs(session_dir)
        .into_iter()
        .filter(|b| b.msg_id <= sum.covers_up_to)
        .collect();

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

    let mut blob_blocks: Vec<String> = Vec::new();
    if !content_hits.is_empty() {
        for hit in content_hits.iter().take(MAX_REHYDRATE) {
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
        for b in &archive_blocks {
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
        assert!(k <= 4);
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
        // keep=4 would start at tool t0 (index 2); snap should move to u1 (keep=2)
        let k = snap_keep_to_exchange(&body, 4);
        assert_eq!(k, 2);
        assert_eq!(body[body.len() - k].role, Role::User);
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
