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
/// - If the (stripped) query begins with `/` it is ABSOLUTE; otherwise it is
///   resolved relative to `cwd`.
/// - The query is split into `(parent, prefix)` at the last `/`. A query ending
///   in `/` means "list everything in this directory" (prefix = "").
/// - `parent` is read with `std::fs::read_dir`; only sub-DIRECTORIES whose file
///   name starts with `prefix` (case-insensitive) are kept. Hidden dirs (leading
///   `.`) are skipped UNLESS the prefix itself starts with `.`.
/// - Each kept directory is rendered back in the user's form (absolute → an
///   absolute path string, relative → a relative string) WITHOUT a trailing
///   slash, sorted, then capped at `limit`.
///
/// Any IO error (unreadable parent, etc.) yields an empty vec — the picker just
/// shows nothing rather than failing.
pub fn list_dirs(query: &str, cwd: &Path, limit: usize) -> Vec<String> {
    // Strip an optional leading '@'; the rest is the path the user is typing.
    let raw = query.strip_prefix('@').unwrap_or(query);
    let is_abs = is_abs_path(raw);

    // Split into the directory part and the in-progress final segment (prefix).
    // A trailing '/' means the whole thing is the parent and the prefix is empty.
    let (dir_part, prefix) = match last_sep(raw) {
        Some(i) => (&raw[..=i], &raw[i + 1..]), // keep the slash on dir_part
        None => ("", raw),                      // no slash: parent is cwd-relative root
    };

    // Resolve the parent directory on the real filesystem.
    let parent: PathBuf = if is_abs {
        // dir_part always starts with '/' here (raw starts with '/').
        PathBuf::from(dir_part)
    } else if dir_part.is_empty() {
        cwd.to_path_buf()
    } else {
        cwd.join(dir_part)
    };

    let entries = match std::fs::read_dir(&parent) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let prefix_lower = prefix.to_lowercase();
    // Honour hidden dirs only when the user is explicitly typing a dotted prefix.
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
        out.push(format!("{dir_part}{name}"));
    }

    out.sort();
    out.truncate(limit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_abs_path_unix() {
        assert!(is_abs_path("/home/user"));
    }

    #[test]
    fn is_abs_path_windows_drive() {
        assert!(is_abs_path("C:\\Users\\foo"));
    }

    #[test]
    fn is_abs_path_windows_unc() {
        assert!(is_abs_path("\\\\server\\share"));
    }

    #[test]
    fn is_abs_path_windows_backslash_root() {
        assert!(is_abs_path("\\Users\\foo"));
    }

    #[test]
    fn is_abs_path_relative_slash() {
        assert!(!is_abs_path("src/main.rs"));
    }

    #[test]
    fn is_abs_path_relative_backslash() {
        assert!(!is_abs_path("src\\main.rs"));
    }

    #[test]
    fn last_sep_forward_slash() {
        assert_eq!(last_sep("src/main.rs"), Some(3));
    }

    #[test]
    fn last_sep_backslash() {
        assert_eq!(last_sep("src\\main.rs"), Some(3));
    }

    #[test]
    fn last_sep_mixed() {
        assert_eq!(last_sep("src/main\\file.rs"), Some(8));
    }

    #[test]
    fn last_sep_no_slash() {
        assert_eq!(last_sep("README.md"), None);
    }
}
