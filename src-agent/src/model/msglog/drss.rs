//! Incremental deterministic archive index. Never indexes the reasoning column.
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

pub fn fingerprint(role: &str, content: &str) -> String {
    let mut h = Sha256::new();
    h.update(role);
    h.update([0]);
    h.update(content);
    format!("{:x}", h.finalize())
}

pub fn terms(text: &str) -> BTreeMap<String, u64> {
    const STOP: &[&str] = &[
        "this",
        "that",
        "with",
        "from",
        "have",
        "will",
        "your",
        "which",
        "there",
        "what",
        "when",
        "then",
        "into",
        "some",
        "been",
        "were",
        "they",
        "their",
        "would",
        "could",
        "should",
        "true",
        "false",
        "null",
        "return",
        "self",
        "continue",
        "assistant",
        "system",
        "user",
        "tool",
    ];
    let mut out = BTreeMap::new();
    for word in text.split(|c: char| !c.is_alphanumeric() && !matches!(c, '_' | '/' | '.' | '-')) {
        let word = word.trim_matches(['.', '/', '-', '_']).to_lowercase();
        let n = word.chars().count();
        if !(4..=80).contains(&n)
            || !word.chars().any(char::is_alphabetic)
            || STOP.contains(&word.as_str())
        {
            continue;
        }
        // Bound per-message vocabulary; repeated terms still get exact counts.
        if out.len() < 512 || out.contains_key(&word) {
            *out.entry(word).or_insert(0) += 1;
        }
    }
    out
}

/// Preserve an exact slice around the match, even in a single-line Unicode dump.
fn matching_excerpt(content: &str, relevant: &[String]) -> String {
    let mut lower = String::new();
    let mut positions = Vec::new();
    for (position, ch) in content.chars().enumerate() {
        for folded in ch.to_lowercase() {
            positions.extend(std::iter::repeat_n(position, folded.len_utf8()));
            lower.push(folded);
        }
    }
    let at = relevant.iter().filter_map(|term| lower.find(term)).min();
    let start = at
        .map(|byte| positions[byte].saturating_sub(100))
        .unwrap_or(0);
    content.chars().skip(start).take(400).collect()
}

