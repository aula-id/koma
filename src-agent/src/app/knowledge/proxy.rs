//! Session-side knowledge daemon proxy.
//!
//! Provides [`proxy_push_fact`] (fire-and-forget) and [`proxy_expand`] (blocking
//! request→response) over the global knowledge daemon's UDS. When the daemon is
//! unreachable, both degrade gracefully — push is silently dropped, expand returns
//! an empty result. No session ever fails because the knowledge daemon is down.

use std::time::Duration;

use anyhow::Context;

use crate::ipc::frame::FrameReader;
use crate::ipc::knowledge_proto::{KnowledgeFact, KnowledgeRequest, KnowledgeResponse};
use crate::ipc::SyncIpcStream;
use crate::model::store;

/// Maximum time a single sync IPC exchange (connect → write → read) may take.
const PROXY_IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Fire-and-forget: push a fact to the global knowledge daemon. Spawns a
/// detached OS thread so callers (including blocking-block'd async contexts)
/// never block on daemon IO. A dead/missing daemon is silently ignored.
pub fn proxy_push_fact(
    fact_id: String,
    content: String,
    category: String,
    confidence: f64,
    embedding: Vec<f32>,
) {
    std::thread::spawn(move || {
        let _ = push_fact_sync(&fact_id, &content, &category, confidence, &embedding);
    });
}

fn push_fact_sync(
    fact_id: &str,
    content: &str,
    category: &str,
    confidence: f64,
    embedding: &[f32],
) -> anyhow::Result<()> {
    let path = store::knowledge_daemon_sock_path()?;
    let mut stream = SyncIpcStream::connect(&path)
        .with_context(|| format!("connect to knowledge daemon at {}", path.display()))?;
    stream.set_read_timeout(Some(PROXY_IO_TIMEOUT))?;
    stream.set_write_timeout(Some(PROXY_IO_TIMEOUT))?;

    let req = KnowledgeRequest::PushFact {
        fact_id: fact_id.to_string(),
        content: content.to_string(),
        category: category.to_string(),
        confidence,
        embedding: embedding.to_vec(),
    };

    send_request(&mut stream, &req)?;
    let _resp = read_response(&mut stream)?;
    // We don't inspect the response — fire-and-forget.

    Ok(())
}

/// Blocking request→response: expand a vector query through the knowledge
/// daemon's graph. Returns the daemon's facts, entities, and related facts,
/// or an empty result if the daemon is unreachable / times out.
pub fn proxy_expand(query_vec: &[f32], limit: usize) -> ExpandResult {
    let qv = query_vec.to_vec();
    expand_sync(&qv, limit).unwrap_or_default()
}

/// Result of a knowledge daemon expand call.
#[derive(Debug, Default)]
pub struct ExpandResult {
    pub facts: Vec<KnowledgeFact>,
    pub related_facts: Vec<KnowledgeFact>,
}

fn expand_sync(query_vec: &[f32], limit: usize) -> anyhow::Result<ExpandResult> {
    let path = store::knowledge_daemon_sock_path()?;
    let mut stream = SyncIpcStream::connect(&path)
        .with_context(|| format!("connect to knowledge daemon at {}", path.display()))?;
    stream.set_read_timeout(Some(PROXY_IO_TIMEOUT))?;
    stream.set_write_timeout(Some(PROXY_IO_TIMEOUT))?;

    let req = KnowledgeRequest::Expand {
        query_vec: query_vec.to_vec(),
        limit,
    };

    send_request(&mut stream, &req)?;
    match read_response(&mut stream)? {
        KnowledgeResponse::ExpandResult {
            facts,
            entities: _,
            related_facts,
        } => Ok(ExpandResult {
            facts,
            related_facts,
        }),
        KnowledgeResponse::Error(e) => Err(anyhow::anyhow!("knowledge daemon expand error: {e}")),
        other => Err(anyhow::anyhow!(
            "unexpected knowledge daemon response: {other:?}"
        )),
    }
}

// ── Wire helpers ──────────────────────────────────────────────────────

fn send_request(stream: &mut SyncIpcStream, req: &KnowledgeRequest) -> anyhow::Result<()> {
    use std::io::Write;
    let payload = serde_json::to_vec(req).context("serialise KnowledgeRequest")?;
    let prefix = (payload.len() as u32).to_be_bytes();
    stream.write_all(&prefix).context("write request prefix")?;
    stream
        .write_all(&payload)
        .context("write request payload")?;
    stream.flush().context("flush request")?;
    Ok(())
}

fn read_response(stream: &mut SyncIpcStream) -> anyhow::Result<KnowledgeResponse> {
    use std::io::Read;
    let mut reader = FrameReader::new();
    loop {
        if let Some(bytes) = reader.next_frame().context("knowledge response frame")? {
            return serde_json::from_slice(&bytes).context("decode KnowledgeResponse");
        }
        let mut chunk = [0u8; 8192];
        let n = stream.read(&mut chunk).context("read knowledge response")?;
        if n == 0 {
            return Err(anyhow::anyhow!("knowledge daemon closed connection"));
        }
        reader.push(&chunk[..n]);
    }
}
