//! Host-side FILE DIFF + USAGE PREVIEW computation for the GUI Explore panel /
//! Usage panel — both work off the daemon (direct fs/git/sqlite access) so
//! they answer identically whether a session is attached or not. Split out of
//! [`super`] (the `client` module) for file size — pure code motion, no
//! behaviour change.
//!
//! `compute_file_diff` and `compute_usage_preview` are bumped to `pub(super)`
//! (were private) since [`super::host`]'s `host_swapper` (a sibling module)
//! calls them; every other item here is only used within this file.

/// The result of a host-side [`compute_file_diff`], pushed to the GUI as a `FileDiff`
/// envelope (`render::push_file_diff`). `error` set means the diff could not be
/// computed at all (both strings then empty); `binary` set means either side isn't
/// valid UTF-8 text (both strings then empty, no `error`).
pub(super) struct FileDiffResult {
    pub path: String,
    pub original: String,
    pub modified: String,
    pub error: Option<String>,
    pub binary: bool,
    /// Where the ORIGINAL side came from: `"git"` (`git show HEAD:`) or `"baseline"`
    /// (the session's "virtual git" first-touch pre-image, used when the file isn't
    /// in a git repository). The GUI shows a dim badge for the baseline case.
    /// NOT meaningful on `binary: true` / `error` replies — the binary short-circuit
    /// fires before any diff source is probed, so those carry the `"git"` default.
    pub origin: &'static str,
}

/// Cap on either side of a diff (~2MiB) — past this we bail with `error: "file too
/// large to diff"` rather than shipping a multi-megabyte string into Monaco.
const FILE_DIFF_SIZE_CAP: usize = 2 * 1024 * 1024;

/// Heuristic binary-content test, mirroring the harness's own sniff: a NUL byte in the
/// first 8KiB, or the bytes failing UTF-8 decode.
fn looks_binary(bytes: &[u8]) -> bool {
    let probe = &bytes[..bytes.len().min(8192)];
    probe.contains(&0) || std::str::from_utf8(bytes).is_err()
}

/// Resolve a `fileChanges` record's path to an absolute path.
///
/// `tool::fs::record_change` stores the SHORTEST workspace-relative rendering across
/// every configured workspace root when the file is under one, falling back to the
/// absolute path otherwise — so `path` here is EITHER already absolute (used as-is) OR
/// relative to one of the session's configured workdirs (ambiguous which one, since
/// the dedup key doesn't carry that). For the relative case this reads the session's
/// on-disk `settings.json` (via the sqlite registry, then `Session::load` +
/// `Session::workdirs()` — no daemon involved) and tries each configured root in
/// order, picking the first whose join either exists on disk or whose parent
/// directory exists (a plausible location for a file that was since deleted);
/// falling back to the first (primary) root if none match, or the bare relative path
/// if the session can't be resolved at all (e.g. `current_session` is `None`).
fn resolve_diff_path(path: &str, current_session: Option<&str>) -> std::path::PathBuf {
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    let roots = current_session
        .and_then(session_workdirs_for)
        .unwrap_or_default();
    for root in &roots {
        let candidate = root.join(p);
        if candidate.exists() || candidate.parent().is_some_and(|d| d.exists()) {
            return candidate;
        }
    }
    match roots.first() {
        Some(root) => root.join(p),
        None => p.to_path_buf(),
    }
}

/// Look up a session's configured workdir roots straight off disk (sqlite registry for
/// the `pwd_hash` bucket, then that session's `settings.json` for the actual list) —
/// no daemon connection required. `None` if the session can't be found on disk at all.
fn session_workdirs_for(uuid: &str) -> Option<Vec<std::path::PathBuf>> {
    let dir = session_dir_for(uuid)?;
    let session = crate::model::session::Session::load(&dir).ok()?;
    Some(session.workdirs())
}

/// Resolve a session's on-disk directory (where `messages.sqlite` and its side tables
/// live) straight off the sqlite registry — no daemon connection required. `None` if
/// the session can't be found on disk at all.
fn session_dir_for(uuid: &str) -> Option<std::path::PathBuf> {
    let row = crate::model::session_registry::get(uuid).ok().flatten()?;
    crate::model::store::session_dir(&row.pwd_hash, uuid).ok()
}