fn tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS drss_index (
        msg_id INTEGER PRIMARY KEY, fingerprint TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS drss_terms (
        msg_id INTEGER NOT NULL, term TEXT NOT NULL, occurrences INTEGER NOT NULL,
        PRIMARY KEY(msg_id, term));
        CREATE INDEX IF NOT EXISTS drss_term_lookup ON drss_terms(term, msg_id);
        CREATE TABLE IF NOT EXISTS drss_state (
        id INTEGER PRIMARY KEY CHECK(id=1), boundary INTEGER NOT NULL, start_id INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS drss_memory (
        id INTEGER PRIMARY KEY CHECK(id=1), cache_key TEXT NOT NULL, content TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS drss_recovery (
        archive_key TEXT PRIMARY KEY, role TEXT NOT NULL, content TEXT NOT NULL,
        covered INTEGER NOT NULL DEFAULT 0);
        CREATE VIRTUAL TABLE IF NOT EXISTS drss_recovery_fts USING fts5(
        content, role UNINDEXED, content='drss_recovery', content_rowid='rowid');
        INSERT OR IGNORE INTO drss_state VALUES(1,0,1);",
    )?;
    Ok(())
}

pub struct Index {
    pub conn: Connection,
    pub boundary: i64,
    pub start_id: i64,
    pub ids: HashMap<String, Vec<i64>>,
}

#[derive(Clone)]
pub struct RecoveryRef {
    pub key: String,
    pub covered: bool,
}

impl Index {
    /// Save exact copies of legacy messages absent from the original archive.
    /// Prefix-derived keys distinguish repeated messages and remain stable as
    /// the conversation grows. Original messages/IDs and usage are untouched.
    pub fn recover_missing(
        &mut self,
        body: &[crate::dto::chat::ChatMessage],
        ids: &[Option<i64>],
    ) -> Result<Vec<Option<RecoveryRef>>> {
        let tx = self.conn.transaction()?;
        let mut prefix = String::from("drss-recovery-v1");
        let mut refs = Vec::with_capacity(body.len());
        for (msg, id) in body.iter().zip(ids) {
            let role = super::schema::role_str(msg.role);
            prefix = fingerprint(&prefix, &fingerprint(role, &msg.content));
            if id.is_some() {
                refs.push(None);
                continue;
            }
            let inserted = tx.execute(
                "INSERT OR IGNORE INTO drss_recovery(archive_key,role,content) VALUES(?1,?2,?3)",
                params![prefix, role, msg.content],
            )?;
            if inserted > 0 {
                tx.execute(
                    "INSERT INTO drss_recovery_fts(rowid,content,role)
                    SELECT rowid,content,role FROM drss_recovery WHERE archive_key=?1",
                    [&prefix],
                )?;
            }
            let covered = tx.query_row(
                "SELECT covered FROM drss_recovery WHERE archive_key=?1",
                [&prefix],
                |r| r.get::<_, bool>(0),
            )?;
            refs.push(Some(RecoveryRef {
                key: prefix.clone(),
                covered,
            }));
        }
        tx.commit()?;
        Ok(refs)
    }

    /// B is a bounded, derived snapshot. Live tool results must not reshuffle
    /// its prefix on every continuation; the caller keys all refresh inputs.
    pub fn cached_memory(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT content FROM drss_memory WHERE id=1 AND cache_key=?1",
                [key],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn save_coverage(
        &mut self,
        boundary: i64,
        keys: &[&str],
        memory: Option<(&str, &str)>,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("UPDATE drss_state SET boundary=?1 WHERE id=1", [boundary])?;
        for key in keys {
            tx.execute(
                "UPDATE drss_recovery SET covered=1 WHERE archive_key=?1",
                [key],
            )?;
        }
        if let Some((key, content)) = memory {
            tx.execute(
                "INSERT INTO drss_memory(id,cache_key,content) VALUES(1,?1,?2)
                 ON CONFLICT(id) DO UPDATE SET cache_key=excluded.cache_key,content=excluded.content",
                params![key, content],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn load(session_dir: &Path) -> Result<Self> {
        let mut conn = super::open(session_dir)?;
        tables(&conn)?;
        // Backfill only new/missing rows. A single transaction keeps terms and
        // fingerprints consistent; existing archives need no destructive migration.
        let tx = conn.transaction()?;
        {
            let mut select = tx.prepare(
                "SELECT m.id,m.role,m.content FROM messages m
                LEFT JOIN drss_index d ON d.msg_id=m.id WHERE d.msg_id IS NULL ORDER BY m.id",
            )?;
            let rows = select.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?;
            let mut insert = tx.prepare("INSERT INTO drss_index VALUES(?1,?2)")?;
            let mut term_insert = tx.prepare("INSERT INTO drss_terms VALUES(?1,?2,?3)")?;
            for row in rows {
                let (id, role, content) = row?;
                insert.execute(params![id, fingerprint(&role, &content)])?;
                for (term, count) in terms(&content) {
                    term_insert.execute(params![id, term, count])?;
                }
            }
        }
        tx.commit()?;
        let (boundary, start_id) = conn.query_row(
            "SELECT boundary,start_id FROM drss_state WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let mut ids: HashMap<String, Vec<i64>> = HashMap::new();
        {
            let mut stmt = conn.prepare(
                "SELECT msg_id,fingerprint FROM drss_index WHERE msg_id>=?1 ORDER BY msg_id",
            )?;
            for row in stmt.query_map([start_id], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })? {
                let (id, hash) = row?;
                ids.entry(hash).or_default().push(id);
            }
        }
        Ok(Self {
            conn,
            boundary,
            start_id,
            ids,
        })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn save_boundary(&self, boundary: i64) -> Result<()> {
        self.conn
            .execute("UPDATE drss_state SET boundary=?1 WHERE id=1", [boundary])?;
        Ok(())
    }

    pub fn statistics(
        &self,
        through: i64,
        relevant: &[String],
    ) -> Result<Vec<(String, u64, u64, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT term,SUM(occurrences),COUNT(*),MAX(msg_id)
            FROM drss_terms WHERE msg_id>=?1 AND msg_id<=?2
            GROUP BY term ORDER BY COUNT(*) DESC,MAX(msg_id) DESC,term LIMIT 128",
        )?;
        let mut stats = stmt
            .query_map(params![self.start_id, through], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, u64>(1)?,
                    r.get::<_, u64>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        // Explicit relevance queries also find rare terms outside the top 128.
        for term in relevant.iter().take(32) {
            if stats.iter().any(|s| &s.0 == term) {
                continue;
            }
            let row: (u64, u64, i64) = self.conn.query_row(
                "SELECT COALESCE(SUM(occurrences),0),COUNT(*),COALESCE(MAX(msg_id),0)
                FROM drss_terms WHERE msg_id>=?1 AND msg_id<=?2 AND term=?3",
                params![self.start_id, through, term],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            if row.1 > 0 {
                stats.push((term.clone(), row.0, row.1, row.2));
            }
        }
        stats.sort_by(|a, b| {
            relevant
                .contains(&b.0)
                .cmp(&relevant.contains(&a.0))
                .then(b.2.cmp(&a.2))
                .then(b.3.cmp(&a.3))
                .then(a.0.cmp(&b.0))
        });
        stats.truncate(16);
        Ok(stats)
    }

    pub fn excerpts(
        &self,
        through: i64,
        relevant: &[String],
        history_ask: bool,
    ) -> Result<Vec<(i64, String, String)>> {
        let mut candidates: BTreeMap<i64, usize> = BTreeMap::new();
        for term in relevant.iter().take(32) {
            let mut stmt = self.conn.prepare("SELECT msg_id FROM drss_terms WHERE term=?1 AND msg_id>=?2 AND msg_id<=?3 ORDER BY msg_id DESC LIMIT 12")?;
            for id in stmt.query_map(params![term, self.start_id, through], |r| {
                r.get::<_, i64>(0)
            })? {
                *candidates.entry(id?).or_default() += 1;
            }
        }
        let mut ranked: Vec<_> = candidates.into_iter().collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.cmp(&a.0)));
        let mut result = Vec::new();
        for (id, _) in ranked {
            let (role, content): (String, String) = self.conn.query_row(
                "SELECT role,content FROM messages WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if role == "system" || (role == "assistant" && !history_ask) {
                continue;
            }
            let excerpt = matching_excerpt(&content, relevant);
            result.push((id, role, excerpt));
            if result.len() >= 4 {
                break;
            }
        }
        Ok(result)
    }

    pub fn user_constraints(&self, through: i64) -> Result<Vec<(i64, String)>> {
        let mut stmt = self.conn.prepare("SELECT id,content FROM messages WHERE role='user' AND id>=?1 AND id<=?2 ORDER BY id DESC LIMIT 50")?;
        let mut out = Vec::new();
        for row in stmt.query_map(params![self.start_id, through], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })? {
            let (id, content) = row?;
            let line = content.lines().find(|line| {
                let lower = line.to_lowercase();
                [
                    "must ", "do not ", "don't ", "never ", "keep ", "only ", "goal:", "instead:",
                ]
                .iter()
                .any(|s| lower.contains(s))
            });
            if let Some(line) = line {
                out.push((id, line.chars().take(300).collect()));
            }
            if out.len() >= 3 {
                break;
            }
        }
        Ok(out)
    }
}

/// Called by /clear and resend/truncate so stale boundaries never hide new work.
pub(super) fn reset(conn: &Connection, clear: bool) -> Result<()> {
    tables(conn)?;
    let max: i64 = conn.query_row("SELECT COALESCE(MAX(id),0) FROM messages", [], |r| r.get(0))?;
    conn.execute_batch(
        "DELETE FROM drss_terms WHERE msg_id NOT IN (SELECT id FROM messages);
        DELETE FROM drss_index WHERE msg_id NOT IN (SELECT id FROM messages);
        DELETE FROM drss_memory;
        UPDATE drss_recovery SET covered=0;",
    )?;
    if clear {
        conn.execute(
            "UPDATE drss_state SET boundary=?1,start_id=?2 WHERE id=1",
            params![max, max + 1],
        )?;
    } else {
        conn.execute(
            "UPDATE drss_state SET boundary=MIN(boundary,?1) WHERE id=1",
            [max],
        )?;
    }
    Ok(())
}
