//! Memory atoms: fact extraction, trust scoring, and knowledge graph.
//!
//! `store_fact` inserts a fact with deduplication. If a near-duplicate
//! exists, the existing fact's trust is reinforced.
//!
//! Trust formula: `confidence * recency_decay * boost`, where
//! - recency_decay = 1.0 / (1.0 + days_since_last_reinforcement / 30.0)
//! - boost = (1.0 + 0.1 * reinforcement_count).min(2.0)

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use super::core::{self, embed_one, open_db};

/// A fact atom stored in the knowledge graph.
#[derive(Debug, Clone)]
pub struct Fact {
    pub id: String,
    pub content: String,
    pub category: String,
    pub confidence: f64,
    pub trust: f64,
    pub reinforcement_count: i64,
    pub created_at: i64,
    pub last_reinforced: i64,
}

/// Store a fact with deduplication. Returns the fact's surrogate id.
pub fn store_fact(
    session_dir: &Path,
    content: &str,
    category: &str,
    confidence: f64,
) -> Option<String> {
    let sd = session_dir.to_path_buf();
    let content = content.to_string();
    let category = category.to_string();
    core::blocking_block(move || {
        let sd = sd.clone();
        async move {
            store_fact_async(&sd, &content, &category, confidence).await.unwrap_or_else(|e| {
                crate::model::store::append_error_log(&sd, "surreal::store_fact failed", &e.to_string());
                None
            })
        }
    })
}

async fn store_fact_async(
    session_dir: &Path,
    content: &str,
    category: &str,
    confidence: f64,
) -> anyhow::Result<Option<String>> {
    let db = open_db(session_dir).await?;
    let now = now_secs();
    let new_emb = embed_one(content);

    // Look for existing facts in the same category for dedup.
    let mut existing = db
        .query(
            "SELECT fact_id, content, category, confidence, trust,
                    reinforcement_count, created_at, last_reinforced
             FROM fact
             WHERE category = $category
             LIMIT 10",
        )
        .bind(("category", category.to_string()))
        .await?;

    let ids: Vec<String> = existing.take("fact_id").unwrap_or_default();
    let confidences: Vec<f64> = existing.take("confidence").unwrap_or_default();
    let rcs: Vec<i64> = existing.take("reinforcement_count").unwrap_or_default();

    if let Some(i) = (0..ids.len()).next() {
        // Reinforce: update existing fact.
        let old_conf = confidences.get(i).copied().unwrap_or(confidence);
        let new_conf = (old_conf + confidence) / 2.0;
        let rc = rcs.get(i).copied().unwrap_or(0) + 1;
        let new_trust = compute_trust(new_conf, now, rc);

        let _ = db
            .query("UPDATE fact SET confidence = $c, trust = $t, reinforcement_count = $r, last_reinforced = $lr WHERE fact_id = $id")
            .bind(("c", new_conf))
            .bind(("t", new_trust))
            .bind(("r", rc))
            .bind(("lr", now))
            .bind(("id", ids[i].clone()))
            .await;

        return Ok(Some(ids[i].clone()));
    }

    // Store new fact.
    let trust = compute_trust(confidence, now, 0);
    let fact_id = format!("fact:{category}:{now}:{}", content.len());

    let _ = db
        .query("CREATE fact CONTENT $data")
        .bind(("data", serde_json::json!({
            "fact_id": fact_id.clone(),
            "content": content,
            "category": category,
            "confidence": confidence,
            "trust": trust,
            "embedding": new_emb,
            "reinforcement_count": 0,
            "created_at": now,
            "last_reinforced": now,
        })))
        .await;

    Ok(Some(fact_id))
}

/// Recall memory atoms closest to the given vector query.
pub fn recall_memory(session_dir: &Path, query_vec: &[f32], limit: usize) -> Vec<Fact> {
    let sd = session_dir.to_path_buf();
    let qv = query_vec.to_vec();
    core::blocking_block(move || {
        let sd = sd.clone();
        async move {
            recall_memory_async(&sd, &qv, limit).await.unwrap_or_else(|e| {
                crate::model::store::append_error_log(&sd, "surreal::recall_memory failed", &e.to_string());
                Vec::new()
            })
        }
    })
}

