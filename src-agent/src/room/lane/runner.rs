//! The lane loop: stream a model reply, run the tools it asks for, feed the
//! results back, and loop until the model stops or the step budget runs out.
//!
//! This is a deliberate clone of the structure in `app/subagent/engine.rs`, not
//! a reuse of it: a work lane has different needs (room tools routed to the
//! coordinator and shelf, a control channel for pause and redirect, a role
//! prompt from the registry), and cloning keeps the room from bending into the
//! sub-agent module's shape or breaking when it changes.
//!
//! The loop talks to the model and to koma's built-in tools only through the
//! [`ModelClient`] and [`ToolRouter`] seams, so the whole thing runs under a
//! scripted model and the real coordinator in tests, with no live provider.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::dto::chat::{ChatMessage, Role, ToolCall};
use crate::room::registry::LaneModel;

use super::event::LaneEvent;

/// Which kind of lane this is. A chat lane answers the user in short turns; a
/// work lane runs one assignment to completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneKind {
    Chat,
    Work,
}

/// A tool offered to the model.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Token and cost accounting for one model step.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cost: f64,
}

/// One model call.
pub struct ModelRequest {
    pub history: Vec<ChatMessage>,
    pub tools: Vec<ToolSpec>,
    pub model: LaneModel,
}

/// One model reply.
#[derive(Debug, Clone, Default)]
pub struct ModelReply {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Option<Usage>,
    /// A fatal error ends the lane.
    pub error: Option<String>,
}

/// The seam to the model provider. The real implementation wraps
/// `OpenRouterClient::stream_complete`; tests script replies.
pub trait ModelClient: Send + Sync {
    fn stream<'a>(&'a self, request: ModelRequest) -> BoxFuture<'a, ModelReply>;
}

/// The result of running one tool call.
#[derive(Debug, Clone, Default)]
pub struct ToolRun {
    /// The text fed back to the model.
    pub result: String,
    /// Events this call produced, forwarded as the lane's own (a shelf publish,
    /// a submit, a clerk flag).
    pub events: Vec<LaneEvent>,
}

/// The seam to the tools. One router is built per lane; it routes the room's
/// own tools (`group_*`, shelf, `clerk_flag`) to the coordinator and shelf, and
/// everything else to koma's built-in tools.
pub trait ToolRouter: Send + Sync {
    /// The tools advertised to the model for this lane.
    fn advertised(&self) -> Vec<ToolSpec>;
    /// Run one tool call.
    fn run(&self, call: &ToolCall) -> ToolRun;
}

/// A message pushed into a running lane from outside.
#[derive(Debug, Clone, PartialEq)]
pub enum LaneInput {
    /// Steering text folded in as a user turn at the next boundary.
    Steer(String),
    /// Stop at the next boundary and wait for `Resume`.
    Pause,
    Resume,
    /// End the lane now (at the next boundary).
    Stop,
}

/// How to seed and bound a lane.
pub struct LaneConfig {
    pub avatar: String,
    pub kind: LaneKind,
    /// The system prompt (role prompt plus any context).
    pub system: String,
    /// The first user turn: the assignment, the goal, or the user's message.
    pub task: String,
    pub model: LaneModel,
    pub max_steps: usize,
}

fn emit(events: &UnboundedSender<LaneEvent>, event: LaneEvent) {
    let _ = events.send(event);
}

/// Drain whatever is waiting on the input channel into `history`, updating the
/// pause flag. Returns `false` when the lane was told to stop.
fn drain_inputs(
    inputs: &mut UnboundedReceiver<LaneInput>,
    history: &mut Vec<ChatMessage>,
    paused: &mut bool,
) -> bool {
    use tokio::sync::mpsc::error::TryRecvError;
    loop {
        match inputs.try_recv() {
            Ok(LaneInput::Steer(text)) => history.push(ChatMessage::new(Role::User, text)),
            Ok(LaneInput::Pause) => *paused = true,
            Ok(LaneInput::Resume) => *paused = false,
            Ok(LaneInput::Stop) => return false,
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return true,
        }
    }
}

