//! One-shot (non-streaming-shaped) completion over the Anthropic Messages wire.

use anyhow::{anyhow, Result};
use futures_util::StreamExt;

use crate::dto::chat::ChatMessage;

use super::super::helpers::clean_error;
use super::super::Conn;
use super::super::OpenRouterClient;
use super::request::{build_messages, AnthropicTool, MessagesRequest};
use super::sse::{parse_event, AnthropicEvent, BlockDelta};
use super::{anthropic_headers, error_message, CLAUDE_MAX_OUTPUT_TOKENS};

impl OpenRouterClient {
    /// Non-streaming-shaped call over the streaming wire: POST `stream: true`,
    /// drain the SSE INLINE (no channel / spawned task), and return the collected
    /// text.
    ///
    /// Backs the Anthropic path of every oneshot method (compact / awareness /
    /// classifier / fold / router). `bearer` comes from the dispatch branch
    /// (already refreshed) — no `fresh_key` here. `account_id` + `effort` are
    /// accepted for call-site parity with `codex_collect` but unused.
    ///
    /// `text_format`: for the STRUCTURED-OUTPUT paths the caller passes the raw
    /// JSON schema. It is realised via a FORCED TOOL — a single `respond` tool
    /// whose `input_schema` is that schema, with `tool_choice` pinned to it — and
    /// the tool_use block's accumulated input-JSON string is returned as the
    /// result. `None` → plain text collection (the `text_delta` stream).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::service::openrouter) async fn anthropic_collect(
        &self,
        conn: Conn<'_>,
        bearer: &str,
        account_id: &str,
        model: &str,
        effort: &str,
        messages: Vec<ChatMessage>,
        text_format: Option<serde_json::Value>,
    ) -> Result<String> {
        let _ = (account_id, effort); // reserved for codex-parity signature
        let url = format!("{}/v1/messages?beta=true", conn.endpoint);

        let (system, msgs) = build_messages(messages, None);

        // Structured output → force a single `respond` tool carrying the schema;
        // the model's tool input IS the structured payload we return.
        let structured = text_format.is_some();
        let (tools, tool_choice) = match text_format {
            Some(schema) => (
                Some(vec![AnthropicTool {
                    name: "respond".to_string(),
                    description: "Respond with the required structured output".to_string(),
                    input_schema: schema,
                }]),
                Some(serde_json::json!({"type": "tool", "name": "respond"})),
            ),
            None => (None, None),
        };
        let body = MessagesRequest {
            model: model.to_string(),
            system,
            messages: msgs,
            tools,
            tool_choice,
            max_tokens: CLAUDE_MAX_OUTPUT_TOKENS,
            stream: true,
        };

        let rb = anthropic_headers(self.http.post(&url), bearer);
        let resp = rb.json(&body).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("{}", clean_error(status, &text)));
        }

        let mut stream = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        // Plain text collects `text_delta`s; the forced-tool path collects the
        // `input_json_delta`s. Only one is ever populated for a given call, so we
        // accumulate both and pick by `structured` at the end.
        let mut text_out = String::new();
        let mut tool_out = String::new();
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
                    AnthropicEvent::ContentBlockDelta { delta, .. } => match delta {
                        BlockDelta::TextDelta { text } => text_out.push_str(&text),
                        BlockDelta::InputJsonDelta { partial_json } => {
                            tool_out.push_str(&partial_json)
                        }
                        BlockDelta::Other => {}
                    },
                    AnthropicEvent::MessageStop => {
                        return Ok(if structured { tool_out } else { text_out });
                    }
                    AnthropicEvent::Error { error } => {
                        let (message, kind) = match error {
                            Some(e) => (e.message, e.kind),
                            None => (None, None),
                        };
                        return Err(anyhow!("{}", error_message(message, kind)));
                    }
                    // message_start / block start+stop / message_delta / other are
                    // irrelevant to a text/structured collect.
                    _ => {}
                }
            }
        }
        // Stream ended without an explicit `message_stop`: return whatever
        // accumulated (mirrors the codex EOF path).
        Ok(if structured { tool_out } else { text_out })
    }
}
