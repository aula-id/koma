//! Host-side COMMIT GRAPH computation for a GitKraken-style Explore "GIT" panel view —
//! `compute_git_graph`/`compute_commit_detail`/`compute_commit_diff` mirror [`super::git`]'s
//! exact host-relay pattern (every git invocation through `git_cmd_env`, an error-flagged
//! result struct instead of a panic, `#[serde(rename_all = "camelCase")]` DTOs) rather than
//! duplicating any of its plumbing: reuses [`super::git::repo_root_for`]/[`git_failure`] and
//! [`super::diff`]'s `looks_binary`/`FILE_DIFF_SIZE_CAP`. Host-local only — never the daemon,
//! never `git_cred.rs`/`git_operator.rs` (the model's own git credential machinery).
//!
//! `compute_git_graph` answers a paginated commit list across every ref (`git log --all`);
//! `compute_commit_detail` answers one commit's full metadata + changed-file list (a graph
//! row click); `compute_commit_diff` answers one file's diff at a commit vs its first parent
//! (a commit-detail file-row click). All three are `pub(super)` — called off a worker thread
//! by [`super::git_host`], exactly like [`super::git::compute_git_status`].

use super::diff::{looks_binary, FILE_DIFF_SIZE_CAP};
use super::git::{git_cmd, git_failure, repo_root_for};

/// One ref (branch/tag/HEAD pointer) attached to a commit, parsed out of `%D` (with
/// `--decorate=full`, so every non-HEAD token is a FULL ref path — `refs/heads/…`,
/// `refs/remotes/…`, `refs/tags/…` — classified unambiguously by prefix rather than by
/// guessing off the short display name). `kind` is `"head"` (the `HEAD -> refs/heads/X`
/// pointer itself), `"local"` (`refs/heads/X`), `"remote"` (`refs/remotes/X` — a real
/// remote-tracking ref, never confusable with a local branch merely NAMED like one, e.g.
/// a local branch literally called `origin/weird`), or `"tag"` (`refs/tags/X`). `is_head`
/// is `true` only for the `"head"` entry.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GitRef {
    pub name: String,
    pub kind: String,
    pub is_head: bool,
}

/// One commit row in a [`GitGraphResult`], parsed off one `git log --parents -z` record.
/// `parents` is empty for a root commit; `refs` is empty for a commit with nothing pointing
/// at it directly (the common case — most commits carry no `%D`).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GitCommitNode {
    pub sha: String,
    pub parents: Vec<String>,
    pub refs: Vec<GitRef>,
    pub author: String,
    pub email: String,
    pub date: String,
    pub subject: String,
}

/// The result of a host-side [`compute_git_graph`], pushed to the GUI as a `GitGraph`
/// envelope. `error` set means the workdir isn't a git repository (or `git log` itself
/// failed) — `commits` is then empty rather than the caller panicking. `has_more` is a
/// pagination hint: a full page of RAW records from `git log` (judged before any
/// malformed-record skip, so a single bad record can't under-report this) likely means more
/// history exists past `skip + limit`, so the panel can offer a "load more" / infinite-scroll
/// continuation without a separate count query.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GitGraphResult {
    pub commits: Vec<GitCommitNode>,
    pub head: Option<String>,
    pub has_more: bool,
    pub error: Option<String>,
}

/// One changed-file entry in a [`CommitDetailResult`], parsed off one `git diff-tree
/// --name-status -z` record. `status` is git's own single-letter/score token (`"M"`/`"A"`/
/// `"D"`/`"R100"`/`"C75"`/…); `orig_path` is `Some` only for a rename/copy record (`path`
/// is then the NEW path, `orig_path` the OLD one).
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CommitFile {
    pub status: String,
    pub path: String,
    pub orig_path: Option<String>,
}

/// The result of a host-side [`compute_commit_detail`], pushed to the GUI as a
/// `CommitDetail` envelope. `error` set means `sha` failed validation, the workdir isn't a
/// git repository, or `git show` itself failed — every other field is then a neutral
/// default (empty strings/lists) rather than a panic. `parents` mirrors
/// [`GitCommitNode::parents`]; `files` is the FIRST-PARENT changed-file view (sensible for
/// a merge commit, exact for an ordinary one).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CommitDetailResult {
    pub sha: String,
    pub author: String,
    pub email: String,
    pub date: String,
    pub subject: String,
    pub body: String,
    pub parents: Vec<String>,
    pub files: Vec<CommitFile>,
    pub error: Option<String>,
}

