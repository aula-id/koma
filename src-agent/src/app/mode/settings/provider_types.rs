//! Provider-related types: [`ProviderDraft`], [`ProviderModal`], [`new_uuid`], and
//! UI-only `impl` blocks for [`ApiType`] and [`ModelRole`].

pub use crate::model::app_config::{ApiType, ModelRole};

impl ModelRole {
    pub fn label(&self) -> &'static str {
        match self {
            ModelRole::Main      => "main",
            ModelRole::Awareness => "awareness",
            ModelRole::Safeguard => "safeguard",
            ModelRole::Compactor => "compactor",
            ModelRole::Planner   => "planner",
        }
    }

    pub const ALL: [ModelRole; 5] = [
        ModelRole::Main,
        ModelRole::Awareness,
        ModelRole::Safeguard,
        ModelRole::Compactor,
        ModelRole::Planner,
    ];
}

impl ApiType {
    /// Short label used in the providers table column.
    pub fn short_label(self) -> &'static str {
        match self {
            ApiType::OpenAiCompatible   => "OpenAI",
            ApiType::AnthropicCompatible => "Anthropic",
        }
    }

    /// Full human-readable label for the api type. The Anthropic variant is tagged
    /// "(not wired)" because native Anthropic is deferred (the type persists but no
    /// role routes to it yet — see [`ApiType::is_routable`]). Kept for
    /// forward-compat; the UI Type field was removed (new providers are always
    /// `OpenAiCompatible`).
    #[allow(dead_code)]
    pub fn full_label(self) -> &'static str {
        match self {
            ApiType::OpenAiCompatible   => "OpenAI compatible",
            ApiType::AnthropicCompatible => "Anthropic compatible (not wired)",
        }
    }

    /// Flip between the two variants. Kept for forward-compat; not called from
    /// the UI since the Type field was removed.
    #[allow(dead_code)]
    pub fn toggle(self) -> Self {
        match self {
            ApiType::OpenAiCompatible   => ApiType::AnthropicCompatible,
            ApiType::AnthropicCompatible => ApiType::OpenAiCompatible,
        }
    }
}

/// Mint a fresh random UUID (v4) as a `String`. Used when CREATING a new
/// provider/model draft in the UI so its identity is stable across the edit
/// session (before the first config save) and matches the persisted
/// [`crate::model::app_config`] uuid scheme.
pub fn new_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// One API provider entry, mirrored to/from a persisted
/// [`crate::model::app_config::ProviderConn`]. `uuid` carries the persisted
/// identity so a reorder/delete/edit round-trips without losing the
/// model→provider linkage.
#[derive(Clone, Debug)]
pub struct ProviderDraft {
    /// Persisted identity (matches the `ProviderConn` uuid). Minted on create.
    pub uuid: String,
    pub name: String,
    pub endpoint: String,
    pub api_type: ApiType,
    pub api_key: String,
}

/// State for the "Add API provider" modal overlay.
#[derive(Clone, Debug)]
pub struct ProviderModal {
    pub name: String,
    pub endpoint: String,
    pub api_type: ApiType,
    pub api_key: String,
    /// Active field: 0=name, 1=endpoint, 2=api_key, 3=Save button, 4=Cancel button.
    pub field: usize,
}

impl ProviderModal {
    pub fn new() -> Self {
        Self {
            name: String::new(),
            endpoint: String::new(),
            api_type: ApiType::OpenAiCompatible,
            api_key: String::new(),
            field: 0,
        }
    }
}
