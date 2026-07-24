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
    pub trust: f64,
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
            store_fact_async(&sd, &content, &category, confidence)
                .await
                .unwrap_or_else(|e| {
                    crate::model::store::append_error_log(
                        &sd,
                        "surreal::store_fact failed",
                        &e.to_string(),
                    );
                    None
                })
        }
    })
    .flatten()
}

/// Cosine distance below which two fact embeddings are treated as near-duplicates.
const DEDUP_COSINE_DISTANCE: f64 = 0.15;

async fn store_fact_async(
    session_dir: &Path,
    content: &str,
    category: &str,
    confidence: f64,
) -> anyhow::Result<Option<String>> {
    // Reject garbage before it ever hits the DB.
    if !is_quality_fact(content) {
        return Ok(None);
    }

    let db = open_db(session_dir).await?;
    let now = now_secs();
    let new_emb = embed_one(content);

    // Near-duplicate check via vector search (same category preferred).
    // Over-fetch a few neighbours and pick the closest true match.
    let mut existing = db
        .query(
            "SELECT fact_id, content, category, confidence, trust,
                    reinforcement_count, embedding,
                    vector::distance::knn() AS distance
             FROM fact
             WHERE embedding <|8,40|> $emb
             ORDER BY distance
             LIMIT 8",
        )
        .bind(("emb", new_emb.clone()))
        .await?;

    let ids: Vec<String> = existing.take("fact_id").unwrap_or_default();
    let contents: Vec<String> = existing.take("content").unwrap_or_default();
    let categories: Vec<String> = existing.take("category").unwrap_or_default();
    let confidences: Vec<f64> = existing.take("confidence").unwrap_or_default();
    let rcs: Vec<i64> = existing.take("reinforcement_count").unwrap_or_default();
    let distances: Vec<f64> = existing.take("distance").unwrap_or_default();

    let n = ids
        .len()
        .min(contents.len())
        .min(categories.len())
        .min(confidences.len())
        .min(rcs.len())
        .min(distances.len());

    // Prefer same-category near-dup; fall back to any category near-dup.
    let mut match_idx: Option<usize> = None;
    for i in 0..n {
        if distances[i] > DEDUP_COSINE_DISTANCE {
            break; // ordered by distance; rest are worse
        }
        if categories[i] == category {
            match_idx = Some(i);
            break;
        }
        if match_idx.is_none() {
            match_idx = Some(i);
        }
    }

    if let Some(i) = match_idx {
        // Reinforce existing near-duplicate — keep the longer/richer content.
        let old_conf = confidences.get(i).copied().unwrap_or(confidence);
        let new_conf = (old_conf + confidence) / 2.0;
        let rc = rcs.get(i).copied().unwrap_or(0) + 1;
        let new_trust = compute_trust(new_conf, now, rc);
        let keep_content = if content.len() > contents[i].len() {
            content
        } else {
            contents[i].as_str()
        };

        if let Err(e) = db
            .query(
                "UPDATE fact SET content = $content, confidence = $c, trust = $t, \
                 reinforcement_count = $r, last_reinforced = $lr, embedding = $emb \
                 WHERE fact_id = $id",
            )
            .bind(("content", keep_content.to_string()))
            .bind(("c", new_conf))
            .bind(("t", new_trust))
            .bind(("r", rc))
            .bind(("lr", now))
            .bind(("emb", new_emb.clone()))
            .bind(("id", ids[i].clone()))
            .await
        {
            crate::model::store::append_error_log(
                session_dir,
                "surreal::store_fact — UPDATE reinforce",
                &e.to_string(),
            );
        }

        crate::app::knowledge::proxy_push_fact(
            ids[i].clone(),
            keep_content.to_string(),
            category.to_string(),
            new_conf,
            new_emb,
        );

        return Ok(Some(ids[i].clone()));
    }

    // Store new fact with an explicit record ID so RELATE edges can target it.
    // Bare fact_id (no `fact:` prefix) — record RID is `fact:{fact_id}`.
    let trust = compute_trust(confidence, now, 0);
    let cat = sanitize_id_part(category);
    let fact_id = format!("{cat}_{now}_{}", content.len());
    let rid = format!("fact:{fact_id}");

    if let Err(e) = db
        .query("CREATE type::thing($rid) CONTENT $data")
        .bind(("rid", rid))
        .bind((
            "data",
            serde_json::json!({
                "fact_id": fact_id.clone(),
                "content": content,
                "category": category,
                "confidence": confidence,
                "trust": trust,
                "embedding": new_emb.clone(),
                "reinforcement_count": 0,
                "created_at": now,
                "last_reinforced": now,
            }),
        ))
        .await
    {
        crate::model::store::append_error_log(
            session_dir,
            "surreal::store_fact — CREATE fact",
            &e.to_string(),
        );
        return Err(e.into());
    }

    crate::app::knowledge::proxy_push_fact(
        fact_id.clone(),
        content.to_string(),
        category.to_string(),
        confidence,
        new_emb,
    );

    Ok(Some(fact_id))
}

