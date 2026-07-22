//! Background refresh of the catalogue overlay from a GitHub release asset.
//!
//! Mirrors `app::version::spawn_check`'s concurrency pattern: the fetch runs
//! on a dedicated `std::thread`, never `tokio::spawn`, because
//! `reqwest::blocking` panics if driven from a tokio worker thread and call
//! sites in `main.rs` run before/outside the runtime. This module owns only
//! the fetch + cache-write; the in-memory swap goes through
//! `super::set_overlay`.

use std::time::Duration;

use super::model::OverlayTable;

/// Where the curated overlay table is published. Kept a plain GitHub release
/// asset (not an API endpoint) so it can be updated independently of a koma
/// release — no auth, no rate-limit budget to manage.
const OVERLAY_URL: &str = "https://github.com/aula-id/koma/releases/latest/download/models.json";

/// Skip the network entirely if the on-disk cache is fresher than this. This
/// is a curated, slow-moving table (pricing/reasoning support changes on the
/// order of weeks, not minutes) — there's no reason to hit the network on
/// every process start.
const CACHE_TTL: Duration = Duration::from_secs(6 * 3600);

/// HTTP timeout for the blocking fetch. Short — this is a best-effort
/// background refresh that must never delay anything if the network is
/// slow/down.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Spawn a NON-BLOCKING background refresh of the overlay table.
///
/// TTL-throttled against the cache file's mtime, conditional via `ETag`, and
/// entirely best-effort: any failure (throttled, network error, non-200/304
/// status, or a body that doesn't parse) leaves the current in-memory overlay
/// untouched and just logs a short line — never panics, never poisons state.
pub(super) fn spawn_refresh() {
    std::thread::spawn(|| {
        if let Err(e) = refresh() {
            crate::model::store::append_global_error_log(
                "catalogue overlay",
                &format!("fetch failed: {e}"),
            );
        }
    });
}

/// The actual refresh body, as a `Result`-returning inner fn so the spawned
/// closure has one place to log an error message and return — no `catch_unwind`
/// needed since every fallible step here is a plain `Result`, not a panic path.
fn refresh() -> anyhow::Result<()> {
    let base = crate::model::store::base_dir()?;
    let cache_path = base.join("models.json");
    let etag_path = base.join("models.json.etag");

    if is_cache_fresh(&cache_path) {
        // Common case: nothing to do, nothing to log.
        return Ok(());
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .user_agent(concat!("koma/", env!("CARGO_PKG_VERSION")))
        .build()?;

    let mut req = client.get(OVERLAY_URL);
    if let Ok(etag) = std::fs::read_to_string(&etag_path) {
        let etag = etag.trim();
        if !etag.is_empty() {
            req = req.header("If-None-Match", etag);
        }
    }

    let resp = req.send()?;
    let status = resp.status();

    if status.as_u16() == 304 {
        crate::model::store::append_global_error_log("catalogue overlay", "not modified");
        return Ok(());
    }

    if !status.is_success() {
        anyhow::bail!("unexpected status {status}");
    }

    // Capture the ETag header before consuming the response body.
    let etag_header = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let body = resp.text()?;
    let table: OverlayTable = serde_json::from_str(&body)?;

    let _ = std::fs::create_dir_all(&base);
    atomic_write(&cache_path, body.as_bytes())?;
    if let Some(etag) = etag_header {
        // Best-effort: a failed etag write just means the next refresh won't
        // send `If-None-Match` and re-fetches unconditionally. Not fatal.
        let _ = atomic_write(&etag_path, etag.as_bytes());
    }

    super::set_overlay(table);
    crate::model::store::append_global_error_log(
        "catalogue overlay",
        &format!("refreshed from {OVERLAY_URL}"),
    );
    Ok(())
}

/// `true` if `cache_path` exists and was modified less than [`CACHE_TTL`] ago.
/// Any I/O error (missing file, unsupported mtime) reads as "not fresh" so the
/// refresh proceeds and — on success — (re)creates a usable cache.
fn is_cache_fresh(cache_path: &std::path::Path) -> bool {
    let modified = match std::fs::metadata(cache_path).and_then(|m| m.modified()) {
        Ok(m) => m,
        Err(_) => return false,
    };
    match modified.elapsed() {
        Ok(age) => age < CACHE_TTL,
        Err(_) => false,
    }
}

/// PID-suffixed temp file + rename, so a crash mid-write never leaves a
/// truncated/partial file at `path` for a concurrent reader to observe.
fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or(std::path::Path::new("."));
    let file_name = path.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no file name")
    })?;
    let mut tmp_name = file_name.to_owned();
    tmp_name.push(format!(".{}$", std::process::id()));
    let tmp_path = parent.join(&tmp_name);
    std::fs::write(&tmp_path, bytes)?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}
