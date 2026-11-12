//! A [`ModelClient`](crate::room::lane::ModelClient) over the real provider.
//!
//! It streams a reply from `OpenRouterClient::stream_complete` and folds the
//! events into a [`ModelReply`], the same way the sub-agent engine does. Every
//! room tool is presented to the model as a function definition.
//!
//! The lane's per-avatar [`LaneModel`] overrides the model, provider and effort
//! on a base route; the credentials (endpoint, key, OAuth) stay the base
//! session's. Two avatars on different providers with different credentials
//! would need a route resolved per provider, which this does not yet do.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use tokio::sync::mpsc;

use crate::app::resolve::Resolved;
use crate::dto::openrouter::{ToolDef, ToolFunctionDef};
use crate::room::lane::{ModelClient, ModelReply, ModelRequest, ToolSpec, Usage};
use crate::room::registry::LaneModel;
use crate::service::openrouter::OpenRouterClient;
use crate::service::StreamEvent;

pub struct ProviderModelClient {
    client: Arc<OpenRouterClient>,
    base: Resolved,
    settings_cap: u32,
    context_window: Option<u64>,
}

impl ProviderModelClient {
    pub fn new(client: Arc<OpenRouterClient>, base: Resolved, settings_cap: u32, context_window: Option<u64>) -> Self {
        Self { client, base, settings_cap, context_window }
    }

    /// The base route with the lane's model, provider and effort applied.
    fn route_for(&self, model: &LaneModel) -> Resolved {
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
        let prompt_est = crate::app::runtime::shortsend::estimate_prompt_tokens_for_max_clamp(&history);
        let max_tokens = crate::service::openrouter::effective_max_output_tokens(
            self.settings_cap,
            self.context_window,
            prompt_est,
        );
        Box::pin(async move {
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
                    StreamEvent::ToolCalls(calls) => reply.tool_calls = calls,
                    StreamEvent::Error(e) => reply.error = Some(e),
                    StreamEvent::Usage { prompt_tokens, completion_tokens, cost, .. } => {
                        reply.usage = Some(Usage { prompt_tokens, completion_tokens, cost });
                    }
                    _ => {}
                }
            }
            let _ = send.await;
            reply
        })
    }
}
