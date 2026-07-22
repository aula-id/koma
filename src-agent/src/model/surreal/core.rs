//! Persistent SurrealDB mirror backed by SurrealKV.
//!
//! `open_db` creates/opens the on-disk database and defines all schemas
//! (message, graph, memory tables + indexes). `start_sync` spawns a
//! fire-and-forget background thread that reads the SQLite message log,
//! embeds every message via fastembed (BGE-small-en-v1.5, 384d), and
//! batch-inserts them into the SurrealDB index.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

use surrealdb::Surreal;
use surrealdb::engine::local::Db;

// ---------------------------------------------------------------------------
// Embedder
// ---------------------------------------------------------------------------

pub type Embedding = Vec<f32>;

static EMBEDDER: OnceLock<TextEmbedding> = OnceLock::new();

fn get_embedder() -> &'static TextEmbedding {
    EMBEDDER.get_or_init(|| {
        TextEmbedding::try_new(InitOptions::new(EmbeddingModel::BGESmallENV15))
            .expect("failed to initialise fastembed BGE-small-en-v1.5 model")
    })
}

pub(crate) fn embed_batch(texts: Vec<String>) -> Vec<Embedding> {
    if texts.is_empty() {
        return Vec::new();
    }
    let refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
    get_embedder().embed(refs, None).unwrap_or_default()
}

pub(crate) fn embed_one(text: &str) -> Embedding {
    let batch = embed_batch(vec![text.to_string()]);
    batch.into_iter().next().unwrap_or_else(|| vec![0.0f32; 384])
}

// ---------------------------------------------------------------------------
// Sync state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncState {
    Unsynced,
    Syncing,
    Done,
}

fn sync_lock_path(session_dir: &Path) -> PathBuf {
    session_dir.join("messages.surreal.sync.lock")
}

fn sync_done_path(session_dir: &Path) -> PathBuf {
    session_dir.join("messages.surreal.sync.done")
}

pub fn sync_state(session_dir: &Path) -> SyncState {
    if sync_done_path(session_dir).exists() {
        SyncState::Done
    } else if sync_lock_path(session_dir).exists() {
        SyncState::Syncing
    } else {
        SyncState::Unsynced
    }
}

// ---------------------------------------------------------------------------
// blocking_block
// ---------------------------------------------------------------------------

pub(crate) fn blocking_block<F, Fut, T>(f: F) -> T
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = T> + Send,
    T: Send + 'static,
{
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime build failed");
        rt.block_on(f())
    })
    .join()
    .expect("surreal blocking task panicked")
}

// ---------------------------------------------------------------------------
// open_db
// ---------------------------------------------------------------------------

pub(crate) async fn open_db(session_dir: &Path) -> anyhow::Result<Surreal<Db>> {
    let db_path = session_dir.join("messages.surreal");
    let path_str = db_path.to_string_lossy().to_string();

    let db = Surreal::<Db>::new(path_str).await?;
    db.use_ns("koma").use_db("messages").await?;

    // ── Message tables ──────────────────────────────────────────────
    db.query(
        "DEFINE TABLE IF NOT EXISTS message SCHEMALESS;
         DEFINE FIELD IF NOT EXISTS sqlite_id ON message TYPE int;
         DEFINE FIELD IF NOT EXISTS role ON message TYPE string;
         DEFINE FIELD IF NOT EXISTS content ON message TYPE string;
         DEFINE FIELD IF NOT EXISTS embedding ON message TYPE array;
         DEFINE FIELD IF NOT EXISTS created_at ON message TYPE int;

         DEFINE ANALYZER IF NOT EXISTS simple TOKENIZERS blank, class, camel;
         DEFINE INDEX IF NOT EXISTS message_fts ON message
             FIELDS content SEARCH ANALYZER simple BM25(1.2, 0.75);
         DEFINE INDEX IF NOT EXISTS message_vec ON message
             FIELDS embedding HNSW DIMENSION 384 DIST COSINE;",
    )
    .await?;

    // ── Graph tables ────────────────────────────────────────────────
    db.query(
        "DEFINE TABLE IF NOT EXISTS agent SCHEMALESS;
         DEFINE FIELD IF NOT EXISTS agent_id ON agent TYPE string;
         DEFINE FIELD IF NOT EXISTS name ON agent TYPE string;
         DEFINE FIELD IF NOT EXISTS last_seen ON agent TYPE int;

         DEFINE TABLE IF NOT EXISTS tool_call SCHEMALESS;
         DEFINE FIELD IF NOT EXISTS tc_id ON tool_call TYPE string;
         DEFINE FIELD IF NOT EXISTS agent_id ON tool_call TYPE string;
         DEFINE FIELD IF NOT EXISTS tool_name ON tool_call TYPE string;
         DEFINE FIELD IF NOT EXISTS args_summary ON tool_call TYPE string;
         DEFINE FIELD IF NOT EXISTS result_snippet ON tool_call TYPE string;
         DEFINE FIELD IF NOT EXISTS embedding ON tool_call TYPE array;
         DEFINE FIELD IF NOT EXISTS timestamp ON tool_call TYPE int;

         DEFINE TABLE IF NOT EXISTS delegation TYPE RELATION FROM agent TO agent SCHEMALESS;
         DEFINE TABLE IF NOT EXISTS called TYPE RELATION FROM agent TO tool_call SCHEMALESS;
         DEFINE TABLE IF NOT EXISTS produced TYPE RELATION FROM tool_call TO message SCHEMALESS;",
    )
    .await?;

    // ── Memory tables ───────────────────────────────────────────────
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

         DEFINE TABLE IF NOT EXISTS episode SCHEMALESS;
         DEFINE FIELD IF NOT EXISTS episode_id ON episode TYPE string;
         DEFINE FIELD IF NOT EXISTS narrative ON episode TYPE string;
         DEFINE FIELD IF NOT EXISTS decision_point ON episode TYPE string;
         DEFINE FIELD IF NOT EXISTS embedding ON episode TYPE array;
         DEFINE FIELD IF NOT EXISTS created_at ON episode TYPE int;

         DEFINE TABLE IF NOT EXISTS entity SCHEMALESS;
         DEFINE FIELD IF NOT EXISTS entity_id ON entity TYPE string;
         DEFINE FIELD IF NOT EXISTS entity_type ON entity TYPE string;
         DEFINE FIELD IF NOT EXISTS name ON entity TYPE string;
         DEFINE FIELD IF NOT EXISTS aliases ON entity TYPE array;

         DEFINE TABLE IF NOT EXISTS memory_edge TYPE RELATION FROM entity TO entity SCHEMALESS;",
    )
    .await?;

    Ok(db)
}