/// "Virtual git" fallback for [`compute_file_diff`] when `path` isn't inside a git
/// repository: diff the session's first-touch BASELINE pre-image (captured by the
/// `write`/`edit`/`delete` tools into the per-session `messages.sqlite`
/// `file_baselines` table) against the current on-disk contents. `path` is the
/// `fileChanges` record's key, which is byte-identical to the key the baseline was
/// stored under (both come from `tool::fs::display_key`), so the lookup is direct.
/// `None` when the session is unknown or no baseline row exists — the caller then
/// reports that neither diff source is available.
fn baseline_diff(path: &str, current_session: Option<&str>, modified: String) -> Option<FileDiffResult> {
    let session_dir = current_session.and_then(session_dir_for)?;
    let baseline = crate::model::msglog::read_file_baseline(&session_dir, path)?;
    let empty = |error: Option<String>, binary: bool| FileDiffResult {
        path: path.to_string(),
        original: String::new(),
        modified: String::new(),
        error,
        binary,
        origin: "baseline",
    };
    Some(match baseline.kind.as_str() {
        // Created by koma this session — a valid all-added diff against nothing.
        "empty" => FileDiffResult {
            path: path.to_string(),
            original: String::new(),
            modified,
            error: None,
            binary: false,
            origin: "baseline",
        },
        "binary" => empty(None, true),
        "toolarge" => empty(Some("file too large to diff".to_string()), false),
        // "text" (and any unknown kind with content, defensively).
        _ => match baseline.content {
            Some(bytes) => FileDiffResult {
                path: path.to_string(),
                original: String::from_utf8_lossy(&bytes).into_owned(),
                modified,
                error: None,
                binary: false,
                origin: "baseline",
            },
            None => empty(Some("baseline unavailable for this file".to_string()), false),
        },
    })
}

/// Compute a host-side FILE DIFF for `path` (a `fileChanges` record's path), answering
/// a [`super::HostCtl::FileDiff`]. Runs entirely off the daemon: resolves `path` to an
/// absolute location ([`resolve_diff_path`]), reads its CURRENT on-disk contents, and
/// shells out to `git show HEAD:<repo-relative-path>` (cwd = the file's own repo,
/// discovered via `git rev-parse --show-toplevel`) for the ORIGINAL side. ALWAYS
/// returns a result — every failure path sets `error` (or `binary`) rather than
/// panicking or dropping the request — mirroring the ListModels/ListRoutes
/// always-reply rule so the GUI diff tab can never hang waiting on a spinner.
///
/// - A read failure on the current file (deleted, or otherwise unreadable) is NOT an
///   error: `modified` is just empty (a valid diff — all-removed).
/// - No git repository at `path`'s location → falls back to the session's "virtual
///   git" BASELINE ([`baseline_diff`]: the first-touch pre-image the fs tools capture
///   into `messages.sqlite`), `origin: "baseline"`; only when THAT also misses does it
///   report `error: "no git repository and no session baseline for this file"`.
/// - `path` untracked / not present at `HEAD` (`git show` exits non-zero) → `original`
///   empty (a valid diff — all-added), no `error`.
/// - Either side failing UTF-8 or containing a NUL in its first 8KiB → `binary: true`,
///   both strings empty.
/// - Either side exceeding [`FILE_DIFF_SIZE_CAP`] → `error: "file too large to diff"`,
///   both strings empty.
pub(super) fn compute_file_diff(path: &str, current_session: Option<&str>) -> FileDiffResult {
    let empty = |error: Option<String>, binary: bool| FileDiffResult {
        path: path.to_string(),
        original: String::new(),
        modified: String::new(),
        error,
        binary,
        origin: "git",
    };
    // Not in a git repo → try the session's "virtual git" baseline before giving up.
    let no_git = |modified: String| {
        baseline_diff(path, current_session, modified.clone()).unwrap_or(FileDiffResult {
            path: path.to_string(),
            original: String::new(),
            modified,
            error: Some("no git repository and no session baseline for this file".to_string()),
            binary: false,
            origin: "git",
        })
    };

    let abs = resolve_diff_path(path, current_session);

    // --- modified: current on-disk contents ---
    let modified_bytes = match std::fs::read(&abs) {
        // Deleted (or otherwise unreadable) — treat as an empty modified side, not an
        // error: the diff still renders (all-removed vs. HEAD).
        Err(_) => Vec::new(),
        Ok(bytes) if bytes.len() > FILE_DIFF_SIZE_CAP => {
            return empty(Some("file too large to diff".to_string()), false);
        }
        Ok(bytes) => bytes,
    };
    if looks_binary(&modified_bytes) {
        return empty(None, true);
    }
    let modified = String::from_utf8_lossy(&modified_bytes).into_owned();

    // --- original: `git show HEAD:<repo-relative-path>`, cwd = the file's own repo ---
    let dir = abs.parent().unwrap_or(&abs);
    let toplevel = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(dir)
        .output();
    let repo_root = match toplevel {
        Ok(out) if out.status.success() => {
            std::path::PathBuf::from(String::from_utf8_lossy(&out.stdout).trim())
        }
        _ => {
            return no_git(modified);
        }
    };
    let Ok(rel) = abs.strip_prefix(&repo_root) else {
        return no_git(modified);
    };
    let show = std::process::Command::new("git")
        .args(["show", &format!("HEAD:{}", rel.to_string_lossy())])
        .current_dir(&repo_root)
        .output();
    match show {
        Ok(out) if out.status.success() => {
            if out.stdout.len() > FILE_DIFF_SIZE_CAP {
                return empty(Some("file too large to diff".to_string()), false);
            }
            if looks_binary(&out.stdout) {
                return empty(None, true);
            }
            FileDiffResult {
                path: path.to_string(),
                original: String::from_utf8_lossy(&out.stdout).into_owned(),
                modified,
                error: None,
                binary: false,
                origin: "git",
            }
        }
        // Non-zero exit: `path` is untracked / didn't exist at HEAD — a valid diff
        // (all-added), not an error.
        _ => FileDiffResult {
            path: path.to_string(),
            original: String::new(),
            modified,
            error: None,
            binary: false,
            origin: "git",
        },
    }
}

