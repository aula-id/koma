//! Host-side per-commit ACTIVITY computation for the upcoming GitKraken-style
//! bubble/activity chart (GK5a) — `compute_git_activity` mirrors [`super::git_graph`]'s
//! exact host-relay pattern (the [`super::git`] `git_cmd` choke point, an error-flagged
//! result struct instead of a panic, `#[serde(rename_all = "camelCase")]` DTOs) rather
//! than duplicating any of its plumbing: reuses [`super::git::repo_root_for`]/
//! `git_failure`/`git_cmd`. Host-local only — never the daemon, never
//! `git_cred.rs`/`git_operator.rs`.
//!
//! `compute_git_activity` answers per-commit author/date/lines-changed totals for the
//! ACTIVE branch (`HEAD`), optionally scoped to one pathspec — the raw series the chart
//! buckets/aggregates client-side. `pub(super)` — called off a worker thread by
//! [`super::git_host`], exactly like [`super::git_graph::compute_git_graph`].

use super::git::{git_cmd, git_failure, repo_root_for};

/// One commit's ACTIVITY row in an [`ActivityResult`], parsed off one `git log
/// --numstat` record. `added`/`deleted` are the SUMMED line counts across every
/// changed file in the commit (binary files, which `--numstat` reports as `-\t-`,
/// contribute `0` to both — there is no meaningful line count for them). `date` is
/// the author date in ISO-8601 (`%aI`), left as a string for the chart to bucket
/// itself rather than parsed host-side.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActivityCommit {
    pub sha: String,
    pub author: String,
    pub email: String,
    pub date: String,
    pub added: u32,
    pub deleted: u32,
}

/// The result of a host-side [`compute_git_activity`], pushed to the GUI as an
/// `Activity` envelope. `error` set means the workdir isn't a git repository (or
/// `git log` itself failed) — `commits` is then empty rather than the caller
/// panicking, mirroring [`super::git_graph::GitGraphResult`]'s always-reply rule.
/// `path` echoes the REQUEST's pathspec (`None` for the whole-branch case) verbatim,
/// mirroring [`super::git_graph::CommitDetailResult`]'s `sha` echo — the reducer on the
/// GUI side compares it against the currently-requested path to drop a stale reply when
/// two `GitActivity` requests race (lock-acquisition/thread-scheduling order isn't FIFO).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActivityResult {
    pub commits: Vec<ActivityCommit>,
    pub path: Option<String>,
    pub error: Option<String>,
}

/// Parse `git log --numstat --pretty=format:%x1e%H%x1f%an%x1f%ae%x1f%aI`'s output into
/// [`ActivityCommit`] rows. `%x1e` (the ASCII record-separator byte) prefixes each
/// commit's header line, so splitting the WHOLE stdout on it yields one record per
/// commit (the first split segment is the empty string before the very first `%x1e` —
/// skipped like every other empty/malformed record). Within a record, the FIRST line is
/// the header (`sha%x1fauthorName%x1fauthorEmail%x1fauthorDateISO`, split on `%x1f`,
/// the ASCII unit-separator byte); every remaining line is a `--numstat` row
/// (`<added>\t<deleted>\t<path>`), summed into the commit's totals. A commit with no
/// file changes (e.g. an empty commit) simply has no numstat lines — totals stay `0`,
/// not an error. `added`/`deleted` parse failures (a non-numeric field, `-` for a
/// binary file, or a truncated line) are silently treated as `0` rather than aborting
/// the whole record — robustness over a single malformed row.
fn parse_activity(stdout: &str) -> Vec<ActivityCommit> {
    let mut commits = Vec::new();

    for record in stdout.split('\u{1e}') {
        if record.trim().is_empty() {
            continue; // the empty lead-in before the first record sep, or a stray blank
        }

        let mut lines = record.lines();
        let Some(header) = lines.next() else { continue };
        let fields: Vec<&str> = header.split('\u{1f}').collect();
        if fields.len() < 4 {
            continue; // malformed header (shouldn't happen) — skip rather than panic
        }

        let mut added: u32 = 0;
        let mut deleted: u32 = 0;
        for line in lines {
            if line.trim().is_empty() {
                continue; // the blank separator line git prints before the next record
            }
            let cols: Vec<&str> = line.splitn(3, '\t').collect();
            if cols.len() < 2 {
                continue; // truncated/malformed numstat row — skip rather than panic
            }
            // `-` marks a binary file (no meaningful line count) — treated as 0, same
            // as any other unparseable field, rather than aborting the commit's totals.
            added = added.saturating_add(cols[0].parse::<u32>().unwrap_or(0));
            deleted = deleted.saturating_add(cols[1].parse::<u32>().unwrap_or(0));
        }

        commits.push(ActivityCommit {
            sha: fields[0].to_string(),
            author: fields[1].to_string(),
            email: fields[2].to_string(),
            date: fields[3].to_string(),
            added,
            deleted,
        });
    }

    commits
}

/// Compute host-side per-commit ACTIVITY (author/date/lines-changed) for the ACTIVE
/// branch (`HEAD`), answering a [`super::HostCtl::GitActivity`]. Resolves the repo root
/// off `session` ([`repo_root_for`]) first — a non-git workdir sets `error` rather than
/// panicking, mirroring [`super::git_graph::compute_git_graph`]'s always-reply rule.
///
/// `path` narrows the log to one pathspec (`git log ... -- <path>`, the chart's
/// per-file activity view); it is validated minimally — rejecting a leading `-` so it
/// can never be parsed as a git OPTION instead of a pathspec — before being appended.
/// No further traversal guard is needed: it's handed to git as a pathspec AFTER `--`,
/// git itself resolves/rejects it, and no filesystem read happens here at all (unlike
/// [`super::git::compute_git_diff`]'s worktree-read side).
///
/// `limit` is floored to `1` (a `0` would ask git for its own "no commits" semantics,
/// not "unlimited" — see [`super::git_graph::compute_git_graph`]'s identical floor).
pub(crate) fn compute_git_activity(
    path: Option<&str>,
    limit: u32,
    session: Option<&str>,
) -> ActivityResult {
    let empty = |error: Option<String>| ActivityResult {
        commits: Vec::new(),
        path: path.map(str::to_string),
        error,
    };

    let Some(root) = repo_root_for(session) else {
        return empty(Some("not a git repository".to_string()));
    };

    if let Some(p) = path {
        if p.starts_with('-') {
            return empty(Some("invalid path".to_string()));
        }
    }

    let limit = limit.max(1);
    let limit_arg = format!("--max-count={limit}");
    const PRETTY: &str = "--pretty=format:%x1e%H%x1f%an%x1f%ae%x1f%aI";

    let mut args: Vec<&str> = vec!["log", "HEAD", "--numstat", PRETTY, &limit_arg];
    if let Some(p) = path {
        args.push("--");
        args.push(p);
    }

    let stdout = match git_cmd(&root, &args) {
        Some(out) if out.status.success() => out.stdout,
        Some(out) => return empty(Some(git_failure(&out, "git log failed"))),
        None => return empty(Some("failed to run git".to_string())),
    };

    let text = String::from_utf8_lossy(&stdout);
    ActivityResult {
        commits: parse_activity(&text),
        path: path.map(str::to_string),
        error: None,
    }
}