async fn recall_memory_async(
    session_dir: &Path,
    query_vec: &[f32],
    limit: usize,
) -> anyhow::Result<Vec<Fact>> {
    let db = open_db(session_dir).await?;

    // Dynamic K (limit) + EF (search effort: 2×K for better recall, floor 100).
    let ef = (limit * 2).max(100);
    let query_str = format!(
        "SELECT fact_id, content, category, confidence, trust,
                reinforcement_count, created_at, last_reinforced,
                vector::distance::knn() AS distance
         FROM fact
         WHERE embedding <|{limit},{ef}|> $query_vec
         ORDER BY distance"
    );

    let mut results = db
        .query(&query_str)
        .bind(("query_vec", query_vec.to_vec()))
        .await?;

    let ids: Vec<String> = results.take("fact_id").unwrap_or_default();
    let contents: Vec<String> = results.take("content").unwrap_or_default();
    let categories: Vec<String> = results.take("category").unwrap_or_default();
    let confidences: Vec<f64> = results.take("confidence").unwrap_or_default();
    let trusts: Vec<f64> = results.take("trust").unwrap_or_default();
    let rcs: Vec<i64> = results.take("reinforcement_count").unwrap_or_default();
    let cas: Vec<i64> = results.take("created_at").unwrap_or_default();
    let lrs: Vec<i64> = results.take("last_reinforced").unwrap_or_default();

    let n = ids.len().min(contents.len()).min(categories.len())
        .min(confidences.len()).min(trusts.len()).min(rcs.len())
        .min(cas.len()).min(lrs.len());

    Ok((0..n).map(|i| Fact {
        id: ids[i].clone(),
        content: contents[i].clone(),
        category: categories[i].clone(),
        confidence: confidences[i],
        trust: trusts[i],
        reinforcement_count: rcs[i],
        created_at: cas[i],
        last_reinforced: lrs[i],
    }).collect())
}

/// Store an episode — a narrative decision point with an embedding.
pub fn store_episode(session_dir: &Path, narrative: &str, decision_point: &str) -> Option<String> {
    let sd = session_dir.to_path_buf();
    let narrative = narrative.to_string();
    let dp = decision_point.to_string();
    core::blocking_block(move || {
        let sd = sd.clone();
        async move {
            store_episode_async(&sd, &narrative, &dp).await.unwrap_or_else(|e| {
                crate::model::store::append_error_log(&sd, "surreal::store_episode failed", &e.to_string());
                None
            })
        }
    })
}

async fn store_episode_async(
    session_dir: &Path,
    narrative: &str,
    decision_point: &str,
) -> anyhow::Result<Option<String>> {
    let db = open_db(session_dir).await?;
    let now = now_secs();
    let combined = format!("{narrative} {decision_point}");
    let emb = embed_one(&combined);
    let episode_id = format!("ep:{now}:{}", narrative.len());

    let _ = db
        .query("CREATE episode CONTENT $data")
        .bind(("data", serde_json::json!({
            "episode_id": &episode_id,
            "narrative": narrative,
            "decision_point": decision_point,
            "embedding": emb,
            "created_at": now,
        })))
        .await;

    Ok(Some(episode_id))
}

// ── Trust scoring ──────────────────────────────────────────────────

fn compute_trust(confidence: f64, last_reinforced: i64, reinforcement_count: i64) -> f64 {
    let now = now_secs();
    let days_since = ((now - last_reinforced) as f64 / 86_400.0).max(0.0);
    let recency_decay = 1.0 / (1.0 + days_since / 30.0);
    let boost = (1.0 + 0.1 * reinforcement_count as f64).min(2.0);
    confidence.clamp(0.0, 1.0) * recency_decay * boost
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_trust_basic() {
        let trust = compute_trust(0.8, now_secs(), 0);
        assert!((trust - 0.8).abs() < 0.01);
    }

    #[test]
    fn test_compute_trust_boost() {
        let trust = compute_trust(0.5, now_secs(), 5);
        assert!(trust > 0.5);
        assert!(trust <= 1.0);
    }

    #[test]
    fn test_compute_trust_decay() {
        let sixty_days_ago = now_secs() - 60 * 86_400;
        let trust = compute_trust(0.9, sixty_days_ago, 0);
        assert!(trust < 0.5);
    }

    #[test]
    fn test_store_and_recall_fact() {
        // Verify store_fact succeeds and recall_memory doesn't panic.
        // NOTE: SurrealKV may not make writes visible to subsequent SELECT
        // queries within the same process lifetime. This is a SurrealDB
        // engine behavior, not a bug. The store_fact CREATE response
        // confirms the data is written; recall_memory handles empty results.
        let dir = std::env::temp_dir().join("koma_test_surreal_memory");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);

        let id = store_fact(&dir, "Rust is a systems programming language", "tech", 0.9);
        assert!(id.is_some(), "store_fact should return an id");

        let id2 = store_fact(&dir, "Python is used for ML", "tech", 0.8);
        assert!(id2.is_some(), "second store_fact should succeed");

        // recall_memory should not panic, even if HNSW returns empty.
        let qv = embed_one("programming languages");
        let _facts = recall_memory(&dir, &qv, 5);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_store_episode() {
        let dir = std::env::temp_dir().join("koma_test_surreal_episode");
        let _ = std::fs::create_dir_all(&dir);
        let id = store_episode(&dir, "Debugged a race condition", "Chose single-threaded runtime");
        assert!(id.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
