use super::*;

struct Archive(std::path::PathBuf);

impl Archive {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("koma-drss-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn append(&self, role: Role, content: &str) -> ChatMessage {
        msglog::append(&self.0, role, content, None, None).unwrap();
        ChatMessage::new(role, content)
    }

    fn tool_ctx(&self) -> crate::tool::ToolCtx {
        crate::tool::ToolCtx {
            workspace: self.0.clone(),
            workspaces: vec![self.0.clone()],
            dir_cache: std::sync::Arc::new(
                std::sync::RwLock::new(crate::tool::DirCache::default()),
            ),
            memory_dir: None,
            worktrees_dir: None,
            download_dir: None,
            scratch_dir: None,
            internet_mode: Default::default(),
            ssh_key: None,
            skill_registry: None,
            active_skill_names: None,
            active_skill_dirs: Vec::new(),
            mcp_manager: None,
            sec_manager: None,
            bash_saving: true,
            bash_log_dir: None,
            session_dir: Some(self.0.clone()),
            allow_scratch: true,
            sdlc_assess: false,
            sdlc_active_node_id: None,
            search_engine: None,
        }
    }
}

impl Drop for Archive {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn unavailable_fold_preserves_every_message_newer_than_summary() {
    let archive = Archive::new();
    let mut history = vec![ChatMessage::new(Role::System, "system")];
    history.push(archive.append(Role::User, "old request"));
    history.push(archive.append(Role::Assistant, "old answer"));
    msglog::write_summary(&archive.0, "Old exchange only.", 2, 3).unwrap();
    for i in 0..60 {
        let role = if i % 2 == 0 {
            Role::User
        } else {
            Role::Assistant
        };
        history.push(archive.append(role, &format!("new-message-{i}: {}", "x".repeat(4000))));
    }

    let out = shape(
        history,
        &archive.0,
        &OpenRouterClient::new(),
        &Settings::default(),
        None,
        "continue",
        true,
        118_000,
        false,
        &GoalWire::default(),
    )
    .await;