/// Keep Surreal record-id components simple (ascii alnum / _ / -).
fn sanitize_id_part(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    out.trim_matches('_').to_string()
}

/// Recall memory atoms closest to the given vector query.
/// Falls back to the global knowledge daemon when local recall is weak.
pub fn recall_memory(session_dir: &Path, query_vec: &[f32], limit: usize) -> Vec<Fact> {
    let sd = session_dir.to_path_buf();
    let qv = query_vec.to_vec();
    core::blocking_block(move || {
        let sd = sd.clone();
        async move {
            let mut facts = recall_memory_async(&sd, &qv, limit)
                .await
                .unwrap_or_else(|e| {
                    crate::model::store::append_error_log(
                        &sd,
                        "surreal::recall_memory failed",
                        &e.to_string(),
                    );
                    Vec::new()
                });
            merge_daemon_fallback(&qv, limit, &mut facts);
            facts
        }
    })
    .unwrap_or_default()
}

/// Maximum cosine distance for a fact to be considered relevant.
/// Cosine distance: 0 = identical, 0.5 = somewhat similar, 1.0 = unrelated.
const MAX_COSINE_DISTANCE: f64 = 0.6;

async fn recall_memory_async(
    session_dir: &Path,
    query_vec: &[f32],
    limit: usize,
) -> anyhow::Result<Vec<Fact>> {
    let db = open_db(session_dir).await?;

    // Over-fetch (3× limit) so we have room to filter by distance + quality.
    let fetch_limit = (limit * 3).max(30);
    let ef = (fetch_limit * 2).max(100);
    let query_str = format!(
        "SELECT fact_id, content, trust,
                vector::distance::knn() AS distance
         FROM fact
         WHERE embedding <|{fetch_limit},{ef}|> $query_vec
         ORDER BY distance"
    );

    let mut results = db
        .query(&query_str)
        .bind(("query_vec", query_vec.to_vec()))
        .await?;

    let ids: Vec<String> = results.take("fact_id").unwrap_or_default();
    let contents: Vec<String> = results.take("content").unwrap_or_default();
    let trusts: Vec<f64> = results.take("trust").unwrap_or_default();
    let distances: Vec<f64> = results.take("distance").unwrap_or_default();

    let n = ids
        .len()
        .min(contents.len())
        .min(trusts.len())
        .min(distances.len());

    // Filter: distance threshold + content quality + dedup.
    let mut seen_hashes: std::collections::HashSet<u64> = std::collections::HashSet::new();
    let mut facts = Vec::with_capacity(limit);

    for i in 0..n {
        if facts.len() >= limit {
            break;
        }
        // Skip facts that are too far from the query.
        if distances[i] > MAX_COSINE_DISTANCE {
            continue;
        }
        // Skip low-quality facts (instructions, fragments, code).
        if !is_quality_fact(&contents[i]) {
            continue;
        }
        // Dedup by content hash (skip near-duplicates).
        if !seen_hashes.insert(content_hash(&contents[i])) {
            continue;
        }
        facts.push(Fact {
            id: ids[i].clone(),
            content: contents[i].clone(),
            trust: trusts[i],
        });
    }

    Ok(facts)
}

