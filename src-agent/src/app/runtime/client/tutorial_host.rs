//! Host-side GUI Tutorial chat — thin koma-free client for the Tutorial tab.
//!
//! Runs on a one-shot [`std::thread::spawn`] worker (blocking `reqwest`), never on
//! the tokio runtime and never through a session daemon. Mirrors [`super::store_host`]:
//! works identically pre-session (hub / swapper) and attached.
//!
//! Auth is the keyless koma-free dual-header pair (`X-Koma` install id +
//! `X-Session` tutorial-scoped id). Does NOT call `SetupKomaFree` / does NOT
//! steal Main roles — tutorial traffic must not mutate the user's model config
//! beyond ensuring a non-empty `install_id` (required header).

use std::sync::mpsc::Sender;

use crate::config::{APP_TITLE, HTTP_REFERER};
use crate::model::app_config::{new_uuid, AppConfig};
use crate::service::koma_free::{KOMA_FREE_ENDPOINT, KOMA_FREE_MODEL};

/// Stable system prompt: short multilingual help + optional tour id router.
/// The model must end with a single `TOUR: <id>` or `TOUR: none` line the host
/// strips before showing the user-facing text.
const SYSTEM_PROMPT: &str = r#"You are koma's in-app GUI tutorial coach. Reply in the user's language.
Be brief (2–6 short sentences). Explain where to click in the real desktop GUI.
Do not invent features. Do not claim you can edit files or run tools — you only guide.

Known guided tours (offer when relevant; user confirms before launch):
- oauth-setup — connect a provider via OAuth, then pick/add a model
- provider-setup — add a custom/API-key provider, add a model, select it in the composer
- activity-bar — activity bar + sidebar panels overview
- sessions-hub — start/resume sessions
- composer — message box, model picker, attachments
- agents — sub-agents panel
- git — source control panel
- mcp — MCP servers panel
- remote — remote SSH hosts
- store — extension store
- settings — settings tab
- connector — Connector panel (providers / OAuth / models)

End EVERY reply with exactly one final line, nothing after it:
TOUR: <id>
or
TOUR: none
"#;

/// One chat message on the wire (role + content).
#[derive(Debug, Clone)]
pub struct TutorialMsg {
    pub role: String,
    pub content: String,
}

/// Result of one tutorial turn (attached channel payload / push fields).
pub(super) struct TutorialChatResult {
    pub id: String,
    pub text: String,
    pub tour: Option<String>,
    pub error: Option<String>,
}

// ─── DETACHED (host_swapper): push straight through the cloned sink ───────────

/// `HostCtl::TutorialChat` while detached.
pub(super) fn spawn_tutorial_chat(
    push: impl Fn(String) + Send + 'static,
    id: String,
    messages: Vec<TutorialMsg>,
) {
    std::thread::spawn(move || {
        let result = run_tutorial_chat(id, messages);
        super::push_proto::push_tutorial_chat_done(
            &push,
            result.id,
            result.text,
            result.tour,
            result.error,
        );
    });
}

// ─── ATTACHED (push_loop): reply over mpsc, drained by the fold loop ──────────

/// `HostCtl::TutorialChat` while attached.
pub(super) fn spawn_tutorial_chat_attached(
    tx: Sender<TutorialChatResult>,
    id: String,
    messages: Vec<TutorialMsg>,
) {
    std::thread::spawn(move || {
        let _ = tx.send(run_tutorial_chat(id, messages));
    });
}

// ─── Core ────────────────────────────────────────────────────────────────────

fn run_tutorial_chat(id: String, messages: Vec<TutorialMsg>) -> TutorialChatResult {
    match complete(messages) {
        Ok(raw) => {
            let (text, tour) = split_tour_trailer(&raw);
            TutorialChatResult {
                id,
                text,
                tour,
                error: None,
            }
        }
        Err(e) => TutorialChatResult {
            id,
            text: String::new(),
            tour: None,
            error: Some(e),
        },
    }
}

