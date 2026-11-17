//! `subagent_risky`: what a sub-agent's risky tool call goes through.
//!
//! The gate unit tests cover every policy; the loop tests run the real
//! sub-agent loop against a local SSE server, so a policy that is only honoured
//! in `risky_gate` and not in the loop would fail here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, RwLock};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use super::engine::{risky_gate, run_agent_loop};
use super::{AgentEvent, ApprovalDecision};
use crate::app::harness::Verdict;
use crate::app::resolve::Resolved;
use crate::dto::chat::{FunctionCall, ToolCall};
use crate::model::app_config::{ApiType, AppConfig, SubagentRisky};
use crate::model::conversation::Conversation;
use crate::model::settings::Settings;
use crate::service::openrouter::OpenRouterClient;

fn call(id: &str, name: &str, args: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        kind: "function".into(),
        function: FunctionCall {
            name: name.into(),
            arguments: args.into(),
        },
    }
}

fn verdict(allow: bool, available: bool) -> Verdict {
    Verdict {
        allow,
        reason: "because".into(),
        available,
    }
}

type Channels = (
    UnboundedSender<AgentEvent>,
    UnboundedReceiver<AgentEvent>,
    UnboundedSender<ApprovalDecision>,
    UnboundedReceiver<ApprovalDecision>,
);

fn channels() -> Channels {
    let (tx, rx) = mpsc::unbounded_channel();
    let (approval_tx, approval_rx) = mpsc::unbounded_channel();
    (tx, rx, approval_tx, approval_rx)
}

#[test]
fn default_policy_is_the_classifier_and_stays_out_of_the_config_file() {
    assert_eq!(AppConfig::default().subagent_risky, SubagentRisky::Classifier);
    let json = serde_json::to_value(AppConfig::default()).unwrap();
    assert!(json.get("subagent_risky").is_none());
}

#[test]
fn policy_round_trips_through_config_json_and_unknown_values_deny() {
    for (mode, text) in [
        (SubagentRisky::Parent, "parent"),
        (SubagentRisky::Deny, "deny"),
    ] {
        let config = AppConfig {
            subagent_risky: mode,
            ..AppConfig::default()
        };
        let json = serde_json::to_value(&config).unwrap();
        assert_eq!(json["subagent_risky"], text);
        let back: AppConfig = serde_json::from_value(json).unwrap();
        assert_eq!(back.subagent_risky, mode);
    }
    let json = serde_json::json!({"subagent_risky": "classifier"});
    let config: AppConfig = serde_json::from_value(json).unwrap();
    assert_eq!(config.subagent_risky, SubagentRisky::Classifier);
    // A typo closes the gate rather than reverting to the classifier.
    let json = serde_json::json!({"subagent_risky": "parnet"});
    let config: AppConfig = serde_json::from_value(json).unwrap();
    assert_eq!(config.subagent_risky, SubagentRisky::Deny);
    // A value that is not a string closes the gate too, and the rest of the
    // config still loads.
    for bad in [serde_json::json!(true), serde_json::json!(1), serde_json::Value::Null] {
        let json = serde_json::json!({"subagent_risky": bad, "palette": "kept"});
        let config: AppConfig = serde_json::from_value(json).unwrap();
        assert_eq!(config.subagent_risky, SubagentRisky::Deny);
        assert_eq!(config.palette, "kept");
    }
}

#[tokio::test]
async fn classifier_policy_keeps_the_existing_verdict_handling() {
    for (allow, available, expect_run) in [(true, true, true), (false, true, false), (true, false, false)] {
        let (tx, _rx, _atx, mut arx) = channels();
        let c = call("c1", "write", "{}");
        let out = risky_gate(SubagentRisky::Classifier, &c, &tx, &mut arx, || async move {
            verdict(allow, available)
        })
        .await;
        assert_eq!(out.is_none(), expect_run, "allow={allow} available={available}");
        if !available {
            assert!(out.unwrap().contains("classifier_unavailable"));
        } else if !allow {
            assert!(out.unwrap().starts_with("blocked by harness: because"));
        }
    }
}

