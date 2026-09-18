//! Heavy-message blob classification, indexing, and recall queries.

use std::path::Path;

use anyhow::Result;

use crate::dto::chat::Role;

use super::schema::{open, HEAVY_TOKEN_EST, SNIPPET_CHARS, TOOL_HEAVY_TOKEN_EST};

/// A pointer into the `blobs` side table: enough metadata to *reference* a
/// heavy message (its kind, size estimate, and preview) without loading the
/// full content. Consumed by the summary builder (P2/P3).
#[allow(dead_code)] // consumed by later phases (short-send summary/router)
#[derive(Debug, Clone)]
pub struct BlobRef {
    pub id: i64,
    pub msg_id: i64,
    pub kind: String,
    pub token_est: i64,
    pub snippet: String,
}

/// True when a line carries NO semantically useful text — it's blank, or every
/// character is whitespace, a code-fence backtick, or a box-drawing / table-border
/// / rule glyph. These are the lines that LEAD code blocks and ASCII diagrams (the
/// ``` fence, then the `┌────┐` top border, etc.), so they're skipped when picking
/// where a snippet should start: the preview then begins at the first line with
/// real words instead of a border.
///
/// "Real text" = any alphanumeric character (`char::is_alphanumeric`, Unicode-aware
/// — not just ASCII). A line with even one alphanumeric char is NOT noise. The
/// extra NOISE set lets all-punctuation rules (e.g. `=====`, `-----`, `+--+--+`)
/// count as noise even though `is_alphanumeric` already rejects them; it documents
/// intent and keeps the rule readable.
fn is_noise_line(line: &str) -> bool {
    // Box-drawing / table-border / rule / fence punctuation: the light + heavy
    // box-drawing set, plus the ASCII rule chars used for hand-drawn tables and
    // separators, plus the backtick for ``` fences.
    const NOISE: &str = "─│┌┐└┘├┤┬┴┼━┃═║╔╗╚╝╠╣╦╩╬╮╭╯╰=-+|*#~_.` \t";
    // Noise = no alphanumeric AND every char is in the NOISE/whitespace set.
    // The alphanumeric check is the primary gate (Unicode-aware); the NOISE
    // membership check guards against punctuation that isn't alphanumeric but
    // also isn't a real word (a stray `!` line shouldn't anchor the snippet).
    !line.chars().any(|c| c.is_alphanumeric())
        && line.chars().all(|c| NOISE.contains(c) || c.is_whitespace())
}

/// Decide whether `content` is a "heavy blob" worth indexing, and if so derive
/// its `(kind, token_est, snippet)`. Returns `None` for ordinary messages.
///
/// - `token_est` is an approximate token count: `chars / 4`.
/// - Heavy when the estimate clears [`HEAVY_TOKEN_EST`], OR the content carries
///   a triple-backtick code fence, OR it's a tool output past
///   [`TOOL_HEAVY_TOKEN_EST`].
/// - `kind`: `"code"` if it has a ``` fence, else `"tool_output"` for tool
///   messages, else `"large_text"`.
/// - `snippet`: a SEMANTICALLY meaningful preview. Leading noise (fence lines,
///   box-drawing/table borders, blanks — see [`is_noise_line`]) is skipped so the
///   snippet starts at the first line with real words; from there newlines are
///   collapsed to spaces and the first [`SNIPPET_CHARS`] chars are kept, trimmed.
///   This is what makes e.g. a fenced ASCII diagram searchable by its labels
///   (`User Message … WIRE RAIL …`) instead of by its top border.
pub(super) fn classify_blob(role: Role, content: &str) -> Option<(&'static str, i64, String)> {
    let token_est = (content.chars().count() / 4) as i64;
    let has_fence = content.contains("```");
    let is_tool = matches!(role, Role::Tool);

    let heavy =
        token_est >= HEAVY_TOKEN_EST || has_fence || (is_tool && token_est >= TOOL_HEAVY_TOKEN_EST);
    if !heavy {
        return None;
    }

    let kind = if has_fence {
        "code"
    } else if is_tool {
        "tool_output"
    } else {
        "large_text"
    };

    // Skip leading noise lines (fence-only / border-only / blank) so the snippet
    // starts at the first line carrying real alphanumeric text. `split('\n')`
    // keeps the per-line view byte-correct; we re-join from the first non-noise
    // line onward. If EVERY line is noise (e.g. a pure diagram with no labels),
    // fall back to the whole content so the snippet is never empty.
    let mut lines = content.split('\n');
    let meaningful: String = {
        // Find the first non-noise line, then take it + everything after it.
        let mut started = false;
        let mut kept: Vec<&str> = Vec::new();
        for line in lines.by_ref() {
            if !started {
                if is_noise_line(line) {
                    continue; // still in the leading border/fence noise — drop it
                }
                started = true;
            }
            kept.push(line);
        }
        if started {
            kept.join("\n")
        } else {
            content.to_string() // all-noise content: keep it rather than emit ""
        }
    };

    // Collapse newlines (and stray carriage returns) to spaces, then take the
    // first SNIPPET_CHARS chars and trim. Char-based so multibyte content can't
    // split a code point.
    let collapsed: String = meaningful
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .take(SNIPPET_CHARS)
        .collect();
    let snippet = collapsed.trim().to_string();

    Some((kind, token_est, snippet))
}

