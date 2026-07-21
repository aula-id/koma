//! Streaming chat completion over the Command Code NDJSON wire.

use anyhow::Result;
use futures_util::StreamExt;
use std::time::SystemTime;
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use crate::dto::chat::{ChatMessage, FunctionCall, ToolCall};
use crate::dto::openrouter::{ImageWireCtx, ToolDef};
use crate::service::StreamEvent;

use super::super::helpers::{clean_error, emit, sanitize_tool_acc};
use super::super::Conn;
use super::super::OpenRouterClient;
use super::ndjson::{parse_line, CcEvent};
use super::request::{build_messages, extract_system, flatten_tools, GenerateRequest, RequestConfig, RequestParams};

impl OpenRouterClient {
    /// Streaming completion over the Command Code `/alpha/generate` NDJSON wire.
    ///
    /// Same [`StreamEvent`] contract as `stream_complete`: every failure emits a
    /// single [`StreamEvent::Error`] and returns `Ok(())`. `bearer` is the
    /// long-lived API key (`user_…`), passed in from the dispatch branch.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::service::openrouter) async fn commandcode_stream_complete(
        &self,
        conn: Conn<'_>,
        bearer: &str,
        model: &str,
        messages: Vec<ChatMessage>,
        advertise: &[String],
        mcp_tools: &[ToolDef],
        image_ctx: Option<ImageWireCtx>,
        tx: UnboundedSender<StreamEvent>,
    ) -> Result<()> {
        let url = format!("{}/alpha/generate", conn.endpoint);

        let system = extract_system(&messages);
        let cc_messages = build_messages(&messages, image_ctx.as_ref());
        let tools = flatten_tools(advertise, mcp_tools);
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
                tools,
                system,
                max_tokens: super::request::DEFAULT_GENERATE_MAX_TOKENS,
                temperature: 0.3,
                stream: true,
            },
            thread_id: Uuid::new_v4().to_string(),
        };

        let rb = self
            .http
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {bearer}"))
            .header("x-command-code-version", super::CC_CLI_VERSION)
            .header("x-cli-environment", "production")
            .header("x-project-slug", slug_from_path(&body.config.working_dir))
            .header("x-taste-learning", "true")
            .header("x-co-flag", "false");

        let resp = match rb.json(&body).send().await {
            Ok(r) => r,
            Err(e) => {
                emit(&tx, StreamEvent::Error(format!("request failed: {e}")));
                return Ok(());
            }
        };
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            emit(&tx, StreamEvent::Error(clean_error(status, &text)));
            return Ok(());
        }

        let mut stream = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        let mut tool_acc: Vec<ToolCall> = Vec::new();
        let mut finished = false;

        while let Some(chunk) = stream.next().await {
            let bytes = match chunk {
                Ok(b) => b,
                Err(e) => {
                    emit(&tx, StreamEvent::Error(format!("stream error: {e}")));
                    return Ok(());
                }
            };
            buf.extend_from_slice(&bytes);
            while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                let line_bytes: Vec<u8> = buf.drain(..=pos).collect();
                let line = String::from_utf8_lossy(&line_bytes);
                let Some(event) = parse_line(&line) else {
                    continue;
                };
                match event {
                    CcEvent::TextDelta { text } => {
                        if !text.is_empty() {
                            emit(&tx, StreamEvent::Token(text));
                        }
                    }
                    CcEvent::ReasoningStart => {}
                    CcEvent::ReasoningDelta { text } => {
                        if !text.is_empty() {
                            emit(&tx, StreamEvent::Reasoning(text));
                        }
                    }
                    CcEvent::ReasoningEnd => {}
                    CcEvent::ToolCall {
                        tool_call_id,
                        tool_name,
                        input,
                    } => {
                        let args = if input.is_object() || input.is_array() {
                            input.to_string()
                        } else {
                            "{}".to_string()
                        };
                        tool_acc.push(ToolCall {
                            id: tool_call_id,
                            kind: "function".to_string(),
                            function: FunctionCall {
                                name: tool_name,
                                arguments: args,
                            },
                        });
                    }
                    CcEvent::Finish {
                        finish_reason: _,
                        total_usage,
                    } => {
                        if let Some(u) = total_usage {
                            let cached = u
                                .input_token_details
                                .as_ref()
                                .map(|d| d.cache_read_tokens)
                                .unwrap_or(0);
                            emit(
                                &tx,
                                StreamEvent::Usage {
                                    prompt_tokens: u.input_tokens,
                                    completion_tokens: u.output_tokens,
                                    cached_tokens: cached,
                                    cost: 0.0,
                                },
                            );
                        }
                        finished = true;
                    }
                    CcEvent::Error { error } => {
                        let msg = error
                            .get("message")
                            .and_then(|m| m.as_str())
                            .unwrap_or("stream error");
                        emit(&tx, StreamEvent::Error(msg.to_string()));
                        return Ok(());
                    }
                    CcEvent::Unknown => {}
                }
                if finished {
                    break;
                }
            }
            if finished {
                break;
            }
        }
        // Finalize: emit accumulated tool calls, then Done.
        if !tool_acc.is_empty() {
            sanitize_tool_acc(&mut tool_acc);
            emit(&tx, StreamEvent::ToolCalls(tool_acc));
        }
        emit(&tx, StreamEvent::Done);
        Ok(())
    }
}

/// Derive a project slug from the working directory path.
/// Mirrors pi-commandcode-provider's `projectSlugFromPath` (strips a leading
/// Windows drive letter, collapses non-alnum runs to `-`, trims edge dashes).
fn slug_from_path(path: &str) -> String {
    let mut s = path.to_lowercase();
    // Strip a leading `c:` / `d:` drive letter (Windows paths).
    if s.len() >= 2 {
        let b = s.as_bytes();
        if b[0].is_ascii_alphabetic() && b[1] == b':' {
            s = s[2..].to_string();
        }
    }
    let slug = s
        .replace(|c: char| !c.is_ascii_alphanumeric(), "-")
        .trim_matches('-')
        .to_string();
    if slug.is_empty() {
        "project".to_string()
    } else {
        slug
    }
}

/// Current date as YYYY-MM-DD using std time (no chrono dependency).
pub(super) fn today_date_string() -> String {
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Simple days-since-epoch → Y/M/D. Good enough for a date string.
    let days = (secs / 86400) as i64;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_basic() {
        assert_eq!(
            slug_from_path("/home/user/my-project"),
            "home-user-my-project"
        );
        // Drive letter stripped (pi projectSlugFromPath).
        assert_eq!(slug_from_path("C:\\Users\\me\\code"), "users-me-code");
        assert_eq!(slug_from_path("/"), "project");
        assert_eq!(slug_from_path(""), "project");
    }

    #[test]
    fn today_is_reasonable() {
        let d = today_date_string();
        // Must be YYYY-MM-DD format and a reasonable year.
        assert_eq!(d.len(), 10);
        assert_eq!(&d[4..5], "-");
        assert_eq!(&d[7..8], "-");
        let year: i32 = d[..4].parse().unwrap();
        assert!(year >= 2024 && year <= 2100);
    }
}
