//! A [`ModelClient`](crate::room::lane::ModelClient) over the real provider.
//!
//! It streams a reply from `OpenRouterClient::stream_complete` and folds the
//! events into a [`ModelReply`], the same way the sub-agent engine does. Every
//! room tool is presented to the model as a function definition.
//!
//! The lane's per-avatar [`LaneModel`] selects the model, provider and effort.
//! A provider that names a configured connection is resolved with that
//! connection's endpoint and key. Anything else overrides the session's Main
//! route in place.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use tokio::sync::mpsc;

use crate::app::resolve::Resolved;
use crate::dto::openrouter::{ToolDef, ToolFunctionDef};
use crate::model::app_config::AppConfig;
use crate::model::settings::Settings;
use crate::room::lane::{ModelClient, ModelReply, ModelRequest, ToolSpec, Usage};
use crate::room::registry::LaneModel;
use crate::service::openrouter::OpenRouterClient;
use crate::service::StreamEvent;

pub struct ProviderModelClient {
    client: Arc<OpenRouterClient>,
    base: Resolved,
    config: AppConfig,
    settings: Settings,
    settings_cap: u32,
    context_window: Option<u64>,
}

impl ProviderModelClient {
    pub fn new(
        client: Arc<OpenRouterClient>,
        base: Resolved,
        config: AppConfig,
        settings: Settings,
        settings_cap: u32,
        context_window: Option<u64>,
    ) -> Self {
        Self { client, base, config, settings, settings_cap, context_window }
    }

    /// The lane's own provider connection when it names one, otherwise the
    /// session Main route with the lane's model, provider slug and effort applied.
    fn route_for(&self, model: &LaneModel) -> Resolved {
        if let Some(provider) = model.provider.as_deref().map(str::trim).filter(|name| !name.is_empty()) {
            if let Some(mut resolved) = crate::app::resolve::resolve_lane_route(
                &self.config,
                &self.settings,
                provider,
                model.model.as_deref(),
            ) {
                if let Some(effort) = model.effort.as_deref().filter(|effort| !effort.is_empty()) {
                    resolved.effort = effort.to_string();
                }
                return resolved;
            }
        }
        let mut route = self.base.clone();
        if let Some(id) = &model.model {
            route.model_id = id.clone();
        }
        if let Some(provider) = &model.provider {
            route.route = Some(provider.clone());
        }
        if let Some(effort) = &model.effort {
            route.effort = effort.clone();
        }
        route
    }
}

fn to_tool_def(spec: &ToolSpec) -> ToolDef {
    ToolDef {
        kind: "function".to_string(),
        function: ToolFunctionDef {
            name: spec.name.clone(),
            description: spec.description.clone(),
            parameters: spec.parameters.clone(),
        },
    }
}

impl ModelClient for ProviderModelClient {
    fn stream<'a>(&'a self, request: ModelRequest) -> BoxFuture<'a, ModelReply> {
        let route = self.route_for(&request.model);
        let client = Arc::clone(&self.client);
        let defs: Vec<ToolDef> = request.tools.iter().map(to_tool_def).collect();
        let advertise: Vec<String> = request.tools.iter().map(|t| t.name.clone()).collect();
        let history = request.history;
        let archive_dir = request.archive_dir;
        let objective = request.objective;
        let settings = self.settings.clone();
        let settings_cap = self.settings_cap;
        let context_window = self.context_window;
        Box::pin(async move {
            // DRSS: the same request shaping a coding session uses, from this
            // lane's own archive, so a long lane stays inside the window.
            let mut drss_active = false;
            let history = match archive_dir {
                Some(dir) if settings.short_send_enabled => {
                    let limits = crate::service::context_limits::discover(
                        &route.model_id,
                        &route.endpoint,
                        &settings.context_model_alias,
                        settings.context_window_limit,
                        &[],
                    )
                    .await;
                    let user = history.iter().rev().find(|m| m.role == crate::dto::chat::Role::User).map(|m| m.content.clone()).unwrap_or_default();
                    let goal = crate::app::runtime::shortsend::GoalWire { source: "room".into(), objective, charter: String::new() };
                    let schemas = crate::app::runtime::shortsend::budget::schema_tokens(&advertise, &defs);
                    match crate::app::runtime::shortsend::shape(history, &dir, &settings, &user, &goal, &limits, schemas) {
                        Ok(shaped) => {
                            drss_active = shaped.drss_active;
                            shaped.history
                        }
                        Err(error) => {
                            return ModelReply { error: Some(error.to_string()), ..Default::default() };
                        }
                    }
                }
                _ => history,
            };
            let prompt_est = crate::app::runtime::shortsend::estimate_prompt_tokens_for_max_clamp(&history);
            let max_tokens = crate::service::openrouter::effective_max_output_tokens(settings_cap, context_window, prompt_est);
            let (tx, mut rx) = mpsc::unbounded_channel();
            let model_id = route.model_id.clone();
            let provider = route.provider().to_string();
            let effort = route.effort.clone();
            let conn_route = route.clone();
            let send = tokio::spawn(async move {
                let conn = conn_route.conn();
                // Room tools are all presented as function defs; no koma-builtin-by-name set.
                let _ = client
                    .stream_complete(
                        conn, &model_id, &provider, &effort, history, &advertise, &defs, None, max_tokens, tx,
                    )
                    .await;
            });
            let mut reply = ModelReply::default();
            while let Some(event) = rx.recv().await {
                match event {
                    StreamEvent::Token(t) => reply.text.push_str(&t),
                    StreamEvent::Reasoning(r) => reply.reasoning.get_or_insert_with(String::new).push_str(&r),
                    StreamEvent::ToolCalls(calls) => reply.tool_calls = calls,
                    StreamEvent::Error(e) => reply.error = Some(e),
                    StreamEvent::Usage { prompt_tokens, completion_tokens, cost, .. } => {
                        reply.usage = Some(Usage { prompt_tokens, completion_tokens, cost });
                    }
                    _ => {}
                }
            }
            let _ = send.await;
            reply.drss_active = drss_active;
            reply
        })
    }
}
