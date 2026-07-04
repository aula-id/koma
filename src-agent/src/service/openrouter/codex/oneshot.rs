//! One-shot (non-streaming-shaped) completion over the Codex Responses wire.

use anyhow::{anyhow, Result};
use futures_util::StreamExt;

use crate::dto::chat::ChatMessage;

use super::super::helpers::clean_error;
use super::super::Conn;
use super::super::OpenRouterClient;
use super::request::{build_input, codex_effort, ResponsesReasoning, ResponsesRequest};
use super::sse::{parse_event, ResponsesEvent};
use super::{codex_headers, error_message, failed_message};

impl OpenRouterClient {
    /// Non-streaming-shaped call over the streaming wire: POST `stream: true`,
    /// drain the SSE INLINE (no channel / spawned task), concatenate the
    /// `output_text` deltas, and return the full text.
    ///
    /// Backs the Codex path of every oneshot method (compact / awareness /
    /// classifier / fold / router). No tools; the encrypted-reasoning `include`
    /// is omitted when the effort maps to `none`. Reasoning deltas and
    /// function_call / reasoning items are ignored (these calls want text only).
    /// Errors on ANY failure, including an in-band `response.failed` / `error`
    /// event. `bearer` + `account_id` come from the dispatch branch (already
    /// refreshed) — no `fresh_key` here.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::service::openrouter) async fn codex_collect(
        &self,
        conn: Conn<'_>,
        bearer: &str,
        account_id: &str,
        model: &str,
        effort: &str,
        messages: Vec<ChatMessage>,
        text_format: Option<serde_json::Value>,
    ) -> Result<String> {
        let url = format!("{}/responses", conn.endpoint);

        let (instructions, input) = build_input(messages, None);
        let (eff, include_encrypted) = codex_effort(effort);
        let include: Vec<&'static str> = if include_encrypted {
            vec!["reasoning.encrypted_content"]
        } else {
            Vec::new()
        };
        let body = ResponsesRequest {
            model: model.to_string(),
            instructions,
            input,
            // Oneshot calls advertise no tools.
            tools: None,
            tool_choice: "auto",
            stream: true,
            store: false,
            reasoning: Some(ResponsesReasoning {
                effort: eff,
                summary: "auto",
            }),
            include,
            prompt_cache_key: self.codex_session_id().to_string(),
            text: text_format,
        };

        let rb = codex_headers(self.http.post(&url), bearer, account_id, self.codex_session_id());
        let resp = rb.json(&body).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("{}", clean_error(status, &text)));
        }

        let mut stream = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        let mut out = String::new();
        while let Some(chunk) = stream.next().await {
            let bytes = chunk?;
            buf.extend_from_slice(&bytes);
            while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                let line_bytes: Vec<u8> = buf.drain(..=pos).collect();
                let line = String::from_utf8_lossy(&line_bytes);
                let line = line.trim_end();
                if line.is_empty() {
                    continue;
                }
                let data = match line
                    .strip_prefix("data: ")
                    .or_else(|| line.strip_prefix("data:"))
                {
                    Some(d) => d.trim(),
                    None => continue,
                };
                let Some(event) = parse_event(data) else {
                    continue;
                };
                match event {
                    ResponsesEvent::OutputTextDelta { delta } => out.push_str(&delta),
                    ResponsesEvent::Completed { .. } => return Ok(out),
                    ResponsesEvent::Failed { response } => {
                        return Err(anyhow!("{}", failed_message(response)));
                    }
                    ResponsesEvent::Error { message, code } => {
                        return Err(anyhow!("{}", error_message(message, code)));
                    }
                    // Reasoning deltas + function_call / reasoning items are
                    // irrelevant to a text-only collect.
                    _ => {}
                }
            }
        }
        // Stream ended without an explicit `response.completed`: return whatever
        // text accumulated (mirrors the streaming EOF path).
        Ok(out)
    }
}