/// The result of a host-side [`compute_commit_diff`], pushed to the GUI as a `CommitDiff`
/// envelope. `error` set means `sha` failed validation, the workdir isn't a git
/// repository, or either side was over the size cap (both strings then empty); `binary`
/// set means either side isn't valid UTF-8 text (both strings then empty, no `error`). A
/// missing blob on either side (the commit is a root commit, or the file was added/deleted
/// in it) is NOT an error — `git show` failing for that one side just leaves it empty (a
/// valid all-added/all-removed diff). SEPARATE struct from [`super::git::GitDiffResult`]
/// (no `staged` field — meaningless for a historical commit diff) so the GUI can route a
/// commit-history diff to its own tab id without colliding with the working-tree/index one.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CommitDiffResult {
    pub sha: String,
    pub path: String,
    pub original: String,
    pub modified: String,
    pub error: Option<String>,
    pub binary: bool,
}

/// Reject anything that isn't a plausible git object reference before it's ever
/// interpolated into a `git` arg (e.g. `<sha>^1:<path>` / `<sha>:<path>`) — no shell is
/// involved (every invocation goes through [`std::process::Command`]'s arg vector, never a
/// shell string), but a bogus/adversarial `sha` should still be bounced host-side rather
/// than handed to git and trusted to fail safely. Allows hex shas, short shas, and any
/// git-ish revision made of `[A-Za-z0-9._/-]` (covers branch/tag names too, since a ref
/// name is accepted anywhere a commit-ish is); rejects whitespace, empty input, and any
/// shell/path metacharacter. Also rejects a LEADING `-` (positionally handed to a `git`
/// subcommand, a leading-dash "sha" like `-1` would be parsed as an OPTION instead of a
/// revision — e.g. `git show -s ... -1 ...` silently falls back to describing `HEAD`, no
/// error, just wrong data) and any `..`/`...` substring (a valid RANGE syntax like
/// `<root>..<HEAD>` that makes `git show`/`git diff-tree` emit multiple records and
/// corrupts the fixed-field-count parse downstream).
pub(super) fn valid_commit_ref(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && !s.contains("..")
        && s.len() <= 200
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'))
}

/// Parse one commit's `%D` (`git log --decorate=full`'s "ref names" field — e.g. `HEAD ->
/// refs/heads/main, refs/remotes/origin/main, refs/tags/v1.0`) into structured [`GitRef`]s,
/// comma-space-separated per git's own convention. Classifying off the FULL ref path (not
/// the short display name) is what makes this unambiguous — a local branch literally named
/// `origin/weird` shows up as `refs/heads/origin/weird`, never confusable with the actual
/// remote-tracking ref `refs/remotes/origin/weird`. Each token classifies as:
/// - bare `HEAD` (no arrow) — a DETACHED HEAD landing on this commit; returned via the bool,
///   not pushed as a ref (there is no branch name to show).
/// - `HEAD -> refs/heads/X` — the current branch pointer; `kind: "head"`, `is_head: true`,
///   name is `X` (the `refs/heads/` prefix stripped).
/// - `refs/heads/X` — a plain local branch, `kind: "local"`, name is `X`.
/// - `refs/remotes/X` — a remote-tracking ref, `kind: "remote"`, name is `X` (e.g.
///   `origin/main` — the `refs/remotes/` prefix stripped, matching git's own short display
///   convention for remotes).
/// - `refs/tags/X` (or the `tag: refs/tags/X` form some git versions emit) — `kind: "tag"`,
///   name is `X`.
/// - anything else (a form `--decorate=full` doesn't document, e.g. a stray note ref) — SKIPPED
///   rather than emitted as a bogus/misclassified ref.
fn parse_refs(raw: &str) -> (Vec<GitRef>, bool) {
    let mut refs = Vec::new();
    let mut detached = false;
    if raw.trim().is_empty() {
        return (refs, detached);
    }
    for token in raw.split(", ") {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        if token == "HEAD" {
            detached = true;
            continue;
        }
        if let Some(name) = token.strip_prefix("HEAD -> ") {
            if let Some(name) = name.strip_prefix("refs/heads/") {
                refs.push(GitRef {
                    name: name.to_string(),
                    kind: "head".to_string(),
                    is_head: true,
                });
            }
            continue;
        }
        // Tolerate the `tag: refs/tags/X` form some git versions emit alongside the plain
        // `refs/tags/X` form.
        let token = token.strip_prefix("tag: ").unwrap_or(token);
        if let Some(name) = token.strip_prefix("refs/heads/") {
            refs.push(GitRef {
                name: name.to_string(),
                kind: "local".to_string(),
                is_head: false,
            });
        } else if let Some(name) = token.strip_prefix("refs/remotes/") {
            refs.push(GitRef {
                name: name.to_string(),
                kind: "remote".to_string(),
                is_head: false,
            });
        } else if let Some(name) = token.strip_prefix("refs/tags/") {
            refs.push(GitRef {
                name: name.to_string(),
                kind: "tag".to_string(),
                is_head: false,
            });
        }
        // anything else — not a form `--decorate=full` documents — skipped.
    }
    (refs, detached)
}

