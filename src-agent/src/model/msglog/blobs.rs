//! Heavy-message blob classification, indexing, and recall queries.

use std::path::Path;

use anyhow::Result;

use crate::dto::chat::Role;

use super::schema::{open, HEAVY_TOKEN_EST, SNIPPET_CHARS, TOOL_HEAVY_TOKEN_EST};

/// A pointer into the `blobs` side table: enough metadata to *reference* a
/// heavy message (its kind, size estimate, and preview) without loading the
/// full content. Consumed by the summary builder (P2/P3).
#[allow(dead_code)] // Legacy archive inspection API.
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

/// Return `messages.content` for a single id, or `None` if absent / unreadable.
/// Lets a summary expand a `blobs` reference back to its full text on demand.
/// Best-effort.
#[allow(dead_code)] // Exact archive reads used by regression tests and legacy inspection.
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

/// Return legacy blob metadata, or an empty vector for an unreadable archive.
#[allow(dead_code)] // Legacy archive inspection API.
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
