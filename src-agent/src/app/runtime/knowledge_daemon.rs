//! The GLOBAL knowledge daemon (`koma --knowledge-daemon`).
//!
//! A SINGLETON headless process that owns the central SurrealKV knowledge store
//! at `~/.koma/knowledge/` so ALL sessions share entity resolution, graph-based
//! recall expansion, and a persistent fact corpus that survives compaction.
//! Sessions connect over IPC (`~/.koma/knowledge.sock`) to push facts and query
//! for graph-expanded recall results.
//!
//! # Idle self-reap (lifecycle)
//!
//! Mirrors the MCP daemon: a background reaper watches [`store::run_dir`] for
//! live session-daemon sockets (`run/*.sock`). Once there are NONE for a
//! sustained grace window it flips `shutting_down` and the normal teardown
//! drops the runtime + unlinks the socket/pidfile.
//!
//! # Shape mirrors `run_mcp_daemon`
//!
//! Startup/teardown deliberately mirror the MCP daemon: ignore SIGPIPE, install
//! signal handlers, write a pidfile, bind the unix socket (bind = liveness
//! oracle), run an accept loop on the tokio runtime, and on shutdown drop the
//! runtime + unlink the socket/pidfile.
//!
//! # Request loop
//!
//! Strictly request→response: a client sends one [`KnowledgeRequest`], gets one
//! [`KnowledgeResponse`], repeat. Each connection is its own tokio task running
//! a simple read-request → handle → write-response loop over the SAME frame
//! codec, until the peer closes.

use std::sync::Arc;

use anyhow::Result;
use surrealdb::engine::local::Db;
use surrealdb::Surreal;

use crate::ipc::frame::{read_frame_from, write_frame_to, FrameReader};
use crate::ipc::knowledge_proto::{KnowledgeRequest, KnowledgeResponse};
use crate::model::store;

use super::signals::install_daemon_signals;

/// Headless entry point: run the GLOBAL knowledge daemon event loop with NO terminal.
///
/// Opens the central SurrealKV store at `~/.koma/knowledge/`, defines the knowledge
/// schema, binds `~/.koma/knowledge.sock`, and serves [`KnowledgeRequest`] frames
/// until signalled. Returns when SIGTERM/SIGINT is observed (via the polled
/// `shutting_down` flag).
pub fn run_knowledge_daemon(_opts: crate::cli::Opts) -> Result<()> {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    #[cfg(windows)]
    super::signals::install_killtree_job();

    store::ensure_dirs()?;

    let rt = tokio::runtime::Runtime::new()?;
    let handle = rt.handle().clone();

    // Open the central knowledge store (blocking on the tokio runtime).
    let knowledge_path = store::base_dir()?.join("knowledge");
    let db = handle.block_on(async { open_knowledge_db(&knowledge_path).await })?;
    let db = Arc::new(db);

    let shutting_down = install_daemon_signals(&handle);

    // Idle self-reap: same logic as the MCP daemon's reaper_loop.
    {
        let flag = Arc::clone(&shutting_down);
        handle.spawn(reaper_loop(flag));
    }

    // Advisory pidfile.
    let pid_path = store::knowledge_daemon_pid_path()?;
    let _ = store::write_knowledge_daemon_pid();

    // Bind the global unix socket (bind = liveness oracle).
    let sock_path = store::knowledge_daemon_sock_path()?;
    let listener = {
        let _enter = handle.enter();
        crate::ipc::server::bind(&sock_path)?
    };

    // Accept loop.
    handle.block_on(accept_loop(listener, db, &shutting_down));

    // Graceful teardown.
    drop(rt);
    #[cfg(unix)]
    let _ = std::fs::remove_file(&sock_path);
    let _ = std::fs::remove_file(&pid_path);

    Ok(())
}

// ── Database open + schema ────────────────────────────────────────────

