//! Model-metadata overlay for non-OpenRouter providers + OAuth catalogues.
//!
//! OAuth providers (codex/claude/xai/…) historically lacked an OpenRouter-style
//! `GET /models` with `reasoning`/`pricing`. This module fills that gap:
//!
//! Loading order: bundled `models.json` (direct-API fallback) → on-disk cache →
//! GitHub release asset (`fetch.rs`) → **koma-landing OA catalogues**
//! (`oa.rs`: `GET /api/v1/oa/providers/{wire_id}`), merged by catalogue identity.
//!
//! Consumers: `models_for` / `models_for_provider`, `effort_caps` input,
//! `overlay_cost`, KomaRun `is_premium_model`.

mod fetch;
mod model;
mod oa;

// dead_code: infra-only for now — no consumer wired yet (W2/W3 will read this).
#[allow(unused_imports)]
pub use model::{OverlayModel, OverlayPricing};

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use crate::dto::openrouter::ModelInfo;
use crate::model::app_config::OAuthProvider;
use model::OverlayTable;

/// The live in-memory overlay table. `OnceLock` because it's initialized
/// exactly once at startup ([`init`]); `RwLock` inside because the background
/// refresh (`fetch::spawn_refresh`) swaps it in from a different thread while
/// [`lookup`]/[`models_for`] may be reading concurrently.
static OVERLAY: OnceLock<RwLock<OverlayState>> = OnceLock::new();

/// Keep OA provenance alongside the composed table, under the same lock. A
/// GitHub base refresh must not erase newer provider rows or model details.
#[derive(Default)]
struct OverlayState {
    table: OverlayTable,
    oa: OverlayTable,
    details: OverlayTable,
    /// What one signed-in Koma account may use on top of the public list,
    /// keyed by connection uuid. Kept out of the shared tables on purpose:
    /// the rows depend on who is signed in, so one account's private models
    /// must never show up for another connection.
    accounts: HashMap<String, Vec<OverlayModel>>,
}

impl OverlayState {
    fn replace_base(&mut self, table: OverlayTable) {
        self.table = table;
        self.table.extend(self.oa.clone());
        self.apply_details();
    }

    fn merge_oa(&mut self, patch: OverlayTable) {
        self.oa.extend(patch.clone());
        self.table.extend(patch);
        self.apply_details();
    }

    fn apply_details(&mut self) {
        for (key, models) in &self.details {
            for model in models {
                upsert_model(&mut self.table, key, model.clone());
            }
        }
    }
}

fn upsert_model(table: &mut OverlayTable, key: &str, model: OverlayModel) {
    let models = table.entry(key.to_string()).or_default();
    if let Some(existing) = models.iter_mut().find(|m| m.id == model.id) {
        *existing = model;
    } else {
        models.push(model);
    }
}

/// Stable capability identity, independent of the selected chat transport.
/// Unknown/direct providers retain their resolved endpoint.
pub fn catalogue_key(endpoint: &str, provider: Option<OAuthProvider>) -> String {
    use crate::service::oauth::registry;
    if provider == Some(OAuthProvider::CommandCode) {
        return registry::COMMANDCODE_API_BASE.to_string();
    }
    if let Some(provider) = provider {
        let meta = registry::meta(provider);
        if !meta.catalogue_endpoint.is_empty() {
            return meta.catalogue_endpoint.to_string();
        }
        if !endpoint.is_empty() {
            return endpoint.to_string();
        }
        return meta.chat_endpoint.to_string();
    }
    endpoint.to_string()
}

pub use oa::fetch_oa_detail;

/// Upsert one OA detail without replacing the provider's membership list.
pub fn merge_detail(catalogue_key: &str, model: OverlayModel) {
    let lock = OVERLAY.get_or_init(|| {
        RwLock::new(OverlayState {
            table: load_initial(),
            ..OverlayState::default()
        })
    });
    if let Ok(mut guard) = lock.write() {
        upsert_model(&mut guard.details, catalogue_key, model.clone());
        upsert_model(&mut guard.table, catalogue_key, model);
    }
}

/// The overlay table compiled into the binary as a fallback for a fresh
/// install (no `~/.koma/models.json` cache yet) or a corrupt cache. Lives at
/// the repo root as a sibling of `version.json`.
const BUNDLED_DEFAULT: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../models.json"));

/// Call ONCE at startup (mirrors `model::store::migrate_legacy_dir`): loads
/// the overlay table (a fresh on-disk cache if present and valid, else the
/// bundled default compiled into the binary), then kicks off a non-blocking
/// background refresh against the GitHub release asset **and** koma-landing
/// `/api/v1/oa/providers/{id}` catalogues (OAuth thinking + membership).
///
/// Never panics: a malformed cache or bundled file degrades to an empty table
/// rather than aborting startup.
pub fn init() {
    let table = load_initial();
    let _ = OVERLAY.set(RwLock::new(OverlayState {
        table,
        ..OverlayState::default()
    }));
    if let Ok(base) = crate::model::store::base_dir() {
        oa::merge_disk_into_overlay(&base.join("oa"));
    }
    fetch::spawn_refresh();
    oa::spawn_refresh();
    oa::init_accounts();
}