    assert!(
        out.len() > 60,
        "the summary does not cover any of the 60 new messages"
    );
    for i in 0..60 {
        assert!(out[1..]
            .iter()
            .any(|m| m.content.contains(&format!("new-message-{i}:"))));
    }
    assert_eq!(msglog::read_summary(&archive.0).unwrap().covers_up_to, 2);
    assert_eq!(msglog::max_message_id(&archive.0), 62);
}

#[test]
fn watermark_span_can_exceed_hot_message_limit() {
    let body: Vec<_> = (0..150)
        .map(|_| ChatMessage::new(Role::User, "x".repeat(4000)))
        .collect();
    assert_eq!(hot_keep_n(&body, 150, 40, 8000), 150);
    assert!(hot_keep_n(&body, 1, 40, 8000) < 40);
}

#[test]
fn live_stub_round_trips_through_message_find_pages() {
    use crate::tool::Tool;
    for payload in ["工具\n", "工具\0\n"] {
        let archive = Archive::new();
        let body = format!("{}\nexact_result=42", payload.repeat(2300));
        let mut message = archive.append(Role::Tool, &body);
        let blob = msglog::list_blobs(&archive.0).pop().unwrap();
        stub_one_message(&mut message, Some(&blob));
        assert!(!message.content.contains("exact_result=42"));
        assert!(message
            .content
            .contains(&format!("\"message_id\":{}", blob.msg_id)));
        let mut offset = 0;
        let mut restored = String::new();
        loop {
            let output = crate::tool::history::MessageFind
                .run(
                    &archive.tool_ctx(),
                    &serde_json::json!({
                        "message_id": blob.msg_id, "offset": offset,
                    }),
                )
                .unwrap();
            let (header, content) = output.split_once('\n').unwrap();
            let header: serde_json::Value = serde_json::from_str(header).unwrap();
            assert_eq!(header["offset"], offset);
            assert!(msg_tok_est(&ChatMessage::new(Role::Tool, &output)) < WIRE_STUB_TOKENS);
            restored.push_str(content);
            match header["next_offset"].as_i64() {
                Some(next) => {
                    assert!(next > offset);
                    offset = next;
                }
                None => break,
            }
        }
        assert_eq!(restored, body);
        assert_eq!(
            msglog::fetch_blob_content(&archive.0, blob.msg_id).unwrap(),
            body
        );
    }
}

#[test]
fn exact_message_reads_reject_ambiguous_or_invalid_requests() {
    use crate::tool::Tool;
    let archive = Archive::new();
    archive.append(Role::User, "short body");
    for args in [
        serde_json::json!({"message_id": 1, "scope": "project"}),
        serde_json::json!({"message_id": 1, "query": "body"}),
        serde_json::json!({"message_id": 1, "offset": -1}),
        serde_json::json!({"message_id": 1, "offset": i64::MAX}),
        serde_json::json!({"message_id": 1, "offset": 100}),
        serde_json::json!({"message_id": 1, "limit": 3001}),
        serde_json::json!({"message_id": 1, "limit": 0}),
        serde_json::json!({"message_id": 999}),
    ] {
        assert!(crate::tool::history::MessageFind
            .run(&archive.tool_ctx(), &args)
            .is_err());
    }
    let other_session = Archive::new();
    assert!(crate::tool::history::MessageFind
        .run(
            &other_session.tool_ctx(),
            &serde_json::json!({"message_id": 1})
        )
        .is_err());
}

#[test]
fn unindexed_body_is_never_stubbed() {
    let content = "unarchived evidence ".repeat(400);
    let mut message = ChatMessage::new(Role::Tool, &content);
    stub_one_message(&mut message, None);
    assert_eq!(message.content, content);
}

#[test]
fn tiny_window_preserves_complete_live_tool_round() {
    let call = |id: &str| {
        serde_json::from_value(serde_json::json!({
            "id": id, "type": "function",
            "function": {"name": "read", "arguments": "{\"path\":\"file.txt\"}"},
        }))
        .unwrap()
    };
    let body = vec![
        ChatMessage::new(Role::User, "current task"),
        ChatMessage::assistant_with_tools(String::new(), vec![call("a"), call("b")]),
        ChatMessage::tool_result("a".into(), "x".repeat(8000)),
        ChatMessage::tool_result("b".into(), "y".repeat(8000)),
    ];
    // Even if a restored watermark places the cut inside this round, its
    // assistant and every tool response must survive together.
    let keep = hot_keep_n(&body, 1, 1, 8000);
    assert_eq!(keep, 3);
    assert_eq!(&body[body.len() - keep..], &body[1..]);
    for requested in 1..=3 {
        assert_eq!(snap_keep_to_round(&body, requested), 3);
    }
}

#[test]
fn warm_cache_cannot_hide_output_pressure() {
    use crate::service::openrouter::{checked_max_output_tokens, output_headroom_is_low};
    let history = vec![
        ChatMessage::new(Role::System, "s".repeat(1000)),
        ChatMessage::new(Role::User, "let value = parse(input);\n".repeat(12000)),
    ];
    let window = 128_000;
    let warm_threshold =
        super::super::ENGAGE_WARM_PCT * (window - super::super::BASE_OVERHEAD) / 100;
    assert!(super::super::estimate_conv_tokens(&history) < warm_threshold);
    let estimate = super::super::estimate_prompt_tokens_for_max_clamp(&history);
    let endpoint = "https://example.invalid/v1";
    assert!(output_headroom_is_low(0, endpoint, window, estimate));
    assert!(checked_max_output_tokens(0, endpoint, window, estimate).is_err());
    assert_eq!(
        checked_max_output_tokens(0, endpoint, window, 20_000).unwrap(),
        32_000
    );
    // Small user-selected caps and direct xAI's independent budget still work.
    assert_eq!(
        checked_max_output_tokens(1, endpoint, window, window - 1025).unwrap(),
        1
    );
    assert!(checked_max_output_tokens(1, endpoint, window, window - 1024).is_err());
    assert_eq!(
        checked_max_output_tokens(0, "https://api.x.ai/v1", window, estimate).unwrap(),
        256_000
    );
    assert_eq!(
        checked_max_output_tokens(0, endpoint, window, window - 1024 - 4096).unwrap(),
        4096
    );
    assert!(checked_max_output_tokens(0, endpoint, window, window - 1024 - 4095).is_err());
}

#[test]
fn multilingual_trajectory_respects_character_budget() {
    let history = vec![
        ChatMessage::new(Role::System, "system"),
        ChatMessage::new(Role::Tool, "界".repeat(800)),
        ChatMessage::new(Role::Tool, "工具日志".repeat(200)),
        ChatMessage::new(Role::Assistant, "🦀 café résumé ".repeat(100)),
        ChatMessage::new(Role::Tool, "trailing text".repeat(100)),
    ];
    let intent = build_recall_intent(&history, "continue");
    let (user, trajectory) = intent
        .split_once("\n\n--- recent trajectory ---\n")
        .unwrap();
    assert_eq!(user, "continue");
    assert_eq!(trajectory.chars().count(), TRAJECTORY_INTENT_CHARS);
    assert!(trajectory.contains("工具日志"));
    assert!(trajectory.contains("🦀 café résumé"));
}

#[tokio::test]
async fn full_blob_replaces_fts_excerpt_for_the_same_message() {
    let archive = Archive::new();
    let body = format!(
        "{} focusneedle exact_archived_result=42",
        "old material ".repeat(200)
    );
    let mut history = vec![ChatMessage::new(Role::System, "system")];
    history.push(archive.append(Role::User, &body));
    history.push(archive.append(Role::Assistant, "acknowledged"));
    msglog::write_summary(&archive.0, "Prior exchange complete.", 2, 5).unwrap();
    history.push(archive.append(Role::User, "focusneedle"));
    history.push(archive.append(Role::Assistant, &"current work ".repeat(100)));
    let hits = msglog::search_messages_before(&archive.0, "focusneedle", 2, 4).unwrap();
    assert_eq!(hits[0].id, 1);
    assert!(!hits[0].snippet.contains("exact_archived_result=42"));
    let out = shape(
        history,
        &archive.0,
        &OpenRouterClient::new(),
        &Settings {
            short_send_tail_n: 1,
            ..Settings::default()
        },
        None,
        "focusneedle",
        true,
        1000,
        false,
        &GoalWire::default(),
    )
    .await;
    assert!(out[0].content.contains("exact_archived_result=42"));
    assert_eq!(
        out[0].content.matches("exact_archived_result=42").count(),
        1
    );
    assert!(!out[0].content.contains("[archive msg #1 |"));
    assert_eq!(msglog::fetch_blob_content(&archive.0, 1).unwrap(), body);
}