async fn open_knowledge_db(knowledge_path: &std::path::Path) -> Result<Surreal<Db>> {
    let path_str = knowledge_path.to_string_lossy().to_string();
    let db = Surreal::<Db>::new(path_str).await?;
    db.use_ns("koma").use_db("knowledge").await?;

    // Schema: fact, entity, and the relationship tables that build the graph.
    db.query(
        "DEFINE TABLE IF NOT EXISTS fact SCHEMALESS;
         DEFINE FIELD IF NOT EXISTS fact_id ON fact TYPE string;
         DEFINE FIELD IF NOT EXISTS content ON fact TYPE string;
         DEFINE FIELD IF NOT EXISTS category ON fact TYPE string;
         DEFINE FIELD IF NOT EXISTS confidence ON fact TYPE float;
         DEFINE FIELD IF NOT EXISTS trust ON fact TYPE float;
         DEFINE FIELD IF NOT EXISTS embedding ON fact TYPE array;
         DEFINE FIELD IF NOT EXISTS reinforcement_count ON fact TYPE int;
         DEFINE FIELD IF NOT EXISTS created_at ON fact TYPE int;
         DEFINE FIELD IF NOT EXISTS last_reinforced ON fact TYPE int;
         DEFINE INDEX IF NOT EXISTS fact_vec ON fact
             FIELDS embedding HNSW DIMENSION 384 DIST COSINE;

         DEFINE TABLE IF NOT EXISTS entity SCHEMALESS;
         DEFINE FIELD IF NOT EXISTS entity_id ON entity TYPE string;
         DEFINE FIELD IF NOT EXISTS entity_type ON entity TYPE string;
         DEFINE FIELD IF NOT EXISTS name ON entity TYPE string;
         DEFINE FIELD IF NOT EXISTS aliases ON entity TYPE array;
         DEFINE FIELD IF NOT EXISTS embedding ON entity TYPE array;
         DEFINE INDEX IF NOT EXISTS entity_vec ON entity
             FIELDS embedding HNSW DIMENSION 384 DIST COSINE;

         DEFINE TABLE IF NOT EXISTS memory_edge TYPE RELATION FROM entity TO entity SCHEMALESS;

         DEFINE TABLE IF NOT EXISTS produced TYPE RELATION FROM fact TO entity SCHEMALESS;",
    )
    .await?;

    Ok(db)
}

// ── Idle reaper (same logic as MCP daemon) ────────────────────────────

const REAPER_POLL: std::time::Duration = std::time::Duration::from_secs(15);
const REAPER_INITIAL_GRACE: std::time::Duration = std::time::Duration::from_secs(15);
const REAPER_EMPTY_STREAK_TO_EXIT: u32 = 2;

async fn reaper_loop(shutting_down: Arc<std::sync::atomic::AtomicBool>) {
    use std::sync::atomic::Ordering;

    tokio::time::sleep(REAPER_INITIAL_GRACE).await;

    let mut empty_streak: u32 = 0;
    loop {
        if shutting_down.load(Ordering::Relaxed) {
            return;
        }
        if run_dir_has_socket() {
            empty_streak = 0;
        } else {
            empty_streak = empty_streak.saturating_add(1);
            if empty_streak >= REAPER_EMPTY_STREAK_TO_EXIT {
                shutting_down.store(true, Ordering::Relaxed);
                return;
            }
        }
        tokio::time::sleep(REAPER_POLL).await;
    }
}

#[cfg(unix)]
fn run_dir_has_socket() -> bool {
    let dir = match store::run_dir() {
        Ok(d) => d,
        Err(_) => return true,
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return true,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("sock") {
            return true;
        }
    }
    false
}

#[cfg(windows)]
fn run_dir_has_socket() -> bool {
    !store::list_koma_session_pipes().is_empty()
}

// ── Accept loop ────────────────────────────────────────────────────────

const ACCEPT_POLL: std::time::Duration = std::time::Duration::from_millis(200);

