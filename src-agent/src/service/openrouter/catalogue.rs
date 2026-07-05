//! Model catalogue helpers: free functions for inspecting `GET /models` data,
//! and `OpenRouterClient` methods that fetch catalogue / endpoint listings.

use anyhow::{anyhow, Result};

use crate::dto::openrouter::{EndpointsResponse, ModelEndpoint, ModelInfo, ModelsResponse};

use super::helpers::{auth_headers_with_account, clean_error};
use super::client::OpenRouterClient;
use super::types::{Conn, EffortCaps};

/// Derive [`EffortCaps`] for `model_id` from a `GET /models` listing.
///
/// Matches the model by exact `id`. Reasoning is considered supported when the
/// model carries a `reasoning` object OR advertises `reasoning` /
/// `include_reasoning` in its `supported_parameters`. The effort list and the
/// mandatory flag come from the `reasoning` object when present. A model absent
/// from the listing yields `supported = false` so the caller can fall back.
pub fn effort_caps(models: &[ModelInfo], model_id: &str) -> EffortCaps {
    let Some(info) = models.iter().find(|m| m.id == model_id) else {
        return EffortCaps {
            supported: false,
            mandatory: false,
            efforts: Vec::new(),
        };
    };
    let has_param = info
        .supported_parameters
        .iter()
        .any(|p| p == "reasoning" || p == "include_reasoning");
    let supported = info.reasoning.is_some() || has_param;
    let efforts = info
        .reasoning
        .as_ref()
        .map(|r| r.supported_efforts.clone())
        .unwrap_or_default();
    let mandatory = info.reasoning.as_ref().map(|r| r.mandatory).unwrap_or(false);
    EffortCaps {
        supported,
        mandatory,
        efforts,
    }
}

/// Return the context-window size (tokens) for `model_id` from a `GET /models`
/// listing. Returns `None` when the model is absent from the listing or its
/// `context_length` field was not reported. The caller falls back to a hardcoded
/// default when `None` is returned — never panics.
///
/// Prefers `top_provider.context_length` (the limit the serving provider
/// actually enforces) over the nominal top-level `context_length` (the
/// model's theoretical maximum). Falls back to the nominal value when the
/// `top_provider` object is absent or its `context_length` is not reported.
pub fn context_length_for(models: &[ModelInfo], model_id: &str) -> Option<u64> {
    models
        .iter()
        .find(|m| m.id == model_id)
        .and_then(|model| {
            model
                .top_provider
                .as_ref()
                .and_then(|tp| tp.context_length)
                .or(model.context_length)
        })
}

/// Tri-state image-capability result from [`model_image_capability`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageCapability {
    /// The catalogue positively confirms the model supports image input.
    Supports,
    /// The catalogue positively confirms the model does NOT support image input
    /// (model found, but `architecture.input_modalities` lacks `"image"`).
    DoesNotSupport,
    /// The catalogue is unavailable, stale, or the model was not found — we
    /// cannot tell. Callers should treat this as "assume capable" for sending.
    Unknown,
}

/// Tri-state image-capability check for `model_id` against a `GET /models`
/// listing.
///
/// Distinguishes "definitely no" from "don't know" so callers can fail-open:
/// only block images when we have positive evidence the model can't read them.
/// When the catalogue is missing, stale, or the model isn't listed, returns
/// [`ImageCapability::Unknown`]. See also `ImageWireCtx` which carries the
/// resolved boolean into the streaming task.
pub fn model_image_capability(models: &[ModelInfo], model_id: &str) -> ImageCapability {
    let Some(info) = models.iter().find(|m| m.id == model_id) else {
        return ImageCapability::Unknown;
    };
    let Some(arch) = info.architecture.as_ref() else {
        return ImageCapability::Unknown;
    };
    if arch.input_modalities.iter().any(|m| m == "image") {
        ImageCapability::Supports
    } else {
        ImageCapability::DoesNotSupport
    }
}

impl OpenRouterClient {
    /// Fetch the OpenRouter model catalogue (`GET /models`) on the connection
    /// `conn` (its `endpoint` + `api_key`).
    ///
    /// Drives the `/effort` capability menu: the returned [`ModelInfo`] list is
    /// passed to [`effort_caps`] to decide which options the current model
    /// supports. The endpoint needs no auth, but we send the bearer header
    /// anyway for consistency with the other calls. Returns the `data` array;
    /// clean errors, no panics. Callers treat any `Err` as "capabilities
    /// unknown" and report that rather than opening a guessed capability menu.
    pub async fn list_models(&self, conn: Conn<'_>) -> Result<Vec<ModelInfo>> {
        // Refresh hook so an OAuth-backed catalogue connection (e.g. Kilo Code)
        // sends a live bearer + its org header; non-OAuth conns pass through.
        let (bearer, acct) =
            crate::service::oauth::manager::fresh_key(conn.oauth_uuid, conn.api_key).await;
        let effective_account = if !acct.is_empty() { Some(acct.as_str()) } else { None };
        let url = format!("{}/models", conn.endpoint);
        let response = auth_headers_with_account(self.http.get(&url), &conn, &bearer, effective_account, self.codex_session_id())
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow!("{}", clean_error(status, &text)));
        }

        let models: ModelsResponse = response.json().await?;
        Ok(models.data)
    }

    /// Fetch the provider endpoint list for a single model
    /// (`GET /models/{model_id}/endpoints`) on the connection `conn` (its
    /// `endpoint` + `api_key`).
    ///
    /// `model_id` is the slash-separated `author/slug` string as returned by
    /// [`Self::list_models`] (e.g. `"openai/gpt-4o-mini"`). The slash is
    /// already the correct path separator for the OpenRouter URL, so the string
    /// is interpolated verbatim: `{endpoint}/models/openai/gpt-4o-mini/endpoints`.
    pub async fn list_model_endpoints(
        &self,
        conn: Conn<'_>,
        model_id: &str,
    ) -> Result<Vec<ModelEndpoint>> {
        let (bearer, acct) =
            crate::service::oauth::manager::fresh_key(conn.oauth_uuid, conn.api_key).await;
        let effective_account = if !acct.is_empty() { Some(acct.as_str()) } else { None };
        let url = format!("{}/models/{}/endpoints", conn.endpoint, model_id);
        let response = auth_headers_with_account(self.http.get(&url), &conn, &bearer, effective_account, self.codex_session_id())
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow!("{}", clean_error(status, &text)));
        }

        let endpoints: EndpointsResponse = response.json().await?;
        Ok(endpoints.data.endpoints)
    }
}
