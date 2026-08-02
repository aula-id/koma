//! OAuth daemon wire protocol — minimal Status/Fingerprint/Shutdown vocabulary.
//!
//! Same 4-byte-BE-len + JSON codec as [`super::mcp_proto`]. The OAuth daemon is
//! much simpler than the MCP daemon: it only tracks token refresh state, so the
//! protocol is a small set of control verbs.

use serde::{Deserialize, Serialize};

/// A request from a management client to the global OAuth keep-alive daemon.
///
/// Framed with the shared 4-byte-BE-len + JSON codec ([`super::frame`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OAuthRequest {
    /// Report whether the daemon is alive and how many connections it's tracking.
    Status,
    /// Build-skew probe (same concept as [`super::mcp_proto::McpRequest::Fingerprint`]).
    Fingerprint,
    /// Graceful stop (Windows path; Unix uses SIGTERM).
    Shutdown,
}

/// The global OAuth daemon's reply to an [`OAuthRequest`].
///
/// Framed with the shared 4-byte-BE-len + JSON codec ([`super::frame`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OAuthResponse {
    /// Answer to [`OAuthRequest::Status`]: how many OAuth connections the daemon
    /// is tracking and how many successful refreshes have occurred.
    Status {
        oauth_connections: usize,
        refreshed_count: u64,
    },
    /// Answer to [`OAuthRequest::Fingerprint`]: this daemon's build fingerprint.
    Fingerprint(String),
    /// Generic acknowledgement (e.g. for [`OAuthRequest::Shutdown`]).
    Ack,
    /// A PROTOCOL error.
    Error(String),
}