async fn accept_loop(
    listener: crate::ipc::IpcListener,
    db: Arc<Surreal<Db>>,
    shutting_down: &Arc<std::sync::atomic::AtomicBool>,
) {
    use std::sync::atomic::Ordering;

    loop {
        if shutting_down.load(Ordering::Relaxed) {
            return;
        }
        match tokio::time::timeout(ACCEPT_POLL, listener.accept()).await {
            Ok(Ok((stream, _addr))) => {
                let db = Arc::clone(&db);
                let flag = Arc::clone(shutting_down);
                tokio::spawn(async move {
                    connection_loop(stream, db, flag).await;
                });
            }
            Err(_elapsed) => {}
            Ok(Err(_e)) => {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }
    }
}

// ── Per-connection request loop ────────────────────────────────────────

async fn connection_loop(
    mut stream: crate::ipc::IpcStream,
    db: Arc<Surreal<Db>>,
    shutting_down: Arc<std::sync::atomic::AtomicBool>,
) {
    let mut reader = FrameReader::new();
    loop {
        let bytes = match read_frame_from(&mut stream, &mut reader).await {
            Ok(b) => b,
            Err(_) => return,
        };
        let req: KnowledgeRequest = match serde_json::from_slice(&bytes) {
            Ok(r) => r,
            Err(e) => {
                let _ = respond(
                    &mut stream,
                    &KnowledgeResponse::Error(format!("bad request: {e}")),
                )
                .await;
                return;
            }
        };

        let resp = handle_request(req, &db, &shutting_down).await;
        if respond(&mut stream, &resp).await.is_err() {
            return;
        }
    }
}

async fn respond(
    stream: &mut crate::ipc::IpcStream,
    resp: &KnowledgeResponse,
) -> std::io::Result<()> {
    let bytes = match serde_json::to_vec(resp) {
        Ok(b) => b,
        Err(e) => serde_json::to_vec(&KnowledgeResponse::Error(format!("encode failed: {e}")))
            .unwrap_or_else(|_| b"{\"Error\":\"encode failed\"}".to_vec()),
    };
    write_frame_to(stream, &bytes).await
}

// ── Request handler ────────────────────────────────────────────────────

async fn handle_request(
    req: KnowledgeRequest,
    db: &Arc<Surreal<Db>>,
    shutting_down: &std::sync::atomic::AtomicBool,
) -> KnowledgeResponse {
    match req {
        KnowledgeRequest::PushFact {
            fact_id,
            content,
            category,
            confidence,
            embedding,
        } => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            // Bare fact_id from the client — record RID is `fact:{fact_id}`.
            let rid = format!("fact:{fact_id}");
            let result = db
                .query("CREATE type::thing($rid) CONTENT $data")
                .bind(("rid", rid))
                .bind((
                    "data",
                    serde_json::json!({
                        "fact_id": fact_id.clone(),
                        "content": content.clone(),
                        "category": category,
                        "confidence": confidence,
                        "trust": confidence,
                        "embedding": embedding,
                        "reinforcement_count": 0,
                        "created_at": now,
                        "last_reinforced": now,
                    }),
                ))
                .await;
            match result {
                Ok(_) => {
                    // Spawn entity extraction in the background — non-blocking,
                    // the Ack returns immediately.
                    let db_ref = db.clone();
                    let fid = fact_id;
                    let c = content;
                    tokio::spawn(async move {
                        match super::extractor::extract_and_resolve(&db_ref, &c).await {
                            Ok(resolved) => {
                                let _ = super::extractor::relate_entities(&db_ref, &fid, &resolved)
                                    .await;
                            }
                            Err(e) => {
                                eprintln!(
                                    "knowledge daemon: entity extraction failed for {fid}: {e}"
                                );
                            }
                        }
                    });
                    KnowledgeResponse::Ack
                }
                Err(e) => KnowledgeResponse::Error(format!("push fact failed: {e}")),
            }
        }

        KnowledgeRequest::Expand { query_vec, limit } => {
            let ef = (limit * 2).max(100);
            let query_str = format!(
                "SELECT fact_id, content, category, confidence, trust,
                        reinforcement_count, created_at, last_reinforced,
                        vector::distance::knn() AS distance
                 FROM fact
                 WHERE embedding <|{limit},{ef}|> $query_vec
                 ORDER BY distance"
            );
            let mut results = match db.query(&query_str).bind(("query_vec", query_vec)).await {
                Ok(r) => r,
                Err(e) => return KnowledgeResponse::Error(format!("expand query failed: {e}")),
            };

            let ids: Vec<String> = results.take("fact_id").unwrap_or_default();
            let contents: Vec<String> = results.take("content").unwrap_or_default();
            let categories: Vec<String> = results.take("category").unwrap_or_default();
            let confidences: Vec<f64> = results.take("confidence").unwrap_or_default();
            let trusts: Vec<f64> = results.take("trust").unwrap_or_default();
            let rcs: Vec<i64> = results.take("reinforcement_count").unwrap_or_default();
            let cas: Vec<i64> = results.take("created_at").unwrap_or_default();
            let lrs: Vec<i64> = results.take("last_reinforced").unwrap_or_default();

            let n = ids
                .len()
                .min(contents.len())
                .min(categories.len())
                .min(confidences.len())
                .min(trusts.len())
                .min(rcs.len())
                .min(cas.len())
                .min(lrs.len());

            let facts: Vec<crate::ipc::knowledge_proto::KnowledgeFact> = (0..n)
                .map(|i| crate::ipc::knowledge_proto::KnowledgeFact {
                    id: ids[i].clone(),
                    content: contents[i].clone(),
                    category: categories[i].clone(),
                    confidence: confidences[i],
                    trust: trusts[i],
                    reinforcement_count: rcs[i],
                    created_at: cas[i],
                    last_reinforced: lrs[i],
                })
                .collect();

            // Graph traversal: for matched facts, fetch connected entities
            // and related facts reachable through the entity graph.
            let (entities, related_facts) = traverse_graph(db, &ids).await.unwrap_or_default();

            KnowledgeResponse::ExpandResult {
                facts,
                entities,
                related_facts,
            }
        }

        KnowledgeRequest::Status => {
            let fact_count: u64 = match db.query("SELECT count() FROM fact GROUP ALL").await {
                Ok(mut r) => {
                    let counts: Vec<u64> = r.take("count").unwrap_or_default();
                    counts.into_iter().next().unwrap_or(0)
                }
                Err(_) => 0,
            };
            let entity_count: u64 = match db.query("SELECT count() FROM entity GROUP ALL").await {
                Ok(mut r) => {
                    let counts: Vec<u64> = r.take("count").unwrap_or_default();
                    counts.into_iter().next().unwrap_or(0)
                }
                Err(_) => 0,
            };
            KnowledgeResponse::Status {
                fact_count,
                entity_count,
            }
        }

        KnowledgeRequest::Shutdown => {
            shutting_down.store(true, std::sync::atomic::Ordering::Relaxed);
            KnowledgeResponse::Ack
        }
    }
}

