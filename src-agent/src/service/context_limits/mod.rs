//! Public OpenRouter capability discovery, independent of dispatch credentials.
//! Context matching never changes the model ID sent to a provider.
mod catalogue;
mod matching;

use crate::dto::openrouter::ModelInfo;
use serde::{Deserialize, Serialize};

pub const OPERATING_CEILING: u64 = 300_000;
pub const FALLBACK_WINDOW: u64 = 128_000;
pub const OUTPUT_MARGIN: u64 = 1_024;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CatalogModel {
    pub id: String,
    #[serde(default)]
    pub canonical_slug: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub context_length: Option<u64>,
    #[serde(default)]
    pub top_provider: TopProvider,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct TopProvider {
    pub context_length: Option<u64>,
    pub max_completion_tokens: Option<u64>,
}

impl CatalogModel {
    fn window(&self) -> Option<u64> {
        self.top_provider
            .context_length
            .filter(|n| *n > 0)
            .or(self.context_length.filter(|n| *n > 0))
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ContextLimits {
    pub requested_model: String,
    pub matched_model: Option<String>,
    pub match_method: String,
    pub catalogue_source: String,
    pub catalog_window: Option<u64>,
    pub route_window: Option<u64>,
    pub effective_window: u64,
    pub max_completion_tokens: Option<u64>,
}

impl ContextLimits {
    pub fn desired_output(&self, configured: u32) -> u64 {
        let desired = if configured == 0 {
            32_000
        } else {
            u64::from(configured)
        };
        self.max_completion_tokens
            .filter(|n| *n > 0)
            .map_or(desired, |n| desired.min(n))
    }

    /// Reserve a useful reply even on small windows. Extra free room can still
    /// be used at dispatch, up to the requested/provider output cap.
    pub fn reserved_output(&self, configured: u32) -> u64 {
        self.desired_output(configured)
            .min((self.effective_window / 5).max(1))
    }

    pub fn output_tokens(&self, configured: u32, prompt: u64) -> anyhow::Result<u32> {
        let room = self
            .effective_window
            .saturating_sub(prompt)
            .saturating_sub(OUTPUT_MARGIN);
        let minimum = self.desired_output(configured).min(4096);
        anyhow::ensure!(room >= minimum,
            "Insufficient context headroom: estimated prompt {prompt}, effective window {}. Conversation preserved; reduce input or configure a verified context limit.", self.effective_window);
        Ok(self
            .desired_output(configured)
            .min(room)
            .min(u64::from(u32::MAX)) as u32)
    }
}

/// The native catalogue is usable only when the caller has verified its endpoint.
/// Public OR metadata is primary; smaller known native/user limits still win.
pub fn resolve(
    requested: &str,
    endpoint: &str,
    alias: &str,
    configured_limit: u64,
    public: &[CatalogModel],
    native: &[ModelInfo],
) -> ContextLimits {
    let query = if alias.trim().is_empty() {
        requested
    } else {
        alias.trim()
    };
    let found = matching::find(query, endpoint, public);
    let native_models: Vec<_> = native
        .iter()
        .map(|m| CatalogModel {
            id: m.id.clone(),
            name: m.name.clone().unwrap_or_default(),
            context_length: m.context_length,
            top_provider: TopProvider {
                context_length: m.top_provider.as_ref().and_then(|p| p.context_length),
                max_completion_tokens: None,
            },
            ..Default::default()
        })
        .collect();
    let route = matching::find(requested, endpoint, &native_models);
    let route_window = route.model.and_then(CatalogModel::window);
    let catalog_window = found.model.and_then(CatalogModel::window);
    let mut window = catalog_window.or(route_window).unwrap_or(FALLBACK_WINDOW);
    // A fuzzy spelling guess or ambiguous name must not inflate the fallback.
    if found.uncertain {
        window = window.min(FALLBACK_WINDOW);
    }
    if let Some(n) = found.conservative_window {
        window = window.min(n);
    }
    if let Some(n) = route_window {
        window = window.min(n);
    }
    if configured_limit > 0 {
        window = window.min(configured_limit);
    }
    ContextLimits {
        requested_model: requested.into(),
        matched_model: found.model.map(|m| m.id.clone()),
        match_method: if alias.trim().is_empty() {
            found.method.into()
        } else {
            format!("configured_alias/{}", found.method)
        },
        catalogue_source: if found.model.is_some() {
            "openrouter"
        } else if route_window.is_some() {
            "active_provider"
        } else {
            "fallback"
        }
        .into(),
        catalog_window,
        route_window,
        effective_window: window.min(OPERATING_CEILING),
        max_completion_tokens: found
            .model
            .and_then(|m| m.top_provider.max_completion_tokens)
            .filter(|n| *n > 0),
    }
}

pub async fn discover(
    requested: &str,
    endpoint: &str,
    alias: &str,
    configured_limit: u64,
    native: &[ModelInfo],
) -> ContextLimits {
    let models = catalogue::models().await;
    let mut result = resolve(
        requested,
        endpoint,
        alias,
        configured_limit,
        &models,
        native,
    );
    if result.matched_model.is_none() {
        let query = if alias.trim().is_empty() {
            requested
        } else {
            alias
        };
        if let Some(model) = catalogue::lookup_alias(query, endpoint).await {
            // The server resolved this alias; bind that identity explicitly.
            result = resolve(
                requested,
                endpoint,
                &model.id,
                configured_limit,
                &[model.clone()],
                native,
            );
            result.match_method = "openrouter_alias".into();
        }
    }
    result
}

#[cfg(test)]
mod tests;