#[tokio::test]
async fn deny_policy_refuses_without_asking_the_classifier_or_the_parent() {
    for name in ["write", "edit", "delete", "bash", "git_operator", "web_download"] {
        assert!(crate::tool::tool_is_risky(name));
        let (tx, mut rx, _atx, mut arx) = channels();
        let c = call("c1", name, "{}");
        let out = risky_gate(SubagentRisky::Deny, &c, &tx, &mut arx, || async {
            panic!("the classifier must not run under deny")
        })
        .await
        .expect("denied");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["error"], "subagent_risky_denied");
        assert_eq!(v["executed"], false);
        assert!(rx.try_recv().is_err(), "no approval request under deny");
    }
}

#[tokio::test]
async fn parent_policy_waits_for_the_parent_and_runs_only_on_approve() {
    let (tx, mut rx, atx, mut arx) = channels();
    let c = call("c1", "write", r#"{"path":"a"}"#);
    atx.send(ApprovalDecision::Approve { call_id: "c1".into() }).unwrap();
    let out = risky_gate(SubagentRisky::Parent, &c, &tx, &mut arx, || async {
        panic!("the classifier must not decide under parent")
    })
    .await;
    assert!(out.is_none());
    match rx.try_recv().unwrap() {
        AgentEvent::ApprovalRequest { call_id, name, args } => {
            assert_eq!((call_id.as_str(), name.as_str()), ("c1", "write"));
            assert_eq!(args, r#"{"path":"a"}"#);
        }
        other => panic!("unexpected event {other:?}"),
    }
}

#[tokio::test]
async fn parent_policy_denial_returns_the_denial_and_ignores_stale_answers() {
    let (tx, _rx, atx, mut arx) = channels();
    let c = call("c2", "bash", "{}");
    atx.send(ApprovalDecision::Approve { call_id: "old".into() }).unwrap();
    atx.send(ApprovalDecision::Deny {
        call_id: "c2".into(),
        result: "denied by user".into(),
    })
    .unwrap();
    let out = risky_gate(SubagentRisky::Parent, &c, &tx, &mut arx, || async {
        panic!("no classifier")
    })
    .await;
    assert_eq!(out.as_deref(), Some("denied by user"));
}

#[tokio::test]
async fn parent_policy_fails_closed_when_the_parent_is_gone() {
    let (tx, _rx, atx, mut arx) = channels();
    drop(atx);
    let c = call("c3", "edit", "{}");
    let out = risky_gate(SubagentRisky::Parent, &c, &tx, &mut arx, || async {
        panic!("no classifier")
    })
    .await
    .expect("denied");
    assert!(out.contains("approval_unavailable"));
}

// ---- the real loop, against a local SSE server ----

/// Serves one canned SSE body per request, in order.
struct Server {
    endpoint: String,
    requests: Arc<Mutex<usize>>,
}

fn tool_stream(id: &str, name: &str, args: &str) -> String {
    let delta = serde_json::json!({"choices": [{"delta": {"tool_calls": [{
        "index": 0, "id": id, "type": "function",
        "function": {"name": name, "arguments": args}}]}, "finish_reason": "tool_calls"}]});
    format!("data: {delta}\n\ndata: [DONE]\n\n")
}

fn text_stream(text: &str) -> String {
    let delta = serde_json::json!({"choices": [{"delta": {"content": text}, "finish_reason": "stop"}]});
    format!("data: {delta}\n\ndata: [DONE]\n\n")
}

async fn serve(bodies: Vec<String>) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let queue = Arc::new(Mutex::new(VecDeque::from(bodies)));
    let requests = Arc::new(Mutex::new(0usize));
    let counted = Arc::clone(&requests);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let queue = Arc::clone(&queue);
            let counted = Arc::clone(&counted);
            tokio::spawn(async move {
                let mut bytes = Vec::new();
                loop {
                    let mut chunk = [0u8; 8192];
                    let n = socket.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&chunk[..n]);
                    if let Some(pos) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&bytes[..pos]).to_lowercase();
                        let length = header
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        if bytes.len() >= pos + 4 + length {
                            break;
                        }
                    }
                }
                // Only the sub-agent's own streaming calls take a canned reply; a
                // classifier call (not streamed) gets a reply that is no verdict.
                let streamed = String::from_utf8_lossy(&bytes).contains("\"stream\":true");
                let (kind, body) = if streamed {
                    *counted.lock().unwrap() += 1;
                    (
                        "text/event-stream",
                        queue.lock().unwrap().pop_front().unwrap_or_else(|| text_stream("done")),
                    )
                } else {
                    (
                        "application/json",
                        r#"{"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"no verdict"}}]}"#
                            .to_string(),
                    )
                };
                let wire = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(wire.as_bytes()).await;
            });
        }
    });
    Server { endpoint, requests }
}

