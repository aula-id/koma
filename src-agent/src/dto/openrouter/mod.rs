//! Wire-format DTOs for the OpenRouter chat-completions API.
//!
//! Three distinct shapes live here, corresponding to the three interaction
//! modes this app uses:
//!
//! 1. **Request** (`ChatRequest`) — sent for every user turn, both streaming
//!    and non-streaming.
//! 2. **Non-streaming response** (`ChatResponse` / `Choice` / `ResponseMessage`)
//!    — used only by the `/compact` summarisation call, which needs the full
//!    reply in one shot before it can rewrite the conversation.
//! 3. **Streaming chunk** (`StreamChunk` / `StreamChoice` / `Delta`) — each
//!    SSE `data:` line from the model during a normal chat turn is parsed into
//!    one of these; `Delta::content` is appended to the in-progress assistant
//!    bubble.
//!
//! All types are serde-only; no business logic lives here.

// Re-exports below preserve the original flat-file public API; some names have no
// in-crate consumer yet, so silence the unused-import lint for the whole facade.
#![allow(unused_imports)]

mod image;
pub mod models;
pub mod request;
pub mod response;
pub mod usage;

// ---------------------------------------------------------------------------
// Re-exports — preserve the public API so all external paths stay identical.
// ---------------------------------------------------------------------------

// request
pub use request::{
    ChatRequest,
    ProviderRouting,
    ReasoningConfig,
    ToolDef,
    ToolFunctionDef,
    UsageRequest,
    StreamOptions,
    ImageWireCtx,
    WireMessage,
    to_wire,
    to_wire_with_images,
};

// models
pub use models::{
    Architecture,
    EndpointsData,
    EndpointsResponse,
    ModelEndpoint,
    ModelInfo,
    ModelPricing,
    ModelReasoning,
    ModelsResponse,
    TopProvider,
};

// usage
pub use usage::{
    PromptTokensDetails,
    Usage,
};

// response
pub use response::{
    ChatResponse,
    Choice,
    Delta,
    FunctionDelta,
    ResponseMessage,
    StreamChunk,
    StreamChoice,
    ToolCallDelta,
};
