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
/// large to diff"` rather than shipping a multi-megabyte string into Monaco. Bumped
/// to `pub(super)` (was private) so [`super::git`]'s git-diff computation reuses the
/// SAME cap rather than duplicating the constant.
pub(super) const FILE_DIFF_SIZE_CAP: usize = 2 * 1024 * 1024;

/// Heuristic binary-content test, mirroring the harness's own sniff: a NUL byte in the
/// first 8KiB, or the bytes failing UTF-8 decode. Bumped to `pub(super)` (was private)
/// so [`super::git`]'s git-diff computation reuses this exact sniff.
pub(super) fn looks_binary(bytes: &[u8]) -> bool {
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
/// if the session can't be resolved at all (e.g. `current_session` is `None`). Bumped
/// to `pub(super)` (was private) so [`super::git`]'s git-diff computation resolves a
/// `path` the SAME way rather than duplicating the workdir-probing logic.
pub(super) fn resolve_diff_path(path: &str, current_session: Option<&str>) -> std::path::PathBuf {
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
/// Bumped to `pub(super)` (was private) so [`super::git`]'s git-status/-diff
/// computation resolves the repo root / a relative path the SAME way.
pub(super) fn session_workdirs_for(uuid: &str) -> Option<Vec<std::path::PathBuf>> {
    let dir = session_dir_for(uuid)?;
    let session = crate::model::session::Session::load(&dir).ok()?;
    Some(session.workdirs())
}

/// Resolve a session's on-disk directory (where `messages.sqlite` and its side tables
/// live) straight off the sqlite registry — no daemon connection required. `None` if
/// the session can't be found on disk at all. Bumped to `pub(super)` (was private) so
/// [`super::git`] can reuse it (currently only via [`session_workdirs_for`], but kept
/// visible for any future direct use).
pub(super) fn session_dir_for(uuid: &str) -> Option<std::path::PathBuf> {
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
    // Routed through [`super::git::git_cmd_env`] (rather than a bare `Command::new`)
    // so this spawn is serialized behind the same process-global git lock as every
    // other host git op — an unlocked spawn here could still race a concurrent
    // `checkout`/`status`/`log` and reintroduce the `.git/index.lock` stall this
    // module's callers were fixed to avoid.
    let dir = abs.parent().unwrap_or(&abs);
    let toplevel = super::git::git_cmd_env(dir, &["rev-parse", "--show-toplevel"], None);
    let repo_root = match toplevel {
        Some(out) if out.status.success() => {
            std::path::PathBuf::from(String::from_utf8_lossy(&out.stdout).trim())
        }
        _ => {
            return no_git(modified);
        }
    };
    let Ok(rel) = abs.strip_prefix(&repo_root) else {
        return no_git(modified);
    };
    let show = super::git::git_cmd_env(
        &repo_root,
        &["show", &format!("HEAD:{}", rel.to_string_lossy())],
        None,
    );
    match show {
        Some(out) if out.status.success() => {
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

/// One model row in a host-side Analytics dashboard reply.
#[derive(Debug, Clone)]
pub(super) struct AnalyticsModelRow {
    pub model_id: String,
    pub cost: f64,
    pub tokens_in: i64,
    pub tokens_cached: i64,
    pub tokens_out: i64,
    pub calls: i64,
}

/// One time-series bucket in a host-side Analytics dashboard reply.
#[derive(Debug, Clone)]
pub(super) struct AnalyticsSeriesPoint {
    pub epoch: i64,
    pub cost: f64,
    pub tokens: i64,
}

/// Host-side Analytics dashboard projection. `status` is always one of
/// `"ok"` / `"empty"` / `"error"` so the GUI can distinguish a successful zero
/// window from a genuine failure. Correlation fields (`req_seq`, `scope`,
/// `session_id`, `range`, `metric`) echo the request so React can drop a stale
/// reply across rapid filter/session changes.
#[derive(Debug, Clone)]
pub(super) struct AnalyticsResult {
    pub req_seq: u64,
    pub scope: String,
    pub session_id: Option<String>,
    pub range: String,
    pub metric: String,
    /// `"ok"` | `"empty"` | `"error"`.
    pub status: String,
    pub error: Option<String>,
    pub cost: f64,
    pub tokens_in: i64,
    pub tokens_cached: i64,
    pub tokens_out: i64,
    pub calls: i64,
    /// Cache rate = tokens_cached / (tokens_in + tokens_cached), or 0 when the
    /// denominator is 0. Defined here so the GUI and host never disagree.
    pub cache_rate: f64,
    pub series: Vec<AnalyticsSeriesPoint>,
    pub models: Vec<AnalyticsModelRow>,
    pub main_cost: f64,
    pub main_calls: i64,
    pub sub_cost: f64,
    pub sub_calls: i64,
}

/// Resolve the Analytics dashboard's range token to a LOCAL-midnight-aligned
/// `since` epoch + bucket size + expected bucket count. Anchoring on local
/// midnight (for day/week ranges) matches `compute_usage_preview`'s window-
/// consistency invariant: totals and the chart describe EXACTLY the same
/// window. `"year"` uses a rolling 365-day cutoff with daily buckets (TUI
/// parity for Year); `"30d"` is a rolling 30-day daily window.
fn analytics_window(range: &str) -> (i64, crate::model::usage::BucketSize, usize) {
    use crate::model::usage::{self, BucketSize};

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let tz = usage::local_utc_offset_secs();
    let local_now = now + tz;
    let today = local_now - local_now % 86400 - tz;

    match range {
        // Local midnight today → 24 hourly buckets.
        "today" => (today, BucketSize::Hour, 24),
        // Local midnight 6 days ago → 7 daily buckets (today inclusive).
        "7d" => (today - 6 * 86400, BucketSize::Day, 7),
        // Rolling 30 days, daily buckets (today inclusive).
        "30d" => (today - 29 * 86400, BucketSize::Day, 30),
        // Last 365 local calendar days (today inclusive), daily buckets.
        // Anchored on local midnight so series epochs match SQL day floors.
        "year" => (today - 364 * 86400, BucketSize::Day, 365),
        // Unknown token → same as 7d (safe default; never panics).
        _ => (today - 6 * 86400, BucketSize::Day, 7),
    }
}

/// Compute a host-side Analytics dashboard reply for the GUI Analytics tab,
/// answering a [`super::HostCtl::Analytics`]. Reads the global
/// `~/.koma/usage.sqlite` ledger directly — no daemon involved, works attached
/// or not (mirrors [`compute_usage_preview`]). ALWAYS returns a result so the
/// tab never hangs loading: a clean install / empty window is `status:
/// "empty"`, a genuine unexpected failure would be `status: "error"` (the
/// underlying queries are non-fatal, so this path is defensive).
///
/// Correlation inputs (`req_seq`/`scope`/`session`/`range`/`metric`) are
/// echoed back verbatim. `session` is `Some(uuid)` for a "session" scope or
/// `None` for "all". `range` is `"today"`/`"7d"`/`"30d"`/`"year"`; `metric` is
/// `"cost"`/`"tokens"` (host-side projection is identical either way — the
/// metric only drives which series field the chart scales against).
pub(super) fn compute_analytics(
    req_seq: u64,
    scope: String,
    session: Option<String>,
    range: String,
    metric: String,
) -> AnalyticsResult {
    use crate::model::usage;

    let session_ref = session.as_deref();
    let (since, bucket, n) = analytics_window(&range);
    let tz = usage::local_utc_offset_secs();

    let totals = usage::range_totals_scoped(since, session_ref);
    let buckets = usage::spend_buckets_scoped(since, bucket, n, tz, session_ref);
    let top_models = usage::top_models_in_range_scoped(since, 20, session_ref);
    let roles = usage::role_split_scoped(since, session_ref);

    let denom = (totals.tokens_in + totals.tokens_cached) as f64;
    let cache_rate = if denom > 0.0 {
        (totals.tokens_cached as f64) / denom
    } else {
        0.0
    };

    // Zero-fill the series to a contiguous window so the chart never has to
    // invent missing buckets. Hourly for "today", daily otherwise.
    let secs = bucket.secs();
    let bucket_map: std::collections::HashMap<i64, (f64, i64)> = buckets
        .into_iter()
        .map(|b| (b.bucket_epoch, (b.cost, b.tokens)))
        .collect();

    // Anchor the first bucket on the SAME floor the SQL uses for Day/Hour.
    let start = match bucket {
        crate::model::usage::BucketSize::Hour => {
            // Floor `since` itself to the hour in local time, then convert back.
            let local = since + tz;
            local - local % 3600 - tz
        }
        crate::model::usage::BucketSize::Day | crate::model::usage::BucketSize::Week => since,
    };
    let series: Vec<AnalyticsSeriesPoint> = (0..n as i64)
        .map(|i| {
            let epoch = start + i * secs;
            let (cost, tokens) = bucket_map.get(&epoch).copied().unwrap_or((0.0, 0));
            AnalyticsSeriesPoint {
                epoch,
                cost,
                tokens,
            }
        })
        .collect();

    let models: Vec<AnalyticsModelRow> = top_models
        .into_iter()
        .map(|m| AnalyticsModelRow {
            model_id: m.model_id,
            cost: m.total_cost,
            tokens_in: m.tokens_in,
            tokens_cached: m.tokens_cached,
            tokens_out: m.tokens_out,
            calls: m.call_count,
        })
        .collect();

    // Empty = zero calls in the window (a successful zero result, NOT an error).
    // The underlying queries never fail loudly, so status is never "error" in
    // practice; the field is still present so the contract can grow.
    let status = if totals.calls == 0 {
        "empty".to_string()
    } else {
        "ok".to_string()
    };

    AnalyticsResult {
        req_seq,
        scope,
        session_id: session,
        range,
        metric,
        status,
        error: None,
        cost: totals.cost,
        tokens_in: totals.tokens_in,
        tokens_cached: totals.tokens_cached,
        tokens_out: totals.tokens_out,
        calls: totals.calls,
        cache_rate,
        series,
        models,
        main_cost: roles.main_cost,
        main_calls: roles.main_calls,
        sub_cost: roles.sub_cost,
        sub_calls: roles.sub_calls,
    }
}
