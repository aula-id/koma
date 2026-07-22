//! `web_search` tool: query DuckDuckGo and return titles, URLs, and snippets.
//!
//! Uses the DDG HTML endpoint (`html.duckduckgo.com/html/`) and parses results
//! with the `scraper` crate.  Page-1 only.
//! TODO: vqd paging for additional result pages.
//!
//! ## Rate-limit hardening
//!
//! DDG returns HTTP 202 when it detects bot-like requests.  This module works
//! around that by:
//!   - Rotating across a pool of Chrome desktop User-Agent strings.
//!   - Sending a real browser POST (form body) instead of a GET with query params.
//!   - Adding browser-like request headers including Chrome client hints
//!     (sec-ch-ua, sec-ch-ua-mobile, sec-ch-ua-platform) paired to each UA's OS.
//!   - Retrying up to 3 times with a different UA after any 202 or zero-result
//!     response, with short jittered backoff sleeps (safe here because `run` is
//!     called from the off-UI async-defer thread).

use super::{Tool, ToolCtx};
use anyhow::Result;
use scraper::{Html, Selector};
use serde_json::{json, Value};
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_REGION: &str = "wt-wt";
const TOP_N: usize = 8;

/// Chrome desktop User-Agent pool with paired platform tokens.
///
/// Client hint headers (sec-ch-ua-platform, sec-ch-ua) are Chrome-only, so
/// every entry here is a Chrome UA.  Each entry is (ua, platform) where
/// platform matches the `sec-ch-ua-platform` value.
const UA_POOL: &[(&str, &str)] = &[
    // Chrome 125 — Windows
    (
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
         (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
        "Windows",
    ),
    // Chrome 125 — macOS
    (
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
         (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
        "macOS",
    ),
    // Chrome 125 — Linux
    (
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
         (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
        "Linux",
    ),
    // Chrome 124 — Windows
    (
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
         (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
        "Windows",
    ),
    // Chrome 124 — macOS
    (
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
         (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
        "macOS",
    ),
    // Chrome 124 — Linux
    (
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
         (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
        "Linux",
    ),
    // Chrome 123 — Windows
    (
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
         (KHTML, like Gecko) Chrome/123.0.0.0 Safari/537.36",
        "Windows",
    ),
    // Chrome 123 — macOS
    (
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
         (KHTML, like Gecko) Chrome/123.0.0.0 Safari/537.36",
        "macOS",
    ),
];

/// Derive a pseudo-random `u64` seed from `SystemTime` nanoseconds.
/// No `rand` crate needed — the nanos component varies per call.
fn nanos_seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 ^ (d.as_secs().wrapping_mul(0x9e37_79b9_7f4a_7c15)))
        .unwrap_or(42)
}

/// Pick a (ua, platform) pair from the pool using the given seed, offset by
/// `skip` so that consecutive retry attempts always pick a *different* UA.
fn pick_ua(seed: u64, skip: usize) -> (&'static str, &'static str) {
    let idx = ((seed ^ (skip as u64).wrapping_mul(0x517c_c1b7_2722_0a95)) % UA_POOL.len() as u64)
        as usize;
    UA_POOL[idx]
}

/// DDG-specific blocking POST (form body).
///
/// Spawns a dedicated OS thread (no tokio context), builds a
/// `reqwest::blocking::Client` with browser-like headers there, performs the
/// POST, and returns `(status_code, body)` over an mpsc channel.
/// Mirrors the `http_get_blocking` pattern exactly — std::thread + recv_timeout.
///
/// The URL is always the clean `https://html.duckduckgo.com/html/` with NO
/// query string.  Form params are sent in the request body with
/// `Content-Type: application/x-www-form-urlencoded` (set automatically by
/// reqwest's `.form()`).
///
/// `ua`: the User-Agent string to use for this attempt.
/// `platform`: the `sec-ch-ua-platform` value matching the UA's OS.
/// `form_params`: slice of (name, value) pairs for the POST body.
fn ddg_post(
    ua: &str,
    platform: &str,
    form_params: &[(&str, &str)],
    timeout: Duration,
) -> Result<(u16, String), String> {
    const DDG_HTML_URL: &str = "https://html.duckduckgo.com/html/";
    const MAX_BODY_BYTES: usize = 5 * 1024 * 1024; // 5 MiB

    let ua_owned = ua.to_string();
    let platform_owned = platform.to_string();
    // Own the form params so they can cross the thread boundary.
    let params_owned: Vec<(String, String)> = form_params
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    let (tx, rx) = mpsc::channel::<Result<(u16, String), String>>();

    std::thread::spawn(move || {
        let result = (|| -> Result<(u16, String), String> {
            // Build per-request headers.
            let mut default_headers = reqwest::header::HeaderMap::new();

            default_headers.insert(
                reqwest::header::ACCEPT,
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
                    .parse()
                    .map_err(|e| format!("header parse: {e}"))?,
            );
            default_headers.insert(
                reqwest::header::ACCEPT_LANGUAGE,
                "en-US,en;q=0.9"
                    .parse()
                    .map_err(|e| format!("header parse: {e}"))?,
            );
            // Referer and Origin both use the html. subdomain, matching the
            // real browser capture.
            default_headers.insert(
                reqwest::header::REFERER,
                "https://html.duckduckgo.com/"
                    .parse()
                    .map_err(|e| format!("header parse: {e}"))?,
            );
            default_headers.insert(
                reqwest::header::ORIGIN,
                "https://html.duckduckgo.com"
                    .parse()
                    .map_err(|e| format!("header parse: {e}"))?,
            );
            default_headers.insert(
                "Sec-Fetch-Site"
                    .parse::<reqwest::header::HeaderName>()
                    .map_err(|e| format!("header name: {e}"))?,
                "same-origin"
                    .parse()
                    .map_err(|e| format!("header parse: {e}"))?,
            );
            default_headers.insert(
                "Sec-Fetch-Mode"
                    .parse::<reqwest::header::HeaderName>()
                    .map_err(|e| format!("header name: {e}"))?,
                "navigate"
                    .parse()
                    .map_err(|e| format!("header parse: {e}"))?,
            );
            default_headers.insert(
                "Sec-Fetch-Dest"
                    .parse::<reqwest::header::HeaderName>()
                    .map_err(|e| format!("header name: {e}"))?,
                "document"
                    .parse()
                    .map_err(|e| format!("header parse: {e}"))?,
            );
            default_headers.insert(
                "Sec-Fetch-User"
                    .parse::<reqwest::header::HeaderName>()
                    .map_err(|e| format!("header name: {e}"))?,
                "?1".parse().map_err(|e| format!("header parse: {e}"))?,
            );
            default_headers.insert(
                "Upgrade-Insecure-Requests"
                    .parse::<reqwest::header::HeaderName>()
                    .map_err(|e| format!("header name: {e}"))?,
                "1".parse().map_err(|e| format!("header parse: {e}"))?,
            );
            // Chrome client hints — must be consistent with the Chrome UA.
            default_headers.insert(
                "sec-ch-ua"
                    .parse::<reqwest::header::HeaderName>()
                    .map_err(|e| format!("header name: {e}"))?,
                "\"Google Chrome\";v=\"125\", \"Chromium\";v=\"125\", \"Not.A/Brand\";v=\"24\""
                    .parse()
                    .map_err(|e| format!("header parse: {e}"))?,
            );
            default_headers.insert(
                "sec-ch-ua-mobile"
                    .parse::<reqwest::header::HeaderName>()
                    .map_err(|e| format!("header name: {e}"))?,
                "?0".parse().map_err(|e| format!("header parse: {e}"))?,
            );
            let platform_header_val = format!("\"{}\"", platform_owned);
            default_headers.insert(
                "sec-ch-ua-platform"
                    .parse::<reqwest::header::HeaderName>()
                    .map_err(|e| format!("header name: {e}"))?,
                platform_header_val
                    .parse()
                    .map_err(|e| format!("header parse: {e}"))?,
            );

            let client = reqwest::blocking::Client::builder()
                .timeout(timeout)
                .user_agent(ua_owned.clone())
                .default_headers(default_headers)
                .build()
                .map_err(|e| format!("client build error: {e}"))?;

            // POST with form body — reqwest sets Content-Type automatically.
            // The URL has no query string; all params go in the body.
            let params_refs: Vec<(&str, &str)> = params_owned
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();

            let resp = client
                .post(DDG_HTML_URL)
                .form(&params_refs)
                .send()
                .map_err(|e| format!("request failed: {e}"))?;

            let status = resp.status().as_u16();

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

    // Outer timeout guard — slightly longer than the inner client timeout so
    // the thread always wins the race under normal network conditions.
    let outer_timeout = timeout + Duration::from_secs(5);
    match rx.recv_timeout(outer_timeout) {
        Ok(r) => r,
        Err(_) => Err(format!("timed out after {}s", timeout.as_secs())),
    }
}

/// Returns `true` when the response looks like a DDG rate-limit / empty page.
fn looks_rate_limited(status: u16, body: &str, result_count: usize) -> bool {
    if status == 202 {
        return true;
    }
    // DDG sometimes returns 200 with a tiny body containing only tracking
    // pixels — treat suspiciously short bodies with zero results as rate-limited.
    if result_count == 0 && body.len() < 2000 {
        return true;
    }
    false
}

// ---------------------------------------------------------------------------
// Backoff delays (ms) for attempt index 0-based.
// attempt 0 -> no sleep (first try)
// attempt 1 -> ~1200ms base + small jitter
// attempt 2 -> ~2500ms base + small jitter
// ---------------------------------------------------------------------------
const BACKOFF_MS: &[u64] = &[0, 1200, 2500];

/// Search DuckDuckGo and return result titles, URLs, and snippets.
pub struct WebSearch;

impl Tool for WebSearch {
    fn name(&self) -> &'static str {
        "web_search"
    }

    fn description(&self) -> &'static str {
        "Search the web (DuckDuckGo) for a query and return result titles, URLs, and snippets. \
        Use to discover pages, then web_fetch the most relevant URL."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query."
                },
                "region": {
                    "type": "string",
                    "description": "DDG region code (kl parameter, e.g. 'us-en', 'de-de'). Defaults to 'wt-wt' (no region)."
                }
            },
            "required": ["query"]
        })
    }

    fn run(&self, _ctx: &ToolCtx, args: &Value) -> Result<String> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing required string argument 'query'"))?;

        let region = args
            .get("region")
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_REGION);

        let per_request_timeout = Duration::from_secs(12);

        const MAX_ATTEMPTS: usize = 3;

        // Seed once per search invocation; each attempt offsets the UA index.
        let base_seed = nanos_seed();

        for attempt in 0..MAX_ATTEMPTS {
            // Sleep before retry attempts (not on first try).
            if attempt > 0 {
                let base_ms = BACKOFF_MS[attempt.min(BACKOFF_MS.len() - 1)];
                // Jitter: add 0-199ms derived from a fresh nanos sample.
                let jitter_ms = nanos_seed() % 200;
                std::thread::sleep(Duration::from_millis(base_ms + jitter_ms));
            }

            // Pick a (UA, platform) pair that differs across attempts.
            let (ua, platform) = pick_ua(base_seed, attempt);

            // Build the POST form body.
            // `b` is the page offset (empty string = first page).
            // `kl` is the region; include only when non-empty.
            let mut form_params: Vec<(&str, &str)> = vec![("q", query), ("b", "")];
            if !region.is_empty() {
                form_params.push(("kl", region));
            }

            let (status, body) = match ddg_post(ua, platform, &form_params, per_request_timeout) {
                Ok(v) => v,
                Err(e) => {
                    // Network-level error; no point retrying further.
                    return Ok(format!("error: {e}"));
                }
            };

            if !(200..300).contains(&status) && status != 202 {
                return Ok(format!("error: HTTP {status} from DuckDuckGo"));
            }

            let results = parse_ddg_results(&body);

            if looks_rate_limited(status, &body, results.len()) {
                // Will retry unless this was the last attempt.
                continue;
            }

            // We have results — format and return immediately.
            if !results.is_empty() {
                let mut out = String::new();
                for (i, r) in results.iter().enumerate() {
                    out.push_str(&format!(
                        "{}. {}\n   {}\n   {}\n",
                        i + 1,
                        r.title,
                        r.url,
                        r.snippet
                    ));
                }
                return Ok(out.trim_end().to_string());
            }

            // 200 OK but zero results — not a rate-limit, just empty.
            return Ok(format!(
                "web_search: no results found for query: {query}\n\
                (DuckDuckGo returned an empty page)"
            ));
        }

        // All attempts were rate-limited.
        Ok(format!(
            "web_search: DuckDuckGo rate-limited after {MAX_ATTEMPTS} attempts, \
             try again shortly or use web_fetch on a known URL."
        ))
    }
}