/// Compute a host-side, paginated COMMIT GRAPH across every ref, answering a
/// [`super::HostCtl::GitGraph`]. Resolves the repo root off `session` ([`repo_root_for`]),
/// then runs `git log --all --date-order --parents -z --decorate=full --pretty=format:%H%x1f
/// %P%x1f%D%x1f%an%x1f%ae%x1f%aI%x1f%s --max-count=<limit> --skip=<skip>`: `-z` NUL-separates
/// commit RECORDS (no shell/parsing ambiguity from a subject containing a literal newline),
/// and `%x1f` (the ASCII unit-separator byte) delimits the 7 FIELDS within each record —
/// chosen specifically because it can never appear in any of git's own field values.
/// `--decorate=full` makes `%D` emit FULL ref paths (`refs/heads/…`/`refs/remotes/…`/
/// `refs/tags/…`) rather than git's short display names, which is what lets [`parse_refs`]
/// classify unambiguously by prefix instead of guessing off a remote-name string match.
/// `--parents` is redundant with `%P` in the format string but kept for clarity/robustness
/// (mirrors the pattern `git log` docs use together). ALWAYS returns a result — a non-git
/// workdir sets `error` rather than panicking, mirroring
/// [`super::git::compute_git_status`]'s always-reply rule.
pub(super) fn compute_git_graph(limit: u32, skip: u32, session: Option<&str>) -> GitGraphResult {
    let empty = |error: Option<String>| GitGraphResult {
        commits: Vec::new(),
        head: None,
        has_more: false,
        error,
    };

    let Some(root) = repo_root_for(session) else {
        return empty(Some("not a git repository".to_string()));
    };

    // A limit of 0 would ask git for "unlimited" (`--max-count=0` is actually "no commits"
    // in git's own semantics, but floor it defensively anyway so a malformed request can
    // never wedge on an unbounded log).
    let limit = limit.max(1);
    let limit_arg = format!("--max-count={limit}");
    let skip_arg = format!("--skip={skip}");
    const PRETTY: &str = "--pretty=format:%H%x1f%P%x1f%D%x1f%an%x1f%ae%x1f%aI%x1f%s";

    let stdout = match git_cmd(
        &root,
        &[
            "log",
            "--all",
            "--date-order",
            "--parents",
            "-z",
            "--decorate=full",
            PRETTY,
            &limit_arg,
            &skip_arg,
        ],
    ) {
        Some(out) if out.status.success() => out.stdout,
        Some(out) => return empty(Some(git_failure(&out, "git log failed"))),
        None => return empty(Some("failed to run git".to_string())),
    };

    let mut commits = Vec::new();
    // Raw record count (before the malformed-skip below) — `has_more` must be judged against
    // how many records git actually returned, not how many survived parsing, or a single
    // skipped malformed record would under-report a full page as "no more history".
    let mut raw_count: u32 = 0;

    for record in stdout.split(|b| *b == 0) {
        if record.is_empty() {
            continue;
        }
        raw_count += 1;
        let record = String::from_utf8_lossy(record);
        let record = record.trim_end_matches('\n');
        let fields: Vec<&str> = record.split('\u{1f}').collect();
        if fields.len() < 7 {
            continue; // malformed record (shouldn't happen) — skip rather than panic
        }
        let (refs, _detached) = parse_refs(fields[2]);
        commits.push(GitCommitNode {
            sha: fields[0].to_string(),
            parents: fields[1].split_whitespace().map(str::to_string).collect(),
            refs,
            author: fields[3].to_string(),
            email: fields[4].to_string(),
            date: fields[5].to_string(),
            subject: fields[6].to_string(),
        });
    }

    let has_more = raw_count >= limit;

    let head = match git_cmd(&root, &["rev-parse", "HEAD"]) {
        Some(out) if out.status.success() => {
            let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if sha.is_empty() {
                None
            } else {
                Some(sha)
            }
        }
        _ => None,
    };

    GitGraphResult {
        commits,
        head,
        has_more,
        error: None,
    }
}

