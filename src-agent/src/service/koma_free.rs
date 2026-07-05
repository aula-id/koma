//! koma-free keyless transport constants.
//!
//! koma-free is an OpenAI-compatible chat-completions gateway served at
//! [`KOMA_FREE_ENDPOINT`]; the client appends `/chat/completions` to it exactly
//! like any other OpenAI-compatible base URL, yielding
//! `https://koma.run/api/v1/koma-free/chat/completions`. Auth is two custom
//! headers (`X-Koma` install id + `X-Session`) with NO `Authorization` bearer —
//! see `service::openrouter::helpers::auth_headers_with_account`. Every request
//! pins [`KOMA_FREE_MODEL`].

/// Base URL for the koma-free gateway. NO trailing slash: the request path is
/// built as `{KOMA_FREE_ENDPOINT}/chat/completions`.
pub const KOMA_FREE_ENDPOINT: &str = "https://koma.run/api/v1/koma-free";

/// The only model id koma-free serves. Forced onto the resolved route so a
/// `/settings` model-id edit can never 404 the request.
pub const KOMA_FREE_MODEL: &str = "koma/apple";
