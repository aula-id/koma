//! Dynamic premium model catalogue for KomaRun.
//!
//! Fetches the live allowlisted premium model list from
//! `GET {KOMA_PREMIUM_CHAT_ENDPOINT}/models` (requires a KomaRun OAuth bearer),
//! caches it on disk (`~/.koma/koma-premium-models.json`) and in memory, and
//! merges it into the picker / routing paths so premium models beyond the
//! bundled `koma/peach` seed appear automatically.
//!
//! This module is intentionally separate from the GitHub-refreshed overlay
//! table (`super::fetch`) — GitHub overlay swaps must never wipe dynamic
//! premium entries.

use std::sync::{OnceLock, RwLock, atomic::{AtomicBool, Ordering}};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::dto::openrouter::ModelInfo;
use crate::model::app_config::OAuthProvider;
use super::model::OverlayModel;

// ---------------------------------------------------------------------------
// In-memory store
// ---------------------------------------------------------------------------

/// TTL before a network refresh is attempted again (1 hour).
const REFRESH_TTL: Duration = Duration::from_secs(3600);

/// HTTP timeout for the premium catalogue fetch.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// The on-disk cache filename (sibling to `models.json`).
const CACHE_FILENAME: &str = "koma-premium-models.json";

static STORE: OnceLock<RwLock<PremiumDynamic>> = OnceLock::new();

/// Single-flight guard: only one fetch task runs at a time.
static IN_FLIGHT: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Default)]
struct PremiumDynamic {
    models: Vec<OverlayModel>,
    fetched_at: Option<u64>,
}

