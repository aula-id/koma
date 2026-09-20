use crate::dto::chat::ChatMessage;
use crate::dto::openrouter::ToolDef;

pub const TARGET_PCT: u64 = 60;
pub const CEILING_PCT: u64 = 75;
pub const INDEX_MAX_TOKENS: u64 = 2_000;
pub const RECOVERY_INDEX_MAX_TOKENS: u64 = 12_000;
pub const FRAMING_TOKENS: u64 = 1_024;

/// Deterministic high-biased estimate. ASCII uses 2.5 chars/token; non-ASCII
/// uses UTF-8 bytes to avoid treating multilingual text as cheap ASCII.
pub fn text_tokens(s: &str) -> u64 {
    let mut ascii = 0u64;
    let mut other = 0u64;
    for c in s.chars() {
        if c.is_ascii() {
            ascii += 1;
        } else {
            other += c.len_utf8() as u64;
        }
    }
    (ascii * 2).div_ceil(5).saturating_add(other)
}

pub fn message_tokens(m: &ChatMessage) -> u64 {
    let mut n = text_tokens(&m.content) + 8;
    if let Some(calls) = &m.tool_calls {
        n += text_tokens(&serde_json::to_string(calls).unwrap_or_default());
    }
    if let Some(details) = &m.reasoning_details {
        n += text_tokens(&serde_json::to_string(details).unwrap_or_default());
    }
    // Image tokenization is provider-specific. Retain attachments and charge a
    // conservative allowance; the estimate is not an exact tokenizer count.
    n + m.attachments.len() as u64 * 4096
}

pub fn prompt_tokens(history: &[ChatMessage], schemas: u64) -> u64 {
    history.iter().map(message_tokens).sum::<u64>() + schemas + FRAMING_TOKENS
}

pub fn schema_tokens(advertise: &[String], extra: &[ToolDef]) -> u64 {
    let builtins: u64 = crate::tool::all_tools()
        .iter()
        .filter(|t| advertise.iter().any(|n| n == t.name()))
        .map(|t| {
            text_tokens(t.name())
                + text_tokens(t.description())
                + text_tokens(&t.parameters().to_string())
                + 16
        })
        .sum();
    builtins + text_tokens(&serde_json::to_string(extra).unwrap_or_default())
}