/// Load the starting table: on-disk cache if it parses, else the bundled
/// default, else an empty table (logged, never a hard failure).
fn load_initial() -> OverlayTable {
    if let Ok(base) = crate::model::store::base_dir() {
        let cache_path = base.join("models.json");
        if let Ok(bytes) = std::fs::read_to_string(&cache_path) {
            match serde_json::from_str::<OverlayTable>(&bytes) {
                Ok(table) => return table,
                Err(e) => crate::model::store::append_global_error_log(
                    "catalogue overlay",
                    &format!("cache parse failed ({e}), falling back to bundled default"),
                ),
            }
        }
    }
    let bundled = BUNDLED_DEFAULT.trim();
    if bundled.is_empty() {
        return HashMap::new();
    }
    match serde_json::from_str::<OverlayTable>(bundled) {
        Ok(table) => table,
        Err(e) => {
            crate::model::store::append_global_error_log(
                "catalogue overlay",
                &format!("bundled default failed to parse ({e}) - starting empty"),
            );
            HashMap::new()
        }
    }
}

/// Swap the in-memory overlay (called by the background fetch on a
/// successful, validated refresh). `pub(super)` so only `fetch` can call it.
pub(super) fn set_overlay(table: OverlayTable) {
    if let Some(lock) = OVERLAY.get() {
        if let Ok(mut guard) = lock.write() {
            guard.replace_base(table);
        }
    }
}

/// Merge endpoint keys from `patch` into the live overlay (OA refresh path).
/// Existing keys not in `patch` are preserved (e.g. DeepSeek direct entries).
pub(super) fn merge_overlay_patch(patch: OverlayTable) {
    if patch.is_empty() {
        return;
    }
    let Some(lock) = OVERLAY.get() else {
        return;
    };
    let Ok(mut guard) = lock.write() else {
        return;
    };
    guard.merge_oa(patch);
}

/// Look up one model by exact `(endpoint, model_id)`. Returns a clone (models
/// are small) or `None` if the endpoint or model isn't in the table (or
/// [`init`] hasn't run yet).
// dead_code: infra-only for now — no consumer wired yet (W2/W3 will read this).
#[allow(dead_code)]
pub fn lookup(endpoint: &str, model_id: &str) -> Option<OverlayModel> {
    let lock = OVERLAY.get()?;
    let guard = lock.read().ok()?;
    guard
        .table
        .get(endpoint)?
        .iter()
        .find(|m| m.id == model_id)
        .cloned()
}

/// Compute a usage-ledger cost (USD) from this overlay's curated per-1M-token
/// pricing, for providers that report zero/no cost themselves (Codex/Claude
/// hardcode `0.0`; direct APIs like DeepSeek may omit it entirely). The
/// overlay's pricing PRESENCE is the toggle: `None` when the model has no
/// `pricing` block in the catalogue (subscription models like Codex/Claude/
/// SuperGrok stay honest at $0 unless the user prices them in `models.json`).
///
/// `cached_tokens` are billed at the discounted `pricing.cached` rate and
/// excluded from the uncached `pricing.input` rate (`prompt_tokens` minus
/// `cached_tokens`, saturating so a cached count that somehow exceeds prompt
/// never underflows). Display-only: koma doesn't track cache-WRITE tokens, so
/// only cache-READ (`cached_tokens`) factors in here.
pub fn overlay_cost(
    endpoint: &str,
    model_id: &str,
    prompt_tokens: u64,
    cached_tokens: u64,
    completion_tokens: u64,
) -> Option<f64> {
    let m = lookup(endpoint, model_id)?;
    let p = m.pricing?;
    let uncached = prompt_tokens.saturating_sub(cached_tokens);
    let cost = (uncached as f64 * p.input
        + cached_tokens as f64 * p.cached
        + completion_tokens as f64 * p.output)
        / 1_000_000.0;
    Some(cost)
}

/// All overlay entries for `endpoint`, mapped to the OpenRouter-shaped
/// `ModelInfo` so a consumer can feed them straight into `effort_caps`/
/// `context_length_for`. Empty vec if the endpoint has no overlay entries (or
/// [`init`] hasn't run — callers must not assume this is non-empty).
///
/// Static-key picker fallback and shared model capability metadata. OAuth
/// picker membership uses [`oauth_catalogue`] instead.
pub fn models_for(endpoint: &str) -> Vec<ModelInfo> {
    let Some(lock) = OVERLAY.get() else {
        return Vec::new();
    };
    let Ok(guard) = lock.read() else {
        return Vec::new();
    };
    guard
        .table
        .get(endpoint)
        .map(|v| v.iter().map(|m| m.to_model_info()).collect())
        .unwrap_or_default()
}

