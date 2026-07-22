//! Entity extraction + resolution for the knowledge daemon.
//!
//! On every `PushFact`, extracts noun-phrase candidates from the fact content,
//! resolves each against the existing entity table via cosine similarity, and
//! creates or reuses entities. Then RELATEs fact→entity and entity↔entity
//! (co-occurrence edges) to build the knowledge graph.
//!
//! # Extraction strategy
//!
//! No LLM — rule-based extraction designed for technical English:
//! - Capitalised phrases (camel/Pascal) → "TokioRuntime", "BTreeMap"
//! - Quoted strings → "systems programming"
//! - Known technical terms from a built-in lexicon
//! - Adjacent capitalised words → "Rust", "Linux" style proper nouns
//!
//! # Resolution
//!
//! Each candidate entity name is embedded via fastembed, then KNN-queried
//! against the entity table's HNSW index. A cosine similarity > 0.85 reuses
//! the existing entity; otherwise a new entity is created.

use std::collections::HashSet;

use surrealdb::Surreal;
use surrealdb::engine::local::Db;

use crate::model::surreal::core;

/// Built-in technical terms that should always be extracted even when not
/// capitalised. Keep short (~50 entries) to avoid false positives.
const KNOWN_TERMS: &[&str] = &[
    "rust", "python", "javascript", "typescript", "golang", "java", "c++",
    "c#", "swift", "kotlin", "zig", "sql", "html", "css", "wasm",
    "linux", "macos", "windows", "bsd", "unix", "android", "ios",
    "docker", "kubernetes", "git", "postgresql", "mysql", "sqlite",
    "mongodb", "redis", "kafka", "nginx", "haproxy", "llvm", "gcc",
    "tokio", "actix", "axum", "rocket", "react", "vue", "svelte",
    "tailwind", "graphql", "grpc", "rest", "websocket", "http",
    "tls", "ssl", "oauth", "jwt", "surreal", "etcd", "raft",
    "paxos", "protobuf", "json", "yaml", "toml", "markdown",
];

/// Extract entity name candidates from a fact's content.
///
/// Returns a deduplicated, normalised list of candidate entity names.
/// Each candidate is trimmed, lowercased for comparison, but the ORIGINAL
/// case is preserved in the second element of each tuple.
pub fn extract_candidates(content: &str) -> Vec<(String, String)> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut candidates: Vec<(String, String)> = Vec::new();

    // 1. Quoted phrases.
    for cap in quoted_phrase().find_iter(content) {
        let s = cap.as_str();
        add(&s[1..s.len() - 1], &mut candidates, &mut seen);
    }

    // 2. CamelCase / PascalCase identifiers.
    for cap in camel_case().find_iter(content) {
        // Split camelCase into words: "TokioRuntime" → ["Tokio", "Runtime"]
        let s = cap.as_str();
        let words: Vec<&str> = s.split(|c: char| c.is_uppercase())
            .filter(|w: &&str| !w.is_empty())
            .collect();
        if words.len() >= 2 {
            for w in &words {
                add(w, &mut candidates, &mut seen);
            }
        }
        add(s, &mut candidates, &mut seen);
    }

    // 3. Capitalised proper nouns (two or more consecutive capital words).
    for cap in proper_noun().find_iter(content) {
        add(cap.as_str(), &mut candidates, &mut seen);
    }

    // 4. Known technical terms (case-insensitive scan).
    let content_lower = content.to_lowercase();
    for term in KNOWN_TERMS {
        if content_lower.contains(term) {
            add(term, &mut candidates, &mut seen);
        }
    }

    // 5. Single capitalised words not yet caught.
    for cap in single_capital().find_iter(content) {
        let s = cap.as_str();
        if !seen.contains(&s.to_lowercase())
            && s.len() > 1
            && !is_stop_word(s)
        {
            add(s, &mut candidates, &mut seen);
        }
    }

    candidates
}

fn add(word: &str, candidates: &mut Vec<(String, String)>, seen: &mut HashSet<String>) {
    let key = word.to_lowercase();
    if key.len() < 2 || seen.contains(&key) {
        return;
    }
    seen.insert(key);
    candidates.push((word.to_string(), word.to_string()));
}

