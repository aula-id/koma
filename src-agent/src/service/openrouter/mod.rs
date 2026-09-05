//! OpenRouter HTTP client: the only thing that talks to the network.
//!
//! Two entry points, both spawned as async tasks by the runtime:
//! - [`OpenRouterClient::stream_complete`] — chat streaming over SSE, emitting
//!   [`StreamEvent`]s down a per-request channel.
//! - [`OpenRouterClient::complete`] — one-shot completion (used by `/compact`).
//!
//! The client knows nothing about the UI; it just pushes `StreamEvent`s. A
//! dropped receiver makes every send a no-op, so an aborted/superseded request
//! cannot corrupt the next one.
//!
//! ## Module layout
//!
//! | Submodule     | Contents                                              |
//! |---------------|-------------------------------------------------------|
//! | `types`       | `Conn`, `EffortCaps` (shared value types)             |
//! | `client`      | `OpenRouterClient` struct, `new`, `plan_word`         |
//! | `helpers`     | Private free functions (emit, routing, error parsing) |
//! | `stream`      | `stream_complete` impl                                |
//! | `oneshot`     | `complete`, `complete_with`, `classify_with`, etc.    |
//! | `catalogue`   | `effort_caps`, `context_length_for`, list methods     |
//! | `codex`       | OpenAI Responses API transport (subscription OAuth)   |
//! | `anthropic`   | Anthropic Messages API transport (Claude OAuth)       |

mod anthropic;
mod catalogue;
mod client;
mod codex;
mod commandcode;
mod debug_dump;
mod helpers;
mod oneshot;
mod stream;
mod think_split;
mod types;

// Re-export the entire public surface so every external path is unchanged.
pub use catalogue::{context_length_for, effort_caps, model_image_capability, ImageCapability};
pub use client::OpenRouterClient;
pub use types::{Conn, EffortCaps};
// `is_openrouter` is crate-internal (not part of the module's public surface)
// but needs to reach `app::runtime::commands::effort` — see its doc comment.
pub(crate) use helpers::is_openrouter;
