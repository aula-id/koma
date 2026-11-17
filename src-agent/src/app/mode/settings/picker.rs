//! A real-filesystem directory picker overlay (the `@`-style descent UI).
//!
//! [`PathPicker`] holds the query, live matches, cursor, and mode.
//! [`list_dirs`] does the actual `read_dir` + prefix filtering.

use std::path::{Path, PathBuf};

/// What a confirmed [`PathPicker`] selection does to the target path list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerMode {
    /// Append the chosen path as a new entry.
    Add,
    /// Replace the entry at this index in the list.
    Replace(usize),
}

/// A real-filesystem directory picker overlay (the `@`-style descent UI).
///
/// `query` is the raw text the user types (a leading `@` is allowed and stripped
/// when matching). `matches` is the live list of directories under the resolved
/// parent whose name starts with the typed prefix, rendered in the SAME form the
/// user is typing (absolute → absolute, relative → relative). `sel` indexes
/// `matches`. `mode` decides whether confirming adds or replaces in the list.
#[derive(Debug, Clone)]
pub struct PathPicker {
    /// Raw query text (may begin with `@`; relative or absolute).
    pub query: String,
    /// Directory matches for the current query, capped at the view limit.
    pub matches: Vec<String>,
    /// Cursor within `matches`.
    pub sel: usize,
    /// Add a new entry, or replace an existing one at the given index.
    pub mode: PickerMode,
}

/// Max directory matches surfaced in the picker overlay (mirrors the chat `@`
/// file palette and the view-side window constant).
pub const PICKER_MAX: usize = 10;

impl PathPicker {
    /// Open a picker in the given `mode`, seeded with `query` (used to prefill a
    /// REPLACE with the current entry). Computes the first match set against `cwd`.
    pub fn new(mode: PickerMode, query: String, cwd: &Path) -> Self {
        let matches = list_dirs(&query, cwd, PICKER_MAX);
        Self {
            query,
            matches,
            sel: 0,
            mode,
        }
    }

    /// Recompute `matches` for the current `query` and clamp `sel` into range.
    pub fn recompute(&mut self, cwd: &Path) {
        self.matches = list_dirs(&self.query, cwd, PICKER_MAX);
        if self.sel >= self.matches.len() {
            self.sel = self.matches.len().saturating_sub(1);
        }
    }

    /// Move the selection up one row (clamps at 0).
    pub fn up(&mut self) {
        self.sel = self.sel.saturating_sub(1);
    }

    /// Move the selection down one row (clamps at the last match).
    pub fn down(&mut self) {
        if self.sel + 1 < self.matches.len() {
            self.sel += 1;
        }
    }

    /// The currently highlighted match, if any.
    pub fn selected(&self) -> Option<&String> {
        self.matches.get(self.sel)
    }
}

/// Whether `raw` looks like an absolute path on any OS.
/// Matches `/home/user`, `\Users\foo`, `C:\Users\foo`, `\\server\share`.
fn is_abs_path(raw: &str) -> bool {
    raw.starts_with('/')
        || raw.starts_with('\\')
        || (raw.len() >= 2 && raw.as_bytes().get(1) == Some(&b':'))
}

/// Position of the last path separator (either `/` or `\`), or `None`.
fn last_sep(raw: &str) -> Option<usize> {
    raw.rfind(['/', '\\'])
}

/// List directories for an `@`-style `query`, rendered in the same form the user
/// is typing them, capped at `limit`.
///
/// Resolution:
/// - A leading `@` is stripped.
/// - `~` and `~/…` (or `~\…`) are the home directory. `./…` (or `.\…`) is `cwd`.
///   Other queries that begin with `/` (or a Windows drive / UNC prefix) are
///   absolute. Anything else is relative to `cwd`.
/// - The query is split into `(parent, prefix)` at the last separator. A query
///   ending in `/` means "list everything in this directory" (prefix = "").
/// - `parent` is read with `std::fs::read_dir`; only sub-DIRECTORIES whose file
///   name starts with `prefix` (case-insensitive) are kept. Hidden dirs (leading
///   `.`) are skipped UNLESS the prefix itself starts with `.`.
/// - Each kept directory is rendered back in the user's form (`~/`, `./`,
///   absolute, or relative) WITHOUT a trailing slash, sorted, then capped at
///   `limit`. Tab descend writes that form back into the query, so the next Tab
///   enters the next directory in the same spelling.
///
/// Any IO error (unreadable parent, missing home, etc.) yields an empty vec —
/// the picker just shows nothing rather than failing.
pub fn list_dirs(query: &str, cwd: &Path, limit: usize) -> Vec<String> {
    list_dirs_in(query, cwd, dirs::home_dir().as_deref(), limit)
}

fn list_dirs_in(query: &str, cwd: &Path, home: Option<&Path>, limit: usize) -> Vec<String> {
    let raw = query.strip_prefix('@').unwrap_or(query);
    let (display_dir, prefix, parent) = plan_listing(raw, cwd, home);
    collect_dirs(&display_dir, prefix, &parent, limit)
}

