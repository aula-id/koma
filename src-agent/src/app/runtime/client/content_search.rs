//! Host-side Coding panel content search + replace.
//!
//! Walks a workspace root (gitignore-aware via `ignore::WalkBuilder`), matches
//! lines with a compiled regex (literal / whole-word / case flags), and returns
//! results grouped by relative path. Replace reuses the same matcher and writes
//! files in place under the same sandbox as [`super::file_ops`].

use std::path::{Path, PathBuf};

use super::file_ops;
use super::push_proto::PushEnvelope;
use super::HostCtl;

const MAX_TOTAL_MATCHES: usize = 2000;
const MAX_PER_FILE: usize = 100;
const MAX_LINE_CHARS: usize = 300;
const MAX_SEARCH_FILE_BYTES: u64 = 5 * 1024 * 1024;

/// Shared flags for content search / replace (mirrors VS Code Search toggles).
#[derive(Debug, Clone, Copy)]
pub(crate) struct ContentQuery<'a> {
    pub root: &'a str,
    pub path: &'a str,
    pub query: &'a str,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub is_regex: bool,
    pub include_glob: Option<&'a str>,
    pub exclude_glob: Option<&'a str>,
    pub request_id: &'a str,
}

/// One match line inside a file (1-based line/col).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContentSearchMatch {
    pub line: u32,
    pub col: u32,
    pub text: String,
}

/// Matches grouped under one workspace-relative path.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContentSearchFileHit {
    pub path: String,
    pub matches: Vec<ContentSearchMatch>,
}

/// Content-search result for Coding panel / remote-fs.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileContentSearchResult {
    pub root: String,
    pub path: String,
    pub request_id: String,
    pub results: Vec<ContentSearchFileHit>,
    pub error: Option<String>,
    pub truncated: bool,
}

/// Content-replace result for Coding panel / remote-fs.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileContentReplaceResult {
    pub root: String,
    pub path: String,
    pub request_id: String,
    pub files_changed: u32,
    pub match_count: u32,
    pub error: Option<String>,
    pub truncated: bool,
    #[serde(skip)]
    pub mutated: bool,
}

/// Handle FileContentSearch / FileContentReplace HostCtl variants.
pub(super) fn handle_content_ctl(
    ctl: &HostCtl,
    push: &dyn Fn(String),
    workdirs: &[PathBuf],
    session: Option<&str>,
) {
    match ctl {
        HostCtl::FileContentSearch {
            root,
            path,
            query,
            case_sensitive,
            whole_word,
            is_regex,
            include_glob,
            exclude_glob,
            request_id,
        } => {
            let r = exec_file_content_search(
                ContentQuery {
                    root,
                    path,
                    query,
                    case_sensitive: *case_sensitive,
                    whole_word: *whole_word,
                    is_regex: *is_regex,
                    include_glob: include_glob.as_deref(),
                    exclude_glob: exclude_glob.as_deref(),
                    request_id,
                },
                workdirs,
            );
            emit(
                push,
                &PushEnvelope::FileContentSearch {
                    root: r.root,
                    path: r.path,
                    request_id: r.request_id,
                    results: r.results,
                    error: r.error,
                    truncated: r.truncated,
                },
            );
        }
        HostCtl::FileContentReplace {
            root,
            path,
            query,
            replacement,
            case_sensitive,
            whole_word,
            is_regex,
            include_glob,
            exclude_glob,
            request_id,
        } => {
            let r = exec_file_content_replace(
                ContentQuery {
                    root,
                    path,
                    query,
                    case_sensitive: *case_sensitive,
                    whole_word: *whole_word,
                    is_regex: *is_regex,
                    include_glob: include_glob.as_deref(),
                    exclude_glob: exclude_glob.as_deref(),
                    request_id,
                },
                replacement,
                workdirs,
            );
            emit(
                push,
                &PushEnvelope::FileContentReplace {
                    root: r.root,
                    path: r.path,
                    request_id: r.request_id,
                    files_changed: r.files_changed,
                    match_count: r.match_count,
                    error: r.error,
                    truncated: r.truncated,
                },
            );
            if r.mutated {
                file_ops::refresh_git_status_pub(push, session);
            }
        }
        _ => {}
    }
}

fn emit(push: &dyn Fn(String), env: &PushEnvelope) {
    if let Ok(json) = serde_json::to_string(env) {
        push(json);
    }
}