/// On-disk JSON shape.
#[derive(serde::Serialize, serde::Deserialize)]
struct DiskCache {
    fetched_at_unix: Option<u64>,
    models: Vec<OverlayModel>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Call once at startup (after `catalogue_overlay::init`). Loads the on-disk
/// cache if present. Best-effort — never panics.
pub fn init_from_disk() {
    let data = load_disk().unwrap_or_default();
    let _ = STORE.set(RwLock::new(data));
}

/// Snapshot of currently known premium models (cloned, cheap).
#[allow(dead_code)]
pub fn models() -> Vec<OverlayModel> {
    let Some(lock) = STORE.get() else {
        return Vec::new();
    };
    let Ok(guard) = lock.read() else {
        return Vec::new();
    };
    guard.models.clone()
}

/// `true` if `model_id` is in the dynamic premium list.
pub fn contains(model_id: &str) -> bool {
    let Some(lock) = STORE.get() else {
        return false;
    };
    let Ok(guard) = lock.read() else {
        return false;
    };
    guard.models.iter().any(|m| m.id == model_id)
}

/// Append dynamic premium models into `models`, deduplicating by id.
/// Used by `models_for_provider` so the picker includes live premium entries.
pub fn merge_into(models: &mut Vec<ModelInfo>) {
    let Some(lock) = STORE.get() else {
        return;
    };
    let Ok(guard) = lock.read() else {
        return;
    };
    let seen: std::collections::HashSet<&str> = models.iter().map(|m| m.id.as_str()).collect();
    let new: Vec<ModelInfo> = guard
        .models
        .iter()
        .filter(|m| !seen.contains(m.id.as_str()))
        .map(|m| m.to_model_info())
        .collect();
    models.extend(new);
}

/// Spawn a non-blocking background refresh of the premium model list.
/// `access_token` is the current KomaRun JWT. Uses the existing tokio handle
/// (safe because this is only called from the event loop or after OAuth
/// success, when a runtime is guaranteed).
pub fn spawn_refresh(access_token: String) {
    if IN_FLIGHT.swap(true, Ordering::AcqRel) {
        return; // already running
    }
    tokio::spawn(async move {
        let result = fetch_and_swap(&access_token).await;
        IN_FLIGHT.store(false, Ordering::Release);
        if let Err(e) = result {
            crate::model::store::append_global_error_log(
                "premium dynamic catalogue",
                &format!("refresh failed: {e}"),
            );
        }
    });
}

/// TTL-gated refresh: only fetches if the cache is older than [`REFRESH_TTL`]
/// or `force` is true. Does nothing if no KomaRun token is available.
pub fn maybe_refresh(access_token: &str, force: bool) {
    if !force {
        let Some(lock) = STORE.get() else {
            return;
        };
        let Ok(guard) = lock.read() else {
            return;
        };
        if let Some(ts) = guard.fetched_at {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if now.saturating_sub(ts) < REFRESH_TTL.as_secs() {
                return; // cache is fresh
            }
        }
    }
    spawn_refresh(access_token.to_string());
}

// ---------------------------------------------------------------------------
// Fetch + parse + swap
// ---------------------------------------------------------------------------

async fn fetch_and_swap(access_token: &str) -> anyhow::Result<()> {
    use crate::service::oauth::registry::KOMA_PREMIUM_CHAT_ENDPOINT;

    let url = format!("{KOMA_PREMIUM_CHAT_ENDPOINT}/models");
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .user_agent(concat!("koma/", env!("CARGO_PKG_VERSION")))
        .build()?;

    let mut resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {access_token}"))
        .send()
        .await?;

    // On 401: try one fresh_key retry, then bail.
    if resp.status().as_u16() == 401 {
        let config = crate::model::app_config::AppConfig::load();
        if let Some(conn) = config.oauth_conns.iter().find(|c| {
            c.provider == OAuthProvider::KomaRun && c.access_token == access_token
        }) {
            let (fresh, _) =
                crate::service::oauth::manager::fresh_key(&conn.uuid, &conn.access_token).await;
            if !fresh.is_empty() && fresh != access_token {
                resp = client
                    .get(&url)
                    .header("Authorization", format!("Bearer {fresh}"))
                    .send()
                    .await?;
            }
        }
    }

    if !resp.status().is_success() {
        anyhow::bail!("HTTP {}", resp.status().as_u16());
    }

    let body = resp.text().await?;
    let parsed: ServerModelList = serde_json::from_str(&body)?;

    let fetched_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let models: Vec<OverlayModel> = parsed
        .data
        .into_iter()
        .filter_map(|item| {
            if item.id.is_empty() {
                return None;
            }
            Some(OverlayModel {
                id: item.id,
                supported_parameters: item.supported_parameters.unwrap_or_default(),
                reasoning: None, // server doesn't send reasoning metadata yet
                context_length: item.context_length,
                pricing: None, // server-metered credits, not per-token USD
            })
        })
        .collect();

    if models.is_empty() {
        anyhow::bail!("server returned empty model list");
    }

    // Ensure the static `koma/peach` seed is always present.
    let mut models = models;
    if !models.iter().any(|m| m.id == "koma/peach") {
        models.push(OverlayModel {
            id: "koma/peach".to_string(),
            supported_parameters: vec![],
            reasoning: None,
            context_length: Some(200_000),
            pricing: None,
        });
    }

    // Swap in-memory store.
    if let Some(lock) = STORE.get() {
        if let Ok(mut guard) = lock.write() {
            guard.models = models.clone();
            guard.fetched_at = Some(fetched_at);
        }
    }

    // Atomic disk write.
    let cache = DiskCache {
        fetched_at_unix: Some(fetched_at),
        models,
    };
    if let Ok(bytes) = serde_json::to_vec_pretty(&cache) {
        if let Ok(base) = crate::model::store::base_dir() {
            let path = base.join(CACHE_FILENAME);
            let parent = path.parent().unwrap_or(std::path::Path::new("."));
            let file_name = path
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("no file name"))?;
            let mut tmp_name = file_name.to_owned();
            tmp_name.push(format!(".{}$", std::process::id()));
            let tmp_path = parent.join(&tmp_name);
            if let Err(e) = std::fs::write(&tmp_path, &bytes) {
                crate::model::store::append_global_error_log(
                    "premium dynamic catalogue",
                    &format!("disk write failed: {e}"),
                );
            } else {
                let _ = std::fs::rename(&tmp_path, &path);
            }
        }
    }