/// Blocking OpenAI-compatible chat-completions call against koma-free (`stream: false`).
fn complete(messages: Vec<TutorialMsg>) -> Result<String, String> {
    let mut cfg = AppConfig::load();
    if cfg.install_id.is_empty() {
        cfg.install_id = new_uuid();
        // Persist only the install id mint — no provider/model mutation.
        let _ = cfg.save();
    }
    let install_id = cfg.install_id.clone();
    // Tutorial-scoped session header — NOT a hub/session uuid.
    let session_id = format!("tutorial-{}", install_id);

    let mut wire_msgs: Vec<serde_json::Value> = Vec::with_capacity(messages.len() + 1);
    wire_msgs.push(serde_json::json!({
        "role": "system",
        "content": SYSTEM_PROMPT,
    }));
    for m in &messages {
        let role = match m.role.as_str() {
            "assistant" => "assistant",
            _ => "user",
        };
        // Cap each message to keep the free-tier payload small.
        let content: String = m.content.chars().take(4000).collect();
        if content.trim().is_empty() {
            continue;
        }
        wire_msgs.push(serde_json::json!({ "role": role, "content": content }));
    }
    // Keep only the last ~12 turns + system to bound context.
    if wire_msgs.len() > 13 {
        let system = wire_msgs.remove(0);
        let keep = wire_msgs.split_off(wire_msgs.len().saturating_sub(12));
        wire_msgs = std::iter::once(system).chain(keep).collect();
    }

    let body = serde_json::json!({
        "model": KOMA_FREE_MODEL,
        "stream": false,
        "messages": wire_msgs,
    });

    let url = format!("{KOMA_FREE_ENDPOINT}/chat/completions");
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("http client: {e}"))?;

    let resp = client
        .post(&url)
        .header("X-Koma", &install_id)
        .header("X-Session", &session_id)
        .header("HTTP-Referer", HTTP_REFERER)
        .header("X-Title", APP_TITLE)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .map_err(|e| format!("koma-free request failed: {e}"))?;

    let status = resp.status();
    let text = resp
        .text()
        .map_err(|e| format!("koma-free read body: {e}"))?;
    if !status.is_success() {
        let snippet: String = text.chars().take(240).collect();
        return Err(format!("koma-free HTTP {status}: {snippet}"));
    }

    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("koma-free bad JSON: {e}"))?;
    let content = v
        .pointer("/choices/0/message/content")
        .and_then(|c| c.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "koma-free returned empty content".to_string())?;
    Ok(content.to_string())
}

/// Split a trailing `TOUR: <id>` / `TOUR: none` line from the model reply.
fn split_tour_trailer(raw: &str) -> (String, Option<String>) {
    let trimmed = raw.trim_end();
    let Some((head, last)) = trimmed.rsplit_once('\n') else {
        // Single-line reply — still accept a bare TOUR line (unlikely).
        return parse_tour_line(trimmed)
            .map(|t| (String::new(), t))
            .unwrap_or_else(|| (trimmed.to_string(), None));
    };
    if let Some(tour) = parse_tour_line(last.trim()) {
        return (head.trim_end().to_string(), tour);
    }
    (trimmed.to_string(), None)
}

fn parse_tour_line(line: &str) -> Option<Option<String>> {
    let rest = line
        .strip_prefix("TOUR:")
        .or_else(|| line.strip_prefix("tour:"))?
        .trim();
    if rest.is_empty() || rest.eq_ignore_ascii_case("none") {
        return Some(None);
    }
    // Allow only known-looking ids (kebab-case).
    if rest
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && rest.len() < 64
    {
        Some(Some(rest.to_string()))
    } else {
        Some(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_tour_trailer_extracts_id() {
        let (text, tour) = split_tour_trailer("Open Connector.\n\nTOUR: oauth-setup\n");
        assert_eq!(text, "Open Connector.");
        assert_eq!(tour.as_deref(), Some("oauth-setup"));
    }

    #[test]
    fn split_tour_trailer_none() {
        let (text, tour) = split_tour_trailer("Just a tip.\nTOUR: none");
        assert_eq!(text, "Just a tip.");
        assert_eq!(tour, None);
    }

    #[test]
    fn split_tour_trailer_missing() {
        let (text, tour) = split_tour_trailer("No trailer here.");
        assert_eq!(text, "No trailer here.");
        assert_eq!(tour, None);
    }
}
