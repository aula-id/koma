use serde::{Deserialize, Serialize};

/// The function-call payload inside a [`ToolCall`]: the tool name plus its
/// arguments as a JSON-encoded string (OpenAI/OpenRouter send `arguments` as a
/// stringified JSON object, not a nested object).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

/// One tool call requested by the assistant. `id` correlates the eventual
/// `tool` result message back to this call; `kind` is always `"function"`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type", default)]
    pub kind: String, // "function"
    pub function: FunctionCall,
}