fn is_stop_word(w: &str) -> bool {
    matches!(
        w.to_lowercase().as_str(),
        "the" | "a" | "an" | "is" | "are" | "was" | "were" | "be" | "been"
            | "have" | "has" | "had" | "do" | "does" | "did" | "will" | "would"
            | "can" | "could" | "may" | "might" | "shall" | "should" | "must"
            | "this" | "that" | "these" | "those" | "it" | "its" | "he" | "she"
            | "they" | "we" | "you" | "i" | "me" | "him" | "her" | "us" | "them"
            | "and" | "or" | "but" | "not" | "if" | "then" | "else" | "when"
            | "where" | "how" | "what" | "which" | "who" | "why" | "with"
            | "from" | "for" | "about" | "into" | "through" | "during"
            | "before" | "after" | "above" | "below" | "between"
            | "really" | "just" | "very" | "also" | "only" | "still"
    )
}

// ── Lazy-compiled regexes ─────────────────────────────────────────────

use regex::Regex;
use std::sync::OnceLock;

fn quoted_phrase() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#""[^"]{2,40}""#).unwrap())
}

fn camel_case() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b([A-Z][a-z]+){2,}\b").unwrap())
}

fn proper_noun() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b(?:[A-Z][a-z]+ ){1,3}[A-Z][a-z]+\b").unwrap())
}

fn single_capital() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b[A-Z][a-z]{2,}\b").unwrap())
}

/// Cosine threshold for entity reuse. Below this, a candidate is treated
/// as a new entity rather than a match to an existing one.
pub const ENTITY_MATCH_THRESHOLD: f64 = 0.85;

/// Result of resolving one candidate against the entity table.
#[derive(Debug)]
pub enum EntityResolution {
    /// Matched an existing entity (entity_id, name, similarity).
    Existing(String, String, f64),
    /// Should create a new entity (candidate name, entity type).
    New(String),
}

/// Resolve a SINGLE candidate entity name against the existing entity table.
/// Uses KNN over the entity_vec HNSW index. If the top result exceeds
/// [`ENTITY_MATCH_THRESHOLD`], reuses it; otherwise returns `New`.
pub async fn resolve_candidate(
    db: &Surreal<Db>,
    name: &str,
) -> anyhow::Result<EntityResolution> {
    let name_emb = core::embed_one(name);

    // KNN with K=1, EF=40 — find the closest entity by cosine similarity
    // (the index was defined with DIST COSINE, so distance = 1 - similarity).
    let mut results = db
        .query(
            "SELECT entity_id, name, entity_type,
                    vector::distance::knn() AS distance
             FROM entity
             WHERE embedding <|1,40|> $name_emb",
        )
        .bind(("name_emb", name_emb))
        .await?;

    let ids: Vec<String> = results.take("entity_id").unwrap_or_default();
    let names: Vec<String> = results.take("name").unwrap_or_default();
    let distances: Vec<f64> = results.take("distance").unwrap_or_default();

    if let (Some(id), Some(existing_name), Some(dist)) =
        (ids.into_iter().next(), names.into_iter().next(), distances.into_iter().next())
    {
        let sim = 1.0 - dist; // COSINE distance → similarity
        if sim >= ENTITY_MATCH_THRESHOLD {
            return Ok(EntityResolution::Existing(id, existing_name, sim));
        }
    }

    Ok(EntityResolution::New(name.to_string()))
}

/// Process a fact's content: extract entity candidates, resolve each,
/// and return the list of resolved entity IDs (existing or newly created).
///
/// Caller should then RELATE fact→entity via `produced` and create
/// entity↔entity `memory_edge` relationships for co-occurring pairs.
pub async fn extract_and_resolve(
    db: &Surreal<Db>,
    content: &str,
) -> anyhow::Result<Vec<(String, String)>> {
    // entity_id, name
    let candidates = extract_candidates(content);
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let mut resolved: Vec<(String, String)> = Vec::with_capacity(candidates.len());

    for (_original, name) in &candidates {
        match resolve_candidate(db, name).await? {
            EntityResolution::Existing(id, existing_name, _sim) => {
                // Update last_seen timestamp
                if let Err(e) = db
                    .query("UPDATE entity SET last_seen = $now WHERE entity_id = $id")
                    .bind(("now", now))
                    .bind(("id", id.clone()))
                    .await
                {
                    crate::model::store::append_global_error_log(
                        "knowledge extractor — UPDATE entity last_seen",
                        &e.to_string(),
                    );
                }
                resolved.push((id, existing_name));
            }
            EntityResolution::New(name) => {
                // Bare entity_id (no `entity:` prefix) — record RID is `entity:{bare_id}`.
                let bare_id = format!("{}_{now}", sanitize_id_part(&name));
                let rid = format!("entity:{bare_id}");
                let emb = core::embed_one(&name);
                if let Err(e) = db
                    .query("CREATE type::thing($rid) CONTENT $data")
                    .bind(("rid", rid))
                    .bind(("data", serde_json::json!({
                        "entity_id": &bare_id,
                        "entity_type": "concept",
                        "name": &name,
                        "aliases": [],
                        "embedding": emb,
                        "last_seen": now,
                    })))
                    .await
                {
                    crate::model::store::append_global_error_log(
                        "knowledge extractor — CREATE entity",
                        &e.to_string(),
                    );
                    continue;
                }
                resolved.push((bare_id, name));
            }
        }
    }

    Ok(resolved)
}

