//! One-shot `DaemonEvent` -> `PushEnvelope` re-push intercepts for [`super::push_loop`]'s
//! attached fold loop. Split out of `push_loop.rs` for file size — PURE code motion, no
//! behaviour change: same checks, same `push` calls, in the same relative order among
//! themselves (each `if let` matches a DISTINCT, mutually-exclusive `DaemonEvent` variant,
//! so at most one branch ever fires per frame — their order relative to EACH OTHER is
//! inert; only their order relative to `apply_frame`, preserved by the caller running this
//! BEFORE it, matters).
//!
//! Each of these daemon replies is a non-visual one-shot the fold treats as a no-op (seq
//! stays gap-free), so it must be re-pushed to JS as its own envelope BEFORE `apply_frame`
//! folds the frame — otherwise the reply would be silently swallowed.

use crate::ipc::proto::DaemonEvent;

use super::push_proto::{PushEnvelope, PushRoute};

/// Check `frame.event` against every one-shot reply variant the GUI re-pushes as its own
/// `PushEnvelope` ahead of the fold, pushing whichever one matches (at most one — see the
/// module doc). Does NOT touch the `Snapshot` config-cache (that stays inline in
/// `push_loop`, which owns the loop-local `current_config`) and does NOT call
/// `apply_frame` (the caller does that right after, unchanged).
pub(super) fn repush_before_fold(frame: &crate::ipc::proto::DaemonFrame, push: &dyn Fn(String)) {
    // Omnisearch reply: intercept the one-shot `FileSearchResults` and re-push it to JS as
    // a `SearchResults` envelope BEFORE folding (the fold treats it as a non-visual no-op,
    // keeping the seq gap-free).
    if let DaemonEvent::FileSearchResults { query, items } = &frame.event {
        let env = PushEnvelope::SearchResults {
            query: query.clone(),
            items: items.clone(),
        };
        if let Ok(json) = serde_json::to_string(&env) {
            push(json);
        }
    }
    // Live model-id catalogue reply (Connector model picker): re-push it as a `ModelList`
    // envelope BEFORE folding (the fold treats it as a non-visual no-op, keeping the seq
    // gap-free).
    if let DaemonEvent::ModelList { provider, models } = &frame.event {
        let env = PushEnvelope::ModelList {
            provider: provider.clone(),
            models: models.clone(),
        };
        if let Ok(json) = serde_json::to_string(&env) {
            push(json);
        }
    }
    // Live provider-route reply (Connector ModelForm route picker): re-push it as a
    // `RouteList` envelope BEFORE folding (a non-visual fold no-op), flattening each wire
    // route to the camelCase `PushRoute` JS contract.
    if let DaemonEvent::ModelRoutes { provider, model_id, routes } = &frame.event {
        let env = PushEnvelope::RouteList {
            provider: provider.clone(),
            model_id: model_id.clone(),
            routes: routes
                .iter()
                .map(|r| PushRoute {
                    name: r.name.clone(),
                    provider_name: r.provider_name.clone(),
                    price_prompt: r.price_prompt.clone(),
                    price_completion: r.price_completion.clone(),
                    uptime_last_30m: r.uptime_last_30m,
                })
                .collect(),
        };
        if let Ok(json) = serde_json::to_string(&env) {
            push(json);
        }
    }
    // GUI Settings-tab reply (GetSettings / post-SetSessionPrefs re-push): re-push it as a
    // `SettingsValues` envelope BEFORE folding (a non-visual fold no-op, keeping the seq
    // gap-free), same as the ModelList/RouteList intercepts above.
    if let DaemonEvent::SettingsValues {
        name,
        workdir,
        short_send,
        sliding_cache,
        bash_saving,
        coding_autosave,
        internet_mode,
        palette,
        effort,
    } = &frame.event
    {
        let env = PushEnvelope::SettingsValues {
            name: name.clone(),
            workdir: workdir.clone(),
            short_send: *short_send,
            sliding_cache: *sliding_cache,
            bash_saving: *bash_saving,
            coding_autosave: *coding_autosave,
            internet_mode: internet_mode.clone(),
            palette: palette.clone(),
            effort: effort.clone(),
        };
        if let Ok(json) = serde_json::to_string(&env) {
            push(json);
        }
    }
    // GUI /agents-dashboard reply (GetAgents / post-SetAgent / -DeleteAgent re-push):
    // re-push it as an `AgentsValues` envelope BEFORE folding (a non-visual fold no-op,
    // keeping the seq gap-free), same as the SettingsValues intercept above.
    if let DaemonEvent::AgentsValues {
        req_seq,
        agents,
        catalogue_models,
        catalogue_providers,
        available_tools,
    } = &frame.event
    {
        let env = PushEnvelope::AgentsValues {
            req_seq: *req_seq,
            agents: agents.clone(),
            catalogue_models: catalogue_models.clone(),
            catalogue_providers: catalogue_providers.clone(),
            available_tools: available_tools.clone(),
        };
        if let Ok(json) = serde_json::to_string(&env) {
            push(json);
        }
    }
    // Daemon agent-mutation result (SetAgent/DeleteAgent from requests_agents.rs):
    // re-push as an `AgentOp` envelope BEFORE folding (a non-visual fold no-op,
    // keeping the seq gap-free). The authoritative success reply is `AgentsValues`,
    // so this is only sent on failure — the GUI surfaces the error as a toast and
    // clears saving state.
    if let DaemonEvent::AgentOp { ok, error, req_seq } = &frame.event {
        let env = PushEnvelope::AgentOp {
            ok: *ok,
            error: error.clone(),
            req_seq: *req_seq,
        };
        if let Ok(json) = serde_json::to_string(&env) {
            push(json);
        }
    }
    // Composer EFFORT-picker reply (GetEffortOptions): re-push it as an `EffortOptions`
    // envelope BEFORE folding (a non-visual fold no-op, keeping the seq gap-free), same as
    // the SettingsValues intercept above.
    if let DaemonEvent::EffortOptions {
        options,
        selected,
        note,
        state,
    } = &frame.event
    {
        let env = PushEnvelope::EffortOptions {
            options: options.clone(),
            selected: *selected,
            note: note.clone(),
            state: state.clone(),
        };
        if let Ok(json) = serde_json::to_string(&env) {
            push(json);
        }
    }
    // Streaming GUI OAuth reply (GetOAuthState / StartOAuth progress / SubmitOAuthPaste /
    // CancelOAuth / DeleteOAuthConn): re-push it as an `OAuthState` envelope BEFORE folding
    // (a non-visual fold no-op, keeping the seq gap-free), same as the
    // SettingsValues/AgentsValues intercepts.
    if let DaemonEvent::OAuthState {
        phase,
        url,
        user_code,
        verification_url,
        error,
        conns,
        providers,
    } = &frame.event
    {
        let env = PushEnvelope::OAuthState {
            phase: phase.clone(),
            url: url.clone(),
            user_code: user_code.clone(),
            verification_url: verification_url.clone(),
            error: error.clone(),
            conns: conns.clone(),
            providers: providers.clone(),
        };
        if let Ok(json) = serde_json::to_string(&env) {
            push(json);
        }
    }
    // GUI extension-STORE replies (StoreBrowse / StoreDetail / ListInstalledExtensions /
    // Install / Uninstall): re-push each as its own envelope BEFORE folding (a non-visual
    // fold no-op, keeping the seq gap-free), same as the OAuthState intercept. The nested
    // wire structs are re-embedded verbatim (they carry their own camelCase serde), so the
    // re-push is a straight field clone.
    if let DaemonEvent::StoreCatalogue { items, error } = &frame.event {
        let env = PushEnvelope::StoreCatalogue {
            items: items.clone(),
            error: error.clone(),
        };
        if let Ok(json) = serde_json::to_string(&env) {
            push(json);
        }
    }
    if let DaemonEvent::StoreItemDetail { detail, error } = &frame.event {
        let env = PushEnvelope::StoreItemDetail {
            detail: detail.as_ref().clone(),
            error: error.clone(),
        };
        if let Ok(json) = serde_json::to_string(&env) {
            push(json);
        }
    }
    if let DaemonEvent::InstalledExtensions { items } = &frame.event {
        let env = PushEnvelope::InstalledExtensions {
            items: items.clone(),
        };
        if let Ok(json) = serde_json::to_string(&env) {
            push(json);
        }
    }
    if let DaemonEvent::ExtensionOpResult { id, ok, error } = &frame.event {
        let env = PushEnvelope::ExtensionOpResult {
            id: id.clone(),
            ok: *ok,
            error: error.clone(),
        };
        if let Ok(json) = serde_json::to_string(&env) {
            push(json);
        }
    }
    // W8 panel bridge: re-push the panel.msg reply + the unsolicited daemon→panel push as their
    // own envelopes BEFORE folding (each a non-visual fold no-op, keeping the seq gap-free), same
    // clone-through as `ExtensionOpResult`. The `payload` rides as an arbitrary JSON value; the
    // GUI push injection re-encodes the whole envelope via `serde_json::to_string` before
    // `evaluate_script` (see `gui::mod`), so no manual escaping is needed here.
    if let DaemonEvent::ExtPanelReply { ext_id, panel_id, req_id, ok, payload, error } = &frame.event {
        let env = PushEnvelope::ExtPanelReply {
            ext_id: ext_id.clone(),
            panel_id: panel_id.clone(),
            req_id: req_id.clone(),
            ok: *ok,
            payload: payload.clone(),
            error: error.clone(),
        };
        if let Ok(json) = serde_json::to_string(&env) {
            push(json);
        }
    }
    if let DaemonEvent::ExtPanelPush { ext_id, panel_id, payload } = &frame.event {
        let env = PushEnvelope::ExtPanelPush {
            ext_id: ext_id.clone(),
            panel_id: panel_id.clone(),
            payload: payload.clone(),
        };
        if let Ok(json) = serde_json::to_string(&env) {
            push(json);
        }
    }
    // Generic daemon-to-GUI error: re-push as an AgentOp envelope so the
    // GUI surfaces it as an error toast and clears any pending saving state.
    // Any DaemonEvent::Error not handled by a more specific intercept above
    // reaches here too — better visible than silently consumed by the shadow's
    // non-visual frame filter. No req_seq (0) means no request correlation.
    if let DaemonEvent::Error(msg) = &frame.event {
        let env = PushEnvelope::AgentOp { ok: false, error: Some(msg.clone()), req_seq: 0 };
        if let Ok(json) = serde_json::to_string(&env) {
            push(json);
        }
    }
    // One-shot MCP status reply: re-push as a `McpStatus` envelope BEFORE folding
    // (a non-visual fold no-op, keeping the seq gap-free).
    if let DaemonEvent::McpStatus { request_id, servers, global_error } = &frame.event {
        let env = PushEnvelope::McpStatus {
            request_id: request_id.clone(),
            servers: servers
                .iter()
                .map(|s| super::push_proto::PushMcpStatusServer {
                    id: s.id.clone(),
                    connected: s.connected,
                    tool_count: s.tool_count,
                    error: s.error.clone(),
                })
                .collect(),
            global_error: global_error.clone(),
        };
        if let Ok(json) = serde_json::to_string(&env) {
            push(json);
        }
    }
}