/// Run a lane to completion. Emits [`LaneEvent`]s as it goes and returns when
/// the model gives a reply with no tool calls, the step budget is spent, the
/// model errors, or a [`LaneInput::Stop`] arrives.
pub async fn run_lane(
    config: LaneConfig,
    model: Arc<dyn ModelClient>,
    router: Arc<dyn ToolRouter>,
    mut inputs: UnboundedReceiver<LaneInput>,
    events: UnboundedSender<LaneEvent>,
) {
    let mut history = vec![
        ChatMessage::new(Role::System, config.system.clone()),
        ChatMessage::new(Role::User, config.task.clone()),
    ];
    let advertised = router.advertised();
    let mut paused = false;
    let mut last_text = String::new();
    let mut step: usize = 0;

    loop {
        if !drain_inputs(&mut inputs, &mut history, &mut paused) {
            emit(&events, LaneEvent::Finished { report: stopped_report(&last_text) });
            return;
        }
        // A paused lane waits at this boundary until resumed or stopped.
        while paused {
            match inputs.recv().await {
                Some(LaneInput::Resume) => paused = false,
                Some(LaneInput::Stop) | None => {
                    emit(&events, LaneEvent::Finished { report: stopped_report(&last_text) });
                    return;
                }
                Some(LaneInput::Steer(text)) => history.push(ChatMessage::new(Role::User, text)),
                Some(LaneInput::Pause) => {}
            }
        }

        emit(&events, LaneEvent::Step(step));
        let reply = model
            .stream(ModelRequest { history: history.clone(), tools: advertised.clone(), model: config.model.clone() })
            .await;

        if let Some(error) = reply.error {
            emit(&events, LaneEvent::Failed { error });
            return;
        }
        if !reply.text.trim().is_empty() {
            last_text = reply.text.clone();
            if config.kind == LaneKind::Chat {
                emit(&events, LaneEvent::Chat(reply.text.clone()));
            }
        }

        if reply.tool_calls.is_empty() {
            // No tools: the lane has said its piece. A chat lane has answered; a
            // work lane that reaches here without submitting is left for the
            // reconciler to read as unresolved.
            emit(&events, LaneEvent::Finished { report: reply.text });
            return;
        }

        history.push(ChatMessage::assistant_with_tools(reply.text.clone(), reply.tool_calls.clone()));
        for call in &reply.tool_calls {
            emit(&events, LaneEvent::ToolStarted { name: call.function.name.clone() });
            let run = router.run(call);
            let ok = !run.result.starts_with("error:") && !run.result.starts_with("blocked");
            emit(&events, LaneEvent::ToolDone { name: call.function.name.clone(), ok });
            for event in run.events {
                emit(&events, event);
            }
            history.push(ChatMessage::tool_result(call.id.clone(), run.result));
        }

        step += 1;
        if step >= config.max_steps {
            emit(&events, LaneEvent::Finished { report: stopped_report(&last_text) });
            return;
        }
    }
}

