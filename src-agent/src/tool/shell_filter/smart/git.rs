//! Smart filter for `git status` / `git log` / `git diff`. Every other git
//! subcommand (and any of these three run with a shape the user explicitly
//! chose, e.g. `--porcelain`, `--oneline`, `--stat`) is left untouched.

use super::super::FilterOutcome;

/// Try to filter a (already `cd`-stripped) command starting with `git `.
/// Returns `None` for any subcommand/flag combination this module doesn't
/// special-case.
pub(crate) fn try_filter(
    command: &str,
    raw: &str,
    exit_code: Option<i32>,
) -> Option<FilterOutcome> {
    let rest = command.strip_prefix("git ")?.trim_start();
    let mut tokens = rest.split_whitespace();
    let sub = tokens.next().unwrap_or("");
    let args: Vec<&str> = tokens.collect();

    // These summaries only make sense on a successful run. On failure (or an
    // unknown exit status) raw output — e.g. "fatal: ..." — must pass through
    // untouched rather than being silently collapsed.
    if exit_code != Some(0) {
        return None;
    }

    match sub {
        "status" => {
            let already_terse = args
                .iter()
                .any(|a| *a == "-s" || *a == "--short" || a.starts_with("--porcelain"));
            if already_terse {
                None
            } else {
                filter_status(raw)
            }
        }
        "log" => {
            let user_shaped = args.iter().any(|a| {
                *a == "-p"
                    || *a == "--patch"
                    || a.starts_with("--format")
                    || a.starts_with("--pretty")
                    || a.starts_with("--oneline")
                    || a.starts_with("--stat")
            });
            if user_shaped {
                None
            } else {
                filter_log(raw)
            }
        }
        "diff" => {
            let already_terse = args.iter().any(|a| {
                *a == "--stat"
                    || *a == "--shortstat"
                    || *a == "--name-only"
                    || *a == "--name-status"
            });
            if already_terse {
                None
            } else {
                filter_diff(raw)
            }
        }
        _ => None,
    }
}

/// Strip the "status:" style label off a file-list entry, e.g.
/// `modified:   src/main.rs` -> `src/main.rs`. Plain untracked paths (no
/// label) pass through unchanged.
fn extract_path(line: &str) -> String {
    const LABELS: &[&str] = &[
        "modified",
        "new file",
        "deleted",
        "renamed",
        "copied",
        "typechange",
        "both modified",
        "added",
    ];
    if let Some(idx) = line.find(':') {
        let label = &line[..idx];
        if LABELS.contains(&label) {
            return line[idx + 1..].trim().to_string();
        }
    }
    line.to_string()
}

fn push_section(out: &mut String, name: &str, paths: &[String]) {
    if paths.is_empty() {
        return;
    }
    const CAP: usize = 50;
    out.push_str(&format!("{} ({}):\n", name, paths.len()));
    for p in paths.iter().take(CAP) {
        out.push_str("  ");
        out.push_str(p);
        out.push('\n');
    }
    if paths.len() > CAP {
        out.push_str(&format!("  ... [{} more]\n", paths.len() - CAP));
    }
}

/// Compress plain `git status` output: branch/ahead-behind header, then
/// per-section file counts + capped path lists. Drops hint lines and blank
/// padding.
fn filter_status(raw: &str) -> Option<FilterOutcome> {
    #[derive(PartialEq)]
    enum Section {
        None,
        Staged,
        Modified,
        Untracked,
        Other,
    }

    let mut header: Vec<&str> = Vec::new();
    let mut staged: Vec<String> = Vec::new();
    let mut modified: Vec<String> = Vec::new();
    let mut untracked: Vec<String> = Vec::new();
    let mut section = Section::None;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("(use \"git") {
            continue;
        }
        if trimmed == "Changes to be committed:" {
            section = Section::Staged;
            continue;
        }
        if trimmed == "Changes not staged for commit:" {
            section = Section::Modified;
            continue;
        }
        if trimmed == "Untracked files:" {
            section = Section::Untracked;
            continue;
        }
        if trimmed.starts_with("Unmerged paths:") {
            section = Section::Other;
            continue;
        }
        if trimmed.starts_with("no changes added to commit")
            || trimmed.starts_with("nothing added to commit")
            || trimmed.starts_with("nothing to commit")
        {
            continue;
        }

        match section {
            Section::None => header.push(trimmed),
            Section::Staged => staged.push(extract_path(trimmed)),
            Section::Modified => modified.push(extract_path(trimmed)),
            Section::Untracked => untracked.push(extract_path(trimmed)),
            Section::Other => {}
        }
    }

    let mut out = String::new();
    for h in &header {
        out.push_str(h);
        out.push('\n');
    }
    push_section(&mut out, "staged", &staged);
    push_section(&mut out, "modified", &modified);
    push_section(&mut out, "untracked", &untracked);

    let text = out.trim_end().to_string();
    super::finalize(raw, text, "git-status")
}

