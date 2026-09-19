//! Deterministic A(system), B(index), C(live context) request construction.
use super::budget::*;
use super::goal::GoalWire;
use crate::dto::chat::{ChatMessage, Role};
use crate::model::msglog::drss::{self, Index};
use crate::model::settings::Settings;
use crate::service::context_limits::{ContextLimits, OUTPUT_MARGIN};
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::Path;

fn role_name(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

fn archive_ids(index: &Index, body: &[ChatMessage]) -> Vec<Option<i64>> {
    let mut ids = vec![None; body.len()];
    let mut before = i64::MAX;
    for (i, msg) in body.iter().enumerate().rev() {
        let hash = drss::fingerprint(role_name(msg.role), &msg.content);
        if let Some(matches) = index.ids.get(&hash) {
            if let Some(id) = matches.iter().rev().find(|id| **id < before) {
                ids[i] = Some(*id);
                before = *id;
            }
        }
    }
    ids
}

#[derive(Debug)]
struct Round {
    start: usize,
    end: usize,
    archived_end: Option<i64>,
    protected: bool,
}
fn rounds(body: &[ChatMessage], ids: &[Option<i64>]) -> Vec<Round> {
    let latest_user = body.iter().rposition(|m| m.role == Role::User);
    let mut result = Vec::new();
    let mut start = 0;
    while start < body.len() {
        let mut end = start + 1;
        // Keep opening assistant plus every result/system interlude as a unit.
        if body[start].role == Role::Assistant {
            while end < body.len() && matches!(body[end].role, Role::Tool | Role::System) {
                end += 1;
            }
        }
        let calls = body[start].tool_calls.as_deref().unwrap_or_default();
        let replies: HashSet<_> = body[start..end]
            .iter()
            .filter_map(|m| m.tool_call_id.as_deref())
            .collect();
        let complete = calls.iter().all(|call| replies.contains(call.id.as_str()));
        let indexed = ids[start..end].iter().all(Option::is_some);
        let protected = !indexed
            || !complete
            || end == body.len()
            || latest_user.is_some_and(|u| u >= start && u < end)
            || body[start..end].iter().any(|m| !m.attachments.is_empty())
            || matches!(body[start].role, Role::Tool | Role::System);
        result.push(Round {
            start,
            end,
            archived_end: ids[start..end].iter().flatten().max().copied(),
            protected,
        });
        start = end;
    }
    result
}

fn history_ask(user: &str) -> bool {
    let words: HashSet<_> = user
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .collect();
    ["plan", "earlier", "previous", "history", "recall"]
        .iter()
        .any(|w| words.contains(*w))
        || user.to_lowercase().contains("remind me")
        || user.to_lowercase().contains("you said")
}

fn relevance(body: &[ChatMessage], user: &str) -> Vec<String> {
    // Reserve half the query slots for fresh diagnostics even on long user turns.
    let fragment: String = user.chars().take(800).collect();
    let mut terms: Vec<_> = drss::terms(&fragment).into_keys().take(16).collect();
    for text in body
        .iter()
        .rev()
        .filter(|m| matches!(m.role, Role::Assistant | Role::Tool))
        .take(8)
        .map(|m| m.content.as_str())
    {
        let fragment: String = text.chars().take(800).collect();
        for term in drss::terms(&fragment).into_keys() {
            if !terms.contains(&term) {
                terms.push(term);
            }
            if terms.len() >= 32 {
                return terms;
            }
        }
    }
    terms
}

fn append_bounded(out: &mut String, line: &str, budget: u64) {
    if text_tokens(out).saturating_add(text_tokens(line)) + 8 <= budget {
        out.push_str(line);
    }
}

fn memory(
    index: &Index,
    through: i64,
    body: &[ChatMessage],
    user: &str,
    goal: &GoalWire,
    budget: u64,
) -> Result<ChatMessage> {
    let mut text = String::new();
    append_bounded(&mut text, "[DRSS archive index: generated context data]\nLive user messages take precedence. Archive excerpts may be obsolete; assistant text is unconfirmed draft material.\n", budget);
    if !goal.objective.is_empty() {
        append_bounded(
            &mut text,
            &format!(
                "Current objective (source={}): {}\n",
                goal.source,
                serde_json::to_string(&goal.objective)?
            ),
            budget,
        );
    }
    if !goal.charter.is_empty() && goal.charter != goal.objective {
        append_bounded(
            &mut text,
            &format!(
                "Kickoff charter: {}\n",
                serde_json::to_string(&goal.charter)?
            ),
            budget,
        );
    }
    if through >= index.start_id {
        append_bounded(&mut text, &format!("Archive range: #{} through #{}. Full text remains stored.\nRead exact messages with message_find({{\"message_id\":N,\"offset\":0,\"limit\":3000}}); follow next_offset.\nTerm counts cover the indexed vocabulary (up to 512 terms per message).\n",index.start_id,through), budget);
        for (id, quote) in index.user_constraints(through)? {
            append_bounded(
                &mut text,
                &format!(
                    "Historical user constraint #{id} (may be superseded): {}\n",
                    serde_json::to_string(&quote)?
                ),
                budget,
            );
        }
        let relevant = relevance(body, user);
        for (id, role, quote) in index.excerpts(through, &relevant, history_ask(user))? {
            append_bounded(
                &mut text,
                &format!(
                    "Excerpt #{id} role={role}: {}\n",
                    serde_json::to_string(&quote)?
                ),
                budget,
            );
        }
        for (term, occurrences, messages, latest) in index.statistics(through, &relevant)? {
            append_bounded(
                &mut text,
                &format!(
                    "Term {}: {occurrences} occurrences / {messages} messages; latest #{latest}\n",
                    serde_json::to_string(&term)?
                ),
                budget,
            );
        }
    }
    // A user-role context message is valid before an assistant/tool round on
    // chat-completions, Responses and Anthropic. Never counterfeit a tool result.
    Ok(ChatMessage::new(Role::User, text))
}

fn stub(msg: &mut ChatMessage, id: Option<i64>) {
    if !matches!(msg.role, Role::Tool | Role::Assistant) || text_tokens(&msg.content) < 1600 {
        return;
    }
    let Some(id) = id else {
        return;
    };
    let preview: String = msg.content.chars().take(240).collect();
    msg.content = format!("[DRSS live body stored as message #{id}]\n{preview}\nRead message_find({{\"message_id\":{id},\"offset\":0,\"limit\":3000}}); follow next_offset.");
}

pub fn shape(
    history: Vec<ChatMessage>,
    session_dir: &Path,
    settings: &Settings,
    user: &str,
    goal: &GoalWire,
    limits: &ContextLimits,
    schemas: u64,
) -> Result<Vec<ChatMessage>> {
    if !settings.short_send_enabled || history.len() <= 1 {
        return Ok(history);
    }
    anyhow::ensure!(
        history[0].role == Role::System,
        "DRSS requires the leading system message"
    );
    let window = limits.effective_window;
    let reserve = limits.reserved_output(settings.max_output_tokens);
    let b_budget = INDEX_MAX_TOKENS.min(window / 50);
    let available = window
        .saturating_sub(message_tokens(&history[0]))
        .saturating_sub(schemas + FRAMING_TOKENS + OUTPUT_MARGIN + reserve + b_budget);
    let ceiling = (window * CEILING_PCT / 100).min(available);
    let target = (window * TARGET_PCT / 100).min(ceiling);
    let body = &history[1..];
    let index = match Index::load(session_dir) {
        Ok(index) => index,
        Err(error) => {
            anyhow::ensure!(body.iter().map(message_tokens).sum::<u64>() <= ceiling,
                "DRSS archive unavailable ({error}); cannot safely reduce context. Conversation preserved.");
            return Ok(history);
        }
    };
    let ids = archive_ids(&index, body);
    let rounds = rounds(body, &ids);
    let mut boundary = index.boundary;
    let mut keep: Vec<bool> = rounds
        .iter()
        .map(|r| r.protected || r.archived_end.is_none_or(|id| id > boundary))
        .collect();
    let mut tokens: u64 = rounds
        .iter()
        .zip(&keep)
        .filter(|(_, k)| **k)
        .map(|(r, _)| body[r.start..r.end].iter().map(message_tokens).sum::<u64>())
        .sum();
    if tokens > ceiling {
        for (round, retained) in rounds.iter().zip(&mut keep) {
            if tokens <= target {
                break;
            }
            if round.protected || !*retained {
                continue;
            }
            *retained = false;
            tokens = tokens.saturating_sub(
                body[round.start..round.end]
                    .iter()
                    .map(message_tokens)
                    .sum::<u64>(),
            );
            boundary = boundary.max(round.archived_end.unwrap_or(0));
        }
    }
    let mut tail = Vec::new();
    let pressure = tokens > ceiling;
    // Paged archive reads must remain readable on the next turn, including
    // Unicode-heavy pages whose token estimate exceeds the normal stub threshold.
    let archive_reads: HashSet<_> = body
        .iter()
        .flat_map(|m| m.tool_calls.iter().flatten())
        .filter(|call| call.function.name == "message_find")
        .map(|call| call.id.as_str())
        .collect();
    for (round, retained) in rounds.iter().zip(&keep) {
        if !retained {
            continue;
        }
        for i in round.start..round.end {
            let mut msg = body[i].clone();
            if pressure
                && !msg
                    .tool_call_id
                    .as_deref()
                    .is_some_and(|id| archive_reads.contains(id))
            {
                stub(&mut msg, ids[i]);
            }
            tail.push(msg);
        }
    }
    let actual_c: u64 = tail.iter().map(message_tokens).sum();
    anyhow::ensure!(actual_c <= ceiling,
        "DRSS live context needs {actual_c} estimated tokens, above its {ceiling}-token ceiling. The current request/tool round is preserved; reduce the oversized input or adjust a verified model limit.");
    let b = memory(&index, boundary, &tail, user, goal, b_budget)?;
    let mut output = Vec::with_capacity(tail.len() + 2);
    output.push(history[0].clone());
    if !b.content.is_empty() {
        output.push(b);
    }
    output.extend(tail);
    anyhow::ensure!(prompt_tokens(&output,schemas) + reserve + OUTPUT_MARGIN <= window,
        "DRSS cannot fit system, memory, live context and reply reserve into {window} tokens. Conversation preserved.");
    index
        .save_boundary(boundary)
        .context("persist DRSS archive boundary")?;
    Ok(output)
}