fn stopped_report(last_text: &str) -> String {
    if last_text.trim().is_empty() {
        "(stopped: step budget reached)".to_string()
    } else {
        last_text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};
    use std::sync::Mutex;
    use std::time::Duration;

    use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

    use crate::dto::chat::FunctionCall;

    use super::*;

    /// A model that hands back canned replies in order and records the history
    /// it was shown each call, so a test can assert what the lane sent.
    struct ScriptedModel {
        replies: Mutex<VecDeque<ModelReply>>,
        seen: Mutex<Vec<Vec<ChatMessage>>>,
    }

    impl ScriptedModel {
        fn new(replies: Vec<ModelReply>) -> Arc<Self> {
            Arc::new(Self { replies: Mutex::new(replies.into()), seen: Mutex::new(Vec::new()) })
        }
        fn last_history(&self) -> Vec<ChatMessage> {
            self.seen.lock().unwrap().last().cloned().unwrap_or_default()
        }
    }

    impl ModelClient for ScriptedModel {
        fn stream<'a>(&'a self, request: ModelRequest) -> BoxFuture<'a, ModelReply> {
            self.seen.lock().unwrap().push(request.history.clone());
            let reply = self.replies.lock().unwrap().pop_front().unwrap_or_default();
            Box::pin(async move { reply })
        }
    }

    struct FakeRouter {
        advertised: Vec<ToolSpec>,
        runs: Mutex<HashMap<String, ToolRun>>,
    }

    impl FakeRouter {
        fn new() -> Arc<Self> {
            Arc::new(Self { advertised: Vec::new(), runs: Mutex::new(HashMap::new()) })
        }
        fn on(self: &Arc<Self>, name: &str, run: ToolRun) {
            self.runs.lock().unwrap().insert(name.to_string(), run);
        }
    }

    impl ToolRouter for FakeRouter {
        fn advertised(&self) -> Vec<ToolSpec> {
            self.advertised.clone()
        }
        fn run(&self, call: &ToolCall) -> ToolRun {
            self.runs.lock().unwrap().get(&call.function.name).cloned().unwrap_or(ToolRun {
                result: format!("error: no such tool {}", call.function.name),
                events: Vec::new(),
            })
        }
    }

    fn call(name: &str) -> ToolCall {
        ToolCall {
            id: format!("call-{name}"),
            kind: "function".into(),
            function: FunctionCall { name: name.into(), arguments: "{}".into() },
        }
    }

    fn reply_tools(text: &str, calls: Vec<ToolCall>) -> ModelReply {
        ModelReply { text: text.into(), tool_calls: calls, ..Default::default() }
    }

    fn reply_text(text: &str) -> ModelReply {
        ModelReply { text: text.into(), ..Default::default() }
    }

    fn config(kind: LaneKind) -> LaneConfig {
        LaneConfig {
            avatar: "red".into(),
            kind,
            system: "you are red".into(),
            task: "scan the login".into(),
            model: LaneModel::default(),
            max_steps: 8,
        }
    }

    async fn collect(mut events: UnboundedReceiver<LaneEvent>) -> Vec<LaneEvent> {
        let mut out = Vec::new();
        while let Some(event) = events.recv().await {
            out.push(event);
        }
        out
    }

    #[tokio::test]
    async fn a_chat_lane_answers_in_one_turn() {
        let model = ScriptedModel::new(vec![reply_text("I've started the team on it.")]);
        let router = FakeRouter::new();
        let (_in_tx, in_rx) = unbounded_channel();
        let (ev_tx, ev_rx) = unbounded_channel();
        run_lane(config(LaneKind::Chat), model, router, in_rx, ev_tx).await;
        let events = collect(ev_rx).await;
        assert_eq!(events[0], LaneEvent::Step(0));
        assert!(events.contains(&LaneEvent::Chat("I've started the team on it.".into())));
        assert_eq!(events.last(), Some(&LaneEvent::Finished { report: "I've started the team on it.".into() }));
    }

    #[tokio::test]
    async fn a_work_lane_runs_a_tool_round_then_finishes_and_forwards_router_events() {
        let model = ScriptedModel::new(vec![
            reply_tools("probing", vec![call("sec_run"), call("put_claim")]),
            reply_text("done"),
        ]);
        let router = FakeRouter::new();
        router.on("sec_run", ToolRun { result: "open port 443".into(), events: vec![] });
        router.on(
            "put_claim",
            ToolRun {
                result: "{\"key\":\"claims/a\",\"revision\":1}".into(),
                events: vec![LaneEvent::Published { key: "claims/a".into(), revision: 1 }],
            },
        );
        let (_in_tx, in_rx) = unbounded_channel();
        let (ev_tx, ev_rx) = unbounded_channel();
        run_lane(config(LaneKind::Work), model, router, in_rx, ev_tx).await;
        let events = collect(ev_rx).await;
        assert!(events.contains(&LaneEvent::ToolStarted { name: "sec_run".into() }));
        assert!(events.contains(&LaneEvent::ToolDone { name: "sec_run".into(), ok: true }));
        assert!(events.contains(&LaneEvent::Published { key: "claims/a".into(), revision: 1 }));
        // a work lane does not narrate to the user
        assert!(!events.iter().any(|e| matches!(e, LaneEvent::Chat(_))));
        assert_eq!(events.last(), Some(&LaneEvent::Finished { report: "done".into() }));
    }

    #[tokio::test]
    async fn an_error_result_marks_the_tool_not_ok() {
        let model = ScriptedModel::new(vec![reply_tools("", vec![call("bash")]), reply_text("ok")]);
        let router = FakeRouter::new();
        router.on("bash", ToolRun { result: "error: command failed".into(), events: vec![] });
        let (_in_tx, in_rx) = unbounded_channel();
        let (ev_tx, ev_rx) = unbounded_channel();
        run_lane(config(LaneKind::Work), model, router, in_rx, ev_tx).await;
        let events = collect(ev_rx).await;
        assert!(events.contains(&LaneEvent::ToolDone { name: "bash".into(), ok: false }));
    }

    #[tokio::test]
    async fn a_model_error_fails_the_lane() {
        let model = ScriptedModel::new(vec![ModelReply { error: Some("provider down".into()), ..Default::default() }]);
        let (_in_tx, in_rx) = unbounded_channel();
        let (ev_tx, ev_rx) = unbounded_channel();
        run_lane(config(LaneKind::Work), model, FakeRouter::new(), in_rx, ev_tx).await;
        let events = collect(ev_rx).await;
        assert_eq!(events.last(), Some(&LaneEvent::Failed { error: "provider down".into() }));
    }

    #[tokio::test]
    async fn the_step_budget_ends_a_runaway_lane_with_its_last_text() {
        // a model that always asks for a tool never finishes on its own
        let replies = (0..20).map(|i| reply_tools(&format!("step {i}"), vec![call("sec_run")])).collect();
        let model = ScriptedModel::new(replies);
        let router = FakeRouter::new();
        router.on("sec_run", ToolRun { result: "ok".into(), events: vec![] });
        let mut cfg = config(LaneKind::Work);
        cfg.max_steps = 3;
        let (_in_tx, in_rx) = unbounded_channel();
        let (ev_tx, ev_rx) = unbounded_channel();
        run_lane(cfg, model, router, in_rx, ev_tx).await;
        let events = collect(ev_rx).await;
        let steps = events.iter().filter(|e| matches!(e, LaneEvent::Step(_))).count();
        assert_eq!(steps, 3, "stopped at the budget");
        assert_eq!(events.last(), Some(&LaneEvent::Finished { report: "step 2".into() }));
    }

    #[tokio::test]
    async fn steering_input_is_folded_in_as_a_user_turn() {
        let model = ScriptedModel::new(vec![
            reply_tools("", vec![call("sec_run")]),
            reply_text("adjusted"),
        ]);
        let router = FakeRouter::new();
        router.on("sec_run", ToolRun { result: "ok".into(), events: vec![] });
        let (in_tx, in_rx) = unbounded_channel();
        let (ev_tx, ev_rx) = unbounded_channel();
        // steer before the second turn: queued now, drained at the next boundary
        in_tx.send(LaneInput::Steer("also check the API".into())).unwrap();
        let model_for_assert = Arc::clone(&model);
        run_lane(config(LaneKind::Work), model, router, in_rx, ev_tx).await;
        let _ = collect(ev_rx).await;
        let history = model_for_assert.last_history();
        assert!(
            history.iter().any(|m| m.content.contains("also check the API")),
            "the steer reached the model's history"
        );
    }

    #[tokio::test]
    async fn stop_ends_the_lane_at_the_next_boundary() {
        let model = ScriptedModel::new(vec![reply_tools("", vec![call("sec_run")]); 5]);
        let router = FakeRouter::new();
        router.on("sec_run", ToolRun { result: "ok".into(), events: vec![] });
        let (in_tx, in_rx) = unbounded_channel();
        let (ev_tx, ev_rx) = unbounded_channel();
        in_tx.send(LaneInput::Stop).unwrap();
        run_lane(config(LaneKind::Work), model, router, in_rx, ev_tx).await;
        let events = collect(ev_rx).await;
        assert!(matches!(events.last(), Some(LaneEvent::Finished { report }) if report.contains("stopped")));
        assert!(!events.iter().any(|e| matches!(e, LaneEvent::Step(_))), "stopped before the first step");
    }

    #[tokio::test]
    async fn pause_blocks_the_lane_until_resume() {
        let model = ScriptedModel::new(vec![reply_text("finished after resume")]);
        let router = FakeRouter::new();
        let (in_tx, in_rx) = unbounded_channel();
        let (ev_tx, mut ev_rx) = unbounded_channel();
        in_tx.send(LaneInput::Pause).unwrap();
        let handle = tokio::spawn(run_lane(config(LaneKind::Work), model, router, in_rx, ev_tx));
        // while paused, the lane blocks before the first step, so no event arrives
        let early = tokio::time::timeout(Duration::from_millis(80), ev_rx.recv()).await;
        assert!(early.is_err(), "the lane is paused and emits nothing");
        in_tx.send(LaneInput::Resume).unwrap();
        handle.await.unwrap();
        let mut events = Vec::new();
        while let Ok(event) = ev_rx.try_recv() {
            events.push(event);
        }
        assert_eq!(events.first(), Some(&LaneEvent::Step(0)), "it runs once resumed");
        assert_eq!(events.last(), Some(&LaneEvent::Finished { report: "finished after resume".into() }));
    }
}
