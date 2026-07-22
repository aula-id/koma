//! Knowledge-daemon wire protocol — the DEDICATED request/response vocabulary the
//! global knowledge daemon (`koma --knowledge-daemon`) and its session clients speak.
//!
//! # Why a separate protocol
//!
//! The session daemon's [`ClientRequest`](super::proto::ClientRequest) /
//! [`DaemonEvent`](super::proto::DaemonEvent) vocabulary is about a TUI attaching to
//! a session. The MCP daemon's [`McpRequest`](super::mcp_proto::McpRequest) is about
//! proxying tool calls. The knowledge daemon is a third service: cross-session
//! entity resolution, graph-based recall expansion, and central fact storage. Its
//! vocabulary is small and self-contained — no reason to tangle it with the others.
//!
//! # Wire format
//!
//! Same 4-byte-BE-length + JSON codec as [`super::frame`] — a `KnowledgeRequest` or
//! `KnowledgeResponse` is `serde_json`-serialised and length-framed exactly like a
//! `ClientRequest` / `McpRequest`. The daemon's accept loop reads one request frame,
//! replies with one response frame, and repeats until the peer closes.
//!
//! Sessions connect-per-call (sync UDS, bounded timeout) — no persistent connection
//! needed. A daemon-unavailable path degrades gracefully to local-only recall.

use serde::{Deserialize, Serialize};

/// A fact atom, mirrored from [`crate::model::surreal::memory::Fact`] for the IPC
/// boundary so the knowledge daemon crate doesn't depend on the surreal module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeFact {
    pub id: String,
    pub content: String,
    pub category: String,
    pub confidence: f64,
    pub trust: f64,
    pub reinforcement_count: i64,
    pub created_at: i64,
    pub last_reinforced: i64,
}

/// An entity node from the knowledge graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeEntity {
    pub entity_id: String,
    pub entity_type: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

/// A request from a session to the global knowledge daemon.
///
/// Framed with the shared 4-byte-BE-len + JSON codec ([`super::frame`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KnowledgeRequest {
    /// Push a fact into the central knowledge store. Fire-and-forget — the session
    /// does not block on the reply. The daemon queues it for entity extraction and
    /// graph building. Answered with [`KnowledgeResponse::Ack`].
    PushFact {
        fact_id: String,
        content: String,
        category: String,
        confidence: f64,
        embedding: Vec<f32>,
    },
    /// Expand a vector query through the entity graph. The daemon runs KNN on the
    /// fact table, then traverses `->produced->entity->memory_edge->entity` to
    /// pull in related facts. Answered with [`KnowledgeResponse::ExpandResult`].
    Expand { query_vec: Vec<f32>, limit: usize },
    /// Daemon health + stats. Answered with [`KnowledgeResponse::Status`].
    Status,
    /// Ask the daemon to shut down gracefully (Windows graceful-stop path; unix
    /// uses SIGTERM). Answered with [`KnowledgeResponse::Ack`].
    Shutdown,
}

/// The global knowledge daemon's reply to a [`KnowledgeRequest`].
///
/// Framed with the shared 4-byte-BE-len + JSON codec ([`super::frame`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KnowledgeResponse {
    /// Answer to [`KnowledgeRequest::PushFact`] and [`KnowledgeRequest::Shutdown`].
    Ack,
    /// Answer to [`KnowledgeRequest::Expand`]: KNN-matched facts, the entities
    /// they connect to, and related facts reachable through the entity graph.
    ExpandResult {
        facts: Vec<KnowledgeFact>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        entities: Vec<KnowledgeEntity>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        related_facts: Vec<KnowledgeFact>,
    },
    /// Answer to [`KnowledgeRequest::Status`].
    Status { fact_count: u64, entity_count: u64 },
    /// A protocol-level error (malformed request, daemon-internal fault).
    Error(String),
}
