//! One-shot (non-streaming-shaped) completion over the Command Code NDJSON wire.

use anyhow::{anyhow, Result};
use futures_util::StreamExt;
use uuid::Uuid;

use crate::dto::chat::ChatMessage;

use super::super::helpers::{
    backoff_delay, clean_error, is_retryable_send_err, is_retryable_status, MAX_ATTEMPTS,
};
use super::super::Conn;
use super::super::OpenRouterClient;
use super::ndjson::{parse_line, CcEvent};
use super::request::{
    build_messages, extract_system, GenerateRequest, RequestConfig, RequestParams,
};
use super::stream::today_date_string;

impl OpenRouterClient {
    /// Non-streaming-shaped call over the NDJSON wire: POST with `stream: true`,
    /// drain the NDJSON events inline (no channel / spawned task), concatenate
    /// the text-deltas, and return the full text.
    ///
    /// Backs the Command Code path of every oneshot method (compact / awareness /
    /// classifier / fold / router). `bearer` comes from the dispatch branch
    /// (already refreshed) — no `fresh_key` here.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::service::openrouter) async fn commandcode_collect(
        &self,
        conn: Conn<'_>,
        bearer: &str,
        model: &str,
        messages: Vec<ChatMessage>,
    ) -> Result<String> {
        let url = format!("{}/alpha/generate", conn.endpoint);

        let system = extract_system(&messages);
        let cc_messages = build_messages(&messages, None);
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let today = today_date_string();

        let body = GenerateRequest {
            config: RequestConfig {
                working_dir: cwd,
                date: today,
                environment: "koma",
                structure: Vec::new(),
                is_git_repo: false,
                current_branch: String::new(),
                main_branch: String::new(),
                git_status: String::new(),
                recent_commits: Vec::new(),
            },
            memory: serde_json::json!(null),
            taste: serde_json::json!(null),
            skills: serde_json::json!(null),
            params: RequestParams {
                model: model.to_string(),
                messages: cc_messages,
                tools: Vec::new(),
                system,
                max_tokens: super::request::DEFAULT_GENERATE_MAX_TOKENS,
                temperature: 0.3,
                stream: true,
            },
            thread_id: Uuid::new_v4().to_string(),
        };

        let resp: reqwest::Response = 'retry: {
            for attempt in 1u32..=MAX_ATTEMPTS {
                let send = self
                    .http
                    .post(&url)
                    .header("Content-Type", "application/json")
                    .header("Authorization", format!("Bearer {bearer}"))
                    .header("x-command-code-version", super::CC_CLI_VERSION)
                    .header("x-cli-environment", "production")
                    .header("x-project-slug", "koma")
                    .header("x-taste-learning", "true")
                    .header("x-co-flag", "false")
                    .json(&body)
                    .send()
                    .await;
                match send {
                    Ok(r) => {
                        let status = r.status();
                        if status.is_success() {
                            break 'retry r;
                        }
                        let text = r.text().await.unwrap_or_default();
                        if is_retryable_status(status) && attempt < MAX_ATTEMPTS {
                            let d = backoff_delay(attempt);
                            tokio::time::sleep(d).await;
                            continue;
                        }
                        return Err(anyhow!("{}", clean_error(status, &text)));
                    }
                    Err(e) if is_retryable_send_err(&e) && attempt < MAX_ATTEMPTS => {
                        let d = backoff_delay(attempt);
                        tokio::time::sleep(d).await;
                        continue;
                    }
                    Err(e) => {
                        return Err(e.into());
                    }
                }
            }
            return Err(anyhow!("all retry attempts exhausted"));
        };

        let mut stream = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        let mut out = String::new();
        while let Some(chunk) = stream.next().await {
            let bytes = chunk?;
            buf.extend_from_slice(&bytes);
            while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                let line_bytes: Vec<u8> = buf.drain(..=pos).collect();
                let line = String::from_utf8_lossy(&line_bytes);
                let Some(event) = parse_line(&line) else {
                    continue;
                };
                match event {
                    CcEvent::TextDelta { text } => out.push_str(&text),
                    CcEvent::Error { error } => {
                        let msg = error
                            .get("message")
                            .and_then(|m| m.as_str())
                            .unwrap_or("stream error");
                        return Err(anyhow!("{msg}"));
                    }
                    // finish / reasoning / tool-call / unknown: irrelevant to text collect.
                    _ => {}
                }
            }
        }
        Ok(out)
    }
}
