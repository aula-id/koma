//! Grounded GUI Help assistant with bounded article retrieval over koma-free.
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

const SYSTEM_PROMPT: &str = r#"You are koma's built-in Help assistant for the GUI (desktop/web app), not the terminal TUI.
Reply in the user's language. Use the shipped catalogue and articles as your only product knowledge.
Always prefer GUI steps: activity bar panels, Settings, Connector, Help → Reference / Guides, buttons, menus, and in-app tours.
Only mention TUI slash commands (like /help) or keybindings when the user explicitly asks about the terminal TUI.
Ask a clarifying question when interface facts or documentation do not support a reliable instruction. Never invent third-party extension behavior.
You only guide. You cannot inspect session chats, credentials, files or terminal output, run commands, change configuration, install, connect, save, commit or delete. Suggestions must use shipped navigation and workflow IDs. The user explicitly starts a guide.
When the user asks for a help list, command list, or what they can do, answer from GUI Reference/Guides and the feature catalogue — point them to Help → Reference and Help → Guides in the GUI, not the TUI /help screen, unless they asked for the TUI.
Return ONLY a single JSON object (no markdown fences, no prose outside JSON) with:
- answer: plain explanatory text focused on GUI steps
- articles: array of known article IDs (may be empty)
- navigation: known view ID or null
- guide: known workflow ID or null
If you need more documentation, return ONLY {"request_articles":["known-id",...]} with at most four IDs. The host permits at most two such rounds. Then produce the answer object. Reference the articles actually supporting your instructions. Never include selectors or scripts in actions.
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
    context: serde_json::Value,
) {
    std::thread::spawn(move || {
        let result = run_tutorial_chat(id, messages, context);
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
    context: serde_json::Value,
) {
    std::thread::spawn(move || {
        let _ = tx.send(run_tutorial_chat(id, messages, context));
    });
}

// ─── Core ────────────────────────────────────────────────────────────────────

fn run_tutorial_chat(
    id: String,
    messages: Vec<TutorialMsg>,
    context: serde_json::Value,
) -> TutorialChatResult {
    match complete(messages, context) {
        Ok(raw) => match crate::model::help_knowledge::parse_answer(&raw) {
            Ok(answer) => TutorialChatResult {
                id,
                text: raw,
                tour: answer.guide,
                error: None,
            },
            Err(e) => TutorialChatResult {
                id,
                text: String::new(),
                tour: None,
                error: Some(e),
            },
        },
        Err(e) => TutorialChatResult {
            id,
            text: String::new(),
            tour: None,
            error: Some(e),
        },
    }
}

/// Blocking OpenAI-compatible chat-completions call against koma-free (`stream: false`).
fn complete(messages: Vec<TutorialMsg>, context: serde_json::Value) -> Result<String, String> {
    let mut cfg = AppConfig::load();
    if cfg.install_id.is_empty() {
        cfg.install_id = new_uuid();
        // Persist only the install id mint — no provider/model mutation.
        let _ = cfg.save();
    }
    let install_id = cfg.install_id.clone();
    // Tutorial-scoped session header — NOT a hub/session uuid.
    let session_id = format!("tutorial-{}", install_id);

    let manifest = crate::model::help_knowledge::manifest()?;
    let catalogue = serde_json::to_string(
        &manifest
            .articles
            .iter()
            .map(|a| {
                serde_json::json!({
                    "id": a.id,
                    "title": a.title,
                    "aliases": a.aliases,
                    "navigation": a.navigation,
                    "workflows": a.workflows,
                })
            })
            .collect::<Vec<_>>(),
    )
    .map_err(|e| format!("help catalogue: {e}"))?;
    let last_user = messages
        .iter()
        .rev()
        .find(|m| m.role != "assistant")
        .map(|m| m.content.as_str())
        .unwrap_or_default();
    let want_tui = {
        let q = last_user.to_lowercase();
        [
            "tui",
            "terminal",
            "slash",
            "keybind",
            "hotkey",
            "/help",
            "/settings",
            "/model",
        ]
        .iter()
        .any(|k| q.contains(k))
    };
    let ranked_articles = crate::model::help_knowledge::ranked(last_user, 4)
        .iter()
        .filter_map(|id| {
            crate::model::help_knowledge::article(id).map(|body| format!("Article {id}:\n{body}"))
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    // Only inject the TUI command/keybinding registries when the user is clearly
    // asking about the terminal product — dumping them on every GUI turn steers
    // the free model toward /help instead of Help → Reference.
    let tui_appendix = if want_tui {
        format!(
            "\nTUI commands and shortcuts (terminal only): {:?} {:?}",
            crate::controller::command::COMMANDS,
            crate::controller::command::KEYBINDINGS
        )
    } else {
        String::new()
    };
    let mut wire_msgs: Vec<serde_json::Value> = Vec::with_capacity(messages.len() + 1);
    wire_msgs.push(serde_json::json!({
        "role": "system",
        "content": format!(
            "{}\nFeature catalogue: {}\nKnown navigation view IDs: {:?}\nKnown guide/workflow IDs: {:?}\nUI context: {}\nRelevant articles:\n{}{}",
            SYSTEM_PROMPT,
            catalogue,
            manifest.navigation,
            manifest.workflows,
            crate::model::help_knowledge::redact_context(&context),
            ranked_articles,
            tui_appendix,
        ),
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

    let url = format!("{KOMA_FREE_ENDPOINT}/chat/completions");
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("http client: {e}"))?;

    crate::model::help_knowledge::grounded_reply(&mut wire_msgs, |messages| {
        request_completion(&client, &url, &install_id, &session_id, messages)
    })
}

fn request_completion(
    client: &reqwest::blocking::Client,
    url: &str,
    install_id: &str,
    session_id: &str,
    messages: &[serde_json::Value],
) -> Result<String, String> {
    let body = serde_json::json!({"model":KOMA_FREE_MODEL,"stream":false,"messages":messages});
    let resp = client
        .post(url)
        .header("X-Koma", install_id)
        .header("X-Session", session_id)
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