/// Search file contents under `root`/`path` with VS Code-like flags.
pub(crate) fn exec_file_content_search(
    q: ContentQuery<'_>,
    workdirs: &[PathBuf],
) -> FileContentSearchResult {
    let fail = |error: String| FileContentSearchResult {
        root: q.root.to_string(),
        path: q.path.to_string(),
        request_id: q.request_id.to_string(),
        results: Vec::new(),
        error: Some(error),
        truncated: false,
    };

    if q.query.is_empty() {
        return FileContentSearchResult {
            root: q.root.to_string(),
            path: q.path.to_string(),
            request_id: q.request_id.to_string(),
            results: Vec::new(),
            error: None,
            truncated: false,
        };
    }

    let re = match build_pattern(q.query, q.case_sensitive, q.whole_word, q.is_regex) {
        Ok(r) => r,
        Err(e) => return fail(format!("invalid pattern: {e}")),
    };

    let include = match compile_globs(q.include_glob) {
        Ok(g) => g,
        Err(e) => return fail(e),
    };
    let exclude = match compile_globs(q.exclude_glob) {
        Ok(g) => g,
        Err(e) => return fail(e),
    };

    let base = match file_ops::resolve_contained_pub(q.root, q.path, workdirs) {
        Ok(p) => p,
        Err(e) => return fail(e),
    };
    let root_abs = match file_ops::resolve_contained_pub(q.root, "", workdirs) {
        Ok(p) => p,
        Err(e) => return fail(e),
    };

    let mut results: Vec<ContentSearchFileHit> = Vec::new();
    let mut total = 0usize;
    let mut truncated = false;

    let walk = build_walker(&base);
    'outer: for entry in walk.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let abs = entry.path();
        let rel = match abs.strip_prefix(&root_abs) {
            Ok(r) => normalize_rel(r),
            Err(_) => continue,
        };
        if rel.is_empty() {
            continue;
        }
        if !path_allowed(&rel, include.as_ref(), exclude.as_ref()) {
            continue;
        }

        let meta = match std::fs::metadata(abs) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.len() > MAX_SEARCH_FILE_BYTES {
            continue;
        }
        let bytes = match std::fs::read(abs) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if file_ops::looks_binary_pub(&bytes) {
            continue;
        }
        let content = String::from_utf8_lossy(&bytes);

        let mut file_matches: Vec<ContentSearchMatch> = Vec::new();
        for (idx, line) in content.lines().enumerate() {
            if let Some(m) = re.find(line) {
                let preview = if line.chars().count() > MAX_LINE_CHARS {
                    let t: String = line.chars().take(MAX_LINE_CHARS).collect();
                    format!("{t}…")
                } else {
                    line.to_string()
                };
                // col: 1-based char offset of first match on the line
                let col = line[..m.start()].chars().count() as u32 + 1;
                file_matches.push(ContentSearchMatch {
                    line: (idx + 1) as u32,
                    col,
                    text: preview,
                });
                total += 1;
                if file_matches.len() >= MAX_PER_FILE || total >= MAX_TOTAL_MATCHES {
                    truncated = true;
                    break;
                }
            }
        }
        if !file_matches.is_empty() {
            results.push(ContentSearchFileHit {
                path: rel,
                matches: file_matches,
            });
        }
        if truncated {
            break 'outer;
        }
    }

    FileContentSearchResult {
        root: q.root.to_string(),
        path: q.path.to_string(),
        request_id: q.request_id.to_string(),
        results,
        error: None,
        truncated,
    }
}