/// The result of a host-side [`compute_usage_preview`], pushed to the GUI as a
/// `UsagePreview` envelope (`render::push_usage_preview`). `days` is EXACTLY 7 entries
/// (oldest first, today last), zero-filled for any day with no ledger rows; `top_models`
/// is capped at 3, ordered by cost descending.
pub(super) struct UsagePreviewResult {
    pub cost: f64,
    pub tokens_in: i64,
    pub tokens_cached: i64,
    pub tokens_out: i64,
    pub calls: i64,
    pub days: Vec<(i64, f64)>,
    pub top_models: Vec<crate::model::usage::ModelCostRange>,
}

/// Compute a host-side LAST-7-DAYS usage preview for the GUI Usage panel, answering a
/// [`super::HostCtl::UsagePreview`]. Reads the global `~/.koma/usage.sqlite` ledger directly —
/// no daemon involved, works attached or not (mirrors [`compute_file_diff`]). Every
/// underlying query (`range_totals_scoped`/`spend_buckets_scoped`/
/// `top_models_in_range_scoped`) is already non-fatal (zeroed/empty on a missing or
/// locked DB), so this never fails either — the caller ALWAYS gets a result to push,
/// even on a clean install with no ledger yet.
///
/// `session` is `Some(uuid)` for the Usage panel's "session" scope toggle — every query
/// is then filtered to that session's rows ONLY — or `None` for the default "all"
/// (global) scope. Either way the query cutoff is the SAME floored anchor the 7-bar
/// chart is built from (today's LOCAL midnight minus 6 days) — not a bare
/// `now - 7*86400` — so the header totals and the top-models list describe EXACTLY the
/// window the bars render, with no up-to-24h sliver of extra data hiding outside every
/// bar. This window-consistency invariant holds in BOTH scopes.
pub(super) fn compute_usage_preview(session: Option<&str>) -> UsagePreviewResult {
    use crate::model::usage::{self, BucketSize};

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let tz = usage::local_utc_offset_secs();

    // Floor to LOCAL midnight first — same day-boundary math as
    // `view::usage::heatmap`'s week chart — then anchor the query window on it, so
    // `since` is the exact start of the oldest bar rather than a rolling 168h cutoff.
    let local_now = now + tz;
    let today = local_now - local_now % 86400 - tz;
    let since = today - 6 * 86400;

    let totals = usage::range_totals_scoped(since, session);
    let buckets = usage::spend_buckets_scoped(since, BucketSize::Day, 0, tz, session);
    let top_models = usage::top_models_in_range_scoped(since, 3, session);

    // Normalize to exactly 7 daily buckets (oldest -> newest, today last), zero-filled
    // for any day the ledger has no rows for.
    let bucket_map: std::collections::HashMap<i64, f64> =
        buckets.into_iter().map(|b| (b.bucket_epoch, b.cost)).collect();
    let days = (0..7)
        .map(|i| {
            let epoch = today - (6 - i) * 86400;
            (epoch, bucket_map.get(&epoch).copied().unwrap_or(0.0))
        })
        .collect();

    UsagePreviewResult {
        cost: totals.cost,
        tokens_in: totals.tokens_in,
        tokens_cached: totals.tokens_cached,
        tokens_out: totals.tokens_out,
        calls: totals.calls,
        days,
        top_models,
    }
}
