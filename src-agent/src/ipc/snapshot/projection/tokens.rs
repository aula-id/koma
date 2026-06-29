use crate::app::mode::agents::{AgentEditField, AgentScope, AgentSubMode};
use crate::app::mode::{UsageMetric, UsageView};
use crate::model::app_config::{ApiType, ModelRole, ThemeMode};

pub fn theme_token(t: &ThemeMode) -> &'static str {
    match t {
        ThemeMode::Dark => "dark",
        ThemeMode::Light => "light",
    }
}

pub fn api_type_token(t: ApiType) -> &'static str {
    match t {
        ApiType::OpenAiCompatible => "openai",
        ApiType::AnthropicCompatible => "anthropic",
    }
}

pub fn role_token(r: ModelRole) -> &'static str {
    match r {
        ModelRole::Main => "main",
        ModelRole::Awareness => "awareness",
        ModelRole::Safeguard => "safeguard",
        ModelRole::Compactor => "compactor",
    }
}

pub fn agent_submode_token(m: AgentSubMode) -> &'static str {
    match m {
        AgentSubMode::Browse => "browse",
        AgentSubMode::Edit => "edit",
        AgentSubMode::Create => "create",
        AgentSubMode::DeleteConfirm => "delete_confirm",
    }
}

pub fn agent_field_token(f: AgentEditField) -> &'static str {
    match f {
        AgentEditField::Name => "name",
        AgentEditField::Description => "description",
        AgentEditField::Conditions => "conditions",
        AgentEditField::Model => "model",
        AgentEditField::Tools => "tools",
        AgentEditField::Body => "prompt",
    }
}

pub fn agent_scope_token(s: AgentScope) -> &'static str {
    match s {
        AgentScope::Session => "session",
        AgentScope::Global => "global",
    }
}

pub fn usage_view_token(v: UsageView) -> &'static str {
    match v {
        UsageView::Global => "global",
        UsageView::Session => "session",
    }
}

pub fn usage_metric_token(m: UsageMetric) -> &'static str {
    match m {
        UsageMetric::Cost => "cost",
        UsageMetric::Tokens => "tokens",
    }
}