/// Compress default (no user format flags) `git log` output to one line per
/// commit: `<short-hash> <first message line> (<author>, <date>)`.
fn filter_log(raw: &str) -> Option<FilterOutcome> {
    const CAP: usize = 100;

    let mut out: Vec<String> = Vec::new();
    let mut hash = String::new();
    let mut author = String::new();
    let mut date = String::new();
    let mut message: Option<String> = None;

    fn flush(
        hash: &str,
        author: &str,
        date: &str,
        message: &Option<String>,
        out: &mut Vec<String>,
    ) {
        if hash.is_empty() {
            return;
        }
        let short: String = hash.chars().take(8).collect();
        let msg = message.as_deref().unwrap_or("");
        out.push(format!("{short} {msg} ({author}, {date})"));
    }

    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("commit ") {
            flush(&hash, &author, &date, &message, &mut out);
            hash = rest.split_whitespace().next().unwrap_or("").to_string();
            author.clear();
            date.clear();
            message = None;
        } else if let Some(rest) = line.strip_prefix("Author:") {
            let rest = rest.trim();
            author = rest.split('<').next().unwrap_or(rest).trim().to_string();
        } else if let Some(rest) = line.strip_prefix("Date:") {
            date = rest.trim().to_string();
        } else if line.starts_with("Merge:") {
            // merge-parent line, not a message — ignore
        } else if message.is_none() {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                message = Some(trimmed.to_string());
            }
        }
    }
    flush(&hash, &author, &date, &message, &mut out);

    // No commit blocks parsed at all (e.g. empty repo, or unexpected output
    // shape) — never emit an empty/near-empty result in place of the raw text.
    if out.is_empty() {
        return None;
    }

    let total = out.len();
    if total > CAP {
        out.truncate(CAP);
        out.push(format!("... [{} more commits]", total - CAP));
    }

    let text = out.join("\n");
    super::finalize(raw, text, "git-log")
}

/// Lines that must never be touched or absorbed into a trimmed context run:
/// diff/hunk headers and every actual `+`/`-` change line.
fn is_diff_structural(line: &str) -> bool {
    line.starts_with("diff --git")
        || line.starts_with("index ")
        || line.starts_with("new file mode")
        || line.starts_with("deleted file mode")
        || line.starts_with("similarity index")
        || line.starts_with("rename from")
        || line.starts_with("rename to")
        || line.starts_with("@@")
        || line.starts_with('+')
        || line.starts_with('-')
}

/// Collapse a run of trimmed context lines to first 3 + elision + last 3
/// (only when the run is longer than 6 lines).
fn flush_context(run: &mut Vec<&str>, out: &mut Vec<String>) {
    if run.len() > 6 {
        for l in run.iter().take(3) {
            out.push((*l).to_string());
        }
        out.push(format!("... [{} context lines trimmed]", run.len() - 6));
        for l in &run[run.len() - 3..] {
            out.push((*l).to_string());
        }
    } else {
        for l in run.iter() {
            out.push((*l).to_string());
        }
    }
    run.clear();
}

/// Conservative `git diff` trim: only acts when the raw diff exceeds 20,000
/// chars. Never touches hunk headers or `+`/`-` lines; trims long unchanged
/// context runs to head/tail + elision marker.
fn filter_diff(raw: &str) -> Option<FilterOutcome> {
    if raw.len() <= 20_000 {
        return None;
    }

    let mut out: Vec<String> = Vec::new();
    let mut context_run: Vec<&str> = Vec::new();

    for line in raw.lines() {
        if is_diff_structural(line) {
            flush_context(&mut context_run, &mut out);
            out.push(line.to_string());
        } else {
            context_run.push(line);
        }
    }
    flush_context(&mut context_run, &mut out);

    let text = out.join("\n");
    super::finalize(raw, text, "git-diff")
}

#[cfg(test)]
#[path = "git_test.rs"]
mod tests;
