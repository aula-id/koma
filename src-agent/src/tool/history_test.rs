use super::*;
use crate::{
    dto::chat::{ChatMessage, Role},
    model::msglog,
};

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("koma-history-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
    fn append(&self, role: Role, text: &str, time: i64) -> i64 {
        msglog::append(self.path(), role, text, None, None).unwrap();
        let id = msglog::max_message_id(self.path());
        msglog::open(self.path())
            .unwrap()
            .execute(
                "UPDATE messages SET created_at=?1 WHERE id=?2",
                rusqlite::params![time, id],
            )
            .unwrap();
        id
    }
    fn target(&self, current: bool) -> SearchTarget {
        SearchTarget {
            path: self.0.clone(),
            uuid: session_uuid_from_dir(self.path()),
            name: "Example session".into(),
            is_current: current,
        }
    }
    fn find(&self, args: Value) -> Value {
        let options = Options::parse(&args).unwrap();
        let page = search_targets(
            &[self.target(true)],
            &options,
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
        serde_json::from_str(&format_page(page, &options).unwrap()).unwrap()
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn search_pages_are_latest_before_limit_and_have_distinct_refs() {
    let dir = TempDir::new();
    dir.append(Role::User, &"needle ".repeat(100), 1);
    for i in 2..=27 {
        dir.append(Role::User, &format!("note {i} with needle"), i);
    }
    let first = dir.find(json!({"query":"needle"}));
    assert_eq!(first["returned"], 10);
    assert_eq!(first["next_skip"], 10);
    assert_eq!(first["has_more"], true);
    assert!(first["results"][0]["ref"]
        .as_str()
        .unwrap()
        .ends_with(":message:27"));
    assert_eq!(first["results"][0]["timestamp"], "1970-01-01T00:00:27Z");
    let second = dir.find(json!({"query":"needle","skip":10}));
    for hit in second["results"].as_array().unwrap() {
        assert!(!first["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["ref"] == hit["ref"]));
    }
    let last = dir.find(json!({"query":"needle","skip":20}));
    assert_eq!(last["returned"], 7);
    assert_eq!(last["has_more"], false);
    assert!(last["next_skip"].is_null());
    let oldest = dir.find(json!({"query":"needle","sort":"oldest","limit":1}));
    assert!(oldest["results"][0]["ref"]
        .as_str()
        .unwrap()
        .ends_with(":message:1"));
}

#[test]
fn project_order_is_global_and_partial_results_are_honest() {
    let current = TempDir::new();
    let sibling = TempDir::new();
    for i in 0..12 {
        current.append(Role::User, "needle", i);
    }
    sibling.append(Role::User, "needle newest sibling", 100);
    let options = Options::parse(&json!({"query":"needle","scope":"project"})).unwrap();
    let targets = [current.target(true), sibling.target(false)];
    let page = search_targets(&targets, &options, Instant::now() + Duration::from_secs(5)).unwrap();
    assert_eq!(page.hits[0].session, session_uuid_from_dir(sibling.path()));
    let page = search_targets(&targets, &options, Instant::now() - Duration::from_secs(1)).unwrap();
    let result: Value = serde_json::from_str(&format_page(page, &options).unwrap()).unwrap();
    assert_eq!(result["complete"], false);
    assert!(result["has_more"].is_null());
    assert!(result["next_skip"].is_null());
}

#[test]
fn time_and_role_filters_apply_before_pagination() {
    let dir = TempDir::new();
    dir.append(Role::User, "needle before", 3599);
    dir.append(Role::User, "needle start", 3600);
    dir.append(Role::Assistant, "needle assistant", 4000);
    dir.append(Role::User, "needle end", 7200);
    let page = dir.find(json!({"role":"user","after":"1970-01-01T10:00:00+09:00","before":"1970-01-01T11:00:00+09:00"}));
    assert_eq!(page["returned"], 1);
    assert_eq!(page["results"][0]["preview"], "needle start");
}

#[test]
fn preview_finds_deep_match_without_reasoning_and_load_is_exact() {
    let dir = TempDir::new();
    let text = format!(
        "{}\nThe user chose needle for the new limit. Preserve Unicode 界 and NUL \0.\nEnd.",
        "unrelated paragraph. ".repeat(1000)
    );
    let id = dir.append(Role::User, &text, 100);
    msglog::open(dir.path())
        .unwrap()
        .execute(
            "UPDATE messages SET reasoning='PRIVATE_REASONING_SENTINEL'",
            [],
        )
        .unwrap();
    let result = dir.find(json!({"query":"needle"}));
    let preview = result["results"][0]["preview"].as_str().unwrap();
    assert!(preview.contains("needle"), "{preview}");
    assert!(!result.to_string().contains("PRIVATE_REASONING_SENTINEL"));
    let reference = result["results"][0]["ref"].clone();
    let mut restored = String::new();
    let mut offset = 0;
    loop {
        let page = page::load(
            dir.path(),
            &json!({"ref":reference,"offset":offset,"max_chars":299}),
        )
        .unwrap();
        let (meta, content) = page.split_once('\n').unwrap();
        let meta: Value = serde_json::from_str(meta).unwrap();
        assert_eq!(meta["message_id"], id);
        assert!(content.chars().count() <= 299);
        restored.push_str(content);
        match meta["next_offset"].as_u64() {
            Some(n) => {
                assert!(n > offset);
                offset = n
            }
            None => break,
        }
    }
    assert_eq!(restored, text);
}

#[test]
fn recovery_refs_load_and_unknown_times_are_excluded_by_ranges() {
    let dir = TempDir::new();
    let body = vec![
        ChatMessage::new(Role::User, "needle first"),
        ChatMessage::new(Role::User, "needle second"),
    ];
    let mut index = msglog::drss::Index::load(dir.path()).unwrap();
    let refs = index.recover_missing(&body, &[None, None]).unwrap();
    let result = dir.find(json!({"query":"needle"}));
    assert_eq!(result["returned"], 2);
    assert_eq!(result["results"][0]["preview"], "needle second");
    assert!(result["results"][0]["timestamp"].is_null());
    assert!(result["results"][0]["ref"]
        .as_str()
        .unwrap()
        .ends_with(&refs[1].as_ref().unwrap().key));
    let loaded = page::load(dir.path(), &json!({"ref":result["results"][0]["ref"]})).unwrap();
    assert!(loaded.ends_with("needle second"));
    let filtered = dir.find(json!({"query":"needle","after":"1970-01-01T00:00:00Z"}));
    assert_eq!(filtered["returned"], 0);
}

#[test]
fn whole_response_is_bounded_and_next_skip_counts_emitted_rows() {
    let dir = TempDir::new();
    for i in 0..30 {
        dir.append(Role::Tool, &format!("needle {}", "界 ".repeat(400)), i);
    }
    let options = Options::parse(&json!({"query":"needle","limit":20})).unwrap();
    let page = search_targets(
        &[dir.target(true)],
        &options,
        Instant::now() + Duration::from_secs(5),
    )
    .unwrap();
    let out = format_page(page, &options).unwrap();
    assert!(out.chars().count() <= MAX_SEARCH_CHARS && out.len() <= MAX_SEARCH_BYTES);
    let result: Value = serde_json::from_str(&out).unwrap();
    assert!(result["returned"].as_u64().unwrap() > 0);
    assert_eq!(result["next_skip"], result["returned"]);
    assert!(result["results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["preview"].as_str().unwrap().chars().count() <= 400));
}

#[test]
fn arguments_reject_invalid_dates_modes_types_and_sizes() {
    for args in [
        json!({}),
        json!({"query":"!"}),
        json!({"query":1}),
        json!({"role":"system"}),
        json!({"query":"needle","skip":-1}),
        json!({"query":"needle","skip":10001}),
        json!({"query":"needle","limit":21}),
        json!({"query":"needle","offset":1}),
        json!({"query":"needle","scope":"all"}),
        json!({"query":"needle","sort":"random"}),
        json!({"query":"needle","after":"2026-02-30T00:00:00Z"}),
        json!({"query":"needle","after":"2026-09-20T00:00:00"}),
        json!({"query":"needle","after":"2026-09-21T00:00:00Z","before":"2026-09-20T00:00:00Z"}),
        json!({"query":"needle","after":"2026-09-20T25:00:00Z"}),
        json!({"query":"needle","after":"2026-09-20T00:00:00+14:30"}),
    ] {
        assert!(Options::parse(&args).is_err(), "{args}");
    }
    assert!(Options::parse(&json!({"after":"2024-02-29T00:00:00Z"})).is_ok());
    for schema in [MessageFind.parameters(), MessageLoad.parameters()] {
        assert_eq!(schema["type"], "object");
        for key in ["anyOf", "oneOf", "allOf"] {
            assert!(schema.get(key).is_none());
        }
    }
}

#[test]
fn load_rejects_ambiguous_ids_and_path_references() {
    let dir = TempDir::new();
    dir.append(Role::User, "hello", 1);
    for args in [
        json!({}),
        json!({"message_id":1,"archive_key":"a".repeat(64)}),
        json!({"ref":"../escape:message:1"}),
        json!({"ref":"missing:message:1"}),
        json!({"ref":"bad","message_id":1}),
        json!({"message_id":1,"query":"hello"}),
        json!({"message_id":1,"max_chars":3001}),
        json!({"message_id":1,"max_chars":0}),
        json!({"message_id":1,"offset":-1}),
        json!({"message_id":1,"limit":10}),
    ] {
        assert!(page::load(dir.path(), &args).is_err(), "{args}");
    }
}

#[test]
fn attachment_hints_are_on_load_and_discovery_is_compact() {
    let dir = TempDir::new();
    std::fs::create_dir_all(dir.path().join("images")).unwrap();
    std::fs::write(dir.path().join("images/03-shot.png"), b"image").unwrap();
    std::fs::create_dir_all(dir.path().join("pastes")).unwrap();
    std::fs::write(dir.path().join("pastes/02-paste.txt"), b"full paste").unwrap();
    dir.append(Role::User, "needle [Image #3] [Pasted Text #2]", 1);
    let found = dir.find(json!({"query":"needle"}));
    assert!(!found.to_string().contains("attachment_hints"));
    let loaded = page::load(dir.path(), &json!({"ref":found["results"][0]["ref"]})).unwrap();
    let header: Value = serde_json::from_str(loaded.split_once('\n').unwrap().0).unwrap();
    assert!(header["attachment_hints"]
        .as_str()
        .unwrap()
        .contains("load_image"));
    assert!(header["attachment_hints"]
        .as_str()
        .unwrap()
        .contains("read("));
}

#[test]
fn scope_and_unicode_helpers_remain_compatible() {
    assert_eq!(parse_scope(None).unwrap(), SearchScope::Session);
    assert_eq!(parse_scope(Some("project")).unwrap(), SearchScope::Project);
    assert!(parse_scope(Some("all")).is_err());
    assert_eq!(floor_chars("ab界cd", 3), "ab界");
    assert_eq!(
        pwd_hash_from_session_dir(Path::new("/tmp/bucket/session")).as_deref(),
        Some("bucket")
    );
    let dir = TempDir::new();
    let targets = resolve_targets(dir.path(), SearchScope::Session).unwrap();
    assert_eq!(targets.len(), 1);
    assert!(targets[0].is_current);
}

#[test]
fn matching_passage_survives_a_giant_preceding_token() {
    for prefix in ["x".repeat(6000), "界".repeat(6000), "nul\0 ".repeat(1000)] {
        let dir = TempDir::new();
        dir.append(
            Role::Tool,
            &format!("{prefix} needle is the selected setting. End."),
            1,
        );
        let found = dir.find(json!({"query":"needle"}));
        let preview = found["results"][0]["preview"].as_str().unwrap();
        assert!(
            preview.contains("needle is the selected setting"),
            "{preview}"
        );
        assert!(preview.chars().count() <= 400);
    }
}

#[test]
fn project_load_resolves_registered_sibling_and_rejects_other_buckets() {
    let bucket = TempDir::new();
    let current = TempDir(bucket.path().join("current"));
    let sibling = TempDir(bucket.path().join("sibling"));
    std::fs::create_dir_all(current.path()).unwrap();
    std::fs::create_dir_all(sibling.path()).unwrap();
    let outside = TempDir::new();
    current.append(Role::User, "current message with ID 1", 1);
    sibling.append(Role::User, "sibling message with ID 1", 2);
    outside.append(Role::User, "outside message with ID 1", 3);
    let targets = [
        current.target(true),
        sibling.target(false),
        outside.target(false),
    ];
    let resolved = page::registered_target(current.path(), "sibling", &targets).unwrap();
    let output = page::read(&resolved, &json!({"message_id":1})).unwrap();
    assert!(output.ends_with("sibling message with ID 1"));
    assert!(page::registered_target(current.path(), "unregistered", &targets).is_err());
    assert!(page::registered_target(
        current.path(),
        &session_uuid_from_dir(outside.path()),
        &targets
    )
    .is_err());
    #[cfg(unix)]
    {
        let link = bucket.path().join("symlink");
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        let target = SearchTarget {
            path: link,
            uuid: "symlink".into(),
            name: String::new(),
            is_current: false,
        };
        assert!(page::registered_target(current.path(), "symlink", &[target]).is_err());
    }
}

#[test]
fn tool_registration_and_prompt_examples_describe_the_same_contract() {
    let names = crate::tool::main_tool_names();
    assert!(names.iter().any(|name| name == "message_load"));
    assert!(crate::tool::tool_allowed_in_plan("message_load"));
    assert!(crate::tool::DEFERRED_TOOLS.contains(&"message_load"));
    let instructions = include_str!("../../../src-misc/system-tools.txt");
    let dir = TempDir::new();
    for i in 1..=42 {
        let body = if i == 42 {
            "context cap ".repeat(400)
        } else {
            format!("context cap {i}")
        };
        dir.append(Role::User, &body, i);
    }
    let mut find_examples = 0;
    let mut load_examples = 0;
    for line in instructions.lines().map(str::trim) {
        if let Some(example) = line
            .strip_prefix("Example: message_find(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let args: Value = serde_json::from_str(example).unwrap();
            Options::parse(&args).unwrap();
            find_examples += 1;
        }
        if let Some(example) = line
            .strip_prefix("Example: message_load(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let args: Value = serde_json::from_str(example).unwrap();
            page::load(dir.path(), &args).unwrap();
            load_examples += 1;
        }
    }
    assert!(find_examples > 0 && load_examples > 0);
    assert!(include_str!("../../../src-misc/system-prompt.txt").contains("message_load"));
}
