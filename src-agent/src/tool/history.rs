//! Chat history search tool: `message_find` queries chat history via SQLite
//! FTS5 full-text search on `messages.sqlite`.
//!
//! Default scope is the **current session only**. Optional `scope: "project"`
//! searches every session under the same pwd-bucket
//! (`~/.koma/sessions/<pwd_hash>/*/messages.sqlite`). No FTS daemon — each call
//! opens DBs on demand inside a 20s worker thread.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::{Tool, ToolCtx};
use anyhow::{bail, Result};
use serde_json::{json, Value};

/// Hard wall-clock budget for one search. On timeout the turn unparks with an
/// error and a deterministic FTS/panic diagnosis + repair runs (no AI).
const MESSAGE_FIND_TIMEOUT: Duration = Duration::from_secs(20);

/// Global hit cap returned to the model.
const MESSAGE_FIND_LIMIT: i64 = 10;

/// Leave a little slack before the outer recv timeout so we stop opening new
/// sibling DBs instead of racing the channel deadline.
const PROJECT_SEARCH_SLACK: Duration = Duration::from_millis(500);

/// Search breadth for `message_find`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchScope {
    /// Only `ctx.session_dir` (default when the arg is omitted).
    Session,
    /// All sessions sharing the current session's pwd-bucket.
    Project,
}

/// One searchable session directory (current and/or siblings).
#[derive(Debug, Clone)]
struct SearchTarget {
    path: PathBuf,
    /// Session UUID (directory basename). Empty for anonymous/test targets.
    uuid: String,
    /// Display name from the registry (falls back to uuid).
    name: String,
    is_current: bool,
}

/// One FTS hit, optionally tagged with the session it came from.
#[derive(Debug, Clone)]
struct LabeledMatch {
    id: i64,
    role: String,
    snippet: String,
    created_at: i64,
    reasoning: Option<String>,
    /// `Some` when the hit should show `@ name (uuid-short)` (project scope).
    session_label: Option<(String, String)>,
    is_current: bool,
    /// Session directory this hit came from (for image path resolution).
    session_path: PathBuf,
}

/// Search the session's `messages.sqlite` full-text index for past
/// conversation turns matching the query. Returns ranked snippets.
pub struct MessageFind;

