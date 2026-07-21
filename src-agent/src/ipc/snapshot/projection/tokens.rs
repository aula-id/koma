use crate::app::mode::agents::{AgentEditField, AgentScope, AgentSubMode};
use crate::app::mode::extensions::ExtSubMode;
use crate::app::mode::help::HelpKind;
use crate::app::mode::mcp::{McpEditField, McpSubMode};
use crate::app::mode::settings::SettingsPage;
use crate::app::mode::store::StoreSubMode;
use crate::app::mode::{UsageMetric, UsageView};
use crate::model::app_config::{ApiType, McpTransport, ModelRole, ThemeMode};

pub fn settings_page_token(p: SettingsPage) -> &'static str {
    match p {
        SettingsPage::Menu         => "menu",
        SettingsPage::Appearance   => "appearance",
        SettingsPage::General      => "general",
        SettingsPage::Providers    => "providers",
        SettingsPage::ProviderForm => "provider_form",
        SettingsPage::OAuth        => "oauth",
        SettingsPage::Models       => "models",
        SettingsPage::ModelForm    => "model_form",
    }
}

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
        ApiType::Codex => "codex",
        ApiType::KomaFree => "koma_free",
        ApiType::CommandCode => "command_code",
    }
}

pub fn role_token(r: ModelRole) -> &'static str {
    match r {
        ModelRole::Main => "main",
        ModelRole::Awareness => "awareness",
        ModelRole::Safeguard => "safeguard",
        ModelRole::Compactor => "compactor",
        ModelRole::Planner => "planner",
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

pub fn mcp_submode_token(m: McpSubMode) -> &'static str {
    match m {
        McpSubMode::Browse => "browse",
        McpSubMode::Edit => "edit",
        McpSubMode::Create => "create",
        McpSubMode::DeleteConfirm => "delete_confirm",
    }
}

pub fn mcp_field_token(f: McpEditField) -> &'static str {
    match f {
        McpEditField::Name => "name",
        McpEditField::Enabled => "enabled",
        McpEditField::Transport => "transport",
        McpEditField::Command => "command",
        McpEditField::Args => "args",
        McpEditField::Env => "env",
        McpEditField::Url => "url",
    }
}

pub fn mcp_transport_token(t: McpTransport) -> &'static str {
    match t {
        McpTransport::Stdio => "stdio",
        McpTransport::Http => "http",
    }
}

pub fn ext_submode_token(m: ExtSubMode) -> &'static str {
    match m {
        ExtSubMode::Browse => "browse",
        ExtSubMode::Detail => "detail",
        ExtSubMode::UninstallConfirm => "uninstall_confirm",
    }
}

pub fn store_submode_token(m: StoreSubMode) -> &'static str {
    match m {
        StoreSubMode::Browse => "browse",
        StoreSubMode::Detail => "detail",
        StoreSubMode::InstallConfirm => "install_confirm",
    }
}

pub fn help_kind_token(k: HelpKind) -> &'static str {
    match k {
        HelpKind::Command => "command",
        HelpKind::Keybinding => "keybinding",
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