/// All overlay entries for `provider`, the OAuth-provider-aware counterpart of
/// [`models_for`]: for [`OAuthProvider::KomaRun`] a SINGLE connection's tokens are
/// valid against BOTH koma.run catalogue-overlay endpoint keys (the base chat
/// endpoint AND `registry::KOMA_PREMIUM_CHAT_ENDPOINT`, which carries the
/// premium-tier models like `koma/peach`) — see `service::oauth::registry::meta`'s
/// docs on the two endpoints. This merges both lists into ONE candidate
/// catalogue, so every koma model is reachable from the one KomaRun OAuth login.
/// The catalogue identity is read first, then legacy chat endpoint entries are
/// appended, deduped by model id. This also preserves bundled OAuth rows while
/// providers migrate from chat keys to catalogue keys.
///
/// Shared capability lookup across provider catalogue aliases and tiers.
/// OAuth picker membership deliberately uses [`oauth_catalogue`] instead.
pub fn models_for_provider(provider: OAuthProvider) -> Vec<ModelInfo> {
    let base = crate::service::oauth::registry::meta(provider).chat_endpoint;
    let key = catalogue_key(base, Some(provider));
    let mut models = models_for(&key);
    let mut aliases = vec![base];
    if provider == OAuthProvider::KomaRun {
        aliases.push(crate::service::oauth::registry::KOMA_PREMIUM_CHAT_ENDPOINT);
    }
    for alias in aliases {
        if alias == key {
            continue;
        }
        for m in models_for(alias) {
            if !models.iter().any(|existing| existing.id == m.id) {
                models.push(m);
            }
        }
    }
    models
}

/// OAuth picker membership comes only from public OA responses (live refresh
/// or its disk cache). Keep bundled metadata and individually fetched details
/// available for capabilities without letting them add picker candidates.
pub fn oauth_catalogue(provider: OAuthProvider) -> Vec<ModelInfo> {
    let Some(lock) = OVERLAY.get() else {
        return Vec::new();
    };
    let Ok(guard) = lock.read() else {
        return Vec::new();
    };
    oa_membership(&guard.oa, provider)
}

/// Picker membership for one signed-in connection: the public list plus the
/// models only this account may use (private and token models for Koma).
///
/// Reading also asks for a quiet, throttled refresh of the account's rows, so
/// a model granted while the app is open shows up on a later read.
pub fn oauth_catalogue_for(conn_uuid: &str, provider: OAuthProvider) -> Vec<ModelInfo> {
    if provider == OAuthProvider::KomaRun {
        oa::refresh_account_in_background(conn_uuid);
    }
    oauth_catalogue_with_account(conn_uuid, provider)
}

/// [`oauth_catalogue_for`] without the refresh trigger.
fn oauth_catalogue_with_account(conn_uuid: &str, provider: OAuthProvider) -> Vec<ModelInfo> {
    let mut models = oauth_catalogue(provider);
    let Some(lock) = OVERLAY.get() else {
        return models;
    };
    let Ok(guard) = lock.read() else {
        return models;
    };
    for model in guard.accounts.get(conn_uuid).into_iter().flatten() {
        if !models.iter().any(|existing| existing.id == model.id) {
            models.push(model.to_model_info());
        }
    }
    models
}

/// Replace one connection's account rows. An empty list is authoritative:
/// a revoked grant must disappear.
pub(super) fn set_account_rows(conn_uuid: &str, rows: Vec<OverlayModel>) {
    let lock = OVERLAY.get_or_init(|| {
        RwLock::new(OverlayState {
            table: load_initial(),
            ..OverlayState::default()
        })
    });
    if let Ok(mut guard) = lock.write() {
        guard.accounts.insert(conn_uuid.to_string(), rows);
    }
}

pub use oa::refresh_account;

fn oa_membership(table: &OverlayTable, provider: OAuthProvider) -> Vec<ModelInfo> {
    let base = crate::service::oauth::registry::meta(provider).chat_endpoint;
    let key = catalogue_key(base, Some(provider));
    let mut keys = vec![key.as_str()];
    if provider == OAuthProvider::KomaRun && base != key {
        keys.push(base);
    }
    let mut models = Vec::<ModelInfo>::new();
    for key in keys {
        for model in table.get(key).into_iter().flatten() {
            if !models.iter().any(|existing| existing.id == model.id) {
                models.push(model.to_model_info());
            }
        }
    }
    models
}

/// Fast membership check: is `model_id` a premium-tier KomaRun model?
///
/// Returns true if the id is in the static premium overlay (e.g. `koma/peach`)
/// OR is not found in the base-tier overlay — i.e. it must have come from the
/// public OA catalogue and therefore routes to the premium endpoint. Used by `resolve.rs` to route requests to the premium endpoint
/// instead of the base endpoint.
pub fn is_premium_model(model_id: &str) -> bool {
    // 1. Explicit static premium overlay hit (koma/peach + curated premium ids).
    if models_for(crate::service::oauth::registry::KOMA_PREMIUM_CHAT_ENDPOINT)
        .iter()
        .any(|m| m.id == model_id)
    {
        return true;
    }
    // 2. Not a known base-tier KomaRun id → treat as premium.
    //    OA refresh splits tier metadata into the base/premium keys, so any
    //    id that isn't in the base overlay hits the premium chat endpoint.
    !models_for(
        crate::service::oauth::registry::meta(crate::model::app_config::OAuthProvider::KomaRun)
            .chat_endpoint,
    )
    .iter()
    .any(|m| m.id == model_id)
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;
