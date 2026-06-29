//! Tool-call types and the two parsing / sanitisation utilities.
//!
//! [`FunctionCall`] and [`ToolCall`] are the wire-format structs for OpenAI-style
//! tool calls. [`extract_text_tool_calls`] handles the text-embedded fallback used
//! by budget/ChatML-trained models, and [`sanitize_tool_arguments`] repairs the
//! duplicate-delta streaming bug found on some providers.

mod types;
mod extract;
mod sanitize;

#[cfg(test)]
mod tests;

pub use types::{FunctionCall, ToolCall};
pub use extract::{extract_text_tool_calls, strip_tool_call_tags};
pub use sanitize::{sanitize_tool_arguments, strip_ansi};
