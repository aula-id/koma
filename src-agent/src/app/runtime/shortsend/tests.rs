use super::*;
use crate::dto::chat::{ChatMessage, FunctionCall, ReasoningDetail, Role, ToolCall};
use crate::model::{msglog, settings::Settings};
use crate::service::context_limits;

struct Archive(std::path::PathBuf);
impl Archive {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("drss-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn append(&self, role: Role, content: impl Into<String>) -> ChatMessage {
        let content = content.into();
        msglog::append(&self.0, role, &content, None, None).unwrap();
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
fn limits(window: u64) -> context_limits::ContextLimits {
    context_limits::resolve(
        "test/model",
        "",
        "",
        0,
        &[context_limits::CatalogModel {
            id: "test/model".into(),
            context_length: Some(window),
            ..Default::default()
        }],
        &[],
    )
}
fn send(history: &[ChatMessage], archive: &Archive, window: u64) -> Vec<ChatMessage> {
    shape(
        history.to_vec(),
        &archive.0,
        &Settings::default(),
        "continue",
        &GoalWire::default(),
        &limits(window),
        0,
    )
    .unwrap()
}
fn live_tokens(out: &[ChatMessage]) -> u64 {
    out[2..].iter().map(budget::message_tokens).sum()
}
fn boundary(archive: &Archive) -> i64 {
    msglog::drss::Index::load(&archive.0).unwrap().boundary
}

#[test]
fn trims_to_60_then_grows_to_75_without_touching_the_visible_rail() {
    let archive = Archive::new();
    let mut history = vec![ChatMessage::new(Role::System, "original system")];
    history.push(archive.append(Role::User, "Keep the public API unchanged."));
    for i in 0..22 {
        history.push(archive.append(Role::Assistant, format!("step-{i} {}", "x".repeat(10000))));
    }
    let snapshot = serde_json::to_vec(&history).unwrap();
    std::fs::write(archive.0.join("messages.json"), &snapshot).unwrap();
    let count = msglog::max_message_id(&archive.0);
    let out = send(&history, &archive, 100_000);
    assert_eq!(out[0], history[0]);
    assert_eq!(out[1].role, Role::User);
    assert!(out[1].content.starts_with("[DRSS archive index"));
    assert!(live_tokens(&out) <= 60_000);
    assert!(out
        .iter()
        .any(|m| m.content == "Keep the public API unchanged."));
    assert_eq!(serde_json::to_vec(&history).unwrap(), snapshot);
    assert_eq!(
        std::fs::read(archive.0.join("messages.json")).unwrap(),
        snapshot
    );
    assert_eq!(msglog::max_message_id(&archive.0), count);
    for (i, m) in history.iter().skip(1).enumerate() {
        assert_eq!(
            msglog::fetch_blob_content(&archive.0, i as i64 + 1).unwrap(),
            m.content
        );
    }
    let first_cut = boundary(&archive);
    assert!(first_cut > 0);
    for i in 0..2 {
        history.push(archive.append(Role::Assistant, format!("more-{i} {}", "y".repeat(10000))));
    }
    let second = send(&history, &archive, 100_000);
    assert!(live_tokens(&second) > 60_000 && live_tokens(&second) <= 75_000);
    assert_eq!(boundary(&archive), first_cut);
    for i in 0..4 {
        history.push(archive.append(Role::Assistant, format!("later-{i} {}", "z".repeat(10000))));
    }
    assert!(live_tokens(&send(&history, &archive, 100_000)) <= 60_000);
    assert!(boundary(&archive) > first_cut);
}

#[test]
fn active_multi_tool_round_keeps_calls_results_and_replay_metadata() {
    let archive = Archive::new();
    let mut history = vec![
        ChatMessage::new(Role::System, "system"),
        archive.append(Role::User, "inspect the result"),
    ];
    let mut assistant = archive.append(Role::Assistant, "reading results");
    assistant.tool_calls = Some(
        (1..=2)
            .map(|i| ToolCall {
                id: format!("call-{i}"),
                kind: "function".into(),
                function: FunctionCall {
                    name: "read_file".into(),
                    arguments: format!("{{\"path\":\"file-{i}\"}}"),
                },
            })
            .collect(),
    );
    assistant.reasoning_details = Some(vec![ReasoningDetail {
        signature: Some("preserve-signature".into()),
        ..Default::default()
    }]);
    history.push(assistant.clone());
    for i in 1..=2 {
        let mut result = archive.append(
            Role::Tool,
            format!("result-{i} {} exact-end-{i}", "界".repeat(20000)),
        );
        result.tool_call_id = Some(format!("call-{i}"));
        history.push(result);
    }
    let out = send(&history, &archive, 100_000);
    let opening = out.iter().position(|m| m.tool_calls.is_some()).unwrap();
    assert_eq!(out[opening], assistant);
    for i in 1..=2 {
        assert_eq!(
            out[opening + i].tool_call_id.as_deref(),
            Some(format!("call-{i}").as_str())
        );

        assert!(msglog::fetch_blob_content(&archive.0, i as i64 + 2)
            .unwrap()
            .contains(&format!("exact-end-{i}")));
    }
    assert_eq!(
        out.iter()
            .filter(|m| m.content.contains("DRSS live body stored"))
            .count(),
        1
    );
    assert!(live_tokens(&out) <= 75_000);
}

#[test]
fn index_is_deterministic_and_ignores_old_inference_summary_and_reasoning() {
    let archive = Archive::new();
    let mut history = vec![ChatMessage::new(Role::System, "unchanged system")];
    for i in 0..23 {
        history.push(archive.append(
            Role::User,
            format!("request-{i} {}", "focusneedle ".repeat(1000)),
        ));
        history.push(archive.append(Role::Assistant, "completed"));
    }
    msglog::write_summary(&archive.0, "DO NOT REPLAY LEGACY SUMMARY", 10, 11).unwrap();
    let conn = msglog::open(&archive.0).unwrap();
    conn.execute(
        "UPDATE messages SET reasoning='PRIVATE_REASONING_SENTINEL'",
        [],
    )
    .unwrap();
    let first = send(&history, &archive, 100_000);
    let second = send(&history, &archive, 100_000);
    assert_eq!(first, second);
    assert!(first[1].content.contains("occurrences /"));
    assert!(!first
        .iter()
        .any(|m| m.content.contains("PRIVATE_REASONING_SENTINEL")
            || m.content.contains("DO NOT REPLAY LEGACY SUMMARY")));
    assert!(
        budget::message_tokens(&first[1]) <= budget::RECOVERY_INDEX_MAX_TOKENS.min(100_000 / 20)
    );
}

#[test]
fn only_raw_user_history_request_enables_assistant_excerpts() {
    let archive = Archive::new();
    archive.append(Role::Assistant, "focusneedle obsolete proposal");
    let index = msglog::drss::Index::load(&archive.0).unwrap();
    assert!(index
        .excerpts(1, &["focusneedle".into()], false)
        .unwrap()
        .is_empty());
    assert_eq!(
        index.excerpts(1, &["focusneedle".into()], true).unwrap()[0].0,
        1
    );
}

#[test]
fn archive_relevance_uses_latest_diagnostics_and_exact_unicode_match_windows() {
    let archive = Archive::new();
    let dump = format!("{} latestdiagnostic exact_result=42", "İ界 ".repeat(2000));
    archive.append(Role::Tool, &dump);
    archive.append(Role::Assistant, "focusneedle obsolete_draft_marker");
    let index = msglog::drss::Index::load(&archive.0).unwrap();
    index.save_boundary(2).unwrap();
    drop(index);
    let history = vec![
        ChatMessage::new(Role::System, "system-only directive"),
        archive.append(Role::User, "continue"),
        archive.append(
            Role::Assistant,
            "My earlier plan: inspect focusneedle latestdiagnostic",
        ),
    ];
    for (user, recall_draft) in [
        ("continue", false),
        ("explain planet configuration", false),
        ("recall our earlier plan", true),
    ] {
        let out = shape(
            history.clone(),
            &archive.0,
            &Settings::default(),
            user,
            &GoalWire::default(),
            &limits(100_000),
            0,
        )
        .unwrap();
        let memory = &out[1].content;
        assert!(memory.contains("latestdiagnostic exact_result=42"));
        assert_eq!(
            memory.contains("focusneedle obsolete_draft_marker"),
            recall_draft
        );
        assert!(!memory.contains("system-only directive"));
        let excerpt = msglog::drss::Index::load(&archive.0)
            .unwrap()
            .excerpts(1, &["latestdiagnostic".into()], false)
            .unwrap();
        assert!(dump.contains(&excerpt[0].2));
    }
}

#[test]
fn paged_unicode_and_nul_reads_round_trip_and_are_not_restubbed() {
    use crate::tool::Tool;
    for payload in ["工具\n", "工具\0\n"] {
        let archive = Archive::new();
        let body = format!("{} exact_result=42", payload.repeat(25000));
        let history = vec![
            ChatMessage::new(Role::System, "system"),
            archive.append(Role::User, "read the result"),
            archive.append(Role::Assistant, &body),
        ];
        assert!(send(&history, &archive, 100_000)[3]
            .content
            .contains("\"message_id\":2"));
        let mut restored = String::new();
        let mut offset = 0;
        loop {
            let output = crate::tool::history::MessageLoad
                .run(
                    &archive.tool_ctx(),
                    &serde_json::json!({"message_id":2,"offset":offset}),
                )
                .unwrap();
            let (header, content) = output.split_once('\n').unwrap();
            let header: serde_json::Value = serde_json::from_str(header).unwrap();
            restored.push_str(content);
            if offset == 0 {
                // This active round has both an oversized result and the archive
                // page: shape must stub only the former despite token pressure.
                let mut live = vec![history[0].clone(), history[1].clone()];
                let mut assistant = archive.append(Role::Assistant, "read both");
                assistant.tool_calls = Some(vec![
                    ToolCall {
                        id: "read-page".into(),
                        kind: "function".into(),
                        function: FunctionCall {
                            name: "message_load".into(),
                            arguments: "{\"message_id\":2}".into(),
                        },
                    },
                    ToolCall {
                        id: "large".into(),
                        kind: "function".into(),
                        function: FunctionCall {
                            name: "read_file".into(),
                            arguments: "{}".into(),
                        },
                    },
                ]);
                live.push(assistant);
                let mut page = archive.append(Role::Tool, &output);
                page.tool_call_id = Some("read-page".into());
                live.push(page.clone());
                let mut large = archive.append(Role::Tool, &body);
                large.tool_call_id = Some("large".into());
                live.push(large);
                let shaped = send(&live, &archive, 100_000);
                assert!(shaped.contains(&page));
                assert!(shaped
                    .last()
                    .unwrap()
                    .content
                    .contains("DRSS live body stored"));
            }
            match header["next_offset"].as_i64() {
                Some(next) => {
                    assert!(next > offset);
                    offset = next;
                }
                None => break,
            }
        }
        assert_eq!(restored, body);
    }
}

#[test]
fn unavailable_archive_preserves_small_requests_and_refuses_unsafe_clipping() {
    let archive = Archive::new();
    let path = archive.0.join("not-a-directory");
    std::fs::write(&path, b"file").unwrap();
    let mut history = vec![
        ChatMessage::new(Role::System, "system"),
        ChatMessage::new(Role::User, "small"),
    ];
    assert_eq!(
        shape(
            history.clone(),
            &path,
            &Settings::default(),
            "",
            &GoalWire::default(),
            &limits(100_000),
            0
        )
        .unwrap(),
        history
    );
    history[1].content = "x".repeat(300000);
    assert!(shape(
        history,
        &path,
        &Settings::default(),
        "",
        &GoalWire::default(),
        &limits(100_000),
        0
    )
    .is_err());
}

#[test]
fn unarchived_live_text_is_preserved_or_fails_explicitly() {
    let archive = Archive::new();
    let history = vec![
        ChatMessage::new(Role::System, "system"),
        ChatMessage::new(Role::User, "unindexed latest request"),
    ];
    let out = send(&history, &archive, 100_000);
    assert_eq!(out.last(), history.last());
    let huge = vec![
        history[0].clone(),
        ChatMessage::new(Role::User, "x".repeat(250000)),
    ];
    assert!(shape(
        huge.clone(),
        &archive.0,
        &Settings::default(),
        "",
        &GoalWire::default(),
        &limits(100_000),
        0
    )
    .is_err());
    assert_eq!(huge[1].content.len(), 250000);
}

#[test]
fn disabled_drss_is_identity_and_full_request_guard_still_applies() {
    let archive = Archive::new();
    let history = vec![
        ChatMessage::new(Role::System, "system"),
        ChatMessage::new(Role::User, "x".repeat(300000)),
    ];
    let settings = Settings {
        short_send_enabled: false,
        ..Default::default()
    };
    assert_eq!(
        shape(
            history.clone(),
            &archive.0,
            &settings,
            "",
            &GoalWire::default(),
            &limits(100_000),
            0
        )
        .unwrap(),
        history
    );
    assert!(limits(100_000)
        .output_tokens(0, budget::prompt_tokens(&history, 0))
        .is_err());
}

#[test]
fn clear_and_resend_invalidate_stale_archive_boundaries() {
    let archive = Archive::new();
    for _ in 0..3 {
        archive.append(Role::User, "oldkeyword");
    }
    let index = msglog::drss::Index::load(&archive.0).unwrap();
    index.save_boundary(3).unwrap();
    drop(index);
    msglog::truncate_after(&archive.0, 2).unwrap();
    assert_eq!(boundary(&archive), 1);
    archive.append(Role::User, "newkeyword");
    let index = msglog::drss::Index::load(&archive.0).unwrap();
    assert_eq!(
        index
            .statistics(20, &["oldkeyword".into()])
            .unwrap()
            .iter()
            .find(|r| r.0 == "oldkeyword")
            .unwrap()
            .2,
        1
    );
    drop(index);
    msglog::clear_rolling_summary(&archive.0).unwrap();
    let index = msglog::drss::Index::load(&archive.0).unwrap();
    assert!(index
        .statistics(20, &["oldkeyword".into()])
        .unwrap()
        .is_empty());
}

#[test]
fn full_budget_includes_system_schemas_and_reserved_reply() {
    let archive = Archive::new();
    let mut history = vec![ChatMessage::new(Role::System, "s".repeat(60000))];
    for _ in 0..22 {
        history.push(archive.append(Role::Assistant, "x".repeat(10000)));
    }
    let limit = limits(100_000);
    let out = shape(
        history,
        &archive.0,
        &Settings::default(),
        "",
        &GoalWire::default(),
        &limit,
        6000,
    )
    .unwrap();
    assert!(
        budget::prompt_tokens(&out, 6000)
            + limit.reserved_output(0)
            + context_limits::OUTPUT_MARGIN
            <= 100_000
    );
    assert!(live_tokens(&out) < 60_000);
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
fn oversized_legacy_session_recovers_immediately_without_changing_history() {
    let archive = Archive::new();
    let mut history = vec![ChatMessage::new(Role::System, "original system")];
    for i in 0..36 {
        history.push(ChatMessage::new(
            Role::User,
            format!("Keep requirement-{i}. {}", "legacy context ".repeat(1800)),
        ));
        history.push(ChatMessage::new(Role::Assistant, "completed"));
    }
    history.push(ChatMessage::new(Role::User, "continue the current work"));
    let bytes = serde_json::to_vec(&history).unwrap();
    std::fs::write(archive.0.join("messages.json"), &bytes).unwrap();
    let first = send(&history, &archive, 100_000);
    assert!(first.len() < history.len());
    assert_eq!(first[0], history[0]);
    assert_eq!(first.last(), history.last());
    assert!(first[1].content.contains("Recovery handoff:"));
    assert!(first[1].content.contains("archive_key"));
    assert!(budget::message_tokens(&first[1]) > budget::INDEX_MAX_TOKENS);
    assert!(budget::message_tokens(&first[1]) <= 5000);
    assert!(live_tokens(&first) <= 60_000);
    assert_eq!(
        std::fs::read(archive.0.join("messages.json")).unwrap(),
        bytes
    );
    assert_eq!(serde_json::to_vec(&history).unwrap(), bytes);
    assert_eq!(msglog::max_message_id(&archive.0), 0);
    let conn = msglog::open(&archive.0).unwrap();
    let covered: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM drss_recovery WHERE covered=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM drss_recovery", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, history.len() as i64 - 1); // repeated "completed" messages stay distinct
    assert!(covered > 0);
    assert_eq!(send(&history, &archive, 100_000), first);
    history.push(archive.append(Role::Assistant, "new live progress"));
    let second = send(&history, &archive, 100_000);
    assert_eq!(second.last(), history.last());
    assert_eq!(
        conn.query_row::<i64, _, _>(
            "SELECT COUNT(*) FROM drss_recovery WHERE covered=1",
            [],
            |r| r.get(0)
        )
        .unwrap(),
        covered
    );
    assert_eq!(msglog::max_message_id(&archive.0), 1);
}

#[test]
fn partial_archive_keeps_existing_ids_and_recovery_pages_round_trip() {
    use crate::tool::Tool;
    let archive = Archive::new();
    let body = format!("{} exact_legacy_end", "工具\0\n".repeat(12000));
    let mut history = vec![ChatMessage::new(Role::System, "system")];
    for _ in 0..3 {
        history.push(ChatMessage::new(Role::User, "old request"));
        history.push(ChatMessage::new(Role::Assistant, &body));
    }
    history.push(archive.append(Role::User, "continue"));
    let out = send(&history, &archive, 100_000);
    assert_eq!(out.last(), history.last());
    assert_eq!(msglog::max_message_id(&archive.0), 1);
    assert_eq!(
        msglog::fetch_blob_content(&archive.0, 1).unwrap(),
        "continue"
    );
    let conn = msglog::open(&archive.0).unwrap();
    let key: String = conn
        .query_row(
            "SELECT archive_key FROM drss_recovery WHERE content=?1 LIMIT 1",
            [&body],
            |r| r.get(0),
        )
        .unwrap();
    let mut restored = String::new();
    let mut offset = 0;
    loop {
        let page = crate::tool::history::MessageFind
            .run(
                &archive.tool_ctx(),
                &serde_json::json!({"archive_key":key,"offset":offset}),
            )
            .unwrap();
        let (header, content) = page.split_once('\n').unwrap();
        let header: serde_json::Value = serde_json::from_str(header).unwrap();
        assert_eq!(header["archive_key"], key);
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
    let found = crate::tool::history::MessageFind
        .run(
            &archive.tool_ctx(),
            &serde_json::json!({"query":"exact_legacy_end", "role":"assistant"}),
        )
        .unwrap();
    assert!(found.contains(":recovery:"));
    assert!(!found.contains("(no matching messages found)"));
    assert!(crate::tool::history::MessageFind
        .run(
            &archive.tool_ctx(),
            &serde_json::json!({"query":"exact_legacy_end", "role":"user"})
        )
        .unwrap()
        .to_lowercase()
        .contains("no matching"));
    // Another session cannot read this recovery record.
    assert!(crate::tool::history::MessageFind
        .run(
            &Archive::new().tool_ctx(),
            &serde_json::json!({"archive_key":key})
        )
        .is_err());
    msglog::truncate_after(&archive.0, 1).unwrap();
    assert_eq!(
        conn.query_row::<i64, _, _>(
            "SELECT COUNT(*) FROM drss_recovery WHERE covered=1",
            [],
            |r| r.get(0)
        )
        .unwrap(),
        0
    );
    send(&history, &archive, 100_000);
    msglog::clear_rolling_summary(&archive.0).unwrap();
    assert_eq!(
        conn.query_row::<i64, _, _>(
            "SELECT COUNT(*) FROM drss_recovery WHERE covered=1",
            [],
            |r| r.get(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row::<String, _, _>(
            "SELECT content FROM drss_recovery WHERE archive_key=?1",
            [&key],
            |r| r.get(0)
        )
        .unwrap(),
        body
    );
}

#[test]
fn active_context_can_borrow_spare_room_without_raising_the_300k_cap() {
    let archive = Archive::new();
    let history = vec![
        ChatMessage::new(Role::System, "system"),
        archive.append(Role::User, "x".repeat(600000)),
    ];
    let limits = limits(1_000_000);
    assert_eq!(limits.effective_window, 300_000);
    let out = send(&history, &archive, 1_000_000);
    assert_eq!(out.last(), history.last());
    assert!(live_tokens(&out) > 225_000);
    let prompt = budget::prompt_tokens(&out, 0);
    let reply = limits.output_tokens(0, prompt).unwrap();
    assert!(reply >= 4096);
    assert!(prompt + u64::from(reply) + context_limits::OUTPUT_MARGIN <= 300_000);
    assert!(!out
        .last()
        .unwrap()
        .content
        .contains("DRSS live body stored"));
}

#[test]
fn message_find_plain_object_schema_still_validates_read_modes() {
    use crate::tool::Tool;
    let tool = crate::tool::history::MessageFind;
    let schema = tool.parameters();
    assert_eq!(schema["type"], "object");
    for union in ["anyOf", "oneOf", "allOf"] {
        assert!(schema.get(union).is_none());
    }
    let archive = Archive::new();
    archive.append(Role::User, "hello");
    for args in [
        serde_json::json!({}),
        serde_json::json!({"archive_key":"bad"}),
        serde_json::json!({"archive_key":"a".repeat(64),"message_id":1}),
        serde_json::json!({"archive_key":"a".repeat(64),"query":"hello"}),
        serde_json::json!({"archive_key":"a".repeat(64),"scope":"project"}),
    ] {
        assert!(tool.run(&archive.tool_ctx(), &args).is_err());
    }
    assert!(tool
        .run(&archive.tool_ctx(), &serde_json::json!({"message_id":1}))
        .unwrap()
        .contains("hello"));
}

#[test]
fn large_recovery_summary_prioritizes_newest_work_and_keeps_kickoff_reference() {
    let archive = Archive::new();
    let mut history = vec![ChatMessage::new(Role::System, "system")];
    for i in 0..20 {
        history.push(ChatMessage::new(
            Role::User,
            format!("turn-{i} 最新工作 {}", "界".repeat(8000)),
        ));
        history.push(ChatMessage::new(Role::Assistant, "done"));
    }
    history.push(ChatMessage::new(Role::User, "continue"));
    let out = shape(
        history.clone(),
        &archive.0,
        &Settings::default(),
        "continue",
        &GoalWire {
            source: "user".into(),
            objective: "goal ".repeat(2400),
            ..Default::default()
        },
        &limits(100_000),
        0,
    )
    .unwrap();
    assert!(out[1].content.contains("Kickoff excerpt:"));
    assert!(out[1].content.contains("turn-0 最新工作"));
    assert!(out[1].content.contains("turn-17 最新工作"));
    assert!(out[1].content.contains("archive_key"));
    assert!(budget::message_tokens(&out[1]) <= 5000);
    assert_eq!(out.last(), history.last());
}
