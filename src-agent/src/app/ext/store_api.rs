//! Shared koma.run extension-STORE HTTP client + JSON mappers.
//!
//! Extracted VERBATIM from `event_loop::daemon::hub::requests_ext` (behaviour-identical)
//! so the store fetch/parse layer has ONE home the daemon store arms and the upcoming
//! `/store` TUI wave both reuse — the network fetches (`fetch_catalogue` / `fetch_detail`
//! / `fetch_install_artifact`), the platform detector, and the defensive JSON→wire
//! mappers. All logging still flows through [`store::append_global_error_log`] — never
//! `eprintln!`/`println!` (this is TUI-owning runtime code).
//!
//! NOTE: `client::store_host.rs` keeps its OWN mirror of these (it is a detached GUI-host
//! path, deliberately not sharing daemon internals) — that module is left untouched.

use crate::ipc::proto::{StoreContributesWire, StoreDetailWire, StoreItemWire};
use crate::model::store;

/// Base URL of the koma.run extension store API (contract v0).
pub(crate) const STORE_API_BASE: &str = "https://koma.run/api/v1/extensions";

/// Detect this build's store platform token (`<os>-<arch>`), or `None` for a platform the
/// v0 store doesn't ship (e.g. windows-arm64). Uses `cfg!`-gated returns so it resolves at
/// compile time to the host triple. The v0 set is
/// `linux-x64` / `linux-arm64` / `darwin-x64` / `darwin-arm64` / `windows-x64`.
pub(crate) fn detect_platform() -> Option<&'static str> {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return Some("linux-x64");
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        return Some("linux-arm64");
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        return Some("darwin-x64");
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return Some("darwin-arm64");
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        return Some("windows-x64");
    }
    #[allow(unreachable_code)]
    {
        None
    }
}

/// A shared reqwest client for the store fetches (default redirect policy — follows the
/// signed-URI redirect on the direct-stream fallback and any CDN hop for browse/detail).
fn http_client() -> reqwest::Client {
    reqwest::Client::new()
}