// ── Graph traversal ───────────────────────────────────────────────────

/// Traverse the entity graph from a set of matched fact IDs.
///
/// For each fact, follows `->produced->entity` to get connected entities,
/// then `->memory_edge->entity` to get related entities, and finally
/// `<-produced<-fact` to pull in related facts from those entities.
///
/// Uses SurrealQL FETCH to do the full 1-hop traversal in a single query.
async fn traverse_graph(
    db: &Surreal<Db>,
    fact_ids: &[String],
) -> anyhow::Result<(
    Vec<crate::ipc::knowledge_proto::KnowledgeEntity>,
    Vec<crate::ipc::knowledge_proto::KnowledgeFact>,
)> {
    if fact_ids.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    // Build a single query that:
    // 1. Selects the matched facts
    // 2. FETCH-es the entities via produced edges
    // 3. From those entities, follows memory_edge to related entities
    // 4. From related entities, follows back through produced to related facts
    //
    // We use an IN clause with the fact_id field (bare, not record ID) since
    // SurrealDB RELATE uses record IDs but the fact_id field is a plain string.

    // Step 1: fetch entities for matched facts
    let entities = fetch_entities_for_facts(db, fact_ids).await?;

    // Step 2: fetch related facts through the entity graph (1-hop)
    let related_facts = fetch_related_facts(db, fact_ids).await?;

    Ok((entities, related_facts))
}