impl Tool for MessageFind {
    fn name(&self) -> &'static str {
        "message_find"
    }

    fn description(&self) -> &'static str {
        "Search chat history (messages.sqlite) via SQLite FTS5 for past \
         conversation turns matching the query. Default scope is the current \
         session only; pass scope \"project\" to search all sessions sharing \
         this working-directory bucket. Returns up to 10 results with message \
         id, role, and the first 300 characters of the matching message. \
         When a hit snippet contains [Image #N], appends a reload path so you \
         can call load_image to re-inspect. Project-scope hits are tagged with \
         session name/id (message ids are per-session). Query is limited to 5 \
         words (extra terms dropped). Times out at 20s. Optionally filter by \
         role (user, assistant, tool). Call this when you are confused, missing \
         context about a past decision, error, tradeoff, or fact that may have \
         scrolled out of the context window — before guessing. Use scope \
         project when the user asks about prior sessions in this project or \
         session-only search misses something that may live in a sibling session."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "At most 5 search words (extra words ignored). Multi-word queries are OR'd as prefix matches (e.g. \"foo bar\" → foo* OR bar*). Prefer short precise terms."
                },
                "role": {
                    "type": "string",
                    "description": "Optional role filter: \"user\" for user messages, \"assistant\" for assistant messages, \"tool\" for tool results. Omit to search all roles.",
                    "enum": ["user", "assistant", "tool"]
                },
                "scope": {
                    "type": "string",
                    "description": "Search breadth. Omit or \"session\" = this session only (default). \"project\" = all sessions sharing this working-directory bucket.",
                    "enum": ["session", "project"]
                }
            },
            "required": ["query"]
        })
    }

    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("missing required string argument 'query'"))?;

        let role_filter = args
            .get("role")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());

        let scope = parse_scope(args.get("scope").and_then(Value::as_str))?;

        let session_dir = match ctx.session_dir.as_ref() {
            Some(d) => d.clone(),
            None => bail!("no active session to search"),
        };
        let session_dir_for_repair = session_dir.clone();

        let query_owned = query.to_string();
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("message-find".into())
            .spawn(move || {
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    run_search(&session_dir, &query_owned, role_filter.as_deref(), scope)
                }));
                let _ = tx.send(outcome);
            })
            .map_err(|e| anyhow::anyhow!("message_find spawn failed: {e}"))?;

        match rx.recv_timeout(MESSAGE_FIND_TIMEOUT) {
            Ok(Ok(Ok(matches))) => {
                let out = format_labeled_matches(&matches);
                if out.is_empty() {
                    return Ok("(no matching messages found)".to_string());
                }
                Ok(out)
            }
            Ok(Ok(Err(e))) => {
                // Surface DB/FTS errors instead of mapping them to "no matches".
                Err(anyhow::anyhow!("message_find failed: {e}"))
            }
            Ok(Err(payload)) => {
                let msg = panic_payload_message(&payload);
                crate::model::store::append_global_error_log(
                    "message_find",
                    &format!("worker panic: {msg}"),
                );
                let repair = crate::model::msglog::diagnose_and_repair_message_find(
                    &session_dir_for_repair,
                );
                Err(anyhow::anyhow!(
                    "message_find panicked: {msg}\n{repair}"
                ))
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                crate::model::store::append_global_error_log(
                    "message_find",
                    "timed out after 20s — running deterministic diagnosis/repair",
                );
                // Worker may still be running; abandon it and repair the *current*
                // session archive only (never mass-repair the pwd bucket).
                let repair = crate::model::msglog::diagnose_and_repair_message_find(
                    &session_dir_for_repair,
                );
                Err(anyhow::anyhow!(
                    "message_find timed out after 20s\n{repair}\n\
                     (retry with ≤5 precise words if needed)"
                ))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err(anyhow::anyhow!("message_find worker dropped without a result"))
            }
        }
    }
}

fn parse_scope(raw: Option<&str>) -> Result<SearchScope> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(SearchScope::Session),
        Some("session") => Ok(SearchScope::Session),
        Some("project") => Ok(SearchScope::Project),
        Some(other) => bail!(
            "invalid scope '{other}': expected \"session\" or \"project\""
        ),
    }
}

/// Derive the pwd-bucket hash from `session_dir`'s parent name — same rule as
/// `Session::load`. Never use process cwd or effective workspace after `cd`.
fn pwd_hash_from_session_dir(session_dir: &Path) -> Option<String> {
    session_dir
        .parent()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
}

