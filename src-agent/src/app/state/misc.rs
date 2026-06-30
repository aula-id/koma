//! Miscellaneous methods on [`super::AppStateRest`]: credentials and model-catalogue
//! requests. (Toast management moved onto [`super::SessionRuntime`] in C6.)

use super::rest::AppStateRest;
use super::types::CataloguePending;

impl AppStateRest {
    pub fn remember_creds(&mut self, key: &str, model: &str, provider: &str) {
        self.last_key = Some(key.to_string());
        self.last_model = Some(model.to_string());
        self.last_provider = Some(provider.to_string());
    }

    /// Request the model catalogue for `endpoint` (debounced).
    ///
    /// The model omnisearch calls this on every query keystroke / provider change
    /// / field focus for whatever provider is being edited. It is cheap and
    /// idempotent:
    /// - empty `endpoint` → no-op (nothing to fetch against);
    /// - the cache already holds this endpoint (`models_cache_endpoint` matches,
    ///   including the terminal empty-result state) → no-op (filter locally);
    /// - a fetch for this endpoint is already in flight → no-op (don't double-fire);
    /// - otherwise (re)arm a pending fetch ~300 ms out. Calling it again on the
    ///   next keystroke pushes `due` forward, collapsing a typing burst into one
    ///   request fired only once the user pauses.
    ///
    /// The event-loop tick fires the pending fetch once `due` passes; the result
    /// lands via the `warm_rx` drain, which sets `models_cache` +
    /// `models_cache_endpoint` and clears `catalogue_fetching`.
    pub fn request_catalogue(&mut self, endpoint: &str, api_key: &str) {
        if endpoint.is_empty() {
            return;
        }
        if self.models_cache_endpoint.as_deref() == Some(endpoint) {
            return; // already have this endpoint's catalogue (or terminal empty)
        }
        if self.catalogue_fetching.as_deref() == Some(endpoint) {
            return; // already fetching this endpoint
        }
        self.catalogue_pending = Some(CataloguePending {
            endpoint: endpoint.to_string(),
            api_key: api_key.to_string(),
            due: std::time::Instant::now() + std::time::Duration::from_millis(300),
        });
    }
}
