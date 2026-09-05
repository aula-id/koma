//! Token and cost accounting types shared by streaming and non-streaming responses.

use serde::Deserialize;

// ---------------------------------------------------------------------------
// Usage (inbound, shared by streaming + non-streaming responses)
// ---------------------------------------------------------------------------

/// Token + cost accounting on chat-completions responses.
///
/// OpenRouter / koma-free populate this when the request sets proprietary
/// `usage: {"include": true}`. Direct OpenAI-compatible streams may emit token
/// counts (often via `stream_options.include_usage`) with `cost` left at 0.
/// On a streaming response this rides the final chunk (empty `choices`). All
/// fields default to zero so a partial/absent `usage` object never fails to
/// deserialise.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    /// Provider total cost (USD) when reported (OpenRouter); else 0.
    #[serde(default)]
    pub cost: f64,
    /// Breakdown of the prompt tokens, including how many were served from the
    /// prompt cache. Present when the provider reports cache stats; `None`/null
    /// otherwise (defaulted, so a missing object never fails to deserialise).
    #[serde(default)]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
}

/// The `prompt_tokens_details` sub-object of [`Usage`]. `cached_tokens` is the
/// count of prompt tokens served from the prompt cache (a cache hit) at the
/// discounted rate — what prompt caching saves. Defaults to 0 so a partial /
/// absent object still deserialises.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PromptTokensDetails {
    #[serde(default)]
    pub cached_tokens: u64,
}