async fn fetch_entities_for_facts(
    db: &Surreal<Db>,
    fact_ids: &[String],
) -> anyhow::Result<Vec<crate::ipc::knowledge_proto::KnowledgeEntity>> {
    let mut seen_entities: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut entities: Vec<crate::ipc::knowledge_proto::KnowledgeEntity> = Vec::new();

    for id in fact_ids {
        let rid = format!("fact:{id}");
        // Forward traversal: fact -> produced -> entity
        let mut result = db
            .query("SELECT ->produced->entity AS entities FROM type::thing($rid)")
            .bind(("rid", rid))
            .await?;

        let entity_rows: Vec<serde_json::Value> = result.take("entities").unwrap_or_default();
        for row in entity_rows {
            if let Some(entity_id) = row.get("entity_id").and_then(|v| v.as_str()) {
                if seen_entities.insert(entity_id.to_string()) {
                    let name = row
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let entity_type = row
                        .get("entity_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("concept")
                        .to_string();
                    let aliases: Vec<String> = row
                        .get("aliases")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    entities.push(crate::ipc::knowledge_proto::KnowledgeEntity {
                        entity_id: entity_id.to_string(),
                        entity_type,
                        name,
                        aliases,
                    });
                }
            }
        }
    }

    Ok(entities)
}

async fn fetch_related_facts(
    db: &Surreal<Db>,
    fact_ids: &[String],
) -> anyhow::Result<Vec<crate::ipc::knowledge_proto::KnowledgeFact>> {
    // Two-step: first collect all entity IDs connected to the matched facts,
    // then find facts connected to those entities (excluding the original facts).
    let mut entity_rids: Vec<String> = Vec::new();
    for id in fact_ids {
        let rid = format!("fact:{id}");
        let mut result = db
            .query("SELECT ->produced->entity.id AS eid FROM ONLY type::thing($rid)")
            .bind(("rid", rid))
            .await?;

        let ids: Vec<String> = result.take("eid").unwrap_or_default();
        entity_rids.extend(ids);
    }

    if entity_rids.is_empty() {
        return Ok(Vec::new());
    }

    let mut seen: std::collections::HashSet<String> = fact_ids.iter().cloned().collect(); // exclude originals
    let mut related: Vec<crate::ipc::knowledge_proto::KnowledgeFact> = Vec::new();

    for e_rid in &entity_rids {
        // For each entity, get facts via reverse produced edge: entity <-produced- fact
        let mut result = db
            .query("SELECT <-produced<-fact.* AS facts FROM type::thing($rid)")
            .bind(("rid", e_rid.clone()))
            .await?;

        let fact_rows: Vec<serde_json::Value> = result.take("facts").unwrap_or_default();
        for row in fact_rows {
            if let Some(fact_id) = row.get("fact_id").and_then(|v| v.as_str()) {
                if seen.insert(fact_id.to_string()) {
                    let content = row
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let category = row
                        .get("category")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let confidence = row
                        .get("confidence")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0);
                    let trust = row.get("trust").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let reinforcement_count = row
                        .get("reinforcement_count")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let created_at = row.get("created_at").and_then(|v| v.as_i64()).unwrap_or(0);
                    let last_reinforced = row
                        .get("last_reinforced")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    related.push(crate::ipc::knowledge_proto::KnowledgeFact {
                        id: fact_id.to_string(),
                        content,
                        category,
                        confidence,
                        trust,
                        reinforcement_count,
                        created_at,
                        last_reinforced,
                    });
                }
            }
        }
    }

    Ok(related)
}
