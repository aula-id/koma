//! The only projections from private config structs to public DTOs. Keys,
//! tokens, sealed values and endpoint credentials never leave these
//! functions; callers serialize the DTOs, never the private structs.
use crate::app::resolve::RoleResolution;
use crate::app::runtime::sdk::dto;
use crate::model::app_config::{ApiType, AppConfig, ModelRole, OAuthConn, ProviderConn, SecretId};

/// Configured secrets shorter than this are not scrubbed from text: a short
/// value would also match ordinary words.
const MIN_SECRET_LEN: usize = 8;
const REDACTED: &str = "[redacted]";

/// The public name of `role`, or `None` for a role outside the five public ones.
pub fn public_role(role: ModelRole) -> Option<&'static str> {
    Some(match role {
        ModelRole::Main => "main",
        ModelRole::Awareness => "awareness",
        ModelRole::Safeguard => "safeguard",
        ModelRole::Compactor => "compactor",
        ModelRole::Planner => "planner",
        ModelRole::CyberRed | ModelRole::CyberBlue | ModelRole::CyberResearch => return None,
    })
}

pub fn api_type(api_type: ApiType) -> &'static str {
    match api_type {
        ApiType::OpenAiCompatible => "openai",
        ApiType::AnthropicCompatible => "anthropic",
        ApiType::Codex => "codex",
        ApiType::KomaFree => "koma_free",
        ApiType::CommandCode => "command_code",
    }
}

/// `endpoint` without URL userinfo, query or fragment, and whether anything
/// was removed. An endpoint with nothing to remove is returned as written.
pub fn strip_endpoint(endpoint: &str) -> (String, bool) {
    if let Ok(mut url) = url::Url::parse(endpoint) {
        let credentials = !url.username().is_empty() || url.password().is_some();
        if !credentials && url.query().is_none() && url.fragment().is_none() {
            return (endpoint.to_string(), false);
        }
        let cleared = url.set_username("").is_ok() && url.set_password(None).is_ok();
        url.set_query(None);
        url.set_fragment(None);
        if cleared {
            return (url.to_string(), true);
        }
    }
    let stripped = strip_text_endpoint(endpoint);
    let changed = stripped != endpoint;
    (stripped, changed)
}

/// The fallback for strings that are not URLs: drop a `scheme://user:pass@`
/// prefix, then cut at the first `?` or `#`.
fn strip_text_endpoint(endpoint: &str) -> String {
    let without_userinfo = strip_userinfo(endpoint);
    let end = without_userinfo
        .find(['?', '#'])
        .unwrap_or(without_userinfo.len());
    without_userinfo[..end].to_string()
}

/// `text` with the userinfo of a leading `scheme://user:pass@host` removed.
fn strip_userinfo(text: &str) -> String {
    let Some(scheme_end) = text.find("://") else {
        return text.to_string();
    };
    let authority_start = scheme_end + 3;
    let rest = &text[authority_start..];
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    match rest[..authority_end].rfind('@') {
        Some(at) => format!("{}{}", &text[..authority_start], &rest[at + 1..]),
        None => text.to_string(),
    }
}

pub fn provider(config: &AppConfig, provider: &ProviderConn) -> dto::Provider {
    let unreadable = config
        .kept_unreadable
        .iter()
        .any(|(id, _)| matches!(id, SecretId::Provider(uuid) if *uuid == provider.uuid));
    let has_key = !provider.api_key.is_empty();
    let (endpoint, endpoint_redacted) = strip_endpoint(&provider.endpoint);
    dto::Provider {
        provider_uuid: provider.uuid.clone(),
        name: provider.name.clone(),
        api_type: api_type(provider.api_type),
        endpoint,
        endpoint_redacted,
        managed: provider.ext_id.is_some(),
        has_key,
        key_state: if has_key {
            "set"
        } else if unreadable {
            "unreadable"
        } else {
            "missing"
        },
        model_count: config
            .models
            .iter()
            .filter(|model| model.provider_uuid == provider.uuid)
            .count(),
    }
}

pub fn oauth(conn: &OAuthConn) -> dto::OAuthConn {
    dto::OAuthConn {
        oauth_uuid: conn.uuid.clone(),
        name: conn.name.clone(),
        provider: conn.provider.wire_id(),
        has_token: !conn.access_token.is_empty(),
        expires_at: conn.expires_at,
    }
}

/// The public route of one resolution; `None` for a role outside the five
/// public ones. `provider_name` looks a provider or OAuth uuid up by name.
pub fn role_route(
    resolution: &RoleResolution,
    provider_name: impl Fn(&str) -> Option<String>,
) -> Option<dto::RoleRoute> {
    Some(dto::RoleRoute {
        role: public_role(resolution.role)?,
        origin: resolution.origin.clone(),
        model_uuid: resolution.assignment_uuid.clone(),
        source_uuid: resolution.source_uuid.clone(),
        configured_model: resolution.configured_model.clone(),
        provider_uuid: resolution.provider_uuid.clone(),
        provider_name: resolution.provider_uuid.as_deref().and_then(provider_name),
        effective_model: resolution.effective_model.clone(),
        reason: resolution.reason.clone(),
    })
}