/// Escape a term for a SQL `LIKE ... ESCAPE '\'` pattern: the LIKE metacharacters
/// `%` and `_`, plus the escape char `\` itself, are backslash-prefixed so a term
/// containing them matches literally instead of as a wildcard. The caller wraps
/// the result in `%...%` for a substring match.
fn escape_like(term: &str) -> String {
    let mut out = String::with_capacity(term.len());
    for c in term.chars() {
        if c == '%' || c == '_' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Return `messages.content` for a single id, or `None` if absent / unreadable.
/// Lets a summary expand a `blobs` reference back to its full text on demand.
/// Best-effort.
pub fn fetch_blob_content(session_dir: &Path, msg_id: i64) -> Option<String> {
    use rusqlite::OptionalExtension;
    let conn = open(session_dir).ok()?;
    conn.query_row(
        "SELECT content FROM messages WHERE id = ?1",
        rusqlite::params![msg_id],
        |r| r.get(0),
    )
    .optional()
    .ok()
    .flatten()
}

/// Role string for a message id (`user` / `assistant` / `tool` / …), if present.
pub fn fetch_message_role(session_dir: &Path, msg_id: i64) -> Option<String> {
    use rusqlite::OptionalExtension;
    let conn = open(session_dir).ok()?;
    conn.query_row(
        "SELECT role FROM messages WHERE id = ?1",
        rusqlite::params![msg_id],
        |r| r.get(0),
    )
    .optional()
    .ok()
    .flatten()
}

/// List every indexed blob reference, ordered by `msg_id` ascending. Returns an
/// empty vec if the DB is absent/unreadable. Best-effort.
#[allow(dead_code)] // consumed by later phases (short-send summary/router)
pub fn list_blobs(session_dir: &Path) -> Vec<BlobRef> {
    fn inner(session_dir: &Path) -> Result<Vec<BlobRef>> {
        let conn = open(session_dir)?;
        let mut stmt = conn.prepare(
            "SELECT id, msg_id, kind, token_est, snippet FROM blobs ORDER BY msg_id ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(BlobRef {
                id: r.get(0)?,
                msg_id: r.get(1)?,
                kind: r.get(2)?,
                token_est: r.get(3)?,
                snippet: r.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
    inner(session_dir).unwrap_or_default()
}

/// Blobs (msg_id <= max_msg_id) whose owning message CONTENT matches any of the
/// given lowercase terms, ranked by number of distinct terms matched (desc), then
/// by **newer** `msg_id` (desc) so equal scores prefer recency over ancient drafts.
///
/// This is the "db lookup" recall path: it finds blobs by what their owning
/// message actually SAYS, independent of the stored snippet — so even a blob with
/// a useless border-first snippet (an old row, or a diagram) is recalled when the
/// query overlaps its text. Matching is a simple case-insensitive substring
/// (`LOWER(messages.content) LIKE '%term%'`) per term, OR'd; the match COUNT
/// (number of distinct terms hit) is the rank key, so a blob matching more of the
/// query floats up. Results are capped at 10. Each `%term%` is bound (with LIKE
/// wildcards in the term escaped via `ESCAPE '\'`), never string-interpolated, so
/// a term can't inject SQL. Best-effort: returns an empty vec on any error or an
/// empty/whitespace-only term list.
pub fn search_blobs(session_dir: &Path, terms: &[String], max_msg_id: i64) -> Vec<BlobRef> {
    fn inner(session_dir: &Path, terms: &[String], max_msg_id: i64) -> Result<Vec<BlobRef>> {
        // Drop blank terms; nothing to match on if none remain.
        let patterns: Vec<String> = terms
            .iter()
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .map(|t| format!("%{}%", escape_like(t)))
            .collect();
        if patterns.is_empty() {
            return Ok(Vec::new());
        }

        let conn = open(session_dir)?;

        // Build the per-term LIKE expression once, reused in both the SELECT (as a
        // summed 0/1 match count) and the WHERE (OR'd, so a row needs >=1 hit).
        // Params: ?1 = max_msg_id, ?2.. = the `%term%` patterns. Numbered params
        // are reused across both spots so each pattern is bound exactly once.
        let likes: Vec<String> = (0..patterns.len())
            .map(|i| format!("(LOWER(m.content) LIKE ?{} ESCAPE '\\')", i + 2))
            .collect();
        // SUM of the boolean LIKEs = number of distinct terms matched (each term
        // appears once, and CASE makes the boolean an explicit 0/1 for SUM).
        let score_expr = likes
            .iter()
            .map(|l| format!("CASE WHEN {l} THEN 1 ELSE 0 END"))
            .collect::<Vec<_>>()
            .join(" + ");
        let where_or = likes.join(" OR ");

        let sql = format!(
            "SELECT b.id, b.msg_id, b.kind, b.token_est, b.snippet,
                    ({score_expr}) AS score
             FROM blobs b
             JOIN messages m ON m.id = b.msg_id
             WHERE b.msg_id <= ?1 AND ({where_or})
             ORDER BY score DESC, b.msg_id DESC
             LIMIT 10"
        );

        // Bind ?1 then the patterns as ?2.. (matches the numbering above).
        let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(patterns.len() + 1);
        params.push(&max_msg_id);
        for p in &patterns {
            params.push(p);
        }

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params.as_slice(), |r| {
            Ok(BlobRef {
                id: r.get(0)?,
                msg_id: r.get(1)?,
                kind: r.get(2)?,
                token_est: r.get(3)?,
                snippet: r.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
    inner(session_dir, terms, max_msg_id).unwrap_or_default()
}
