//! PRE-SESSION (swapper) config-apply helpers for the GUI host-relay layer —
//! split out of `host.rs` for file size (pure code motion, no behaviour change).
//!
//! [`push_swapper_config`]/[`apply_swapper_config_mutation`] serve the swapper's
//! Connector/MCP panels (no attached daemon to source config from); both funnel
//! through [`apply_global_config_req`], which mirrors — for the pre-session path
//! — exactly what the daemon's `hub::requests` dispatch does for these
//! `ClientRequest` variants, reusing the SAME config-layer setters/parsers.

use crate::ipc::proto::ClientRequest;

use super::host_catalogue::build_host_agents_values;
use super::project_config::{push_config, ConfigProjection};
use super::push_loop;
use super::push_proto::push_agents_values;

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
    // GUI /agents mutations (`SetAgent` / `DeleteAgent`) write agent `.md` files, not
    // `config.json`, so they take a dedicated pre-session path handled BEFORE the config
    // path (it never touches `cfg`): apply the GLOBAL-scope subset directly, then re-push a
    // host-built `AgentsValues` so the dashboard refreshes and the webview never hangs on a
    // spinner — even when the mutation was a session-scoped no-op (no session dir here).
    if apply_swapper_agent_mutation(req) {
        let (agents, catalogue_models, catalogue_providers) = build_host_agents_values();
        push_agents_values(
            push,
            agents,
            catalogue_models,
            catalogue_providers,
            crate::tool::agent_selectable_tools(),
        );
        return;
    }
    let mut cfg = crate::model::app_config::AppConfig::load();
    if apply_global_config_req(&mut cfg, req) {
        if let Err(e) = cfg.save() {
            eprintln!("[gui] pre-session config save failed: {e}");
        }
        let projection = ConfigProjection::from_app_config(&cfg);
        push_config(Some(&projection), push, push_state);
    }
}

/// Apply the GLOBAL-scope subset of a GUI /agents mutation (`SetAgent` / `DeleteAgent`)
/// directly to the on-disk agent registry while PRE-SESSION (no attached daemon). Returns
/// `true` iff `req` WAS an agent mutation — the caller then re-pushes a host-built
/// `AgentsValues` (so the dashboard refreshes and never hangs), whether or not anything was
/// actually written.
///
/// Only the GLOBAL tier is reachable pre-session (there is no session dir): a session-scoped
/// create, a session / built-in-source edit, or a built-in override all resolve to "session"
/// and are clean no-ops here; a built-in DELETE is rejected. Mirrors the daemon's
/// `set_agent` / `delete_agent` for the global tier, minus `rebuild_system` (no session).
/// EDIT vs CREATE follows the SAME derived-scope rule (`original_name` present = derive from
/// the existing def's source; absent = honour the wire `scope`), so only a Global-source edit
/// is global-writable here.
fn apply_swapper_agent_mutation(req: &ClientRequest) -> bool {
    use crate::model::agent_def::{
        delete_agent, load_registry, save_agent, AgentDef, AgentScope, AgentSource,
    };
    match req {
        ClientRequest::SetAgent {
            original_name,
            scope,
            name,
            description,
            conditions,
            model_uuid,
            tools,
            prompt,
        } => {
            let is_edit = original_name.is_some();
            let lookup = original_name.clone().unwrap_or_else(|| name.clone());
            // `None` — the swapper has no session, so only built-in + global agents load.
            let registry = load_registry(None);
            let existing = registry.get(&lookup).cloned();
            // Only a GLOBAL write is reachable pre-session. CREATE: honour a "global" wire
            // scope. EDIT: derive from source — only a Global-source agent is global-writable
            // (a session / built-in source needs a session dir we don't have → clean no-op).
            let write_global = if is_edit {
                existing.as_ref().map(|d| d.source) == Some(AgentSource::Global)
            } else {
                scope == "global"
            };
            if write_global {
                let mut def = if is_edit {
                    existing.unwrap_or_default()
                } else {
                    AgentDef::default()
                };
                def.name = name.clone();
                def.description = description.trim().to_string();
                def.conditions = conditions.trim().to_string();
                def.model_uuid = model_uuid.clone();
                def.model = None;
                def.provider = None;
                def.provider_uuid = None;
                def.tools = tools.clone();
                def.prompt = prompt.clone();
                def.source = AgentSource::Global;
                def.file_path = None;
                if let Err(e) = save_agent(AgentScope::Global, &def) {
                    eprintln!("[gui] pre-session agent save failed: {e}");
                } else if let Some(orig) = original_name.as_deref() {
                    // Rename: drop the OLD global file (same tier) after the new one landed.
                    if orig != name.as_str() {
                        if let Err(e) = delete_agent(AgentScope::Global, orig) {
                            eprintln!("[gui] pre-session agent rename left old file {orig}: {e}");
                        }
                    }
                }
            }
            true
        }
        ClientRequest::DeleteAgent { scope, name } => {
            // Built-in delete rejected; only the GLOBAL tier is writable pre-session.
            let registry = load_registry(None);
            let is_builtin =
                registry.get(name).map(|d| d.source) == Some(AgentSource::Builtin);
            if scope == "global" && !is_builtin {
                if let Err(e) = delete_agent(AgentScope::Global, name) {
                    eprintln!("[gui] pre-session agent delete failed: {e}");
                }
            }
            true
        }
        _ => false,
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
                source_uuid: None,
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