/// The five public routes, in role order. A role the source did not report
/// reads as unassigned.
pub fn role_routes(
    resolutions: &[RoleResolution],
    provider_name: impl Fn(&str) -> Option<String>,
) -> Vec<dto::RoleRoute> {
    [
        ModelRole::Main,
        ModelRole::Awareness,
        ModelRole::Safeguard,
        ModelRole::Compactor,
        ModelRole::Planner,
    ]
    .into_iter()
    .filter_map(|role| match resolutions.iter().find(|r| r.role == role) {
        Some(resolution) => role_route(resolution, &provider_name),
        None => Some(dto::RoleRoute {
            role: public_role(role)?,
            origin: "default".into(),
            model_uuid: None,
            source_uuid: None,
            configured_model: None,
            provider_uuid: None,
            provider_name: None,
            effective_model: None,
            reason: Some("unassigned".into()),
        }),
    })
    .collect()
}

/// The name of the provider or OAuth connection with `uuid` in `config`.
pub fn provider_name(config: &AppConfig, uuid: &str) -> Option<String> {
    config
        .providers
        .iter()
        .find(|p| p.uuid == uuid)
        .map(|p| p.name.clone())
        .or_else(|| {
            config
                .oauth_conns
                .iter()
                .find(|c| c.uuid == uuid)
                .map(|c| c.name.clone())
        })
}

/// Every configured secret value of at least [`MIN_SECRET_LEN`] bytes: provider
/// keys, OAuth tokens, web search keys and MCP env values.
pub fn secret_values(config: &AppConfig) -> Vec<String> {
    let mut values: Vec<String> = Vec::new();
    let mut push = |value: &str| {
        if value.len() >= MIN_SECRET_LEN && !values.iter().any(|v| v == value) {
            values.push(value.to_string());
        }
    };
    for provider in &config.providers {
        push(&provider.api_key);
    }
    for conn in &config.oauth_conns {
        push(&conn.access_token);
        push(&conn.refresh_token);
        push(&conn.id_token);
    }
    push(&config.web_search.firecrawl_key.0);
    push(&config.web_search.tavily_key.0);
    push(&config.web_search.exa_key.0);
    for server in &config.mcp_servers {
        for (_, value) in &server.env {
            push(value);
        }
    }
    for (_, ciphertext) in &config.kept_unreadable {
        push(ciphertext);
    }
    // Longest first, so a value that contains another is replaced whole.
    values.sort_by_key(|v| std::cmp::Reverse(v.len()));
    values
}

/// `text` with every value in `secrets` and the userinfo of every URL
/// replaced by `[redacted]`. Used on text that passes through from a daemon.
pub fn scrub(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_string();
    for secret in secrets {
        if out.contains(secret.as_str()) {
            out = out.replace(secret.as_str(), REDACTED);
        }
    }
    scrub_userinfo(&out)
}

/// Replace `user:pass` in every `scheme://user:pass@host` of `text`.
fn scrub_userinfo(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let authority_start = at + 3;
        out.push_str(&rest[..authority_start]);
        let tail = &rest[authority_start..];
        let authority_end = tail
            .find(|c: char| c == '/' || c == '?' || c == '#' || c.is_whitespace())
            .unwrap_or(tail.len());
        match tail[..authority_end].rfind('@') {
            Some(userinfo_end) => {
                out.push_str(REDACTED);
                rest = &tail[userinfo_end..];
            }
            None => rest = tail,
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_credentials_query_and_fragment_are_removed() {
        let cases = [
            (
                "https://api.example.com/v1",
                "https://api.example.com/v1",
                false,
            ),
            ("http://127.0.0.1:8080", "http://127.0.0.1:8080", false),
            (
                "https://user:hunter2@proxy.example.com/v1",
                "https://proxy.example.com/v1",
                true,
            ),
            (
                "https://token@proxy.example.com/v1",
                "https://proxy.example.com/v1",
                true,
            ),
            (
                "https://proxy.example.com/v1?key=abc#frag",
                "https://proxy.example.com/v1",
                true,
            ),
            ("not a url://u:p@host/x?y", "not a url://host/x", true),
            ("plain-text?api_key=1", "plain-text", true),
        ];
        for (input, expected, redacted) in cases {
            assert_eq!(
                strip_endpoint(input),
                (expected.to_string(), redacted),
                "{input}"
            );
        }
    }

    #[test]
    fn scrub_replaces_configured_values_and_userinfo_only() {
        let secrets = vec!["sk-configured-123456".to_string()];
        let text = "failed with sk-configured-123456 at https://bob:pw@h.example/x; sk-other";
        assert_eq!(
            scrub(text, &secrets),
            "failed with [redacted] at https://[redacted]@h.example/x; sk-other"
        );
    }

    #[test]
    fn short_values_are_not_collected() {
        let mut config = AppConfig::default();
        config.providers.push(ProviderConn {
            uuid: "p".into(),
            api_key: "short".into(),
            ..ProviderConn::default()
        });
        config.providers.push(ProviderConn {
            uuid: "q".into(),
            api_key: "long-enough-key".into(),
            ..ProviderConn::default()
        });
        assert_eq!(secret_values(&config), vec!["long-enough-key".to_string()]);
    }
}
