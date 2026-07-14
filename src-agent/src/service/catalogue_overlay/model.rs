//! Types for the catalogue overlay table: one entry per non-OpenRouter model
//! (OAuth codex/claude/xai, direct APIs like deepseek), keyed by resolved
//! endpoint string in [`super::OverlayTable`].
//!
//! These providers don't expose an OpenRouter-style `GET /models` with
//! `reasoning`/`pricing` metadata, so this curated table fills that gap from a
//! bundled + remotely-refreshed JSON file (see `fetch.rs` and `mod.rs`).

use serde::{Deserialize, Serialize};

use crate::dto::openrouter::{ModelInfo, ModelReasoning};

/// Per-million-token USD pricing for one overlay model.
///
/// Unlike [`crate::dto::openrouter::ModelPricing`] (OpenRouter's per-token
/// decimal-string shape, e.g. `"0.00000015"`), overlay pricing is authored by
/// hand in the curated JSON as plain per-million-token floats (e.g. `15.0` ==
/// $15/M input tokens) — easier to write and read in the source file.
/// `cached` is the discounted per-million rate for prompt-cache hits; it
/// defaults to `0.0` when a model/provider doesn't offer cache pricing.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct OverlayPricing {
    pub input: f64,
    #[serde(default)]
    pub cached: f64,
    pub output: f64,
}

/// One curated model entry in the overlay table.
///
/// Mirrors the subset of [`ModelInfo`] this table can usefully author by hand:
/// `id` (matched by exact string equality against the model id in use),
/// `supported_parameters` (OpenRouter-style capability flags, empty when not
/// authored), `reasoning` (effort-menu capability), `context_length`, and
/// `pricing` in the overlay's own per-million-token shape (see
/// [`OverlayPricing`]). All metadata fields are optional — an entry can exist
/// purely to declare reasoning support, purely for pricing, or both.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct OverlayModel {
    pub id: String,
    #[serde(default)]
    pub supported_parameters: Vec<String>,
    #[serde(default)]
    pub reasoning: Option<ModelReasoning>,
    #[serde(default)]
    pub context_length: Option<u64>,
    #[serde(default)]
    pub pricing: Option<OverlayPricing>,
}

impl OverlayModel {
    /// Maps into the existing OpenRouter-shaped `ModelInfo` so it can feed the
    /// existing `effort_caps` / `context_length_for` resolvers unchanged.
    ///
    /// `name`, `top_provider`, and `architecture` are left `None` — this table
    /// doesn't author them. `pricing` is ALSO left `None` here: OpenRouter's
    /// `ModelPricing` is a per-token decimal-string shape, while
    /// [`OverlayModel::pricing`] is a per-million-token float shape; a future
    /// consumer that wants overlay pricing should read `OverlayModel::pricing`
    /// directly rather than through `ModelInfo.pricing`.
    pub fn to_model_info(&self) -> ModelInfo {
        ModelInfo {
            id: self.id.clone(),
            name: None,
            supported_parameters: self.supported_parameters.clone(),
            reasoning: self.reasoning.clone(),
            context_length: self.context_length,
            top_provider: None,
            pricing: None,
            architecture: None,
        }
    }
}

/// The overlay table: resolved endpoint string -> its curated model list.
/// Top-level shape of both the bundled default and the remotely-fetched
/// `models.json` (see `mod.rs`/`fetch.rs`).
pub type OverlayTable = std::collections::HashMap<String, Vec<OverlayModel>>;