    crate::model::store::append_global_error_log(
        "premium dynamic catalogue",
        &format!("refreshed {} models", models_len_from_store()),
    );

    Ok(())
}

/// Helper to log the current store size after a refresh.
fn models_len_from_store() -> usize {
    STORE
        .get()
        .and_then(|lock| lock.read().ok())
        .map(|g| g.models.len())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Server response types
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct ServerModelList {
    data: Vec<ServerModelItem>,
}

#[derive(serde::Deserialize)]
struct ServerModelItem {
    id: String,
    #[serde(default)]
    supported_parameters: Option<Vec<String>>,
    #[serde(default)]
    context_length: Option<u64>,
}

// ---------------------------------------------------------------------------
// Disk I/O
// ---------------------------------------------------------------------------

fn load_disk() -> Option<PremiumDynamic> {
    let base = crate::model::store::base_dir().ok()?;
    let path = base.join(CACHE_FILENAME);
    let bytes = std::fs::read_to_string(&path).ok()?;
    let cache: DiskCache = serde_json::from_str(&bytes).ok()?;
    Some(PremiumDynamic {
        models: cache.models,
        fetched_at: cache.fetched_at_unix,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_server_response() {
        let json = r#"{
            "object": "list",
            "data": [
                { "id": "openai/gpt-4o", "context_length": 128000, "supported_parameters": ["tools"] },
                { "id": "anthropic/claude-sonnet-4" }
            ]
        }"#;
        let parsed: ServerModelList = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.data.len(), 2);
        assert_eq!(parsed.data[0].id, "openai/gpt-4o");
        assert_eq!(parsed.data[0].context_length, Some(128000));
        assert_eq!(
            parsed.data[0].supported_parameters,
            Some(vec!["tools".to_string()])
        );
        assert_eq!(parsed.data[1].id, "anthropic/claude-sonnet-4");
        assert!(parsed.data[1].context_length.is_none());
    }

    #[test]
    fn parse_disk_cache_roundtrip() {
        let cache = DiskCache {
            fetched_at_unix: Some(1710000000),
            models: vec![OverlayModel {
                id: "openai/gpt-4o".to_string(),
                supported_parameters: vec!["tools".to_string()],
                reasoning: None,
                context_length: Some(128000),
                pricing: None,
            }],
        };
        let json = serde_json::to_string(&cache).unwrap();
        let parsed: DiskCache = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.fetched_at_unix, Some(1710000000));
        assert_eq!(parsed.models.len(), 1);
        assert_eq!(parsed.models[0].id, "openai/gpt-4o");
    }

    #[test]
    fn contains_respects_store() {
        // Without init_from_disk, store is unset — contains returns false.
        assert!(!contains("koma/peach"));
    }

    #[test]
    fn merge_into_dedupes() {
        // Directly set up a store for test.
        let store = PremiumDynamic {
            models: vec![
                OverlayModel {
                    id: "koma/peach".to_string(),
                    supported_parameters: vec![],
                    reasoning: None,
                    context_length: Some(200_000),
                    pricing: None,
                },
                OverlayModel {
                    id: "openai/gpt-4o".to_string(),
                    supported_parameters: vec!["tools".to_string()],
                    reasoning: None,
                    context_length: Some(128_000),
                    pricing: None,
                },
            ],
            fetched_at: None,
        };
        let _ = STORE.set(RwLock::new(store));

        let mut existing = vec![ModelInfo {
            id: "koma/peach".to_string(),
            name: None,
            supported_parameters: vec![],
            reasoning: None,
            context_length: Some(200_000),
            top_provider: None,
            pricing: None,
            architecture: None,
        }];
        merge_into(&mut existing);
        // koma/peach deduped, gpt-4o added
        assert_eq!(existing.len(), 2);
        assert!(existing.iter().any(|m| m.id == "openai/gpt-4o"));
    }
}
