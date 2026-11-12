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
    /// The coding session this room belongs to, whose usage the lanes' calls are recorded under.
    usage_session: Option<(String, String)>,
}

/// How a lane's spend is filed in the usage ledger: the Chief is the main role, a minion is a `sub:` role.
pub(super) fn usage_role(avatar: &str) -> String {
    if avatar == "chief" {
        "main".to_string()
    } else {
        format!("sub:{avatar}")
    }
}

/// The cost of a call: the provider's own figure, or the catalogue's price for the tokens when it reports none.
pub(super) fn effective_cost(endpoint: &str, model_id: &str, prompt: u64, cached: u64, completion: u64, reported: f64) -> f64 {
    if reported != 0.0 {
        return reported;
    }
    crate::service::catalogue_overlay::overlay_cost(endpoint, model_id, prompt, cached, completion).unwrap_or(reported)
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
        Self { client, base, config, settings, settings_cap, context_window, usage_session: None }
    }

    /// Record every call into the usage ledger under this coding session.
    pub fn with_usage(mut self, session_uuid: String, pwd_hash: String) -> Self {
        self.usage_session = Some((session_uuid, pwd_hash));
        self
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

/// The tool list a request carries: the built-in names to add (none) and the definitions.
/// Every room tool, host tools included, goes out as an explicit definition. A names list
/// would make the provider layer add the built-in version of the same tool again, and a
/// provider rejects two definitions of one function.
pub(super) fn wire_tools(tools: &[ToolSpec]) -> (Vec<String>, Vec<ToolDef>) {
    (Vec::new(), tools.iter().map(to_tool_def).collect())
}

impl ModelClient for ProviderModelClient {
    fn stream<'a>(&'a self, request: ModelRequest) -> BoxFuture<'a, ModelReply> {
        let route = self.route_for(&request.model);
        let client = Arc::clone(&self.client);
        let (advertise, defs) = wire_tools(&request.tools);
        let history = request.history;
        let sink = request.deltas;
        let archive_dir = request.archive_dir;
        // Images a lane has loaded live in its own directory, so that is where the wire reads them.
        // A model the catalogue does not rule out is assumed to take images, as in a cold chat session.
        let image_ctx = archive_dir.clone().map(|dir| crate::dto::openrouter::ImageWireCtx { session_dir: dir, model_takes_images: true });
        let objective = request.objective;
        let avatar = request.avatar;
        let usage_session = self.usage_session.clone();
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
                        conn, &model_id, &provider, &effort, history, &advertise, &defs, image_ctx, max_tokens, tx,
                    )
                    .await;
            });
            let mut reply = ModelReply::default();
            let mut call_usage: Option<(u64, u64, u64, f64)> = None;
            let mut live = Live::new(sink);
            while let Some(event) = rx.recv().await {
                match event {
                    StreamEvent::Token(t) => {
                        live.text(&t);
                        reply.text.push_str(&t);
                    }
                    StreamEvent::Reasoning(r) => {
                        live.reasoning(&r);
                        reply.reasoning.get_or_insert_with(String::new).push_str(&r);
                    }
                    StreamEvent::ToolCalls(calls) => reply.tool_calls = calls,
                    StreamEvent::Error(e) => reply.error = Some(e),
                    StreamEvent::Usage { prompt_tokens, completion_tokens, cached_tokens, cost } => {
                        reply.usage = Some(Usage { prompt_tokens, completion_tokens, cost });
                        call_usage = Some((prompt_tokens, cached_tokens, completion_tokens, cost));
                    }
                    _ => {}
                }
            }
            live.flush();
            let _ = send.await;
            // Every lane's call is spend like the coding chat's, so it goes in the ledger.
            if let (Some((uuid, pwd_hash)), Some((prompt, cached, completion, reported))) = (&usage_session, call_usage) {
                let cost = effective_cost(&route.endpoint, &route.model_id, prompt, cached, completion, reported);
                crate::model::usage::record_usage(&route.model_id, &usage_role(&avatar), uuid, pwd_hash, prompt, cached, completion, cost);
            }
            reply.drss_active = drss_active;
            reply
        })
    }
}

/// How often live pieces are handed on. A token at a time would flood the socket.
const LIVE_MS: u128 = 60;

/// Gathers streamed text and thinking and reports them in batches.
struct Live {
    sink: Option<crate::room::lane::DeltaSink>,
    text: String,
    reasoning: String,
    last: std::time::Instant,
}

impl Live {
    fn new(sink: Option<crate::room::lane::DeltaSink>) -> Self {
        Self { sink, text: String::new(), reasoning: String::new(), last: std::time::Instant::now() }
    }

    fn text(&mut self, piece: &str) {
        self.text.push_str(piece);
        self.maybe_flush();
    }

    fn reasoning(&mut self, piece: &str) {
        self.reasoning.push_str(piece);
        self.maybe_flush();
    }

    fn maybe_flush(&mut self) {
        if self.last.elapsed().as_millis() >= LIVE_MS {
            self.flush();
        }
    }

    fn flush(&mut self) {
        self.last = std::time::Instant::now();
        let Some(sink) = &self.sink else { return };
        // Thinking comes before the reply it led to.
        if !self.reasoning.is_empty() {
            sink(crate::room::lane::Delta::Reasoning(std::mem::take(&mut self.reasoning)));
        }
        if !self.text.is_empty() {
            sink(crate::room::lane::Delta::Text(std::mem::take(&mut self.text)));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::room::lane::Delta;

    #[test]
    fn live_batches_pieces_and_puts_thinking_before_text() {
        let seen: Arc<Mutex<Vec<Delta>>> = Arc::new(Mutex::new(Vec::new()));
        let sink_seen = Arc::clone(&seen);
        let mut live = Live::new(Some(Arc::new(move |d| sink_seen.lock().unwrap().push(d))));
        live.text("Hel");
        live.reasoning("th");
        live.text("lo");
        live.reasoning("ink");
        assert!(seen.lock().unwrap().is_empty(), "pieces within the interval are held back");
        live.flush();
        assert_eq!(
            *seen.lock().unwrap(),
            vec![Delta::Reasoning("think".into()), Delta::Text("Hello".into())]
        );
        live.flush();
        assert_eq!(seen.lock().unwrap().len(), 2, "an empty flush sends nothing");
    }

    #[test]
    fn every_lane_files_its_spend_under_its_name() {
        assert_eq!(usage_role("chief"), "main");
        assert_eq!(usage_role("red"), "sub:red");
        assert_eq!(usage_role("red-web"), "sub:red-web");
        // The provider's own figure wins; with none, an unknown model costs nothing rather than guessing.
        assert_eq!(effective_cost("https://example.invalid", "no-such-model", 100, 0, 50, 0.0123), 0.0123);
        assert_eq!(effective_cost("https://example.invalid", "no-such-model", 100, 0, 50, 0.0), 0.0);
    }

    #[test]
    fn live_without_a_sink_is_silent() {
        let mut live = Live::new(None);
        live.text("x");
        live.flush();
    }
}