/// Create `produced` edges (fact → entity) and `memory_edge` edges
/// (entity ↔ entity for co-occurring pairs in the same fact).
///
/// `fact_id` and `resolved` entity IDs are **bare** (no table prefix).
/// Record RIDs are `fact:{fact_id}` / `entity:{entity_id}`.
pub async fn relate_entities(
    db: &Surreal<Db>,
    fact_id: &str,
    resolved: &[(String, String)], // (bare_entity_id, name)
) -> anyhow::Result<()> {
    // produced edges: fact → entity
    for (entity_id, _name) in resolved {
        let fact_rid = format!("fact:{fact_id}");
        let entity_rid = format!("entity:{entity_id}");
        if let Err(e) = db
            .query("RELATE $fact->produced->$entity")
            .bind(("fact", fact_rid))
            .bind(("entity", entity_rid))
            .await
        {
            crate::model::store::append_global_error_log(
                "knowledge extractor — RELATE produced",
                &e.to_string(),
            );
        }
    }

    // memory_edge: entity ↔ entity for co-occurring pairs
    for i in 0..resolved.len() {
        for j in (i + 1)..resolved.len() {
            let a = format!("entity:{}", resolved[i].0);
            let b = format!("entity:{}", resolved[j].0);
            if let Err(e) = db
                .query("RELATE $a->memory_edge->$b")
                .bind(("a", a))
                .bind(("b", b))
                .await
            {
                crate::model::store::append_global_error_log(
                    "knowledge extractor — RELATE memory_edge",
                    &e.to_string(),
                );
            }
        }
    }

    Ok(())
}

/// Keep Surreal record-id components simple (ascii alnum / _ / -).
pub(crate) fn sanitize_id_part(s: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_quoted_phrase() {
        let content = "The concept of \"systems programming\" is key.";
        let cands = extract_candidates(content);
        let names: Vec<&str> = cands.iter().map(|(_, n)| n.as_str()).collect();
        assert!(names.contains(&"systems programming"), "got: {names:?}");
    }

    #[test]
    fn test_extract_camel_case() {
        let content = "We used TokioRuntime for the async layer.";
        let cands = extract_candidates(content);
        let names: Vec<&str> = cands.iter().map(|(_, n)| n.as_str()).collect();
        assert!(names.contains(&"TokioRuntime"), "got: {names:?}");
    }

    #[test]
    fn test_extract_known_terms() {
        let content = "Rust is great with tokio and postgresql.";
        let cands = extract_candidates(content);
        let names: Vec<&str> = cands.iter().map(|(_, n)| n.as_str()).collect();
        assert!(names.iter().any(|n| n.to_lowercase() == "tokio"), "got: {names:?}");
        assert!(names.iter().any(|n| n.to_lowercase() == "postgresql"), "got: {names:?}");
    }

    #[test]
    fn test_extract_proper_noun() {
        let content = "The Systems Programming Language Rust is popular.";
        let cands = extract_candidates(content);
        let names: Vec<&str> = cands.iter().map(|(_, n)| n.as_str()).collect();
        // The full phrase should be extracted (alongside individual capitals).
        assert!(
            names.iter().any(|n| n.contains("Systems Programming Language")),
            "expected 'Systems Programming Language' in: {names:?}"
        );
    }

    #[test]
    fn test_extract_dedup() {
        let content = "Rust and Rust are the same.";
        let cands = extract_candidates(content);
        let names: Vec<&str> = cands.iter().map(|(_, n)| n.as_str()).collect();
        let rust_count = names.iter().filter(|n| n.to_lowercase() == "rust").count();
        assert_eq!(rust_count, 1, "duplicate 'Rust': {names:?}");
    }

    #[test]
    fn test_stop_words_filtered() {
        assert!(is_stop_word("the"));
        assert!(is_stop_word("and"));
        assert!(!is_stop_word("Rust"));
        assert!(!is_stop_word("tokio"));
    }
}