/// Parse `git diff-tree --name-status -z`'s NUL-separated output into [`CommitFile`]
/// entries. An ordinary record is `<status>\0<path>\0`; a rename/copy record (status
/// starts with `R`/`C`, e.g. `R100`) is `<status>\0<origPath>\0<path>\0` — one EXTRA NUL
/// field. Mirrors [`super::git::compute_git_status`]'s NUL-field-walking style for the
/// analogous rename record in `--porcelain=v2`.
fn parse_name_status(raw: &[u8]) -> Vec<CommitFile> {
    let fields: Vec<String> = raw
        .split(|b| *b == 0)
        .filter(|f| !f.is_empty())
        .map(|f| String::from_utf8_lossy(f).into_owned())
        .collect();

    let mut files = Vec::new();
    let mut i = 0;
    while i < fields.len() {
        let status = fields[i].clone();
        i += 1;
        if status.starts_with('R') || status.starts_with('C') {
            if i + 1 >= fields.len() {
                break; // truncated/malformed — stop rather than index out of bounds
            }
            let orig_path = fields[i].clone();
            let path = fields[i + 1].clone();
            i += 2;
            files.push(CommitFile {
                status,
                path,
                orig_path: Some(orig_path),
            });
        } else {
            if i >= fields.len() {
                break;
            }
            let path = fields[i].clone();
            i += 1;
            files.push(CommitFile {
                status,
                path,
                orig_path: None,
            });
        }
    }
    files
}

/// The changed-file list for commit `sha`, FIRST-PARENT only (a sensible single-parent view
/// for a merge commit, exact for an ordinary one) via `git diff-tree --no-commit-id
/// --name-status -r -z --first-parent <sha>`. An empty result (root commit with nothing to
/// diff against, or the command failing) is fine — [`compute_commit_detail`] treats a
/// missing file list as "no changes shown", not an error in itself.
fn commit_files(root: &std::path::Path, sha: &str) -> Vec<CommitFile> {
    match git_cmd(
        root,
        &[
            "diff-tree",
            "--no-commit-id",
            "--name-status",
            "-r",
            "-z",
            "--first-parent",
            sha,
        ],
    ) {
        Some(out) if out.status.success() => parse_name_status(&out.stdout),
        _ => Vec::new(),
    }
}

