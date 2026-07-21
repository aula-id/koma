//! SurrealDB mirror — turbo read layer on top of SQLite.
//!
//! Phase 1: single best-effort search function. Builds an in-memory SurrealDB
//! from SQLite on every call, no persistence. Caller falls back to FTS5 on any
//! error.
//!
//! Future phases: persistent mirror, graph edges, vector search.

use std::path::Path;

/// Search messages via embedded SurrealDB full-text index.
/// Best-effort: returns empty vec on any error.
pub fn search_messages(session_dir: &Path, query: &str, limit: usize) -> Vec<MessageMatch> {
    let q = query.trim();
    if q.is_empty() || !session_dir.join("messages.sqlite").exists() {
        return Vec::new();
    }

    let sd = session_dir.to_path_buf();
    let q_owned = q.to_string();
    std::thread::spawn(move || -> Vec<MessageMatch> {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(_) => return Vec::new(),
        };
        rt.block_on(async { search_async(&sd, &q_owned, limit).await.unwrap_or_default() })
    })
    .join()
    .unwrap_or_default()
}

async fn search_async(
    session_dir: &Path,
    query: &str,
    limit: usize,
) -> anyhow::Result<Vec<MessageMatch>> {
    use surrealdb::Surreal;
    use surrealdb::engine::local::Mem;

    let db = Surreal::new::<Mem>(()).await?;
    db.use_ns("koma").use_db("messages").await?;

    db.query(
        "DEFINE TABLE IF NOT EXISTS message SCHEMALESS;
         DEFINE ANALYZER IF NOT EXISTS simple TOKENIZERS blank, class, camel;
         DEFINE INDEX IF NOT EXISTS message_fts ON message
             FIELDS content SEARCH ANALYZER simple BM25(1.2,0.75);"
    ).await?;

    // Index from sqlite
    let conn = crate::model::msglog::open(session_dir)?;
    let mut stmt = conn.prepare("SELECT id, role, content FROM messages ORDER BY id ASC")?;
    let rows: Vec<_> = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0).unwrap_or(0),
                r.get::<_, String>(1).unwrap_or_default(),
                r.get::<_, String>(2).unwrap_or_default(),
            ))
        })?
        .filter_map(|r| r.ok())
        .take(100_000)
        .collect();

    for (sqlite_id, role, content) in &rows {
        let _ = db
            .query("CREATE message CONTENT $data")
            .bind(("data", serde_json::json!({
                "sqlite_id": *sqlite_id,
                "role": role.as_str(),
                "content": content.as_str(),
            })))
            .await;
    }

    // Search
    let terms: Vec<String> = query
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(|t| format!("@@{}", t))
        .collect();
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let match_expr = terms.join(" OR ");
    let surql = format!(
        "SELECT sqlite_id, role, string::slice(content, 0, 300) AS snippet
         FROM message WHERE content {match_expr}
         ORDER BY search::score(content) DESC LIMIT {limit}"
    );
    let mut res = db.query(surql).await?;
    let ids: Vec<i64> = res.take("sqlite_id").unwrap_or_default();
    let roles: Vec<String> = res.take("role").unwrap_or_default();
    let snippets: Vec<String> = res.take("snippet").unwrap_or_default();
    let n = ids.len().min(roles.len()).min(snippets.len());
    Ok((0..n)
        .map(|i| MessageMatch {
            id: ids[i],
            role: roles[i].clone(),
            snippet: snippets[i].trim().to_string(),
        })
        .collect())
}

#[derive(Debug, Clone, Default)]
pub struct MessageMatch {
    pub id: i64,
    pub role: String,
    pub snippet: String,
}
