//! Models-list and per-model endpoint types for the OpenRouter REST API.
//!
//! Covers the responses from:
//! - `GET /models` — drives the `/effort` capability menu.
//! - `GET /models/{author}/{slug}/endpoints` — per-provider endpoint details.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Models list (inbound, GET /models — drives the /effort capability menu)
// ---------------------------------------------------------------------------

/// The `reasoning` sub-object of a model entry in `GET /models`.
///
/// Both fields default so a model that omits one (or omits `reasoning`
/// entirely) still deserialises. `supported_efforts` is the list of effort
/// tokens the model accepts (e.g. `["high","low"]`); empty means the model
/// either takes no discrete efforts (on/off only) or none were reported.
/// `mandatory` is true when reasoning can't be turned off.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct ModelReasoning {
    #[serde(default)]
    pub mandatory: bool,
    #[serde(default)]
    pub supported_efforts: Vec<String>,
}

/// The `top_provider` sub-object of a model entry in `GET /models`.
///
/// OpenRouter exposes both a nominal `context_length` (the model's theoretical
/// maximum) and `top_provider.context_length` (the limit actually enforced by
/// the serving provider). The provider-served value is what matters for
/// summarisation thresholds; the nominal value is the fallback when this
/// object is absent.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct TopProvider {
    #[serde(default)]
    pub context_length: Option<u64>,
}

/// Per-token USD pricing for a model or endpoint. Fields are strings because
/// OpenRouter represents these as decimal strings (e.g. `"0.00000015"`).
// dead_code: UI layer will consume these fields once the models-select feature lands.
#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ModelPricing {
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub completion: Option<String>,
}

/// The `architecture` sub-object of a model entry in `GET /models`.
///
/// `input_modalities` is the list of input kinds the model accepts (e.g.
/// `["text","image"]`). A model can take images iff this contains `"image"`
/// (see [`crate::service::openrouter::model_takes_images`]). Defaults so a model
/// that omits the field (or the whole `architecture` object) still deserialises.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct Architecture {
    #[serde(default)]
    pub input_modalities: Vec<String>,
}

/// One model entry from `GET /models`. Only the fields the effort-capability
/// derivation needs are modelled; the rest of OpenRouter's rich model record is
/// ignored. `reasoning` is absent for models with no thinking support.
/// `context_length` is the model's maximum context window in tokens, taken from
/// the top-level field OpenRouter exposes on each model object.
/// `top_provider` carries the provider-served context limit, which takes
/// precedence over the nominal `context_length` when computing thresholds.
/// `name` is the human-readable display name; `pricing` is the per-token cost.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ModelInfo {
    pub id: String,
    // dead_code: consumed by models-select UI (not yet wired).
    #[allow(dead_code)]
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub supported_parameters: Vec<String>,
    #[serde(default)]
    pub reasoning: Option<ModelReasoning>,
    #[serde(default)]
    pub context_length: Option<u64>,
    #[serde(default)]
    pub top_provider: Option<TopProvider>,
    // dead_code: consumed by models-select UI (not yet wired).
    #[allow(dead_code)]
    #[serde(default)]
    pub pricing: Option<ModelPricing>,
    /// Input modalities the model accepts. Drives the image-capability gate:
    /// `architecture.input_modalities` contains `"image"` iff the model can read
    /// images. `Option` + `#[serde(default)]` so a model record without an
    /// `architecture` object still deserialises (treated as text-only).
    #[serde(default)]
    pub architecture: Option<Architecture>,
}

/// Top-level envelope of `GET /models`: `{ "data": [ ModelInfo, ... ] }`.
#[derive(Debug, Deserialize)]
pub struct ModelsResponse {
    pub data: Vec<ModelInfo>,
}

// ---------------------------------------------------------------------------
// Per-model provider endpoints  (`GET /models/{author}/{slug}/endpoints`)
// ---------------------------------------------------------------------------

/// One provider entry from `GET /models/{author}/{slug}/endpoints`.
// dead_code: consumed by models-select UI (not yet wired).
#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
pub struct ModelEndpoint {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub provider_name: Option<String>,
    #[serde(default)]
    pub pricing: Option<ModelPricing>,
    #[serde(default)]
    pub context_length: Option<u64>,
    #[serde(default)]
    pub quantization: Option<String>,
    #[serde(default)]
    pub max_completion_tokens: Option<u64>,
    #[serde(default)]
    pub uptime_last_30m: Option<f64>,
    #[serde(default)]
    pub status: Option<i64>,
}

/// Inner `data` object of [`EndpointsResponse`].
// dead_code: consumed by models-select UI (not yet wired).
#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
pub struct EndpointsData {
    #[serde(default)]
    pub endpoints: Vec<ModelEndpoint>,
}

/// Top-level envelope of `GET /models/{author}/{slug}/endpoints`:
/// `{ "data": { "endpoints": [ ModelEndpoint, ... ] } }`.
// dead_code: consumed by models-select UI (not yet wired).
#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
pub struct EndpointsResponse {
    pub data: EndpointsData,
}