fn test_ctx(root: &std::path::Path) -> crate::tool::ToolCtx {
    crate::tool::ToolCtx {
        plan_read_only: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        workspace: root.to_path_buf(),
        workspaces: vec![root.to_path_buf()],
        dir_cache: Arc::new(RwLock::new(crate::tool::DirCache::default())),
        memory_dir: None,
        worktrees_dir: None,
        download_dir: None,
        scratch_dir: None,
        internet_mode: crate::model::settings::InternetMode::default(),
        ssh_key: None,
        skill_registry: None,
        active_skill_names: None,
        active_skill_dirs: Vec::new(),
        skill_scope: None,
        skill_rank: None,
        mcp_manager: None,
        bash_saving: true,
        bash_log_dir: None,
        session_dir: None,
        allow_scratch: true,
        sdlc_assess: false,
        sdlc_active_node_id: None,
        search_engine: None,
        call_track: crate::tool::CallTrack::new(),
    }
}

/// A config whose Safeguard and Main routes are the local server, so the
/// classifier never leaves the machine (an empty config falls back to the
/// keyless koma-free route).
fn classifier_config(endpoint: &str) -> AppConfig {
    use crate::model::app_config::{ModelEntry, ModelRole, ProviderConn};
    let mut config = AppConfig::default();
    config.providers.push(ProviderConn {
        uuid: "local".into(),
        endpoint: endpoint.to_string(),
        api_key: "test".into(),
        ..Default::default()
    });
    for role in [ModelRole::Safeguard, ModelRole::Main] {
        config.models.push(ModelEntry {
            uuid: format!("{role:?}"),
            model_id: "local-model".into(),
            provider_uuid: "local".into(),
            roles: vec![role],
            ..Default::default()
        });
    }
    config
}

struct Run {
    events: Vec<AgentEvent>,
    root: std::path::PathBuf,
    requests: usize,
}

impl Run {
    fn tool_results(&self) -> Vec<String> {
        let messages = self
            .events
            .iter()
            .rev()
            .find_map(|e| match e {
                AgentEvent::Snapshot(m) => Some(m.clone()),
                _ => None,
            })
            .unwrap_or_default();
        messages
            .iter()
            .filter(|m| m.role == crate::dto::chat::Role::Tool)
            .map(|m| m.content.clone())
            .collect()
    }
    fn done(&self) -> bool {
        self.events.iter().any(|e| matches!(e, AgentEvent::Done(_)))
    }
}