fn session_uuid_from_dir(session_dir: &Path) -> String {
    session_dir
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn resolve_targets(session_dir: &Path, scope: SearchScope) -> Result<Vec<SearchTarget>> {
    let current_uuid = session_uuid_from_dir(session_dir);
    match scope {
        SearchScope::Session => Ok(vec![SearchTarget {
            path: session_dir.to_path_buf(),
            uuid: current_uuid,
            name: String::new(),
            is_current: true,
        }]),
        SearchScope::Project => {
            let pwd_hash = pwd_hash_from_session_dir(session_dir).ok_or_else(|| {
                anyhow::anyhow!("cannot derive pwd_hash from session_dir for project scope")
            })?;
            let rows = crate::model::session_registry::list_by_pwd(&pwd_hash).unwrap_or_default();

            let mut targets: Vec<SearchTarget> = Vec::new();

            // Current session first so project search prefers it under the
            // shared 20s budget and merge rank.
            targets.push(SearchTarget {
                path: session_dir.to_path_buf(),
                uuid: current_uuid.clone(),
                name: rows
                    .iter()
                    .find(|r| r.uuid == current_uuid)
                    .map(|r| r.name.clone())
                    .unwrap_or_else(|| current_uuid.clone()),
                is_current: true,
            });

            for row in rows {
                if !current_uuid.is_empty() && row.uuid == current_uuid {
                    continue;
                }
                let path = match crate::model::store::session_dir(&pwd_hash, &row.uuid) {
                    Ok(p) => p,
                    Err(e) => {
                        crate::model::store::append_global_error_log(
                            "message_find",
                            &format!("project scope: session_dir({}) failed: {e:#}", row.uuid),
                        );
                        continue;
                    }
                };
                // If registry somehow missed current, avoid duplicating by path.
                if path == session_dir {
                    continue;
                }
                targets.push(SearchTarget {
                    path,
                    uuid: row.uuid,
                    name: row.name,
                    is_current: false,
                });
            }
            Ok(targets)
        }
    }
}

fn run_search(
    session_dir: &Path,
    query: &str,
    role_filter: Option<&str>,
    scope: SearchScope,
) -> Result<Vec<LabeledMatch>> {
    let targets = resolve_targets(session_dir, scope)?;
    let label_sessions = scope == SearchScope::Project;
    let deadline = Instant::now() + MESSAGE_FIND_TIMEOUT.saturating_sub(PROJECT_SEARCH_SLACK);
    search_targets(&targets, query, role_filter, label_sessions, deadline)
}

/// Search each target until the deadline or enough ranked hits.
///
/// Current-session FTS/open errors are hard failures. Sibling errors are
/// logged and skipped. Missing `messages.sqlite` is skipped quietly.
fn search_targets(
    targets: &[SearchTarget],
    query: &str,
    role_filter: Option<&str>,
    label_sessions: bool,
    deadline: Instant,
) -> Result<Vec<LabeledMatch>> {
    let mut collected: Vec<LabeledMatch> = Vec::new();
    let mut searched_current = false;

    for target in targets {
        if Instant::now() >= deadline {
            break;
        }
        // Early-stop once we have a full page *and* the current session was
        // already attempted (so project scope never skips current entirely).
        if collected.len() as i64 >= MESSAGE_FIND_LIMIT && searched_current {
            break;
        }

        if !target.path.join("messages.sqlite").exists() {
            if target.is_current {
                searched_current = true;
            }
            continue;
        }

        match crate::model::msglog::search_messages(
            &target.path,
            query,
            MESSAGE_FIND_LIMIT,
            role_filter,
        ) {
            Ok(hits) => {
                if target.is_current {
                    searched_current = true;
                }
                let label = if label_sessions {
                    let name = if target.name.is_empty() {
                        target.uuid.clone()
                    } else {
                        target.name.clone()
                    };
                    Some((name, target.uuid.clone()))
                } else {
                    None
                };
                for h in hits {
                    collected.push(LabeledMatch {
                        id: h.id,
                        role: h.role,
                        snippet: h.snippet,
                        created_at: h.created_at,
                        reasoning: h.reasoning,
                        session_label: label.clone(),
                        is_current: target.is_current,
                        session_path: target.path.clone(),
                    });
                }
            }
            Err(e) if target.is_current => {
                return Err(e);
            }
            Err(e) => {
                crate::model::store::append_global_error_log(
                    "message_find",
                    &format!(
                        "project scope: skip sibling {} ({}): {e:#}",
                        target.uuid,
                        target.path.display()
                    ),
                );
            }
        }
    }

    Ok(merge_and_cap(collected, MESSAGE_FIND_LIMIT as usize))
}

/// v1 rank: current-session hits first, then newer `created_at`, then lower id.
fn merge_and_cap(mut hits: Vec<LabeledMatch>, limit: usize) -> Vec<LabeledMatch> {
    hits.sort_by(|a, b| {
        b.is_current
            .cmp(&a.is_current)
            .then_with(|| b.created_at.cmp(&a.created_at))
            .then_with(|| a.id.cmp(&b.id))
    });
    hits.truncate(limit);
    hits
}

fn panic_payload_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        return (*s).to_string();
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    "unknown panic payload".into()
}