/// Replace all matches under `root`/`path` with the same flags as search.
pub(crate) fn exec_file_content_replace(
    q: ContentQuery<'_>,
    replacement: &str,
    workdirs: &[PathBuf],
) -> FileContentReplaceResult {
    let fail = |error: String| FileContentReplaceResult {
        root: q.root.to_string(),
        path: q.path.to_string(),
        request_id: q.request_id.to_string(),
        files_changed: 0,
        match_count: 0,
        error: Some(error),
        truncated: false,
        mutated: false,
    };

    if q.query.is_empty() {
        return fail("empty search query".to_string());
    }

    let re = match build_pattern(q.query, q.case_sensitive, q.whole_word, q.is_regex) {
        Ok(r) => r,
        Err(e) => return fail(format!("invalid pattern: {e}")),
    };

    let include = match compile_globs(q.include_glob) {
        Ok(g) => g,
        Err(e) => return fail(e),
    };
    let exclude = match compile_globs(q.exclude_glob) {
        Ok(g) => g,
        Err(e) => return fail(e),
    };

    let base = match file_ops::resolve_contained_pub(q.root, q.path, workdirs) {
        Ok(p) => p,
        Err(e) => return fail(e),
    };
    let root_abs = match file_ops::resolve_contained_pub(q.root, "", workdirs) {
        Ok(p) => p,
        Err(e) => return fail(e),
    };

    let mut files_changed = 0u32;
    let mut match_count = 0u32;
    let mut truncated = false;
    let mut scanned_matches = 0usize;

    let walk = build_walker(&base);
    for entry in walk.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let abs = entry.path();
        let rel = match abs.strip_prefix(&root_abs) {
            Ok(r) => normalize_rel(r),
            Err(_) => continue,
        };
        if rel.is_empty() {
            continue;
        }
        if !path_allowed(&rel, include.as_ref(), exclude.as_ref()) {
            continue;
        }

        let meta = match std::fs::metadata(abs) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.len() > MAX_SEARCH_FILE_BYTES {
            continue;
        }
        let bytes = match std::fs::read(abs) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if file_ops::looks_binary_pub(&bytes) {
            continue;
        }
        let content = match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let count = re.find_iter(&content).count();
        if count == 0 {
            continue;
        }
        scanned_matches += count;
        if scanned_matches > MAX_TOTAL_MATCHES {
            truncated = true;
            // Still apply this file's replacements, then stop.
        }

        let replaced = re.replace_all(&content, replacement).into_owned();
        if replaced == content {
            continue;
        }
        if let Err(e) = std::fs::write(abs, replaced.as_bytes()) {
            return fail(format!("failed to write {rel}: {e}"));
        }
        files_changed += 1;
        match_count += count as u32;

        if truncated || scanned_matches >= MAX_TOTAL_MATCHES {
            truncated = true;
            break;
        }
    }

    FileContentReplaceResult {
        root: q.root.to_string(),
        path: q.path.to_string(),
        request_id: q.request_id.to_string(),
        files_changed,
        match_count,
        error: None,
        truncated,
        mutated: files_changed > 0,
    }
}

fn build_pattern(
    query: &str,
    case_sensitive: bool,
    whole_word: bool,
    is_regex: bool,
) -> Result<regex::Regex, String> {
    let body = if is_regex {
        query.to_string()
    } else {
        regex::escape(query)
    };
    let pat = if whole_word {
        format!(r"\b(?:{body})\b")
    } else {
        body
    };
    regex::RegexBuilder::new(&pat)
        .case_insensitive(!case_sensitive)
        .dot_matches_new_line(false)
        .build()
        .map_err(|e| e.to_string())
}

fn compile_globs(raw: Option<&str>) -> Result<Option<globset::GlobSet>, String> {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let mut builder = globset::GlobSetBuilder::new();
    for part in raw.split(&[',', '\n'][..]) {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        let g = globset::Glob::new(p).map_err(|e| format!("invalid glob '{p}': {e}"))?;
        builder.add(g);
    }
    builder
        .build()
        .map(Some)
        .map_err(|e| format!("invalid glob set: {e}"))
}

fn path_allowed(
    rel: &str,
    include: Option<&globset::GlobSet>,
    exclude: Option<&globset::GlobSet>,
) -> bool {
    if let Some(ex) = exclude {
        if ex.is_match(rel) {
            return false;
        }
        // Also try basename-only patterns like `*.rs` against the file name.
        if let Some(name) = Path::new(rel).file_name().and_then(|n| n.to_str()) {
            if ex.is_match(name) {
                return false;
            }
        }
    }
    if let Some(inc) = include {
        if inc.is_match(rel) {
            return true;
        }
        if let Some(name) = Path::new(rel).file_name().and_then(|n| n.to_str()) {
            return inc.is_match(name);
        }
        return false;
    }
    true
}

fn build_walker(base: &Path) -> ignore::Walk {
    ignore::WalkBuilder::new(base)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .filter_entry(|entry| {
            // Skip common junk dirs even if not gitignored.
            if entry.depth() > 0 {
                let name = entry.file_name().to_string_lossy();
                if matches!(
                    name.as_ref(),
                    ".git" | ".koma" | "node_modules" | "target" | "dist" | "build" | ".next"
                        | "vendor" | "__pycache__" | ".venv" | "venv"
                ) {
                    return false;
                }
            }
            true
        })
        .build()
}

fn normalize_rel(p: &Path) -> String {
    p.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
