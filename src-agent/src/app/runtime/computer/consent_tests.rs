//! Dispatch-only fixtures: never capture or inject native desktop input.
use super::*;
use crate::{
    app::{harness::Verdict, mode::Mode, state::AgentMode},
    dto::chat::FunctionCall,
    model::{conversation::Conversation, session::Session},
    service::openrouter::OpenRouterClient,
};
use serde_json::json;
use std::sync::Arc;

#[test]
fn computer_consent_bypasses_workspace_gates_but_pause_and_stop_revoke_it() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = Some(Arc::new(OpenRouterClient::new()));
    for mode in [
        AgentMode::Normal,
        AgentMode::Auto,
        AgentMode::Yolo,
        AgentMode::Plan,
        AgentMode::Sdlc,
    ] {
        for lifecycle in ["active", "paused", "stopped"] {
            for tool in [
                "computer_windows",
                "computer_select_window",
                "computer_observe",
                "computer_act",
            ] {
                let dir =
                    std::env::temp_dir().join(format!("koma-consent-{}", uuid::Uuid::new_v4()));
                std::fs::create_dir_all(&dir).unwrap();
                let mut state = AppState::new(Mode::Chat);
                let rt = &mut state.rest.sessions[0];
                rt.agent_mode = mode;
                rt.sdlc_phase = Some("assess".into());
                let mut session = Session::new(
                    rt.id.clone(),
                    dir.clone(),
                    "fixture".into(),
                    Default::default(),
                    Conversation::from_messages(vec![]),
                );
                session.settings.classifier_enabled = true;
                rt.session = Some(session);
                rt.computer
                    .enable(
                        1,
                        &rt.id,
                        "fixture",
                        Capabilities {
                            capture: true,
                            focus: true,
                            windows: true,
                            pointer: true,
                            keyboard: true,
                            ..Default::default()
                        },
                        &dir.join("desktop.lock"),
                    )
                    .unwrap();
                let rect = Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 100.0,
                };
                rt.computer.status.observation = Some(Observation {
                    id: "frame".into(),
                    session: rt.id.clone(),
                    generation: rt.computer.status.generation.clone(),
                    window: Window {
                        id: "display:fixture".into(),
                        application: "Desktop".into(),
                        title: "Fixture".into(),
                        geometry: rect,
                        focused: true,
                        focus: None,
                    },
                    transform: Transform {
                        desktop: rect,
                        width: 100,
                        height: 100,
                    },
                    captured_ms: 1,
                    elements: vec![],
                    accessibility_status: String::new(),
                    ocr_status: String::new(),
                    image_path: String::new(),
                });
                rt.computer.actionable = true;
                let args = match tool {
                    "computer_select_window" => {
                        json!({"window":"display:fixture","generation":rt.computer.status.generation})
                    }
                    "computer_act" => {
                        json!({"observation":"frame","actions":[{"kind":"scroll","x":50,"y":50,"delta":-3}]})
                    }
                    _ => json!({}),
                };
                let call = ToolCall {
                    id: "consented-call".into(),
                    kind: "function".into(),
                    function: FunctionCall {
                        name: tool.into(),
                        arguments: args.to_string(),
                    },
                };
                // A prior classifier denial must not re-gate computer actions.
                // A live client/session also exposes accidental TAC dispatch.
                rt.pending_classify_verdict = Some((
                    call.id.clone(),
                    Verdict {
                        allow: false,
                        available: true,
                        reason:
                            "GUI automation is outside workspace sandbox and needs human approval"
                                .into(),
                    },
                ));
                rt.pending_tool_calls.push(call);
                match lifecycle {
                    "paused" => {
                        rt.computer.pause(true);
                    }
                    "stopped" => {
                        rt.computer.stop("user stopped");
                    }
                    _ => {}
                }
                crate::app::runtime::stream::process_tools(
                    &mut state,
                    0,
                    &client,
                    runtime.handle(),
                );
                let rt = &state.rest.sessions[0];
                assert!(
                    !rt.awaiting_approval && !rt.awaiting_classify,
                    "{mode:?} {tool} {lifecycle}"
                );
                assert!(rt.classify_tx.is_none());
                assert_eq!(rt.tool_idx, 1);
                if lifecycle == "active" {
                    assert!(
                        rt.tool_results.is_empty(),
                        "{mode:?} {tool}: {:?}",
                        rt.tool_results
                    );
                    assert!(rt.computer.outbound.is_some(), "{mode:?} {tool}");
                    assert_eq!(rt.pending_tool_tasks, ["consented-call"]);
                } else {
                    assert!(rt.computer.outbound.is_none());
                    assert!(rt.pending_tool_tasks.is_empty());
                    assert!(rt.tool_results[0]
                        .1
                        .contains("no active local GUI controller"));
                }
                drop(state);
                std::fs::remove_dir_all(dir).unwrap();
            }
        }
    }
}