/// `GET /extensions[?q&category]` → the mapped catalogue rows. PUBLIC (no auth). A non-2xx
/// status or a parse error is an `Err(String)` the caller surfaces as the catalogue's error.
pub(crate) async fn fetch_catalogue(
    query: Option<String>,
    category: Option<String>,
) -> std::result::Result<Vec<StoreItemWire>, String> {
    // Build the URL with proper query-param encoding via reqwest::Url.
    let mut pairs: Vec<(&str, String)> = Vec::new();
    if let Some(q) = query {
        let q = q.trim().to_string();
        if !q.is_empty() {
            pairs.push(("q", q));
        }
    }
    if let Some(c) = category {
        let c = c.trim().to_string();
        if !c.is_empty() {
            pairs.push(("category", c));
        }
    }
    let url = reqwest::Url::parse_with_params(STORE_API_BASE, &pairs)
        .map_err(|e| format!("bad store url: {e}"))?;

    let resp = http_client()
        .get(url)
        .send()
        .await
        .map_err(|e| format!("store request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("store returned HTTP {}", resp.status().as_u16()));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("store response parse failed: {e}"))?;
    let items = body
        .get("items")
        .and_then(|x| x.as_array())
        .map(|arr| arr.iter().map(map_summary).collect())
        .unwrap_or_default();
    Ok(items)
}

/// `GET /extensions/{id}` → the mapped detail. PUBLIC (no auth).
pub(crate) async fn fetch_detail(id: &str) -> std::result::Result<StoreDetailWire, String> {
    let url = format!("{STORE_API_BASE}/{id}");
    let resp = http_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("store request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("store returned HTTP {}", resp.status().as_u16()));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("store response parse failed: {e}"))?;
    Ok(map_detail(&body))
}

/// `GET /extensions/{id}/download?version&platform` with the account Bearer, resolving the
/// artifact per the store contract's TWO shapes:
///
/// * **302 redirect** (preferred): the response carries a `Location` (the short-lived signed
///   URI) plus a JSON body echoing `{ sha256, signature }`; we read the integrity from the
///   body, then GET the signed URI for the `.zip` bytes.
/// * **direct stream** (v0 fallback): a `200` whose body IS the `.zip`, with integrity in the
///   `X-Koma-Sha256` / `X-Koma-Signature` headers.
///
/// Redirects are DISABLED on the first hop so we can read the 302 body + `Location` ourselves
/// (an auto-follow would swallow the integrity body). Returns `(zip_bytes, sha256,
/// signature)`; `signature` is `None` when the server advertised none (→ the caller's dev
/// unsigned fallback). A 401/402/404/… maps to a friendly error string.
pub(crate) async fn fetch_install_artifact(
    id: &str,
    version: Option<&str>,
    platform: &str,
    bearer: &str,
) -> std::result::Result<(Vec<u8>, String, Option<String>), String> {
    let no_redirect = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("http client build failed: {e}"))?;

    let mut pairs: Vec<(&str, &str)> = vec![("platform", platform)];
    if let Some(v) = version {
        if !v.is_empty() {
            pairs.push(("version", v));
        }
    }
    let url = reqwest::Url::parse_with_params(&format!("{STORE_API_BASE}/{id}/download"), &pairs)
        .map_err(|e| format!("bad download url: {e}"))?;

    let resp = no_redirect
        .get(url)
        .bearer_auth(bearer)
        .send()
        .await
        .map_err(|e| {
            let msg = format!("download request failed: {e}");
            store::append_global_error_log(
                "ext download",
                &format!("{id} (platform {platform}): {msg}"),
            );
            msg
        })?;
    let status = resp.status();

    if status.is_redirection() {
        // 302: Location → signed URI; body echoes the integrity fields.
        let location = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .ok_or_else(|| {
                let msg = "download redirect missing Location header".to_string();
                store::append_global_error_log(
                    "ext download",
                    &format!("{id} (platform {platform}): {msg}"),
                );
                msg
            })?;
        let body = resp.text().await.unwrap_or_default();
        let (sha256, signature) = parse_integrity_json(&body);

        // The signed URI is public (auth is in the query signature) — a plain follow.
        let zresp = http_client()
            .get(&location)
            .send()
            .await
            .map_err(|e| format!("signed download failed: {e}"))?;
        if !zresp.status().is_success() {
            let signed_status = zresp.status().as_u16();
            store::append_global_error_log(
                "ext download",
                &format!(
                    "{id} (platform {platform}): signed download returned HTTP {signed_status}"
                ),
            );
            return Err(format!("signed download returned HTTP {signed_status}"));
        }
        let bytes = zresp
            .bytes()
            .await
            .map_err(|e| format!("reading artifact failed: {e}"))?
            .to_vec();
        Ok((bytes, sha256, signature))
    } else if status.is_success() {
        // Direct stream: integrity in headers, body IS the zip.
        let sha256 = header_str(&resp, "x-koma-sha256").unwrap_or_default();
        let signature = header_str(&resp, "x-koma-signature").filter(|s| !s.is_empty());
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("reading artifact failed: {e}"))?
            .to_vec();
        Ok((bytes, sha256, signature))
    } else {
        let code = status.as_u16();
        let msg = match code {
            401 => "koma.run rejected the session — sign in again".to_string(),
            402 => "this extension needs an active koma.run entitlement".to_string(),
            404 => "extension not found for this version/platform".to_string(),
            429 => "koma.run is rate limiting — try again shortly".to_string(),
            other => format!("download failed (HTTP {other})"),
        };
        store::append_global_error_log(
            "ext download",
            &format!("{id} (platform {platform}): HTTP {code}: {msg}"),
        );
        Err(msg)
    }
}

/// Read a response header as a `String`, or `None` if absent / non-ASCII.
fn header_str(resp: &reqwest::Response, name: &str) -> Option<String> {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

/// Pull `{ sha256, signature }` out of a 302 integrity body (best-effort). A malformed /
/// empty body yields `(String::new(), None)` — the caller then treats it as unsigned.
fn parse_integrity_json(body: &str) -> (String, Option<String>) {
    let v: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return (String::new(), None),
    };
    let sha = str_field(&v, "sha256");
    let sig = v
        .get("signature")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
    (sha, sig)
}

/// Map one store `ExtensionSummary` JSON object to [`StoreItemWire`] (defensive — a missing
/// field degrades to empty rather than failing the whole list parse).
fn map_summary(v: &serde_json::Value) -> StoreItemWire {
    StoreItemWire {
        id: str_field(v, "id"),
        name: str_field(v, "name"),
        tagline: str_field(v, "tagline"),
        tier: str_field(v, "tier"),
        kind: str_field(v, "kind"),
        latest_version: str_field(v, "latest_version"),
        icon_url: str_field(v, "icon_url"),
        categories: arr_str(v, "categories"),
        author: str_field(v, "author"),
        updated_at: str_field(v, "updated_at"),
    }
}

/// Map one store `ExtensionDetail` JSON object to [`StoreDetailWire`] (defensive, like
/// [`map_summary`]).
fn map_detail(v: &serde_json::Value) -> StoreDetailWire {
    StoreDetailWire {
        id: str_field(v, "id"),
        name: str_field(v, "name"),
        tagline: str_field(v, "tagline"),
        tier: str_field(v, "tier"),
        kind: str_field(v, "kind"),
        latest_version: str_field(v, "latest_version"),
        icon_url: str_field(v, "icon_url"),
        categories: arr_str(v, "categories"),
        author: str_field(v, "author"),
        updated_at: str_field(v, "updated_at"),
        description_md: str_field(v, "description_md"),
        screenshots: arr_str(v, "screenshots"),
        contributes: map_contributes(v.get("contributes")),
        requires: arr_str(v, "requires"),
        versions: arr_str(v, "versions"),
    }
}

/// Collapse the detail's `contributes` object to per-kind COUNTS. Accepts both the array
/// shape (`{ models: [..], tools: [..] }` → counts) — a missing kind is 0.
fn map_contributes(v: Option<&serde_json::Value>) -> StoreContributesWire {
    let count = |key: &str| -> u32 {
        v.and_then(|c| c.get(key))
            .and_then(|x| x.as_array())
            .map(|a| a.len() as u32)
            .unwrap_or(0)
    };
    StoreContributesWire {
        models: count("models"),
        panels: count("panels"),
        tools: count("tools"),
        sub_agents: count("sub_agents"),
    }
}

/// A string field of a JSON object, or `""` if absent / not a string.
fn str_field(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string()
}

/// A `Vec<String>` field of a JSON object (its string elements), or empty.
fn arr_str(v: &serde_json::Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|e| e.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// The build host is always one of the v0 platforms (the test binary itself is one of
    /// them), so `detect_platform` must resolve to a `Some` in the advertised set.
    #[test]
    fn detect_platform_is_a_known_v0_token() {
        let plat = detect_platform().expect("build host must be a v0 store platform");
        assert!(
            [
                "linux-x64",
                "linux-arm64",
                "darwin-x64",
                "darwin-arm64",
                "windows-x64"
            ]
            .contains(&plat),
            "unexpected platform token: {plat}"
        );
    }

    /// The summary mapping pulls exactly the wire fields from an `ExtensionSummary`-shaped
    /// object, degrading a missing field to empty rather than failing.
    #[test]
    fn map_summary_projects_summary_fields() {
        let v = serde_json::json!({
            "id": "run.koma.gateway",
            "name": "koma Gateway",
            "tagline": "Premium koma models, one endpoint.",
            "tier": "paid",
            "kind": "daemon",
            "latest_version": "0.3.1",
            "icon_url": "https://cdn.koma.run/ext/run.koma.gateway/icon.png",
            "categories": ["models", "gateway"],
            "author": "koma",
            "updated_at": "2026-07-10T12:00:00Z"
        });
        let item = map_summary(&v);
        assert_eq!(item.id, "run.koma.gateway");
        assert_eq!(item.name, "koma Gateway");
        assert_eq!(item.tier, "paid");
        assert_eq!(item.kind, "daemon");
        assert_eq!(item.latest_version, "0.3.1");
        assert_eq!(item.categories, vec!["models", "gateway"]);
        assert_eq!(item.author, "koma");
    }

    /// The detail mapping projects the long-form fields AND collapses `contributes` to
    /// per-kind counts + carries the `requires` grant list (the install card's inputs).
    #[test]
    fn map_detail_counts_contributions_and_reads_requires() {
        let v = serde_json::json!({
            "id": "run.koma.gateway",
            "name": "koma Gateway",
            "tagline": "one endpoint",
            "tier": "paid",
            "kind": "daemon",
            "latest_version": "0.3.1",
            "icon_url": "",
            "categories": ["models"],
            "author": "koma",
            "updated_at": "2026-07-10T12:00:00Z",
            "description_md": "# koma Gateway\n\nlong",
            "screenshots": ["https://cdn.koma.run/ext/run.koma.gateway/1.png"],
            "contributes": {
                "models": [{ "id": "a" }, { "id": "b" }],
                "panels": [],
                "tools": [{ "name": "t" }],
                "sub_agents": []
            },
            "requires": ["agents:read"],
            "versions": ["0.3.1", "0.3.0"]
        });
        let d = map_detail(&v);
        assert_eq!(d.description_md, "# koma Gateway\n\nlong");
        assert_eq!(d.screenshots.len(), 1);
        assert_eq!(d.contributes.models, 2);
        assert_eq!(d.contributes.panels, 0);
        assert_eq!(d.contributes.tools, 1);
        assert_eq!(d.contributes.sub_agents, 0);
        assert_eq!(d.requires, vec!["agents:read"]);
        assert_eq!(d.versions, vec!["0.3.1", "0.3.0"]);
    }

    /// A 302 integrity body yields `(sha, Some(sig))`; an empty / malformed body yields the
    /// unsigned shape `(empty, None)` — the caller's dev-unsigned trigger.
    #[test]
    fn parse_integrity_json_reads_or_degrades() {
        let (sha, sig) =
            parse_integrity_json(r#"{"sha256":"3b1f","signature":"MEUCIQ==","size":123}"#);
        assert_eq!(sha, "3b1f");
        assert_eq!(sig.as_deref(), Some("MEUCIQ=="));

        let (sha2, sig2) = parse_integrity_json("");
        assert!(sha2.is_empty());
        assert!(sig2.is_none());

        // Present-but-empty signature is treated as unsigned.
        let (_sha3, sig3) = parse_integrity_json(r#"{"sha256":"aa","signature":""}"#);
        assert!(sig3.is_none());
    }
}
