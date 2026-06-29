//! Self-update awareness: know the compiled-in koma version and, on every new
//! session spawn, fire a NON-BLOCKING check against the public version endpoint.
//!
//! [`spawn_check`] mirrors `tool::internet::http_get_blocking`'s concurrency
//! pattern: `reqwest::blocking` panics if run on a tokio runtime thread, so the
//! whole fetch happens inside a freshly spawned `std::thread` (which has no tokio
//! context). The result — a parsed [`VersionInfo`] — is handed back over a
//! `tokio::sync::mpsc` sender, drained in the event-loop tick and stashed in
//! `AppStateRest::latest_version` for the UI to read.
//!
//! GRACEFUL DEGRADATION: if the endpoint is unreachable, returns a non-success
//! status, or sends a body that doesn't parse, the check does NOTHING — no error,
//! no toast, no panic. The app simply never receives an update and shows only the
//! current version. The entire fetch body is a closure returning `Result`; an
//! `Err` at any step is ignored.

use std::time::Duration;

/// The public version manifest served at `https://koma.run/api/v1/version`.
///
/// `version` is the latest published koma version; `message` is an optional
/// human-readable note (release blurb) the UI may surface. Only the `version`
/// field is required to parse — a manifest without `message` still deserializes.
// Fields are populated by serde from the fetched manifest; they are READ by the
// version/update UI (next stage), so they are not yet referenced in Rust.
#[allow(dead_code)]
#[derive(Debug, Clone, serde::Deserialize)]
pub struct VersionInfo {
    pub version: String,
    pub message: Option<String>,
}

/// The version endpoint hit on every session spawn.
const VERSION_URL: &str = "https://koma.run/api/v1/version";

/// HTTP timeout for the blocking version fetch. Short — this is a best-effort
/// background poll that must never delay anything if the network is slow/down.
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

/// Semver-ish "is `latest` strictly newer than `current`?" comparison.
///
/// Splits each string on `'.'`, parses each component as `u64` (defaulting to 0
/// on a non-numeric component — so a pre-release suffix like `1.2.0-rc1` reads as
/// `1.2.0`), and compares component-wise. Tolerates a differing number of
/// components (the shorter side is zero-padded) and a single leading `'v'`
/// (`v1.2.3` == `1.2.3`). Returns `true` only when `latest` orders strictly above
/// `current`; equal or older yields `false`.
///
/// Used by the UI (next stage) to decide whether to advertise an update.
#[allow(dead_code)]
pub fn is_newer(latest: &str, current: &str) -> bool {
    fn parts(s: &str) -> Vec<u64> {
        s.trim()
            .strip_prefix('v')
            .unwrap_or(s.trim())
            .split('.')
            .map(|c| c.trim().parse::<u64>().unwrap_or(0))
            .collect()
    }

    let a = parts(latest);
    let b = parts(current);
    let len = a.len().max(b.len());
    for i in 0..len {
        // Zero-pad the shorter side so `1.2` vs `1.2.0` compares equal.
        let l = a.get(i).copied().unwrap_or(0);
        let c = b.get(i).copied().unwrap_or(0);
        if l != c {
            return l > c;
        }
    }
    false
}

/// Spawn a NON-BLOCKING background check for a newer koma version.
///
/// Runs the blocking HTTP GET on a dedicated OS thread (no tokio context — same
/// reason as `http_get_blocking`), parses the JSON manifest, and on FULL success
/// sends the [`VersionInfo`] back on `tx`. Any failure (client build, network,
/// non-success status, body read, or JSON parse) is swallowed silently: the whole
/// body is a closure returning `Result`, and an `Err` simply returns nothing. A
/// dropped receiver (app closing) makes the send a no-op.
pub fn spawn_check(tx: tokio::sync::mpsc::UnboundedSender<VersionInfo>) {
    std::thread::spawn(move || {
        // Best-effort: build client → GET → status check → read body → parse.
        // ANY step failing returns `Err`, which we ignore (graceful degrade).
        let fetched = (|| -> Result<VersionInfo, ()> {
            let client = reqwest::blocking::Client::builder()
                .timeout(FETCH_TIMEOUT)
                .user_agent(concat!("koma/", env!("CARGO_PKG_VERSION")))
                .build()
                .map_err(|_| ())?;

            let resp = client.get(VERSION_URL).send().map_err(|_| ())?;
            // Treat any non-success status (4xx/5xx) as "no update available".
            if !resp.status().is_success() {
                return Err(());
            }

            let body = resp.text().map_err(|_| ())?;
            serde_json::from_str::<VersionInfo>(&body).map_err(|_| ())
        })();

        // Only forward a clean success. On `Err`, do nothing at all.
        if let Ok(info) = fetched {
            let _ = tx.send(info);
        }
    });
}
