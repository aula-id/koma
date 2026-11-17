//! The hard tool deny list inside a sub-agent. A real `run_agent_loop` talks to
//! a scripted OpenAI-compatible server on loopback: the first reply asks for a
//! tool, the second ends the run. The second request carries whatever the
//! agent was told about its tool call.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc::unbounded_channel;

use super::engine::run_agent_loop;
use super::AgentEvent;
use crate::tool::policy::{refusal, test_deny};

/// What `pong` returns when it runs. The tool's description also says "pong!".
const PONG_RESULT: &str = "pong! \u{1F3D3}";

/// Serve `replies` one per connection, recording each request body.
fn scripted_server(replies: Vec<String>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&bodies);
    std::thread::spawn(move || {
        for reply in replies {
            let Ok((mut socket, _)) = listener.accept() else {
                return;
            };
            let mut raw = Vec::new();
            let mut chunk = [0u8; 8192];
            let body = loop {
                let n = socket.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break String::new();
                }
                raw.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&raw).to_string();
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let want = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if body.len() >= want {
                        break body.to_string();
                    }
                }
            };
            seen.lock().unwrap().push(body);
            let head =
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n";
            let _ = socket.write_all(head.as_bytes());
            let _ = socket.write_all(reply.as_bytes());
            let _ = socket.flush();
        }
    });
    (endpoint, bodies)
}

fn sse(frames: &[serde_json::Value]) -> String {
    let mut out = String::new();
    for f in frames {
        out.push_str(&format!("data: {f}\n\n"));
    }
    out.push_str("data: [DONE]\n\n");
    out
}

fn tool_reply(name: &str) -> String {
    sse(&[
        serde_json::json!({"choices":[{"index":0,"delta":{"tool_calls":[
        {"index":0,"id":"call_1","type":"function","function":{"name":name,"arguments":"{}"}}
    ]},"finish_reason":"tool_calls"}]}),
    ])
}

fn final_reply() -> String {
    sse(&[
        serde_json::json!({"choices":[{"index":0,"delta":{"content":"all done"},"finish_reason":"stop"}]}),
    ])
}

fn tool_ctx() -> crate::tool::ToolCtx {
    crate::tool::ToolCtx {
        plan_read_only: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        workspace: std::env::temp_dir(),
        workspaces: vec![std::env::temp_dir()],
        dir_cache: Arc::new(std::sync::RwLock::new(crate::tool::DirCache::default())),
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

/// Run one sub-agent whose allow-list holds `pong` and whose model asks for it.
/// Returns the events it emitted and the bodies of the requests it sent.
fn run_agent(tool: &str) -> (Vec<AgentEvent>, Vec<String>) {
    let (endpoint, bodies) = scripted_server(vec![tool_reply(tool), final_reply()]);
    let resolved = crate::app::resolve::Resolved {
        model_id: "fake".into(),
        provider_uuid: String::new(),
        endpoint,
        api_key: "k".into(),
        api_type: crate::model::app_config::ApiType::OpenAiCompatible,
        route: None,
        effort: String::new(),
        account_id: String::new(),
        oauth_uuid: String::new(),
        install_id: String::new(),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (tx, mut rx) = unbounded_channel();
    let (_inject_tx, inject_rx) = unbounded_channel();
    let mut convo = crate::model::conversation::Conversation::new("You are a test agent.");
    convo.push_user("call the tool");
    runtime.block_on(run_agent_loop(
        Arc::new(crate::service::openrouter::OpenRouterClient::new()),
        resolved,
        crate::model::app_config::AppConfig::default(),
        crate::model::settings::Settings::default(),
        vec![tool.to_string()],
        Vec::new(),
        tool_ctx(),
        convo,
        "call the tool".into(),
        Some(4),
        tx,
        inject_rx,
        "test".into(),
        1,
        None,
        false,
    ));
    let mut events = Vec::new();
    while let Ok(e) = rx.try_recv() {
        events.push(e);
    }
    let sent = bodies.lock().unwrap().clone();
    (events, sent)
}

#[test]
fn a_sub_agent_cannot_run_a_denied_tool_its_definition_lists() {
    let _deny = test_deny(&["pong"]);
    let (events, sent) = run_agent("pong");
    assert_eq!(sent.len(), 2, "the agent should ask the model twice");
    assert!(sent[1].contains(&refusal("pong")), "{}", sent[1]);
    assert!(!sent[1].contains(PONG_RESULT));
    assert!(!events
        .iter()
        .any(|e| matches!(e, AgentEvent::ToolStarted { .. })));
    assert!(events.iter().any(|e| matches!(e, AgentEvent::Done(_))));
}

#[test]
fn a_sub_agent_runs_the_same_tool_when_it_is_not_denied() {
    let (events, sent) = run_agent("pong");
    assert_eq!(sent.len(), 2);
    assert!(sent[1].contains(PONG_RESULT), "{}", sent[1]);
    assert!(events
        .iter()
        .any(|e| matches!(e, AgentEvent::ToolStarted { .. })));
}