/// Truncate on a char boundary so multi-byte UTF-8 never panics the deferred
/// tool thread (which would leave the round stuck on a running message_find).
fn floor_chars(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

fn short_uuid(uuid: &str) -> &str {
    let len = uuid.chars().count().min(8);
    match uuid.char_indices().nth(len) {
        Some((idx, _)) => &uuid[..idx],
        None => uuid,
    }
}

fn format_labeled_matches(matches: &[LabeledMatch]) -> String {
    let mut out = String::new();
    for m in matches {
        let role_prefix = match m.role.as_str() {
            "user" => "[user]",
            "assistant" => "[assistant]",
            "tool" => "[tool]",
            "system" => "[system]",
            _ => "[?]",
        };
        let snippet = floor_chars(m.snippet.trim(), 300);
        match &m.session_label {
            Some((name, uuid)) if !uuid.is_empty() || !name.is_empty() => {
                let name = if name.is_empty() {
                    uuid.as_str()
                } else {
                    name.as_str()
                };
                let id_part = short_uuid(uuid);
                out.push_str(&format!(
                    "{} #{} @ {} ({}): {}\n",
                    role_prefix, m.id, name, id_part, snippet
                ));
            }
            _ => {
                out.push_str(&format!("{} #{}: {}\n", role_prefix, m.id, snippet));
            }
        }
        append_image_reload_lines(&mut out, &m.session_path, m.snippet.as_str());
        append_paste_reload_lines(&mut out, &m.session_path, m.snippet.as_str());
        out.push('\n');
        if let Some(thinking) = m.reasoning.as_deref() {
            let thinking = thinking.trim();
            if !thinking.is_empty() {
                let t = floor_chars(thinking, 300);
                out.push_str(&format!("  thinking: {}\n\n", t));
            }
        }
    }
    out
}

/// When a hit snippet mentions `[Image #N]`, resolve N under that hit's session
/// dir and append a reload hint. Only markers present in the snippet; never
/// dumps the whole session images dir.
fn append_image_reload_lines(out: &mut String, session_path: &Path, snippet: &str) {
    let markers =
        crate::tool::internet::load_image::marker_numbers_in_text(snippet);
    for n in markers {
        let Some(path) =
            crate::tool::internet::load_image::resolve_image_marker_in_session(session_path, n)
        else {
            out.push_str(&format!(
                "  image: [Image #{n}] (file missing under this session's images/) — try load_image({{\"image_n\":{n}}}) if it still exists\n"
            ));
            continue;
        };
        out.push_str(&format!(
            "  image: [Image #{n}] {} — call load_image({{\"path\":\"{}\"}}) to re-inspect\n",
            path.display(),
            path.display()
        ));
    }
}

/// When a hit snippet mentions `[Pasted Text #N]` or a paste fence `n=N`,
/// resolve the session `pastes/NN-paste.txt` path and hint `read` (not vision).
fn append_paste_reload_lines(out: &mut String, session_path: &Path, snippet: &str) {
    let mut ns: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    // Composer-style markers.
    const PREFIX: &str = "[Pasted Text #";
    for (i, _) in snippet.match_indices(PREFIX) {
        let after = &snippet[i + PREFIX.len()..];
        let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() || !after[digits.len()..].starts_with(']') {
            continue;
        }
        if let Ok(n) = digits.parse::<usize>() {
            ns.insert(n);
        }
    }
    // Fence form: <<<pasted_text n=N
    const FENCE: &str = "<<<pasted_text n=";
    for (i, _) in snippet.match_indices(FENCE) {
        let after = &snippet[i + FENCE.len()..];
        let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = digits.parse::<usize>() {
            ns.insert(n);
        }
    }
    for n in ns {
        let name = format!("{n:02}-paste.txt");
        let path = session_path.join("pastes").join(&name);
        if path.is_file() {
            out.push_str(&format!(
                "  paste: [Pasted Text #{n}] {} — call read({{\"path\":\"{}\"}}) to reload the body\n",
                path.display(),
                path.display()
            ));
        } else {
            out.push_str(&format!(
                "  paste: [Pasted Text #{n}] (file missing under this session's pastes/)\n"
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::chat::Role;

    /// Local temp dir (no tempfile dep) — mirrors msglog query_test helper.
    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "koma-history-test-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn floor_chars_does_not_split_multibyte_at_boundary() {
        let mut s = String::new();
        while s.len() < 298 {
            s.push('a');
        }
        s.push('─'); // 3-byte box drawing (U+2500)
        s.push_str("tail");
        let cut = floor_chars(&s, 300);
        assert!(cut.is_char_boundary(cut.len()));
        assert!(!cut.ends_with('\u{FFFD}'));
        assert!(cut.chars().count() <= 300);
    }

    #[test]
    fn parse_scope_defaults_and_rejects_unknown() {
        assert_eq!(parse_scope(None).unwrap(), SearchScope::Session);
        assert_eq!(parse_scope(Some("")).unwrap(), SearchScope::Session);
        assert_eq!(parse_scope(Some("  ")).unwrap(), SearchScope::Session);
        assert_eq!(parse_scope(Some("session")).unwrap(), SearchScope::Session);
        assert_eq!(parse_scope(Some("project")).unwrap(), SearchScope::Project);
        assert!(parse_scope(Some("all")).is_err());
        assert!(parse_scope(Some("PROJECT")).is_err());
    }

    #[test]
    fn format_session_scope_omits_label() {
        let hits = vec![LabeledMatch {
            id: 7,
            role: "user".into(),
            snippet: "hello world".into(),
            created_at: 1,
            reasoning: None,
            session_label: None,
            is_current: true,
            session_path: PathBuf::from("/tmp/fake-sess"),
        }];
        let out = format_labeled_matches(&hits);
        assert_eq!(out, "[user] #7: hello world\n\n");
        assert!(!out.contains('@'));
    }

    #[test]
    fn format_project_scope_includes_session_label() {
        let hits = vec![LabeledMatch {
            id: 3,
            role: "assistant".into(),
            snippet: "attach freeze fix".into(),
            created_at: 2,
            reasoning: Some("thinking about webkit".into()),
            session_label: Some(("feature-chat".into(), "abcdef12-9999-0000".into())),
            is_current: false,
            session_path: PathBuf::from("/tmp/fake-sess"),
        }];
        let out = format_labeled_matches(&hits);
        assert!(out.contains("[assistant] #3 @ feature-chat (abcdef12): attach freeze fix"));
        assert!(out.contains("thinking: thinking about webkit"));
    }

    #[test]
    fn format_match_with_image_marker_appends_reload_path() {
        let dir = TempDir::new("img-hit");
        let images = dir.path().join("images");
        std::fs::create_dir_all(&images).unwrap();
        let img = images.join("03-shot.png");
        std::fs::write(
            &img,
            b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89",
        )
        .unwrap();
        let hits = vec![LabeledMatch {
            id: 9,
            role: "user".into(),
            snippet: "look at [Image #3] please".into(),
            created_at: 1,
            reasoning: None,
            session_label: None,
            is_current: true,
            session_path: dir.path().to_path_buf(),
        }];
        let out = format_labeled_matches(&hits);
        assert!(out.contains("[user] #9: look at [Image #3] please"));
        assert!(out.contains("image: [Image #3]"));
        assert!(out.contains("load_image"));
        assert!(out.contains(&img.display().to_string()));
    }

    #[test]
    fn merge_and_cap_prefers_current_then_newer() {
        let hits = vec![
            LabeledMatch {
                id: 1,
                role: "user".into(),
                snippet: "sib old".into(),
                created_at: 100,
                reasoning: None,
                session_label: Some(("S".into(), "sib".into())),
                is_current: false,
                session_path: PathBuf::from("/tmp/sib"),
            },
            LabeledMatch {
                id: 2,
                role: "user".into(),
                snippet: "cur older".into(),
                created_at: 50,
                reasoning: None,
                session_label: Some(("C".into(), "cur".into())),
                is_current: true,
                session_path: PathBuf::from("/tmp/cur"),
            },
            LabeledMatch {
                id: 3,
                role: "user".into(),
                snippet: "sib new".into(),
                created_at: 200,
                reasoning: None,
                session_label: Some(("S".into(), "sib".into())),
                is_current: false,
                session_path: PathBuf::from("/tmp/sib"),
            },
            LabeledMatch {
                id: 4,
                role: "user".into(),
                snippet: "cur new".into(),
                created_at: 150,
                reasoning: None,
                session_label: Some(("C".into(), "cur".into())),
                is_current: true,
                session_path: PathBuf::from("/tmp/cur"),
            },
        ];
        let ranked = merge_and_cap(hits, 10);
        assert_eq!(ranked[0].id, 4); // current, newer
        assert_eq!(ranked[1].id, 2); // current, older
        assert_eq!(ranked[2].id, 3); // sibling, newer
        assert_eq!(ranked[3].id, 1); // sibling, older
        assert_eq!(merge_and_cap(ranked.clone(), 2).len(), 2);
    }

    #[test]
    fn search_targets_session_only_and_merge_prefers_current() {
        let bucket = TempDir::new("bucket");
        let cur = bucket.path().join("sess-current");
        let sib = bucket.path().join("sess-sibling");
        std::fs::create_dir_all(&cur).unwrap();
        std::fs::create_dir_all(&sib).unwrap();

        crate::model::msglog::append(
            &cur,
            Role::User,
            "unique_token_alpha current session note",
            None,
            None,
        )
        .unwrap();
        crate::model::msglog::append(
            &sib,
            Role::User,
            "unique_token_alpha sibling session note",
            None,
            None,
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(15));
        crate::model::msglog::append(
            &cur,
            Role::Assistant,
            "unique_token_alpha later current reply",
            None,
            None,
        )
        .unwrap();

        let targets = vec![
            SearchTarget {
                path: cur,
                uuid: "sess-current".into(),
                name: "Current".into(),
                is_current: true,
            },
            SearchTarget {
                path: sib,
                uuid: "sess-sibling".into(),
                name: "Sibling".into(),
                is_current: false,
            },
        ];
        let deadline = Instant::now() + Duration::from_secs(10);
        let hits = search_targets(
            &targets,
            "unique_token_alpha",
            None,
            true,
            deadline,
        )
        .unwrap();
        assert!(!hits.is_empty());
        assert!(hits.len() <= 10);
        assert!(
            hits[0].is_current,
            "first hit should be from current session, got {:?}",
            hits[0].session_label
        );
        assert!(hits.iter().all(|h| h.session_label.is_some()));
    }

    #[test]
    fn search_targets_skips_missing_sqlite_siblings() {
        let bucket = TempDir::new("missing-sib");
        let cur = bucket.path().join("a");
        let empty = bucket.path().join("b");
        std::fs::create_dir_all(&cur).unwrap();
        std::fs::create_dir_all(&empty).unwrap();
        crate::model::msglog::append(&cur, Role::User, "only_here_token_xyz", None, None)
            .unwrap();

        let targets = vec![
            SearchTarget {
                path: cur,
                uuid: "a".into(),
                name: "A".into(),
                is_current: true,
            },
            SearchTarget {
                path: empty,
                uuid: "b".into(),
                name: "B".into(),
                is_current: false,
            },
        ];
        let hits = search_targets(
            &targets,
            "only_here_token_xyz",
            None,
            false,
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].session_label.is_none());
    }

    #[test]
    fn resolve_targets_session_scope_is_single() {
        let dir = TempDir::new("one");
        let targets = resolve_targets(dir.path(), SearchScope::Session).unwrap();
        assert_eq!(targets.len(), 1);
        assert!(targets[0].is_current);
        assert_eq!(targets[0].path, dir.path());
    }

    #[test]
    fn pwd_hash_from_session_dir_uses_parent_name() {
        let p = PathBuf::from("/tmp/koma-fake/sessions/abc123hash/sess-uuid");
        assert_eq!(
            pwd_hash_from_session_dir(&p).as_deref(),
            Some("abc123hash")
        );
        assert_eq!(session_uuid_from_dir(&p), "sess-uuid");
    }
}
