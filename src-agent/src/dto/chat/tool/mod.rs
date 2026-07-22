//! Tool-call types and the two parsing / sanitisation utilities.
//!
//! [`FunctionCall`] and [`ToolCall`] are the wire-format structs for OpenAI-style
//! tool calls. [`extract_text_tool_calls`] handles the text-embedded fallback used
//! by budget/ChatML-trained models, and [`sanitize_tool_arguments`] repairs the
//! duplicate-delta streaming bug found on some providers.

mod extract;
mod sanitize;
mod types;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;

pub use extract::{extract_text_tool_calls, strip_tool_call_tags};
pub use sanitize::{sanitize_tool_arguments, strip_ansi};
pub use types::{FunctionCall, ToolCall};
