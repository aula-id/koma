//! Internet tools: `web_fetch` (page reader) and `web_search` (DDG search).
//!
//! Both are read-only and auto-run without approval.
//!
//! ## Concurrency pattern
//!
//! `Tool::run` is synchronous but called from a tokio runtime thread. Using
//! `reqwest::blocking` directly in that context panics ("Cannot start a runtime
//! from within a runtime"). Every blocking HTTP call therefore happens inside a
//! freshly spawned `std::thread` (which has no tokio context), and the result is
//! returned via an `mpsc::channel` with `recv_timeout` — exactly the same pattern
//! used by `shell::Bash`.

mod web_fetch;
mod web_search;
mod web_download;

pub use web_fetch::WebFetch;
pub use web_search::WebSearch;
pub use web_download::WebDownload;

use super::{Tool, ToolCtx};
use std::sync::mpsc;
use std::time::Duration;

/// Shared HTTP GET helper.  Spawns a dedicated OS thread (no tokio context),
/// builds a `reqwest::blocking::Client` there, performs the GET, and returns
/// `(status_code, body)`.  The body is capped at 5 MiB before returning.
///
/// Errors are returned as `Err(String)` so the caller can produce a readable
/// tool-output message without panicking.
pub(super) fn http_get_blocking(
    url: &str,
    timeout: Duration,
) -> Result<(u16, String), String> {
    const MAX_BODY_BYTES: usize = 5 * 1024 * 1024; // 5 MiB

    let url_owned = url.to_string();
    let (tx, rx) = mpsc::channel::<Result<(u16, String), String>>();

    std::thread::spawn(move || {
        let result = (|| -> Result<(u16, String), String> {
            let client = reqwest::blocking::Client::builder()
                .timeout(timeout)
                .user_agent(
                    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
                     (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
                )
                .default_headers({
                    let mut headers = reqwest::header::HeaderMap::new();
                    headers.insert(
                        reqwest::header::ACCEPT,
                        "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
                            .parse()
                            .map_err(|e| format!("header parse error: {e}"))?,
                    );
                    headers.insert(
                        reqwest::header::ACCEPT_LANGUAGE,
                        "en-US,en;q=0.9"
                            .parse()
                            .map_err(|e| format!("header parse error: {e}"))?,
                    );
                    headers
                })
                .build()
                .map_err(|e| format!("client build error: {e}"))?;

            let resp = client
                .get(&url_owned)
                .send()
                .map_err(|e| format!("request failed: {e}"))?;

            let status = resp.status().as_u16();

            // Read body, capping at MAX_BODY_BYTES.
            let bytes = resp
                .bytes()
                .map_err(|e| format!("failed to read body: {e}"))?;

            let capped = if bytes.len() > MAX_BODY_BYTES {
                &bytes[..MAX_BODY_BYTES]
            } else {
                &bytes[..]
            };

            let body = String::from_utf8_lossy(capped).into_owned();
            Ok((status, body))
        })();

        let _ = tx.send(result);
    });

    // Outer timeout guard — slightly longer than the inner client timeout so the
    // thread always wins the race under normal network conditions.
    let outer_timeout = timeout + Duration::from_secs(5);
    match rx.recv_timeout(outer_timeout) {
        Ok(r) => r,
        Err(_) => Err(format!("timed out after {}s", timeout.as_secs())),
    }
}

/// Heuristic: did the server serve a Cloudflare challenge page?
pub(super) fn looks_like_cloudflare(status: u16, body: &str) -> bool {
    matches!(status, 403 | 503)
        && (body.contains("Just a moment")
            || body.contains("cf-chl")
            || body.contains("challenge-platform")
            || body.contains("Cloudflare"))
}
