//! Fetch OAuth provider catalogues from koma-landing `/api/v1/oa/providers/{id}`
//! and merge them into the catalogue overlay table (keyed by chat_endpoint).

use std::time::Duration;

use serde::Deserialize;

use crate::dto::openrouter::ModelReasoning;
use crate::model::app_config::OAuthProvider;
use crate::service::oauth::registry::{self, KOMA_PREMIUM_CHAT_ENDPOINT};

use super::model::{OverlayModel, OverlayPricing, OverlayTable};

/// Public OA catalogue base. Override with `KOMA_OA_BASE` for local landing.
const DEFAULT_OA_BASE: &str = "https://koma.run/api/v1/oa";
const OA_TTL: Duration = Duration::from_secs(3600);
const FETCH_TIMEOUT: Duration = Duration::from_secs(12);

/// Wire ids fetched on refresh (matches landing registry + `OAuthProvider::wire_id`).
const OA_PROVIDERS: &[(OAuthProvider, &str)] = &[
    (OAuthProvider::Codex, "codex"),
    (OAuthProvider::Xai, "xai"),
    (OAuthProvider::ClaudeAI, "claudeai"),
    (OAuthProvider::CommandCode, "commandcode"),
    (OAuthProvider::Kilocode, "kilocode"),
    (OAuthProvider::KomaRun, "komarun"),
];

#[derive(Debug, Deserialize)]
struct OaListResponse {
    #[serde(default)]
    data: Vec<OaRow>,
}