/// Turn a confirmed picker row into a filesystem path. `~/…` and `./…` stay in
/// that form while browsing; the saved workspace or allowed-dir entry is the
/// absolute directory.
pub(crate) fn materialize_picker_path(chosen: &str, cwd: &Path) -> String {
    materialize_in(chosen, cwd, dirs::home_dir().as_deref())
}

fn materialize_in(chosen: &str, cwd: &Path, home: Option<&Path>) -> String {
    let raw = trim_picker_text(chosen);
    if raw.is_empty() {
        return String::new();
    }
    let path = filesystem_path(raw, cwd, home);
    crate::tool::winpath::canonical(&path)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn trim_picker_text(chosen: &str) -> &str {
    let raw = chosen.strip_prefix('@').unwrap_or(chosen).trim();
    if raw == "/" || raw == "\\" {
        return raw;
    }
    let bytes = raw.as_bytes();
    if bytes.len() == 3 && bytes[1] == b':' && (bytes[2] == b'/' || bytes[2] == b'\\') {
        return raw;
    }
    let trimmed = raw.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        raw
    } else {
        trimmed
    }
}

fn filesystem_path(raw: &str, cwd: &Path, home: Option<&Path>) -> PathBuf {
    if let Some(home) = home {
        if raw == "~" {
            return home.to_path_buf();
        }
        if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
            return home.join(rest);
        }
    }
    if raw == "." {
        return cwd.to_path_buf();
    }
    if let Some(rest) = raw.strip_prefix("./").or_else(|| raw.strip_prefix(".\\")) {
        return if rest.is_empty() {
            cwd.to_path_buf()
        } else {
            cwd.join(rest)
        };
    }
    if is_abs_path(raw) {
        PathBuf::from(raw)
    } else {
        cwd.join(raw)
    }
}

/// `(display directory including its trailing separator, in-progress name, fs parent)`.
fn plan_listing<'a>(raw: &'a str, cwd: &Path, home: Option<&Path>) -> (String, &'a str, PathBuf) {
    if let Some(planned) = tilde_listing(raw, home) {
        return planned;
    }
    if let Some(planned) = dot_listing(raw, cwd) {
        return planned;
    }
    let is_abs = is_abs_path(raw);
    let (dir_part, prefix) = split_dir_prefix(raw);
    let parent = if is_abs {
        PathBuf::from(dir_part)
    } else if dir_part.is_empty() {
        cwd.to_path_buf()
    } else {
        cwd.join(dir_part)
    };
    (dir_part.to_string(), prefix, parent)
}

/// `~`, `~/rest`, and `~\rest`. A `~user` name is not expanded.
fn tilde_listing<'a>(raw: &'a str, home: Option<&Path>) -> Option<(String, &'a str, PathBuf)> {
    let home = home?;
    let (sep, rest) = if raw == "~" {
        ('/', "")
    } else if let Some(rest) = raw.strip_prefix("~/") {
        ('/', rest)
    } else {
        let rest = raw.strip_prefix("~\\")?;
        ('\\', rest)
    };
    let (dir, prefix) = split_dir_prefix(rest);
    let display = if dir.is_empty() {
        format!("~{sep}")
    } else {
        format!("~{sep}{dir}")
    };
    let parent = if dir.is_empty() {
        home.to_path_buf()
    } else {
        home.join(dir)
    };
    Some((display, prefix, parent))
}

/// `./rest` and `.\rest`. A bare `.` stays a relative prefix, not "this directory".
fn dot_listing<'a>(raw: &'a str, cwd: &Path) -> Option<(String, &'a str, PathBuf)> {
    let (sep, rest) = if let Some(rest) = raw.strip_prefix("./") {
        ('/', rest)
    } else {
        let rest = raw.strip_prefix(".\\")?;
        ('\\', rest)
    };
    let (dir, prefix) = split_dir_prefix(rest);
    let display = if dir.is_empty() {
        format!(".{sep}")
    } else {
        format!(".{sep}{dir}")
    };
    let parent = if dir.is_empty() {
        cwd.to_path_buf()
    } else {
        cwd.join(dir)
    };
    Some((display, prefix, parent))
}

fn split_dir_prefix(rest: &str) -> (&str, &str) {
    match last_sep(rest) {
        Some(i) => (&rest[..=i], &rest[i + 1..]),
        None => ("", rest),
    }
}

fn collect_dirs(display_dir: &str, prefix: &str, parent: &Path, limit: usize) -> Vec<String> {
    if limit == 0 {
        return Vec::new();
    }
    let entries = match std::fs::read_dir(parent) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let prefix_lower = prefix.to_lowercase();
    let want_hidden = prefix.starts_with('.');
    let mut out: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue,
        };
        if name.starts_with('.') && !want_hidden {
            continue;
        }
        if !name.to_lowercase().starts_with(&prefix_lower) {
            continue;
        }
        out.push(format!("{display_dir}{name}"));
    }
    out.sort();
    out.truncate(limit);
    out
}

#[cfg(test)]
#[path = "picker_test.rs"]
mod tests;