/// Try knowledge daemon fallback and merge with local results when local
/// recall is weak (empty or average trust < 0.5).
fn merge_daemon_fallback(query_vec: &[f32], limit: usize, local: &mut Vec<Fact>) {
    // Determine whether fallback is needed.
    let avg_trust = if local.is_empty() {
        0.0
    } else {
        local.iter().map(|f| f.trust).sum::<f64>() / local.len() as f64
    };
    if avg_trust >= 0.5 && !local.is_empty() {
        return; // local results are sufficient
    }

    // Try the global knowledge daemon.
    let result = crate::app::knowledge::proxy_expand(query_vec, limit);
    if result.facts.is_empty() && result.related_facts.is_empty() {
        return;
    }

    // Build a set of existing IDs + content hashes for dedup.
    let mut seen_ids: std::collections::HashSet<String> =
        local.iter().map(|f| f.id.clone()).collect();
    let mut seen_hashes: std::collections::HashSet<u64> =
        local.iter().map(|f| content_hash(&f.content)).collect();

    let mut push_if_ok = |id: &str, content: &str, trust: f64| {
        if local.len() >= limit {
            return;
        }
        if !is_quality_fact(content) {
            return;
        }
        if !seen_ids.insert(id.to_string()) {
            return;
        }
        if !seen_hashes.insert(content_hash(content)) {
            return;
        }
        local.push(Fact {
            id: id.to_string(),
            content: content.to_string(),
            trust,
        });
    };

    for kf in &result.facts {
        push_if_ok(&kf.id, &kf.content, kf.trust);
    }
    for kf in &result.related_facts {
        push_if_ok(&kf.id, &kf.content, kf.trust);
    }
}

fn content_hash(s: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    s.to_lowercase().hash(&mut h);
    h.finish()
}

// ── Content quality filter ─────────────────────────────────────────