#[derive(Debug, Deserialize)]
struct OaRow {
    id: String,
    #[serde(default)]
    supported_parameters: Vec<String>,
    #[serde(default)]
    reasoning: Option<ModelReasoning>,
    #[serde(default)]
    context_length: Option<u64>,
    #[serde(default)]
    pricing: Option<OaPricing>,
    /// free | premium | exclusive — KomaRun endpoint split.
    #[serde(default)]
    tier: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OaPricing {
    #[serde(default)]
    input: f64,
    #[serde(default)]
    cached: f64,
    #[serde(default)]
    output: f64,
}

impl From<OaRow> for OverlayModel {
    fn from(r: OaRow) -> Self {
        OverlayModel {
            id: r.id,
            supported_parameters: r.supported_parameters,
            reasoning: r.reasoning,
            context_length: r.context_length,
            pricing: r.pricing.map(|p| OverlayPricing {
                input: p.input,
                cached: p.cached,
                output: p.output,
            }),
        }
    }
}

fn oa_base() -> String {
    std::env::var("KOMA_OA_BASE").unwrap_or_else(|_| DEFAULT_OA_BASE.to_string())
}

/// Background thread: refresh OA catalogues and merge into the live overlay.
pub(super) fn spawn_refresh() {
    std::thread::spawn(|| {
        if let Err(e) = refresh_all() {
            crate::model::store::append_global_error_log(
                "catalogue oa",
                &format!("refresh failed: {e}"),
            );
        }
    });
}

fn refresh_all() -> anyhow::Result<()> {
    let base_dir = crate::model::store::base_dir()?;
    let oa_dir = base_dir.join("oa");
    let _ = std::fs::create_dir_all(&oa_dir);

    // TTL gate: skip network if every provider disk cache is fresh.
    if OA_PROVIDERS
        .iter()
        .all(|(_, id)| is_fresh(&oa_dir.join(format!("{id}.json"))))
    {
        // Still merge disk into memory (covers cold start after init from models.json).
        merge_disk_into_overlay(&oa_dir);
        return Ok(());
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .user_agent(concat!("koma/", env!("CARGO_PKG_VERSION")))
        .build()?;

    let base = oa_base();
    let mut patch = OverlayTable::new();

    for (provider, wire_id) in OA_PROVIDERS {
        let url = format!("{base}/providers/{wire_id}");
        let cache_path = oa_dir.join(format!("{wire_id}.json"));

        let body = match client.get(&url).send() {
            Ok(resp) if resp.status().is_success() => match resp.text() {
                Ok(t) => t,
                Err(e) => {
                    crate::model::store::append_global_error_log(
                        "catalogue oa",
                        &format!("{wire_id}: body {e}"),
                    );
                    // fall through to disk
                    String::new()
                }
            },
            Ok(resp) => {
                crate::model::store::append_global_error_log(
                    "catalogue oa",
                    &format!("{wire_id}: status {}", resp.status()),
                );
                String::new()
            }
            Err(e) => {
                crate::model::store::append_global_error_log(
                    "catalogue oa",
                    &format!("{wire_id}: {e}"),
                );
                String::new()
            }
        };

        let text = if body.is_empty() {
            std::fs::read_to_string(&cache_path).unwrap_or_default()
        } else {
            let _ = atomic_write(&cache_path, body.as_bytes());
            body
        };

        if text.is_empty() {
            continue;
        }

        let parsed: OaListResponse = match serde_json::from_str(&text) {
            Ok(p) => p,
            Err(e) => {
                crate::model::store::append_global_error_log(
                    "catalogue oa",
                    &format!("{wire_id}: parse {e}"),
                );
                continue;
            }
        };

        apply_provider_rows(*provider, parsed.data, &mut patch);
    }

    if !patch.is_empty() {
        super::merge_overlay_patch(patch);
        crate::model::store::append_global_error_log(
            "catalogue oa",
            &format!("merged OA catalogues from {base}"),
        );
    }
    Ok(())
}

fn merge_disk_into_overlay(oa_dir: &std::path::Path) {
    let mut patch = OverlayTable::new();
    for (provider, wire_id) in OA_PROVIDERS {
        let path = oa_dir.join(format!("{wire_id}.json"));
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<OaListResponse>(&text) else {
            continue;
        };
        apply_provider_rows(*provider, parsed.data, &mut patch);
    }
    if !patch.is_empty() {
        super::merge_overlay_patch(patch);
    }
}

fn apply_provider_rows(provider: OAuthProvider, rows: Vec<OaRow>, patch: &mut OverlayTable) {
    if provider == OAuthProvider::KomaRun {
        let mut base_rows = Vec::new();
        let mut premium_rows = Vec::new();
        for r in rows {
            let tier = r.tier.clone().unwrap_or_default();
            let model = OverlayModel::from(r);
            match tier.as_str() {
                "free" => base_rows.push(model),
                "premium" | "exclusive" => premium_rows.push(model),
                _ => {
                    // Untagged: peach / koma/* → premium; else base.
                    if model.id.starts_with("koma/") {
                        premium_rows.push(model);
                    } else {
                        // Gateway allowlist ids live on premium chat path for KomaRun.
                        premium_rows.push(model);
                    }
                }
            }
        }
        // Free apple on base endpoint; everything else (peach + gateway) on premium.
        let base_ep = registry::meta(OAuthProvider::KomaRun).chat_endpoint;
        if !base_rows.is_empty() {
            patch.insert(base_ep.to_string(), base_rows);
        }
        if !premium_rows.is_empty() {
            patch.insert(KOMA_PREMIUM_CHAT_ENDPOINT.to_string(), premium_rows);
        }
        return;
    }

    let endpoint = registry::meta(provider).chat_endpoint;
    if endpoint.is_empty() {
        return;
    }
    let models: Vec<OverlayModel> = rows.into_iter().map(OverlayModel::from).collect();
    if !models.is_empty() {
        patch.insert(endpoint.to_string(), models);
    }
}

fn is_fresh(path: &std::path::Path) -> bool {
    let modified = match std::fs::metadata(path).and_then(|m| m.modified()) {
        Ok(m) => m,
        Err(_) => return false,
    };
    match modified.elapsed() {
        Ok(age) => age < OA_TTL,
        Err(_) => false,
    }
}

fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or(std::path::Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "no file name"))?;
    let tmp = parent.join(format!(
        ".{}.tmp.{}",
        file_name.to_string_lossy(),
        std::process::id()
    ));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_oa_row_to_overlay() {
        let raw = r#"{
          "data": [{
            "id": "grok-4.7",
            "supported_parameters": ["reasoning"],
            "reasoning": { "mandatory": false, "supported_efforts": ["low","high"] },
            "context_length": 256000,
            "pricing": { "input": 1.0, "cached": 0.1, "output": 2.0 }
          }]
        }"#;
        let parsed: OaListResponse = serde_json::from_str(raw).unwrap();
        let mut patch = OverlayTable::new();
        apply_provider_rows(OAuthProvider::Xai, parsed.data, &mut patch);
        let ep = registry::meta(OAuthProvider::Xai).chat_endpoint;
        let m = &patch.get(ep).unwrap()[0];
        assert_eq!(m.id, "grok-4.7");
        assert!(m.reasoning.is_some());
        assert_eq!(m.pricing.as_ref().unwrap().input, 1.0);
    }

    #[test]
    fn komarun_splits_tiers() {
        let raw = r#"{
          "data": [
            { "id": "koma/apple", "tier": "free" },
            { "id": "koma/peach", "tier": "exclusive" },
            { "id": "openai/gpt-5", "tier": "premium" }
          ]
        }"#;
        let parsed: OaListResponse = serde_json::from_str(raw).unwrap();
        let mut patch = OverlayTable::new();
        apply_provider_rows(OAuthProvider::KomaRun, parsed.data, &mut patch);
        let base = registry::meta(OAuthProvider::KomaRun).chat_endpoint;
        assert_eq!(patch.get(base).unwrap()[0].id, "koma/apple");
        let prem = patch.get(KOMA_PREMIUM_CHAT_ENDPOINT).unwrap();
        assert!(prem.iter().any(|m| m.id == "koma/peach"));
        assert!(prem.iter().any(|m| m.id == "openai/gpt-5"));
    }
}
