//! Core message types shared across the whole application.
//!
//! `Role` and `ChatMessage` are the atoms of every conversation. They flow
//! upward through `Conversation` → `Session` → the OpenRouter wire format
//! (`dto/openrouter.rs`), and are persisted verbatim to `messages.json` on
//! disk. Keeping them in a separate module avoids circular imports between the
//! model layer and the wire-format layer.

// Re-exports below preserve the original flat-file public API; some names have no
// in-crate consumer yet, so silence the unused-import lint for the whole facade.
#![allow(unused_imports)]

mod attachment;
mod message;
mod role;
mod tool;

pub use attachment::Attachment;
pub use message::ChatMessage;
pub use role::{Role, BASH_NUDGE_MARK, CACHE_SPLIT_MARK, PLAN_NUDGE_MARK, SHELL_MARK};
pub use tool::{extract_text_tool_calls, sanitize_tool_arguments, strip_ansi, strip_tool_call_tags, FunctionCall, ToolCall};
