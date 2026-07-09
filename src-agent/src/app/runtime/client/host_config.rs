//! PRE-SESSION (swapper) config-apply helpers for the GUI host-relay layer —
//! split out of `host.rs` for file size (pure code motion, no behaviour change).
//!
//! [`push_swapper_config`]/[`apply_swapper_config_mutation`] serve the swapper's
//! Connector/MCP panels (no attached daemon to source config from); both funnel
//! through [`apply_global_config_req`], which mirrors — for the pre-session path
//! — exactly what the daemon's `hub::requests` dispatch does for these
//! `ClientRequest` variants, reusing the SAME config-layer setters/parsers.

use crate::ipc::proto::ClientRequest;

use super::project_config::{push_config, ConfigProjection};
use super::push_loop;

/// Read the loaded GLOBAL config off disk and push a `Config` envelope so the GUI's
/// Connector + MCP panels show the real providers/models/mcp while the host is in the
/// SWAPPER (bug #3/#4). The swapper holds no daemon connection/snapshot, so the attached
/// `push_loop`'s snapshot-sourced Config push never runs there; without this the panels
/// cold-open EMPTY even though `~/.koma/config.json` has providers/models. `push_config`
/// dedups on `push_state.config_json`, so callers `reset()` first to force a re-emit.
pub(super) fn push_swapper_config(push: &dyn Fn(String), push_state: &mut push_loop::PushState) {
    let cfg = crate::model::app_config::AppConfig::load();
    let projection = ConfigProjection::from_app_config(&cfg);
    push_config(Some(&projection), push, push_state);
}

/// Apply a PRE-SESSION config mutation (a [`HostCtl::ConfigMutate`]) directly to
/// `~/.koma/config.json` and re-push a fresh `Config` envelope.
///
/// The swapper/onboarding state has no attached daemon, so the theme + provider + model
/// setters onboarding drives can't ride the normal `live_req` → daemon path. Instead the
/// host loads the on-disk config, applies the config-GLOBAL subset via [`apply_global_config_req`],
/// persists it, and re-pushes so the Connector panels + the live theme repaint and the
/// `needsOnboarding` flag clears the instant a provider + Main model land. `push_config`
/// dedups on the last-pushed JSON, so an unchanged config emits nothing.
pub(super) fn apply_swapper_config_mutation(
    req: &ClientRequest,
    push: &dyn Fn(String),
    push_state: &mut push_loop::PushState,
) {
    let mut cfg = crate::model::app_config::AppConfig::load();
    if apply_global_config_req(&mut cfg, req) {
        if let Err(e) = cfg.save() {
            eprintln!("[gui] pre-session config save failed: {e}");
        }
        let projection = ConfigProjection::from_app_config(&cfg);
        push_config(Some(&projection), push, push_state);
    }
}

/// Apply the config-GLOBAL subset of a [`ClientRequest`] to an in-memory [`AppConfig`],
/// returning `true` if it mutated `cfg` (the caller then persists + re-pushes).
///
/// This mirrors — for the PRE-SESSION swapper path — exactly what the daemon's
/// `dispatch_request` does for these variants (see
/// `runtime::event_loop::daemon::hub::requests`), reusing the SAME config-layer setters
/// (`upsert_provider`/`upsert_model`/`upsert_mcp_server`/…) and the SAME MCP arg/env
/// parsers, so the on-disk result is identical whether a setter runs attached or during
/// onboarding. Session-scoped operations (`SetModel { scope:"local" }`, `SetSessionMain`,
/// the MCP live-reconnect) have no session/manager pre-session and are treated as no-ops
/// / config-write-only here. Any non-config request returns `false` untouched.
fn apply_global_config_req(
    cfg: &mut crate::model::app_config::AppConfig,
    req: &ClientRequest,
) -> bool {
    use crate::model::app_config::{McpServerEntry, McpTransport, ModelEntry, ModelRole};
    match req {
        ClientRequest::SetTheme { name } => {
            cfg.palette = name.clone();
            true
        }
        ClientRequest::SetProvider {
            uuid,
            name,
            endpoint,
            api_key,
        } => {
            cfg.upsert_provider(
                uuid.clone(),
                name.trim().to_string(),
                endpoint.trim().to_string(),
                api_key.clone(),
            );
            true
        }
        ClientRequest::DeleteProvider { uuid } => {
            cfg.remove_provider_by_uuid(uuid);
            true
        }
        ClientRequest::SetModel {
            uuid,
            name,
            model_id,
            provider_uuid,
            route,
            roles,
            scope,
        } => {
            // Pre-session there is no foreground session to hold a LOCAL override, so a
            // `local`-scope model can't be applied here — only the GLOBAL catalogue.
            if scope == "local" {
                return false;
            }
            let roles: Vec<ModelRole> = roles
                .iter()
                .filter_map(|r| match r.as_str() {
                    "main" => Some(ModelRole::Main),
                    "awareness" => Some(ModelRole::Awareness),
                    "safeguard" => Some(ModelRole::Safeguard),
                    "compactor" => Some(ModelRole::Compactor),
                    "planner" => Some(ModelRole::Planner),
                    _ => None,
                })
                .collect();
            cfg.upsert_model(ModelEntry {
                uuid: uuid.clone().unwrap_or_default(),
                name: name.trim().to_string(),
                model_id: model_id.trim().to_string(),
                provider_uuid: provider_uuid.clone(),
                route: ModelEntry::normalize_route(route.clone()),
                roles,
                role: None,
            });
            true
        }
        ClientRequest::DeleteModel { uuid, scope } => {
            if scope == "local" {
                return false;
            }
            cfg.remove_model_by_uuid(uuid);
            true
        }
        ClientRequest::SetMcpServer {
            uuid,
            name,
            enabled,
            transport,
            command,
            args,
            env,
            url,
        } => {
            cfg.upsert_mcp_server(McpServerEntry {
                uuid: uuid.clone().unwrap_or_default(),
                name: name.trim().to_string(),
                enabled: *enabled,
                transport: if transport == "http" {
                    McpTransport::Http
                } else {
                    McpTransport::Stdio
                },
                command: command.trim().to_string(),
                args: crate::app::mode::mcp::parse_args(args),
                env: crate::app::mode::mcp::parse_env(env),
                url: url.trim().to_string(),
            });
            true
        }
        ClientRequest::DeleteMcpServer { uuid } => {
            cfg.remove_mcp_server_by_uuid(uuid);
            true
        }
        ClientRequest::EnableMcpServer { uuid, enabled } => {
            cfg.set_mcp_enabled_by_uuid(uuid, *enabled);
            true
        }
        // GUI onboarding "koma free" (pre-session): mint/reuse the keyless Koma Free
        // provider + Main model directly in the on-disk config — the SAME
        // `ensure_koma_free_config` the daemon + TUI paths use, so the entries are
        // identical whether this runs attached or during onboarding. Returning `true`
        // makes the caller persist + re-push `Config`, which clears `firstRun`.
        ClientRequest::SetupKomaFree => {
            crate::service::koma_free::ensure_koma_free_config(cfg);
            true
        }
        _ => false,
    }
}