struct SearchResult {
    title: String,
    url: String,
    snippet: String,
}

/// Parse DuckDuckGo HTML results page into structured results.
///
/// DDG result structure (HTML endpoint):
///   - Result anchors: `a.result__a`  (title text + href with DDG redirect)
///   - Snippets: `.result__snippet`
///
/// DDG wraps real URLs in a redirect:
///   `//duckduckgo.com/l/?uddg=<encoded>&rut=...`
/// We decode the `uddg` param to recover the real URL.
fn parse_ddg_results(html: &str) -> Vec<SearchResult> {
    let document = Html::parse_document(html);

    let title_sel = match Selector::parse("a.result__a") {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    let snippet_sel = match Selector::parse(".result__snippet") {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    let titles: Vec<(String, String)> = document
        .select(&title_sel)
        .map(|el| {
            let title = el.text().collect::<String>().trim().to_string();
            let raw_href = el.value().attr("href").unwrap_or("").to_string();
            let real_url = decode_ddg_redirect(&raw_href);
            (title, real_url)
        })
        .collect();

    let snippets: Vec<String> = document
        .select(&snippet_sel)
        .map(|el| el.text().collect::<String>().trim().to_string())
        .collect();

    titles
        .into_iter()
        .zip(snippets.into_iter().chain(std::iter::repeat(String::new())))
        .take(TOP_N)
        .filter(|((title, _url), _snippet)| !title.is_empty())
        .map(|((title, url), snippet)| SearchResult {
            title,
            url,
            snippet,
        })
        .collect()
}

/// Decode a DDG redirect URL to the real target URL.
///
/// DDG format: `//duckduckgo.com/l/?uddg=<percent-encoded-url>&rut=...`
/// or already a direct URL (`https://...`).
fn decode_ddg_redirect(href: &str) -> String {
    // Already an absolute URL — return as-is.
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_string();
    }

    // Normalise `//duckduckgo.com/l/...` to a parseable form.
    let normalised = if href.starts_with("//") {
        format!("https:{href}")
    } else if href.starts_with('/') {
        format!("https://duckduckgo.com{href}")
    } else {
        href.to_string()
    };

    // Extract the `uddg` query parameter.
    if let Ok(parsed) = url::Url::parse(&normalised) {
        for (key, value) in parsed.query_pairs() {
            if key == "uddg" {
                return value.into_owned();
            }
        }
    }

    // Fallback: return the normalised URL.
    normalised
}