/// Compute a host-side COMMIT DETAIL for `sha` (a [`GitCommitNode`]'s sha, or any
/// git-ish revision — validated via [`valid_commit_ref`] before ever touching `git`),
/// answering a [`super::HostCtl::GitCommitDetail`]. Runs `git show -s --pretty=format:
/// %H%x1f%P%x1f%an%x1f%ae%x1f%aI%x1f%s%x1f%b <sha>` for full metadata INCLUDING the commit
/// body (`%b` — absent from [`compute_git_graph`]'s per-row format to keep the graph list
/// lean), then [`commit_files`] for the changed-file list. `body` is `splitn`'d as the LAST
/// field so an (unlikely) literal `\x1f` byte inside a multi-line body can't truncate it.
pub(super) fn compute_commit_detail(sha: &str, session: Option<&str>) -> CommitDetailResult {
    let empty = |error: Option<String>| CommitDetailResult {
        sha: sha.to_string(),
        author: String::new(),
        email: String::new(),
        date: String::new(),
        subject: String::new(),
        body: String::new(),
        parents: Vec::new(),
        files: Vec::new(),
        error,
    };

    if !valid_commit_ref(sha) {
        return empty(Some("invalid commit reference".to_string()));
    }

    let Some(root) = repo_root_for(session) else {
        return empty(Some("not a git repository".to_string()));
    };

    const PRETTY: &str = "--pretty=format:%H%x1f%P%x1f%an%x1f%ae%x1f%aI%x1f%s%x1f%b";
    let stdout = match git_cmd(&root, &["show", "-s", PRETTY, sha]) {
        Some(out) if out.status.success() => out.stdout,
        Some(out) => return empty(Some(git_failure(&out, "git show failed"))),
        None => return empty(Some("failed to run git".to_string())),
    };

    let text = String::from_utf8_lossy(&stdout);
    // `splitn(7, ..)` — NOT a plain `split` — so the LAST field (`%b`, the body) keeps
    // everything from its start onward verbatim, even if a multi-line body happened to
    // contain a literal `\x1f` byte (git never emits one itself, but the body is
    // free-form commit-message text, unlike the other machine fields).
    let fields: Vec<&str> = text.splitn(7, '\u{1f}').collect();
    if fields.len() < 7 {
        return empty(Some("could not parse commit metadata".to_string()));
    }
    // Field map: [0]=%H [1]=%P [2]=%an [3]=%ae [4]=%aI [5]=%s [6]=%b
    let files = commit_files(&root, sha);

    CommitDetailResult {
        sha: fields[0].trim().to_string(),
        parents: fields[1].split_whitespace().map(str::to_string).collect(),
        author: fields[2].to_string(),
        email: fields[3].to_string(),
        date: fields[4].to_string(),
        subject: fields[5].to_string(),
        body: fields[6].trim_end_matches('\n').to_string(),
        files,
        error: None,
    }
}

/// Compute a host-side COMMIT DIFF of `path` at commit `sha` vs its FIRST PARENT, answering
/// a [`super::HostCtl::GitCommitDiff`] (a commit-detail file-row click). `sha` is validated
/// via [`valid_commit_ref`] first; `path` is repo-root-relative straight off a
/// [`CommitFile`]'s path and passed directly into `git show <rev>:<path>` — git itself is
/// what resolves/rejects that spec (no filesystem read happens here at all, so no
/// `safe_join` is needed, unlike [`super::git::compute_git_diff`]'s worktree-read side).
/// - `original` = `git show <sha>^1:<path>` — empty when `sha` is a root commit (no first
///   parent) or the file didn't exist at the parent (i.e. it was ADDED in this commit);
///   either way that's a valid all-added diff, not an error.
/// - `modified` = `git show <sha>:<path>` — empty when the file was DELETED in this commit
///   (a valid all-removed diff).
///
/// Same binary/size-cap handling as [`super::git::compute_git_diff`] (reusing its
/// [`looks_binary`]/[`FILE_DIFF_SIZE_CAP`]) — only "invalid sha" / "not a git repository" is
/// reported as `error`; a missing blob on either side is silently an empty side.
pub(super) fn compute_commit_diff(
    sha: &str,
    path: &str,
    session: Option<&str>,
) -> CommitDiffResult {
    let empty = |error: Option<String>, binary: bool| CommitDiffResult {
        sha: sha.to_string(),
        path: path.to_string(),
        original: String::new(),
        modified: String::new(),
        error,
        binary,
    };

    if !valid_commit_ref(sha) {
        return empty(Some("invalid commit reference".to_string()), false);
    }

    let Some(root) = repo_root_for(session) else {
        return empty(Some("not a git repository".to_string()), false);
    };

    let show = |spec: &str| -> Vec<u8> {
        match git_cmd(&root, &["show", spec]) {
            Some(out) if out.status.success() => out.stdout,
            _ => Vec::new(),
        }
    };

    let original_bytes = show(&format!("{sha}^1:{path}"));
    let modified_bytes = show(&format!("{sha}:{path}"));

    if original_bytes.len() > FILE_DIFF_SIZE_CAP || modified_bytes.len() > FILE_DIFF_SIZE_CAP {
        return empty(Some("file too large to diff".to_string()), false);
    }
    if looks_binary(&original_bytes) || looks_binary(&modified_bytes) {
        return empty(None, true);
    }

    CommitDiffResult {
        sha: sha.to_string(),
        path: path.to_string(),
        original: String::from_utf8_lossy(&original_bytes).into_owned(),
        modified: String::from_utf8_lossy(&modified_bytes).into_owned(),
        error: None,
        binary: false,
    }
}