/// Run the real loop for one agent with `tools`, answering parent approvals with
/// `answer` (called with each request's call id).
async fn run_loop(
    tag: &str,
    risky: SubagentRisky,
    bodies: Vec<String>,
    answer: impl Fn(&str) -> Option<bool> + Send + 'static,
) -> Run {
    let root = std::env::temp_dir().join(format!("koma-risky-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("seed.txt"), "seed").unwrap();
    let server = serve(bodies).await;
    let resolved = Resolved {
        model_id: "m".into(),
        provider_uuid: "p".into(),
        endpoint: server.endpoint.clone(),
        api_key: "k".into(),
        api_type: ApiType::OpenAiCompatible,
        route: None,
        effort: String::new(),
        account_id: String::new(),
        oauth_uuid: String::new(),
        install_id: String::new(),
    };
    let config = classifier_config(&server.endpoint);
    let mut convo = Conversation::new("persona");
    convo.push_user("do the task");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let (_inject_tx, inject_rx) = mpsc::unbounded_channel();
    let (approval_tx, approval_rx) = mpsc::unbounded_channel();
    let tools = ["read", "write", "edit", "delete", "bash", "git_worktree"]
        .map(String::from)
        .to_vec();
    let task = tokio::spawn(run_agent_loop(
        Arc::new(OpenRouterClient::new()),
        resolved,
        config,
        Settings::default(),
        tools,
        Vec::new(),
        test_ctx(&root),
        convo,
        "do the task".into(),
        Some(6),
        tx,
        inject_rx,
        "tester".into(),
        1,
        None,
        false,
        risky,
        approval_rx,
    ));
    let mut events = Vec::new();
    let finish = async {
        while let Some(event) = rx.recv().await {
            if let AgentEvent::ApprovalRequest { call_id, .. } = &event {
                if let Some(approve) = answer(call_id) {
                    let decision = if approve {
                        ApprovalDecision::Approve { call_id: call_id.clone() }
                    } else {
                        ApprovalDecision::Deny {
                            call_id: call_id.clone(),
                            result: "denied by user".into(),
                        }
                    };
                    approval_tx.send(decision).unwrap();
                }
            }
            let end = matches!(event, AgentEvent::Done(_) | AgentEvent::Error(_));
            events.push(event);
            if end {
                break;
            }
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(20), finish)
        .await
        .expect("the sub-agent loop finished");
    let _ = task.await;
    let requests = *server.requests.lock().unwrap();
    Run { events, root, requests }
}

fn write_args() -> String {
    serde_json::json!({"path": "out.txt", "content": "written"}).to_string()
}

#[tokio::test]
async fn loop_under_deny_never_runs_a_risky_call_and_still_runs_safe_ones() {
    let run = run_loop(
        "deny",
        SubagentRisky::Deny,
        vec![
            tool_stream("w1", "write", &write_args()),
            tool_stream("r1", "read", r#"{"path":"seed.txt"}"#),
            text_stream("report"),
        ],
        |_| None,
    )
    .await;
    assert!(run.done());
    assert!(!run.root.join("out.txt").exists(), "the write must not run");
    let results = run.tool_results();
    assert!(results[0].contains("subagent_risky_denied"), "{results:?}");
    assert!(results[1].contains("seed"), "read still works: {results:?}");
    assert!(!run.events.iter().any(|e| matches!(e, AgentEvent::ApprovalRequest { .. })));
}

#[tokio::test]
async fn loop_under_parent_runs_the_call_only_after_approval() {
    let run = run_loop(
        "parent-yes",
        SubagentRisky::Parent,
        vec![tool_stream("w1", "write", &write_args()), text_stream("report")],
        |_| Some(true),
    )
    .await;
    assert!(run.done());
    assert_eq!(std::fs::read_to_string(run.root.join("out.txt")).unwrap(), "written");
    assert!(run.events.iter().any(|e| matches!(e, AgentEvent::ApprovalRequest { call_id, .. } if call_id == "w1")));
}

#[tokio::test]
async fn loop_under_parent_does_not_run_a_denied_call_and_carries_on() {
    let run = run_loop(
        "parent-no",
        SubagentRisky::Parent,
        vec![tool_stream("w1", "write", &write_args()), text_stream("report")],
        |_| Some(false),
    )
    .await;
    assert!(run.done(), "a denied call does not end the sub-agent");
    assert!(!run.root.join("out.txt").exists());
    assert_eq!(run.tool_results(), vec!["denied by user".to_string()]);
}

#[tokio::test]
async fn loop_under_deny_refuses_worktree_remove_but_not_list() {
    let run = run_loop(
        "wt-deny",
        SubagentRisky::Deny,
        vec![
            tool_stream("g1", "git_worktree", r#"{"action":"remove","name":"x"}"#),
            tool_stream("g2", "git_worktree", r#"{"action":"list"}"#),
            text_stream("report"),
        ],
        |_| None,
    )
    .await;
    assert!(run.done());
    let results = run.tool_results();
    assert!(results[0].contains("subagent_risky_denied"), "{results:?}");
    assert!(!results[1].contains("subagent_risky_denied"), "{results:?}");
}

#[tokio::test]
async fn loop_under_parent_asks_before_worktree_remove() {
    let run = run_loop(
        "wt-parent",
        SubagentRisky::Parent,
        vec![
            tool_stream("g1", "git_worktree", r#"{"action":"remove","name":"x"}"#),
            text_stream("report"),
        ],
        |_| Some(false),
    )
    .await;
    assert!(run.done());
    assert_eq!(run.tool_results(), vec!["denied by user".to_string()]);
    assert!(run.events.iter().any(|e| matches!(e, AgentEvent::ApprovalRequest { call_id, name, .. } if call_id == "g1" && name == "git_worktree")));
}

#[tokio::test]
async fn loop_under_parent_blocks_until_an_answer_arrives() {
    // No answer is sent: the call must stay parked, not run and not time out.
    let root = std::env::temp_dir().join(format!("koma-risky-wait-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let server = serve(vec![tool_stream("w1", "write", &write_args()), text_stream("report")]).await;
    let resolved = Resolved {
        model_id: "m".into(),
        provider_uuid: "p".into(),
        endpoint: server.endpoint.clone(),
        api_key: "k".into(),
        api_type: ApiType::OpenAiCompatible,
        route: None,
        effort: String::new(),
        account_id: String::new(),
        oauth_uuid: String::new(),
        install_id: String::new(),
    };
    let mut convo = Conversation::new("persona");
    convo.push_user("do the task");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let (_inject_tx, inject_rx) = mpsc::unbounded_channel();
    let (_approval_tx, approval_rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(run_agent_loop(
        Arc::new(OpenRouterClient::new()),
        resolved,
        classifier_config(&server.endpoint),
        Settings::default(),
        vec!["write".to_string()],
        Vec::new(),
        test_ctx(&root),
        convo,
        "do the task".into(),
        Some(6),
        tx,
        inject_rx,
        "tester".into(),
        1,
        None,
        false,
        SubagentRisky::Parent,
        approval_rx,
    ));
    let mut asked = false;
    let waited = tokio::time::timeout(std::time::Duration::from_millis(1500), async {
        while let Some(event) = rx.recv().await {
            if matches!(event, AgentEvent::ApprovalRequest { .. }) {
                asked = true;
            }
            assert!(!matches!(event, AgentEvent::Done(_) | AgentEvent::ToolStarted { .. }));
        }
    })
    .await;
    assert!(asked && waited.is_err(), "the loop must wait for the parent");
    assert!(!root.join("out.txt").exists());
    task.abort();
}

#[tokio::test]
async fn loop_with_the_default_policy_still_fails_closed_without_a_classifier() {
    // No Safeguard route is configured, so the classifier is unavailable and the
    // call is blocked, exactly as before this option existed.
    let run = run_loop(
        "classifier",
        SubagentRisky::Classifier,
        vec![tool_stream("w1", "write", &write_args()), text_stream("report")],
        |_| None,
    )
    .await;
    assert!(run.done());
    assert!(!run.root.join("out.txt").exists());
    assert!(
        run.tool_results()[0].contains("classifier_unavailable"),
        "{:?}",
        run.tool_results()
    );
    assert!(!run.events.iter().any(|e| matches!(e, AgentEvent::ApprovalRequest { .. })));
}

#[tokio::test]
async fn text_harvested_tool_calls_never_reach_a_sub_agent_tool() {
    // A sub-agent does not turn `<tool_call>` text into calls (only the main loop
    // does), so markup in its reply cannot run a tool under any policy.
    let markup = format!(
        r#"<tool_call>{{"name":"write","arguments":{}}}</tool_call>"#,
        write_args()
    );
    for (tag, risky) in [
        ("text-deny", SubagentRisky::Deny),
        ("text-parent", SubagentRisky::Parent),
        ("text-classifier", SubagentRisky::Classifier),
    ] {
        let run = run_loop(tag, risky, vec![text_stream(&markup)], |_| Some(true)).await;
        assert!(run.done(), "{tag}");
        assert!(!run.root.join("out.txt").exists(), "{tag}: the write must not run");
        assert!(!run.events.iter().any(|e| matches!(e, AgentEvent::ToolStarted { .. })), "{tag}");
        assert_eq!(run.requests, 1, "{tag}");
    }
}
