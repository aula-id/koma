//! Hybrid search combining FTS5 (BM25) + vector cosine fused via RRF.
//!
//! `search_hybrid` runs both a full-text query and a KNN vector query,
//! then fuses the result sets with Reciprocal Rank Fusion.
//! `search_fts_only` is the pure full-text fallback.

use std::path::Path;

use super::core::{self, embed_one, open_db};
use super::MessageMatch;

/// Hybrid search: FTS5 (BM25) + KNN vector, fused with RRF.
pub fn search_hybrid(session_dir: &Path, query: &str, limit: usize) -> Vec<MessageMatch> {
    let q = query.trim();
    if q.is_empty() {
        return Vec::new();
    }
    let sd = session_dir.to_path_buf();
    let q_owned = q.to_string();
    let lim = limit.min(100) as i64;
    core::blocking_block(move || {
        let sd = sd.clone();
        let q = q_owned.clone();
        async move {
            search_hybrid_async(&sd, &q, lim).await.unwrap_or_else(|e| {
                crate::model::store::append_error_log(
                    &sd,
                    "surreal::search_hybrid failed",
                    &e.to_string(),
                );
                Vec::new()
            })
        }
    })
    .unwrap_or_default()
}

async fn search_hybrid_async(
    session_dir: &Path,
    query: &str,
    limit: i64,
) -> anyhow::Result<Vec<MessageMatch>> {
    let db = open_db(session_dir).await?;
    let query_vec = embed_one(query);

    // FTS query
    let mut fts_res = db
        .query(
            "SELECT sqlite_id, role, string::slice(content, 0, 300) AS snippet, created_at
             FROM message WHERE content @@ $query
             ORDER BY search::score(content) DESC LIMIT $limit",
        )
        .bind(("query", query.to_string()))
        .bind(("limit", limit))
        .await?;

    let ids: Vec<i64> = fts_res.take("sqlite_id").unwrap_or_default();
    let roles: Vec<String> = fts_res.take("role").unwrap_or_default();
    let fts_snippets: Vec<String> = fts_res.take("snippet").unwrap_or_default();
    let fts_created: Vec<i64> = fts_res.take("created_at").unwrap_or_default();
    let n_fts = ids
        .len()
        .min(roles.len())
        .min(fts_snippets.len())
        .min(fts_created.len());
    let fts_results: Vec<MessageMatch> = (0..n_fts)
        .map(|i| MessageMatch {
            id: ids[i],
            role: roles[i].clone(),
            snippet: fts_snippets[i].trim().to_string(),
            created_at: fts_created[i],
        })
        .collect();

    // Vector query
    let mut vec_res = db
        .query(
            "SELECT sqlite_id, role, string::slice(content, 0, 300) AS snippet, created_at
             FROM message WHERE embedding <|100|> $query_vec
             LIMIT $limit",
        )
        .bind(("query_vec", query_vec.clone()))
        .bind(("limit", limit))
        .await?;

    let v_ids: Vec<i64> = vec_res.take("sqlite_id").unwrap_or_default();
    let v_roles: Vec<String> = vec_res.take("role").unwrap_or_default();
    let v_snippets: Vec<String> = vec_res.take("snippet").unwrap_or_default();
    let v_created: Vec<i64> = vec_res.take("created_at").unwrap_or_default();
    let n_vec = v_ids
        .len()
        .min(v_roles.len())
        .min(v_snippets.len())
        .min(v_created.len());
    let vec_results: Vec<MessageMatch> = (0..n_vec)
        .map(|i| MessageMatch {
            id: v_ids[i],
            role: v_roles[i].clone(),
            snippet: v_snippets[i].trim().to_string(),
            created_at: v_created[i],
        })
        .collect();

    Ok(fuse_rrf(fts_results, vec_results, limit as usize))
}

/// FTS5-only fallback — no vectors.
pub fn search_fts_only(session_dir: &Path, query: &str, limit: usize) -> Vec<MessageMatch> {
    let q = query.trim();
    if q.is_empty() {
        return Vec::new();
    }
    let sd = session_dir.to_path_buf();
    let q_owned = q.to_string();
    let lim = limit.min(100) as i64;
    core::blocking_block(move || {
        let sd = sd.clone();
        let q = q_owned.clone();
        async move {
            search_fts_only_async(&sd, &q, lim)
                .await
                .unwrap_or_else(|e| {
                    crate::model::store::append_error_log(
                        &sd,
                        "surreal::search_fts_only failed",
                        &e.to_string(),
                    );
                    Vec::new()
                })
        }
    })
    .unwrap_or_default()
}