// ---------------------------------------------------------------------------
// Sync worker
// ---------------------------------------------------------------------------

pub fn start_sync(session_dir: &Path) {
    let lock_path = sync_lock_path(session_dir);
    let _ = std::fs::write(&lock_path, format!("{}", std::process::id()));

    let sd = session_dir.to_path_buf();
    std::thread::spawn(move || {
        let result = do_sync(&sd);
        let _ = std::fs::remove_file(sync_lock_path(&sd));
        match result {
            Ok(n) => {
                let _ = std::fs::write(sync_done_path(&sd), format!("{n}"));
            }
            Err(e) => {
                crate::model::store::append_error_log(
                    &sd,
                    "SurrealDB sync failed",
                    &e.to_string(),
                );
            }
        }
    });
}

fn do_sync(session_dir: &Path) -> anyhow::Result<usize> {
    let conn = crate::model::msglog::open(session_dir)?;
    let mut stmt = conn.prepare(
        "SELECT id, role, content, created_at FROM messages ORDER BY id ASC",
    )?;
    let rows: Vec<(i64, String, String, i64)> = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0).unwrap_or(0),
                r.get::<_, String>(1).unwrap_or_default(),
                r.get::<_, String>(2).unwrap_or_default(),
                r.get::<_, i64>(3).unwrap_or(0),
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();

    if rows.is_empty() {
        return Ok(0);
    }

    let texts: Vec<String> = rows.iter().map(|(_, _, c, _)| c.clone()).collect();
    let embeddings = embed_batch(texts);
    let total = rows.len();

    let sd = session_dir.to_path_buf();
    blocking_block(move || {
        let sd = sd.clone();
        async move {
            let db = open_db(&sd).await?;

            for (i, (sqlite_id, role, content, created_at)) in rows.iter().enumerate() {
                let emb = embeddings.get(i).cloned().unwrap_or_else(|| vec![0.0f32; 384]);
                let _ = db
                    .query("DELETE FROM message WHERE sqlite_id = $id")
                    .bind(("id", *sqlite_id))
                    .await;
                let _ = db
                    .query("CREATE message CONTENT $data")
                    .bind(("data", serde_json::json!({
                        "sqlite_id": *sqlite_id,
                        "role": role,
                        "content": content,
                        "embedding": emb,
                        "created_at": *created_at,
                    })))
                    .await;
            }

            Ok::<usize, anyhow::Error>(total)
        }
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn test_dir() -> PathBuf {
        std::env::temp_dir().join("koma_test_surreal_core")
    }

    #[test]
    fn test_sync_state_transitions() {
        let dir = test_dir();
        let _ = fs::create_dir_all(&dir);
        let _ = fs::remove_file(sync_lock_path(&dir));
        let _ = fs::remove_file(sync_done_path(&dir));
        assert_eq!(sync_state(&dir), SyncState::Unsynced);
        fs::write(sync_lock_path(&dir), "123").unwrap();
        assert_eq!(sync_state(&dir), SyncState::Syncing);
        fs::write(sync_done_path(&dir), "42").unwrap();
        assert_eq!(sync_state(&dir), SyncState::Done);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_embed_batch_returns_correct_dimensions() {
        let embeddings = embed_batch(vec!["hello world".into(), "test".into()]);
        assert_eq!(embeddings.len(), 2);
        for emb in &embeddings {
            assert_eq!(emb.len(), 384);
        }
    }

    #[test]
    fn test_embed_one_returns_correct_dimensions() {
        let emb = embed_one("hello");
        assert_eq!(emb.len(), 384);
    }
}
