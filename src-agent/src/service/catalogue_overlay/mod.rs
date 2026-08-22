//! Curated model-metadata overlay for non-OpenRouter providers.
//!
//! OAuth providers (codex/claude/xai) and direct APIs (e.g. deepseek) don't
//! expose an OpenRouter-style `GET /models` endpoint carrying `reasoning` and
//! `pricing` metadata — there's simply no wire call that returns it. This
//! module fills that gap with a hand-curated table, keyed by the resolved
//! endpoint string (see `service::oauth::registry`'s `meta().chat_endpoint`
//! for how those strings are produced), so a future consumer can feed
//! `effort_caps`/pricing lookups for these providers the same way it already
//! does for OpenRouter models.
//!
//! Loading order: bundled default (compiled into the binary via
//! `include_str!`) -> on-disk cache (`~/.koma/models.json`, if present and
//! valid) -> background-refreshed from a GitHub release asset. See `fetch.rs`
//! for the refresh policy (TTL + ETag).
//!
//! NOT WIRED INTO ANY CONSUMER YET — this is infra + a lookup API only. No
//! effort-menu UI, no usage/cost code reads from here.

mod fetch;
mod model;

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
static OVERLAY: OnceLock<RwLock<OverlayTable>> = OnceLock::new();

/// The overlay table compiled into the binary as a fallback for a fresh
/// install (no `~/.koma/models.json` cache yet) or a corrupt cache. Lives at
/// the repo root as a sibling of `version.json`.
const BUNDLED_DEFAULT: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../models.json"));

/// Call ONCE at startup (mirrors `model::store::migrate_legacy_dir`): loads
/// the overlay table (a fresh on-disk cache if present and valid, else the
/// bundled default compiled into the binary), then kicks off a non-blocking
/// background refresh against the GitHub release asset.
///
/// Never panics: a malformed cache or bundled file degrades to an empty table
/// rather than aborting startup.
pub fn init() {
    let table = load_initial();
    let _ = OVERLAY.set(RwLock::new(table));
    fetch::spawn_refresh();
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
    match serde_json::from_str::<OverlayTable>(BUNDLED_DEFAULT) {
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
            *guard = table;
        }
    }
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
/// Consumer: the GUI/TUI model-id suggestion paths (`ListModels` hub handler,
/// its un-attached `host.rs` mirror, and the TUI's `candidate_model_ids`) fall
/// back to this for OAuth-conn providers (Codex/Claude have no live `/models`
/// endpoint at all) and for any live fetch that comes back empty.
pub fn models_for(endpoint: &str) -> Vec<ModelInfo> {
    let Some(lock) = OVERLAY.get() else {
        return Vec::new();
    };
    let Ok(guard) = lock.read() else {
        return Vec::new();
    };
    guard
        .get(endpoint)
        .map(|v| v.iter().map(|m| m.to_model_info()).collect())
        .unwrap_or_default()
}

/// All overlay entries for `provider`, the OAuth-provider-aware counterpart of
/// [`models_for`]: for [`OAuthProvider::KomaRun`] a SINGLE connection's tokens are
/// valid against BOTH koma.run catalogue-overlay endpoint keys (the base chat
/// endpoint AND `registry::KOMA_PREMIUM_CHAT_ENDPOINT`, which carries the
/// premium-tier models like `koma/peach`) — see `service::oauth::registry::meta`'s
/// docs on the two endpoints. This merges both lists (base first, premium
/// appended, deduped by model id in case a future overlay update lists the same
/// id under both keys) into ONE candidate catalogue, so every koma model is
/// reachable from the one KomaRun OAuth login. Every other provider is a
/// pass-through to [`models_for`] on its single `registry::meta(provider).chat_endpoint`.
///
/// The single merge implementation every candidate-list call site (onboard model
/// select, the GUI Connector model picker, the daemon `ListModels` handler, the
/// `/effort` capability lookup) should call instead of `models_for(meta(p).chat_endpoint)`
/// directly, so a future third koma tier only needs a change here.
pub fn models_for_provider(provider: OAuthProvider) -> Vec<ModelInfo> {
    let base = crate::service::oauth::registry::meta(provider).chat_endpoint;
    if provider != OAuthProvider::KomaRun {
        return models_for(base);
    }
    let mut models = models_for(base);
    let seen: std::collections::HashSet<String> = models.iter().map(|m| m.id.clone()).collect();
    for m in models_for(crate::service::oauth::registry::KOMA_PREMIUM_CHAT_ENDPOINT) {
        if !seen.contains(&m.id) {
            models.push(m);
        }
    }
    models
}

/// Fast membership check: is `model_id` a premium-tier KomaRun model?
///
/// Returns true if the id is in the static premium overlay (e.g. `koma/peach`)
/// OR is not found in the base-tier overlay — i.e. it must have come from the
/// live `GET …/koma-premium/models` fetch and therefore routes to the premium
/// endpoint. Used by `resolve.rs` to route requests to the premium endpoint
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
    //    Live catalogue is sole-sourced from koma-premium/models, so any id
    //    that isn't in the base overlay must hit the premium chat endpoint.
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