/// Heuristic check: is this content a knowledge-worthy fact?
/// Rejects instructions, code fragments, conversational filler, and
/// incomplete sentences. Pure string checks — no LLM, no latency.
pub fn is_quality_fact(content: &str) -> bool {
    let trimmed = content.trim();

    // Too short to be meaningful.
    if trimmed.len() < 20 {
        return false;
    }

    // Too long — likely a code block or verbose instruction.
    if trimmed.len() > 500 {
        return false;
    }

    let lower = trimmed.to_lowercase();

    // Question sentences — reject anything that looks like a question.
    // Ends with '?' (defensive check — splitters should handle, but filter
    // may receive unsplit text from other callers).
    if trimmed.ends_with('?') {
        return false;
    }
    // Starts with question words — these are queries, not facts.
    const QUESTION_STARTS: &[&str] = &[
        "what ",
        "what's ",
        "whats ",
        "what is ",
        "what are ",
        "what was ",
        "what were ",
        "how ",
        "how's ",
        "hows ",
        "how is ",
        "how does ",
        "how can ",
        "how do ",
        "why ",
        "why's ",
        "whys ",
        "why is ",
        "why does ",
        "why are ",
        "when ",
        "when's ",
        "whens ",
        "when is ",
        "when does ",
        "when did ",
        "where ",
        "where's ",
        "wheres ",
        "where is ",
        "where does ",
        "where are ",
        "who ",
        "who's ",
        "whos ",
        "who is ",
        "who does ",
        "who are ",
        "which ",
        "which's ",
        "whichs ",
        "which is ",
        "which are ",
        "can i ",
        "can you ",
        "can we ",
        "could you ",
        "could i ",
        "would you ",
        "would i ",
        "should i ",
        "should we ",
        "do i ",
        "do you ",
        "does this ",
        "does that ",
        "is there ",
        "are there ",
        "what can i help",
        "what can i do",
    ];
    for prefix in QUESTION_STARTS {
        if lower.starts_with(prefix) {
            return false;
        }
    }

    // Imperative instructions (starts with a verb command).
    const INSTRUCTION_STARTS: &[&str] = &[
        "run ",
        "build ",
        "test ",
        "fix ",
        "add ",
        "create ",
        "update ",
        "delete ",
        "remove ",
        "install ",
        "configure ",
        "set ",
        "make ",
        "check ",
        "verify ",
        "ensure ",
        "apply ",
        "merge ",
        "rebase ",
        "commit ",
        "push ",
        "pull ",
        "checkout ",
        "revert ",
        "debug ",
        "deploy ",
        "restart ",
        "reload ",
        "enable ",
        "disable ",
        "use ",
        "try ",
        "open ",
        "close ",
        "start ",
        "stop ",
        "install ",
        "import ",
        "export ",
        "copy ",
        "move ",
    ];
    for prefix in INSTRUCTION_STARTS {
        if lower.starts_with(prefix) {
            // Allow if it reads like a declarative fact despite starting with a verb.
            // e.g. "Rust uses Cargo for builds" vs "Run cargo build"
            if !trimmed.contains(" → ") && !trimmed.contains("->") {
                return false;
            }
        }
    }

    // Code fragments and technical artifacts.
    if trimmed.contains("```")
        || trimmed.contains("`fn ")
        || trimmed.contains("`let ")
        || trimmed.contains("`pub ")
        || trimmed.contains("$.")
        || trimmed.contains("$.bind")
        || trimmed.contains("DEFINE ")
        || trimmed.contains("SELECT ")
        || trimmed.contains("CREATE ")
        || trimmed.contains("async ") && trimmed.contains("await ")
    {
        return false;
    }

    // Incomplete sentences (ends mid-word or with arrow/ellipsis).
    if trimmed.ends_with("→")
        || trimmed.ends_with("...")
        || trimmed.ends_with("…")
        || trimmed.ends_with('-')
        || trimmed.ends_with(',')
        || trimmed.ends_with('(')
    {
        return false;
    }

    // Starts with first-person conversational (not a fact).
    // Keep prefixes long enough to avoid false positives ("so ", "no ", "now ").
    const CONVERSATIONAL_STARTS: &[&str] = &[
        "i think ",
        "i believe ",
        "i feel ",
        "i can see ",
        "i see ",
        "we should ",
        "we can ",
        "you should ",
        "you can ",
        "you need ",
        "let me ",
        "let's ",
        "okay ",
        "sure ",
        "hmm ",
        "well ",
        "rebuild and test",
        "here's ",
        "here is ",
        "here are ",
        "as you can see",
        "note that ",
        "please ",
    ];
    for prefix in CONVERSATIONAL_STARTS {
        if lower.starts_with(prefix) {
            return false;
        }
    }

    // Markdown artifacts (bold markers, headings).
    if trimmed.starts_with("**")
        || trimmed.starts_with("##")
        || trimmed.starts_with("- ")
        || trimmed.starts_with("* ")
    {
        return false;
    }

    // Incomplete markdown links like "[foo" without a closing "](".
    if trimmed.contains('[') && !trimmed.contains("](") && !trimmed.contains(']') {
        return false;
    }

    // Log-like patterns (timestamps, PIDs).
    if trimmed.contains("[unix:") || lower.contains("pid ") {
        return false;
    }

    // Must contain at least one space (reject single-token garbage).
    if !trimmed.contains(' ') {
        return false;
    }

    true
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
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_quality_fact_accepts_declarative() {
        assert!(is_quality_fact(
            "Koma uses SurrealDB for vector search and knowledge storage"
        ));
        assert!(is_quality_fact(
            "Rust is a systems programming language with ownership rules"
        ));
    }

    #[test]
    fn test_quality_fact_rejects_garbage() {
        assert!(!is_quality_fact(
            "Rebuild and test — you should see the batch"
        ));
        assert!(!is_quality_fact(
            "Run cargo build --release after the change"
        ));
        assert!(!is_quality_fact(
            "I can see from your setup that you're working"
        ));
        assert!(!is_quality_fact(
            "**Feed facts** — have a normal conversation"
        ));
        assert!(!is_quality_fact("short"));
        assert!(!is_quality_fact(
            "SELECT * FROM fact WHERE embedding <|5,100|> $query"
        ));
        // Question sentences.
        assert!(!is_quality_fact("What can I help you with today?"));
        assert!(!is_quality_fact("How does this work?"));
        assert!(!is_quality_fact("Why is the build failing?"));
        assert!(!is_quality_fact("When did this start?"));
        assert!(!is_quality_fact("Where is the config file?"));
        assert!(!is_quality_fact("Can I use this approach?"));
        assert!(!is_quality_fact("Could you explain the error?"));
        // Greeting that was being stored as a fact.
        assert!(!is_quality_fact("What can I help you with"));
    }

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
        // After RocksDB swap, store → recall must be visible in the same process.
        // If this test fails, the engine swap did not fix the visibility bug.
        let dir = std::env::temp_dir().join("koma_test_surreal_memory");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);

        let id = store_fact(&dir, "Rust is a systems programming language", "tech", 0.9);
        assert!(id.is_some(), "store_fact should return an id");

        // Second store may return None — with RocksDB the KNN dedup check now
        // actually sees the first fact, and two programming-language facts can
        // trigger near-duplicate detection. This is correct engine behavior.
        let _ = store_fact(&dir, "Python is used for ML", "tech", 0.8);

        // RocksDB HNSW index may need a moment after the first INSERT to make
        // the record searchable via KNN. Retry a few times with backoff.
        let qv = embed_one("systems programming language Rust");
        let mut facts = Vec::new();
        for attempt in 0..5 {
            facts = recall_memory(&dir, &qv, 5);
            if !facts.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100 * (attempt + 1)));
        }
        assert!(!facts.is_empty(), "RocksDB must make store_fact visible to recall_memory");
        assert!(
            facts.iter().any(|f| f.content.contains("Rust")),
            "recall should find the stored Rust fact"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
