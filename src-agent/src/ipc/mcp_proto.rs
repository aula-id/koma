//! MCP-proxy wire protocol — the DEDICATED request/response vocabulary the global
//! MCP daemon and its session-daemon proxy clients speak.
//!
//! # Why a separate protocol from the session IPC
//!
//! The session daemon's [`ClientRequest`](super::proto::ClientRequest) /
//! [`DaemonEvent`](super::proto::DaemonEvent) vocabulary is about a TUI attaching to
//! a session (attach, submit, keypresses, snapshots, deltas). The GLOBAL MCP daemon
//! is a completely different service: one process that owns every configured MCP
//! server connection so N session-daemons proxy to it instead of each spawning their
//! own copies of a heavyweight server (e.g. `serena`). Overloading the session enums
//! with MCP verbs would tangle two unrelated concerns, so this is its own tiny pair
//! of enums.
//!
//! # Wire format — the SAME codec as the session IPC
//!
//! These frames ride the identical 4-byte-big-endian-length + JSON codec that
//! [`super::frame`] defines and the session daemon uses. There is NO second wire
//! format: a `McpRequest`/`McpResponse` is `serde_json`-serialised and length-framed
//! exactly like a `ClientRequest`/`DaemonFrame`. The global daemon's accept loop
//! reads an `McpRequest` frame, replies with an `McpResponse` frame, and repeats
//! until the peer closes — so both one-shot probes (bind-check, `Status`) and
//! long-lived proxy connections (a session-daemon dispatching many `Call`s) work over
//! the same connection shape.
//!
//! # Types reused verbatim
//!
//! - [`ToolDef`] is the crate's real advertised-tool type — the exact value
//!   [`McpManager::tool_defs`](crate::app::mcp::McpManager::tool_defs) returns and the
//!   session runtime appends to the model-API request `tools` array. It gained a
//!   `Deserialize` derive (alongside its existing `Serialize`) so it can ride this
//!   proxy IPC in BOTH directions: the global daemon serialises the defs it
//!   discovered, and a session-daemon deserialises them to advertise them to its
//!   model.
//! - [`McpServerEntry`] is the persisted config entry (already Serialize/Deserialize);
//!   a `Reconnect` carries the fresh server set after a `/mcp` config save.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::dto::openrouter::ToolDef;
use crate::model::app_config::McpServerEntry;

/// A request from a session-daemon (the proxy client) to the global MCP daemon.
///
/// Framed with the shared 4-byte-BE-len + JSON codec ([`super::frame`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum McpRequest {
    /// Fetch the advertised tool definitions + their namespaced names — everything
    /// the session runtime needs to advertise the global daemon's MCP tools to its
    /// model. Answered with [`McpResponse::Tools`].
    List,
    /// Dispatch ONE MCP tool call and return its flattened text result.
    ///
    /// `tool` is the NAMESPACED tool name (`mcp__<server>__<tool>`) — the exact key
    /// [`McpManager::execute_blocking`](crate::app::mcp::McpManager::execute_blocking)
    /// resolves against; the daemon passes it straight through. `args` is the JSON
    /// arguments object, serialised to a string so this enum needs no
    /// `serde_json::Value` field (the daemon parses it back before dispatch). `server_uuid`
    /// is carried for routing/diagnostics by the session-daemon proxy in the next commit
    /// (the manager itself resolves the owning server from the namespaced `tool`, so the
    /// daemon does not need `server_uuid` to dispatch). Answered with
    /// [`McpResponse::CallResult`].
    Call {
        server_uuid: String,
        tool: String,
        args: String,
    },
    /// Apply a NEW server set live (after a `/mcp` config save/delete): tear down the
    /// current connections and reconnect from `servers`. Answered with
    /// [`McpResponse::Ack`].
    Reconnect { servers: Vec<McpServerEntry> },
    /// Per-server connection state, for the `/mcp` panel. Answered with
    /// [`McpResponse::Status`].
    Status,
}

/// The global MCP daemon's reply to an [`McpRequest`].
///
/// Framed with the shared 4-byte-BE-len + JSON codec ([`super::frame`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum McpResponse {
    /// Answer to [`McpRequest::List`]: the advertised tool definitions plus the
    /// flattened list of their namespaced names (the advertise allow-list the stream
    /// filter keys on). `defs` and `names` are the exact values
    /// [`McpManager::tool_defs`](crate::app::mcp::McpManager::tool_defs) /
    /// [`McpManager::tool_names`](crate::app::mcp::McpManager::tool_names) return.
    Tools {
        defs: Vec<ToolDef>,
        names: Vec<String>,
    },
    /// Answer to [`McpRequest::Call`]: the tool's flattened text output. A dispatch
    /// error is surfaced HERE, in-band, as the result string — mirroring how the
    /// session tool path hands the model an MCP error as the tool result rather than a
    /// transport failure (see
    /// [`McpManager::execute_blocking`](crate::app::mcp::McpManager::execute_blocking),
    /// whose `Err(String)` the daemon folds into this variant). [`McpResponse::Error`]
    /// is reserved for PROTOCOL faults (malformed request, un-parseable args), not tool
    /// errors.
    CallResult(String),
    /// Answer to [`McpRequest::Status`]: per-server discovered-tool count keyed by
    /// server uuid — the exact shape
    /// [`McpManager::server_status`](crate::app::mcp::McpManager::server_status)
    /// returns, which the `/mcp` dashboard already consumes (a present key = connected;
    /// its value = tool count).
    Status(HashMap<String, usize>),
    /// Answer to [`McpRequest::Reconnect`]: the reconnect was accepted (it runs in the
    /// background, so this acks receipt, not completion).
    Ack,
    /// A PROTOCOL error (e.g. a request whose `args` string is not valid JSON). Tool
    /// dispatch errors do NOT come back here — they ride [`McpResponse::CallResult`]
    /// so the model still sees them as a tool result.
    Error(String),
}