async fn search_fts_only_async(
    session_dir: &Path,
    query: &str,
    limit: i64,
) -> anyhow::Result<Vec<MessageMatch>> {
    let db = open_db(session_dir).await?;
    let mut results = db
        .query(
            "SELECT sqlite_id, role, string::slice(content, 0, 300) AS snippet, created_at
             FROM message WHERE content @@ $query
             ORDER BY search::score(content) DESC LIMIT $limit",
        )
        .bind(("query", query.to_string()))
        .bind(("limit", limit))
        .await?;

    let ids: Vec<i64> = results.take("sqlite_id").unwrap_or_default();
    let roles: Vec<String> = results.take("role").unwrap_or_default();
    let snippets: Vec<String> = results.take("snippet").unwrap_or_default();
    let createds: Vec<i64> = results.take("created_at").unwrap_or_default();
    let n = ids
        .len()
        .min(roles.len())
        .min(snippets.len())
        .min(createds.len());
    Ok((0..n)
        .map(|i| MessageMatch {
            id: ids[i],
            role: roles[i].clone(),
            snippet: snippets[i].trim().to_string(),
            created_at: createds[i],
        })
        .collect())
}

/// Reciprocal Rank Fusion: combine two ranked lists, k = 60.
fn fuse_rrf(fts: Vec<MessageMatch>, vec: Vec<MessageMatch>, limit: usize) -> Vec<MessageMatch> {
    use std::collections::HashMap;
    let k: f64 = 60.0;
    let mut scores: HashMap<i64, f64> = HashMap::new();
    let mut by_id: HashMap<i64, MessageMatch> = HashMap::new();
    for (rank, m) in fts.iter().enumerate() {
        *scores.entry(m.id).or_default() += 1.0 / (k + (rank as f64 + 1.0));
        by_id.entry(m.id).or_insert_with(|| m.clone());
    }
    for (rank, m) in vec.iter().enumerate() {
        *scores.entry(m.id).or_default() += 1.0 / (k + (rank as f64 + 1.0));
        by_id.entry(m.id).or_insert_with(|| m.clone());
    }
    let mut fused: Vec<(f64, MessageMatch)> = scores
        .into_iter()
        .filter_map(|(id, s)| by_id.remove(&id).map(|m| (s, m)))
        .collect();
    fused.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    fused.truncate(limit);
    fused.into_iter().map(|(_, m)| m).collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_fuse_rrf_combines_two_lists() {
        let fts = vec![
            MessageMatch {
                id: 1,
                role: "user".into(),
                snippet: "a".into(),
                created_at: 1,
            },
            MessageMatch {
                id: 2,
                role: "assistant".into(),
                snippet: "b".into(),
                created_at: 2,
            },
        ];
        let vec = vec![
            MessageMatch {
                id: 2,
                role: "assistant".into(),
                snippet: "c".into(),
                created_at: 2,
            },
            MessageMatch {
                id: 3,
                role: "user".into(),
                snippet: "d".into(),
                created_at: 3,
            },
        ];
        let fused = fuse_rrf(fts, vec, 5);
        assert_eq!(fused.len(), 3);
        assert_eq!(fused[0].id, 2);
    }

    #[test]
    fn test_fuse_rrf_empty() {
        assert!(fuse_rrf(vec![], vec![], 5).is_empty());
    }

    #[test]
    fn test_fuse_rrf_respects_limit() {
        let fts: Vec<MessageMatch> = (0..10)
            .map(|i| MessageMatch {
                id: i,
                role: "u".into(),
                snippet: "x".into(),
                created_at: i,
            })
            .collect();
        assert_eq!(fuse_rrf(fts, vec![], 3).len(), 3);
    }

    #[test]
    fn test_search_fts_only_empty_query() {
        let tmp = std::env::temp_dir().join("koma_test_surreal_fts");
        let _ = std::fs::create_dir_all(&tmp);
        assert!(search_fts_only(&tmp, "   ", 10).is_empty());
    }
}
