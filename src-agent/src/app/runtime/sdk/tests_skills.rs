//! Tests for the skills methods. Every test uses a data dir and a workdir of
//! its own; the global tier lives under the test's data dir.
use super::binding::BindingState;
use super::host::config::ConfigStore;
use super::host::skills::{check_new_name, propagation, INLINE_B64_CHARS};
use super::protocol::{Request, RpcError};
use super::*;
use base64::Engine as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

const PLANTED_KEY: &str = "sk-planted-skills-0123456789";

/// A fresh data dir and workdir under the system temp dir, removed on drop.
struct Fixture {
    base: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!("koma-skl-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(base.join("data")).unwrap();
        std::fs::create_dir_all(base.join("work")).unwrap();
        let base = std::fs::canonicalize(base).unwrap();
        let config = crate::model::app_config::AppConfig {
            providers: vec![crate::model::app_config::ProviderConn {
                uuid: "p-local".into(),
                name: "local".into(),
                api_type: crate::model::app_config::ApiType::OpenAiCompatible,
                endpoint: "http://127.0.0.1:9/v1".into(),
                api_key: PLANTED_KEY.into(),
                ext_id: None,
            }],
            ..Default::default()
        };
        let key = crate::model::secret_box::key_for_write().unwrap();
        let bytes = config.to_disk_bytes(&key).unwrap();
        crate::model::secret_box::write_private(&base.join("data").join("config.json"), &bytes)
            .unwrap();
        Self { base }
    }
    fn data(&self) -> PathBuf {
        self.base.join("data")
    }
    fn work(&self) -> PathBuf {
        self.base.join("work")
    }
    fn workdir(&self) -> String {
        self.work().to_string_lossy().into_owned()
    }
    fn global_root(&self) -> PathBuf {
        self.data().join(".agents").join("skills")
    }
    fn tier_root(&self, dir: &str) -> PathBuf {
        self.work().join(dir).join("skills")
    }
    fn adapter(&self) -> (Adapter, watch::Receiver<Option<State>>) {
        let (events, rx) = watch::channel(None);
        let mut adapter = Adapter::new(events);
        adapter.config = ConfigStore::at(&self.data());
        (adapter, rx)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn write_skill(root: &Path, name: &str, description: &str, body: &str) {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        format!("---\ndescription: {description}\n---\n{body}\n"),
    )
    .unwrap();
}

async fn call(adapter: &mut Adapter, method: &str, params: Value) -> Result<Value, RpcError> {
    let result = dispatch(
        Request {
            id: method.into(),
            method: method.into(),
            params,
        },
        adapter,
    )
    .await;
    let text = match &result {
        Ok(value) => value.to_string(),
        Err(error) => serde_json::to_string(error).unwrap(),
    };
    assert!(!text.contains(PLANTED_KEY), "secret in {text}");
    result
}

fn reason(error: &RpcError) -> Option<&'static str> {
    error.extra().and_then(|extra| extra.reason)
}

fn rows(list: &Value) -> Vec<(String, String)> {
    list["skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["tier"].as_str().unwrap().to_string(),
                row["name"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default();
        for (name, bytes) in files {
            writer.start_file(*name, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }
    cursor.into_inner()
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

const ENTRY: &[u8] = b"---\ndescription: Packaged skill\n---\nDo the packaged thing.\n";

// ── names and params ───────────────────────────────────────────────────────

#[test]
fn the_name_rule_is_the_sdk_rule() {
    for good in ["demo", "a", "demo-2", "x1-y2-z3", &"a".repeat(64)] {
        assert_eq!(check_new_name(good).unwrap(), good);
    }
    assert_eq!(check_new_name("  demo  ").unwrap(), "demo");
    for bad in [
        "",
        "Demo",
        "-demo",
        "demo-",
        "de--mo",
        "de_mo",
        "de.mo",
        "../demo",
        "de/mo",
        &"a".repeat(65),
        "skill",
        "readme",
    ] {
        let error = check_new_name(bad).unwrap_err();
        assert_eq!(error.code, "invalid_request", "{bad}");
        assert_eq!(reason(&error), Some("invalid_name"), "{bad}");
    }
    let error = check_new_name("de\u{7}mo").unwrap_err();
    assert_eq!(reason(&error), Some("invalid_name"));
}

#[test]
fn a_room_binding_reports_room_restart() {
    assert_eq!(propagation(BindingState::Unbound).skills, "next_rebuild");
    assert_eq!(propagation(BindingState::Coding).skills, "next_rebuild");
    assert_eq!(propagation(BindingState::RoomLive).skills, "room_restart");
    assert_eq!(propagation(BindingState::RoomClosed).skills, "room_restart");
}

#[tokio::test]
async fn project_tiers_need_a_workdir_while_unbound() {
    let fx = Fixture::new();
    let (mut adapter, _rx) = fx.adapter();
    for (method, params) in [
        ("skills.list", json!({"scope":"project"})),
        ("skills.list", json!({"scope":"external"})),
        ("skills.list", json!({"tier":"claude"})),
        (
            "skills.create",
            json!({"scope":"project","name":"demo","description":"d","instruction":"i"}),
        ),
        ("skills.get", json!({"tier":"project-agents","name":"demo"})),
    ] {
        let error = call(&mut adapter, method, params).await.unwrap_err();
        assert_eq!(reason(&error), Some("workdir_required"), "{method}");
    }
    let list = call(&mut adapter, "skills.list", json!({})).await.unwrap();
    assert_eq!(list["project_workdir"], Value::Null);
    let error = call(
        &mut adapter,
        "skills.list",
        json!({"workdir":"relative/dir"}),
    )
    .await
    .unwrap_err();
    assert_eq!(reason(&error), Some("out_of_range"));
}

// ── create, update, list ───────────────────────────────────────────────────

#[tokio::test]
async fn create_then_rename_then_list() {
    let fx = Fixture::new();
    let (mut adapter, rx) = fx.adapter();
    let created = call(
        &mut adapter,
        "skills.create",
        json!({"scope":"computer","name":"demo","description":"Demo skill",
               "instruction":"Do the demo.","triggers":["demo","show"],"allowed_tools":["bash"]}),
    )
    .await
    .unwrap();
    assert_eq!(created["propagation"], json!({"skills":"next_rebuild"}));
    let skill = &created["skill"];
    assert_eq!(skill["tier"], "global");
    assert_eq!(skill["scope"], "computer");
    assert_eq!(skill["triggers"], json!(["demo", "show"]));
    assert_eq!(skill["declared_tools"], json!(["bash"]));
    assert_eq!(skill["instruction"], "Do the demo.");
    assert_eq!(skill["raw"], Value::Null);
    assert_eq!(skill["effective"], true);
    assert!(fx.global_root().join("demo").join("SKILL.md").is_file());

    let again = call(
        &mut adapter,
        "skills.create",
        json!({"scope":"computer","name":"demo","description":"x","instruction":"y"}),
    )
    .await
    .unwrap_err();
    assert_eq!(reason(&again), Some("exists"));

    let generation = skill["generation"].as_str().unwrap().to_string();
    let updated = call(
        &mut adapter,
        "skills.update",
        json!({"tier":"global","name":"demo","generation":generation,"patch":{"name":"demo2"}}),
    )
    .await
    .unwrap();
    assert_eq!(updated["renamed"], true);
    assert_eq!(updated["skill"]["name"], "demo2");
    assert_eq!(updated["skill"]["description"], "Demo skill");
    assert_eq!(updated["skill"]["instruction"], "Do the demo.");
    assert_eq!(updated["skill"]["triggers"], json!(["demo", "show"]));

    let list = call(&mut adapter, "skills.list", json!({})).await.unwrap();
    assert_eq!(rows(&list), vec![("global".into(), "demo2".into())]);

    // A rename keeps the file's bytes, so its generation still holds.
    let edited = call(
        &mut adapter,
        "skills.update",
        json!({"tier":"global","name":"demo2","generation":generation,"patch":{"description":"New"}}),
    )
    .await
    .unwrap();
    assert_eq!(edited["renamed"], false);
    assert_ne!(edited["skill"]["generation"], json!(generation));
    let stale = call(
        &mut adapter,
        "skills.update",
        json!({"tier":"global","name":"demo2","generation":generation,"patch":{"description":"Newer"}}),
    )
    .await
    .unwrap_err();
    assert_eq!(stale.code, "invalid_state");
    assert_eq!(reason(&stale), Some("stale"));

    let empty = call(
        &mut adapter,
        "skills.update",
        json!({"tier":"global","name":"demo2","patch":{}}),
    )
    .await
    .unwrap_err();
    assert_eq!(reason(&empty), Some("missing_params"));

    let bad_name = call(
        &mut adapter,
        "skills.update",
        json!({"tier":"global","name":"demo2","patch":{"name":"Bad Name"}}),
    )
    .await
    .unwrap_err();
    assert_eq!(reason(&bad_name), Some("invalid_name"));
    // No mutation emits an event.
    assert!(!rx.has_changed().unwrap());
}

#[tokio::test]
async fn bounds_are_refused_not_clamped() {
    let fx = Fixture::new();
    let (mut adapter, _rx) = fx.adapter();
    let base = json!({"scope":"computer","name":"demo","description":"d","instruction":"i"});
    for (key, value) in [
        ("description", json!("")),
        ("description", json!("x".repeat(2001))),
        ("instruction", json!("   ")),
        ("triggers", json!(["a,b"])),
        ("triggers", json!(vec!["t"; 33])),
        ("allowed_tools", json!([""])),
    ] {
        let mut params = base.clone();
        params[key] = value;
        let error = call(&mut adapter, "skills.create", params).await.unwrap_err();
        assert_eq!(reason(&error), Some("out_of_range"), "{key}");
    }
    for limit in [json!(0), json!(1001), json!("5")] {
        let error = call(&mut adapter, "skills.list", json!({"limit":limit}))
            .await
            .unwrap_err();
        assert_eq!(reason(&error), Some("out_of_range"));
    }
    let error = call(&mut adapter, "skills.list", json!({"cursor":"%%%"}))
        .await
        .unwrap_err();
    assert_eq!(reason(&error), Some("bad_cursor"));
}

#[tokio::test]
async fn list_pages_by_tier_then_name() {
    let fx = Fixture::new();
    for name in ["c-one", "a-one", "b-one"] {
        write_skill(&fx.global_root(), name, "global", "Body.");
    }
    write_skill(&fx.tier_root(".agents"), "a-one", "project", "Body.");
    let (mut adapter, _rx) = fx.adapter();
    let mut seen = Vec::new();
    let mut cursor = Value::Null;
    loop {
        let mut params = json!({"workdir":fx.workdir(),"limit":2});
        if !cursor.is_null() {
            params["cursor"] = cursor.clone();
        }
        let page = call(&mut adapter, "skills.list", params).await.unwrap();
        assert_eq!(page["project_workdir"], fx.workdir());
        seen.extend(rows(&page));
        cursor = page["next_cursor"].clone();
        if cursor.is_null() {
            break;
        }
    }
    assert_eq!(
        seen,
        vec![
            ("global".into(), "a-one".into()),
            ("global".into(), "b-one".into()),
            ("global".into(), "c-one".into()),
            ("project-agents".into(), "a-one".into()),
        ]
    );
    let all = call(&mut adapter, "skills.list", json!({"workdir":fx.workdir()}))
        .await
        .unwrap();
    let shadowed = &all["skills"][0];
    assert_eq!(shadowed["effective"], false);
    assert_eq!(shadowed["shadowed_by"], "project-agents");
    let project = call(
        &mut adapter,
        "skills.list",
        json!({"workdir":fx.workdir(),"scope":"project"}),
    )
    .await
    .unwrap();
    assert_eq!(rows(&project), vec![("project-agents".into(), "a-one".into())]);
}

// ── addressing ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_same_name_in_both_project_tiers_is_addressed_by_tier() {
    let fx = Fixture::new();
    write_skill(&fx.tier_root(".agent"), "demo", "from agent", "Agent body.");
    write_skill(&fx.tier_root(".agents"), "demo", "from agents", "Agents body.");
    let (mut adapter, _rx) = fx.adapter();
    let workdir = fx.workdir();

    let winner = call(
        &mut adapter,
        "skills.get",
        json!({"workdir":workdir,"name":"demo"}),
    )
    .await
    .unwrap();
    assert_eq!(winner["tier"], "project-agents");
    let agents = call(
        &mut adapter,
        "skills.get",
        json!({"workdir":workdir,"tier":"project-agents","name":"demo"}),
    )
    .await
    .unwrap();
    assert_eq!(agents["description"], "from agents");

    // Discovery keeps only the winner of the project tiers, so the shadowed
    // copy is not addressable until the winner goes.
    let hidden = call(
        &mut adapter,
        "skills.update",
        json!({"workdir":workdir,"tier":"project-agent","name":"demo","patch":{"description":"x"}}),
    )
    .await
    .unwrap_err();
    assert_eq!(reason(&hidden), Some("not_found"));
    assert_eq!(
        std::fs::read_to_string(fx.tier_root(".agent").join("demo").join("SKILL.md")).unwrap(),
        "---\ndescription: from agent\n---\nAgent body.\n"
    );

    let removed = call(
        &mut adapter,
        "skills.remove",
        json!({"workdir":workdir,"tier":"project-agents","name":"demo",
               "generation":agents["generation"]}),
    )
    .await
    .unwrap();
    assert_eq!(removed["removed"], true);
    assert_eq!(removed["now_effective"]["tier"], "project-agent");
    assert_eq!(removed["now_effective"]["description"], "from agent");
    assert_eq!(removed["propagation"], json!({"skills":"next_rebuild"}));
    assert!(!fx.tier_root(".agents").join("demo").exists());

    let updated = call(
        &mut adapter,
        "skills.update",
        json!({"workdir":workdir,"tier":"project-agent","name":"demo","patch":{"description":"edited"}}),
    )
    .await
    .unwrap();
    assert_eq!(updated["skill"]["tier"], "project-agent");
    assert_eq!(updated["skill"]["description"], "edited");
    assert_eq!(updated["skill"]["instruction"], "Agent body.");

    let gone = call(
        &mut adapter,
        "skills.remove",
        json!({"workdir":workdir,"tier":"project-agents","name":"demo"}),
    )
    .await
    .unwrap_err();
    assert_eq!(reason(&gone), Some("not_found"));
}

#[tokio::test]
async fn read_only_tiers_refuse_writes() {
    let fx = Fixture::new();
    write_skill(&fx.tier_root(".claude"), "imported", "external", "Body.");
    let (mut adapter, _rx) = fx.adapter();
    let workdir = fx.workdir();
    for tier in ["claude", "extra-root:0"] {
        let update = call(
            &mut adapter,
            "skills.update",
            json!({"workdir":workdir,"tier":tier,"name":"imported","patch":{"description":"x"}}),
        )
        .await
        .unwrap_err();
        assert_eq!(update.code, "invalid_state");
        assert_eq!(reason(&update), Some("read_only"), "{tier}");
        let remove = call(
            &mut adapter,
            "skills.remove",
            json!({"workdir":workdir,"tier":tier,"name":"imported"}),
        )
        .await
        .unwrap_err();
        assert_eq!(reason(&remove), Some("read_only"), "{tier}");
    }
    let row = call(
        &mut adapter,
        "skills.get",
        json!({"workdir":workdir,"tier":"claude","name":"imported"}),
    )
    .await
    .unwrap();
    assert_eq!(row["scope"], "external");
    assert_eq!(row["editable"], false);
    assert_eq!(row["structured_save_supported"], false);
    assert!(fx.tier_root(".claude").join("imported").join("SKILL.md").is_file());
}

// ── reads ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn reads_are_verbatim_and_stay_inside_the_skill() {
    let fx = Fixture::new();
    let root = fx.tier_root(".agents");
    write_skill(&root, "docs", "Docs", "Read the docs.");
    let dir = root.join("docs");
    std::fs::create_dir_all(dir.join("scripts")).unwrap();
    std::fs::write(dir.join("scripts").join("run.sh"), "#!/bin/sh\necho hi\n").unwrap();
    std::fs::write(dir.join("logo.bin"), [0xff, 0xfe, 0x00, 0x81]).unwrap();
    std::fs::write(fx.base.join("outside.txt"), "outside").unwrap();
    let (mut adapter, _rx) = fx.adapter();
    let workdir = fx.workdir();

    let detail = call(
        &mut adapter,
        "skills.get",
        json!({"workdir":workdir,"name":"docs","include_raw":true}),
    )
    .await
    .unwrap();
    assert_eq!(
        detail["raw"],
        "---\ndescription: Docs\n---\nRead the docs.\n"
    );
    assert_eq!(detail["companion_files"], json!(["logo.bin", "scripts/run.sh"]));

    let file = call(
        &mut adapter,
        "skills.read_file",
        json!({"workdir":workdir,"name":"docs","path":"scripts/run.sh"}),
    )
    .await
    .unwrap();
    assert_eq!(file["text"], "#!/bin/sh\necho hi\n");
    assert_eq!(file["bytes"], 18);

    let binary = call(
        &mut adapter,
        "skills.read_file",
        json!({"workdir":workdir,"name":"docs","path":"logo.bin"}),
    )
    .await
    .unwrap_err();
    assert_eq!(reason(&binary), Some("binary"));

    let outside = fx.base.join("outside.txt").to_string_lossy().into_owned();
    for path in ["../../../../outside.txt", "scripts/../../x", outside.as_str(), "./SKILL.md"] {
        let error = call(
            &mut adapter,
            "skills.read_file",
            json!({"workdir":workdir,"name":"docs","path":path}),
        )
        .await
        .unwrap_err();
        assert_eq!(reason(&error), Some("out_of_range"), "{path}");
    }
    let missing = call(
        &mut adapter,
        "skills.read_file",
        json!({"workdir":workdir,"name":"docs","path":"SKILL.md"}),
    )
    .await
    .unwrap_err();
    assert_eq!(reason(&missing), Some("not_found"));

    let stale = call(
        &mut adapter,
        "skills.read_file",
        json!({"workdir":workdir,"name":"docs","path":"scripts/run.sh","generation":"nope"}),
    )
    .await
    .unwrap_err();
    assert_eq!(reason(&stale), Some("stale"));
}

#[tokio::test]
async fn advanced_yaml_is_not_saved() {
    let fx = Fixture::new();
    let dir = fx.tier_root(".agents").join("anchored");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        "---\ndescription: &d Anchored\nalias: *d\n---\nBody.\n",
    )
    .unwrap();
    let (mut adapter, _rx) = fx.adapter();
    let error = call(
        &mut adapter,
        "skills.update",
        json!({"workdir":fx.workdir(),"tier":"project-agents","name":"anchored","patch":{"description":"x"}}),
    )
    .await
    .unwrap_err();
    assert_eq!(reason(&error), Some("unsafe_yaml"));
}

// ── install ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn inline_size_is_checked_before_decoding() {
    let fx = Fixture::new();
    let (mut adapter, _rx) = fx.adapter();
    // Not base64 at all: a decode would fail with invalid_package.
    let oversized = "!".repeat(INLINE_B64_CHARS + 1);
    let error = call(
        &mut adapter,
        "skills.install",
        json!({"scope":"computer","name":"big","data_b64":oversized}),
    )
    .await
    .unwrap_err();
    assert_eq!(reason(&error), Some("too_large"));
    let error = call(
        &mut adapter,
        "skills.install",
        json!({"scope":"computer","name":"big","data_b64":"!!!!"}),
    )
    .await
    .unwrap_err();
    assert_eq!(reason(&error), Some("invalid_package"));
}

#[tokio::test]
async fn install_takes_exactly_one_source() {
    let fx = Fixture::new();
    let (mut adapter, _rx) = fx.adapter();
    let both = call(
        &mut adapter,
        "skills.install",
        json!({"scope":"computer","data_b64":"UEs=","path":"/tmp/x.zip"}),
    )
    .await
    .unwrap_err();
    assert_eq!(reason(&both), Some("conflicting_params"));
    let neither = call(&mut adapter, "skills.install", json!({"scope":"computer"}))
        .await
        .unwrap_err();
    assert_eq!(reason(&neither), Some("missing_params"));
    let relative = call(
        &mut adapter,
        "skills.install",
        json!({"scope":"computer","path":"pack.zip"}),
    )
    .await
    .unwrap_err();
    assert_eq!(reason(&relative), Some("out_of_range"));
    let missing = fx.base.join("missing.zip").to_string_lossy().into_owned();
    let error = call(
        &mut adapter,
        "skills.install",
        json!({"scope":"computer","path":missing}),
    )
    .await
    .unwrap_err();
    assert_eq!(reason(&error), Some("not_found"));
}

#[tokio::test]
async fn install_preflight_names_the_common_mistakes() {
    let fx = Fixture::new();
    let (mut adapter, _rx) = fx.adapter();
    let cases: Vec<(Vec<u8>, &str, &str)> = vec![
        (
            zip_of(&[("pack/SKILL.md", ENTRY), ("__MACOSX/pack/._SKILL.md", b"x")]),
            "invalid_package",
            "__MACOSX",
        ),
        (zip_of(&[("pack/skill.md", ENTRY)]), "invalid_package", "SKILL.md"),
        (zip_of(&[("skill.md", ENTRY)]), "invalid_package", "SKILL.md"),
        (zip_of(&[("a/SKILL.md", ENTRY), ("b/x.txt", b"x")]), "invalid_package", "one top-level"),
        (b"not a zip".to_vec(), "invalid_package", "zip"),
        (zip_of(&[("SKILL.md", ENTRY)]), "invalid_name", "name"),
        (zip_of(&[("Bad_Name/SKILL.md", ENTRY)]), "invalid_name", "skill name"),
    ];
    for (bytes, want, text) in cases {
        let error = call(
            &mut adapter,
            "skills.install",
            json!({"scope":"computer","data_b64":b64(&bytes)}),
        )
        .await
        .unwrap_err();
        assert_eq!(reason(&error), Some(want), "{}", error.message);
        assert!(error.message.contains(text), "{}", error.message);
    }
    assert!(!fx.global_root().exists() || std::fs::read_dir(fx.global_root()).unwrap().next().is_none());
}

#[tokio::test]
async fn install_inline_then_read_then_remove() {
    let fx = Fixture::new();
    let (mut adapter, rx) = fx.adapter();
    let bytes = zip_of(&[
        ("packed/SKILL.md", ENTRY),
        ("packed/notes/readme.txt", b"companion text\n"),
    ]);
    let installed = call(
        &mut adapter,
        "skills.install",
        json!({"scope":"project","workdir":fx.workdir(),"data_b64":b64(&bytes)}),
    )
    .await
    .unwrap();
    let skill = &installed["skill"];
    assert_eq!(skill["name"], "packed");
    assert_eq!(skill["tier"], "project-agents");
    assert_eq!(skill["companion_files"], json!(["notes/readme.txt"]));
    assert_eq!(installed["propagation"], json!({"skills":"next_rebuild"}));

    let again = call(
        &mut adapter,
        "skills.install",
        json!({"scope":"project","workdir":fx.workdir(),"data_b64":b64(&bytes)}),
    )
    .await
    .unwrap_err();
    assert_eq!(reason(&again), Some("exists"));

    let file = call(
        &mut adapter,
        "skills.read_file",
        json!({"workdir":fx.workdir(),"tier":"project-agents","name":"packed","path":"notes/readme.txt"}),
    )
    .await
    .unwrap();
    assert_eq!(file["text"], "companion text\n");

    let renamed = call(
        &mut adapter,
        "skills.install",
        json!({"scope":"computer","name":"other-name","data_b64":b64(&bytes)}),
    )
    .await
    .unwrap();
    assert_eq!(renamed["skill"]["name"], "other-name");
    assert_eq!(renamed["skill"]["tier"], "global");

    let removed = call(
        &mut adapter,
        "skills.remove",
        json!({"workdir":fx.workdir(),"tier":"project-agents","name":"packed"}),
    )
    .await
    .unwrap();
    assert_eq!(removed["now_effective"], Value::Null);
    assert!(!fx.tier_root(".agents").join("packed").exists());
    assert!(!rx.has_changed().unwrap());
}

#[tokio::test]
async fn install_from_a_regular_file() {
    let fx = Fixture::new();
    let (mut adapter, _rx) = fx.adapter();
    let package = fx.base.join("pack.zip");
    std::fs::write(&package, zip_of(&[("from-path/SKILL.md", ENTRY)])).unwrap();
    let installed = call(
        &mut adapter,
        "skills.install",
        json!({"scope":"computer","path":package.to_string_lossy()}),
    )
    .await
    .unwrap();
    assert_eq!(installed["skill"]["name"], "from-path");
    let dir = fx.base.join("a-dir");
    std::fs::create_dir_all(&dir).unwrap();
    let error = call(
        &mut adapter,
        "skills.install",
        json!({"scope":"computer","name":"x","path":dir.to_string_lossy()}),
    )
    .await
    .unwrap_err();
    assert_eq!(reason(&error), Some("out_of_range"));
}

#[cfg(unix)]
#[tokio::test]
async fn install_never_follows_a_symlink() {
    let fx = Fixture::new();
    let (mut adapter, _rx) = fx.adapter();
    let package = fx.base.join("real.zip");
    std::fs::write(&package, zip_of(&[("linked/SKILL.md", ENTRY)])).unwrap();
    let link = fx.base.join("link.zip");
    std::os::unix::fs::symlink(&package, &link).unwrap();
    let error = call(
        &mut adapter,
        "skills.install",
        json!({"scope":"computer","path":link.to_string_lossy()}),
    )
    .await
    .unwrap_err();
    assert_eq!(reason(&error), Some("out_of_range"));
    assert!(!fx.global_root().join("linked").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn remove_refuses_a_tree_with_a_symlink() {
    let fx = Fixture::new();
    let root = fx.tier_root(".agents");
    write_skill(&root, "linked", "Linked", "Body.");
    std::fs::write(fx.base.join("target.txt"), "keep").unwrap();
    std::os::unix::fs::symlink(fx.base.join("target.txt"), root.join("linked").join("l.txt"))
        .unwrap();
    let (mut adapter, _rx) = fx.adapter();
    let error = call(
        &mut adapter,
        "skills.remove",
        json!({"workdir":fx.workdir(),"tier":"project-agents","name":"linked"}),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "invalid_state");
    assert!(root.join("linked").join("SKILL.md").is_file());
    assert_eq!(std::fs::read_to_string(fx.base.join("target.txt")).unwrap(), "keep");
}

// ── bound ──────────────────────────────────────────────────────────────────

/// A coding binding whose session cwd is `cwd` and whose peer never answers.
/// The peer end is returned so a test can check that nothing was sent.
#[cfg(unix)]
fn coding(fx: &Fixture, cwd: &Path) -> (Adapter, tokio::net::UnixStream) {
    let (a, b) = tokio::net::UnixStream::pair().unwrap();
    let mut connection = connection::Connection::for_test(a, "session");
    connection.projection.policy = Some(protocol::Policy::default());
    let snapshot = json!({"foreground_id":"session","sessions":[{
     "id":"session","name":"s","cwd":cwd.to_string_lossy(),"messages":[],"streaming":null,"stream_reasoning":"",
     "tokens_in":0,"tokens_out":0,"tokens_cached":0,"cost":0.0,"waiting":false,"awaiting_approval":false,
     "approval_reason":null,"pending_tool_calls":[],"tool_idx":0,"working":false,"finished_unseen":false,
     "subagents":[],"pending_subagents":[],"resolved_model_id":"m","effort":"",
     "model_routes":[],
     "context_usage":{"prompt_tokens":10,"effective_window":131072,"estimated":true}
    }],"global":{
     "input":"","cursor":0,"scroll":0,"follow":true,"status":"","work_elapsed_ms":null,"theme":"dark","accent":"blue","palette":"default","mode":"Chat","toast":null,"models_cache":null,"models_cache_endpoint":null,"models_cache_failed":null,"agent_viewer":null,"agent_viewer_scroll":0,"agent_viewer_follow":true,"subagents_open":false,"subagent_sel":0,"palette_sel":0,"pending_attachments":[],"file_palette":null,"agent_mode":"auto","latest_version":null,
     "providers":[],"config_models":[]
    }});
    connection.projection.apply(&crate::ipc::proto::DaemonFrame {
        seq: 1,
        event: crate::ipc::proto::DaemonEvent::Snapshot(Box::new(
            serde_json::from_value(snapshot).unwrap(),
        )),
    });
    assert!(connection.projection.session.is_some());
    let (mut adapter, _rx) = fx.adapter();
    adapter.binding = Binding::Coding(Box::new(connection));
    (adapter, b)
}

#[cfg(unix)]
#[tokio::test]
async fn a_coding_binding_uses_its_cwd_and_sends_no_skill_ipc() {
    use tokio::io::AsyncReadExt;
    let fx = Fixture::new();
    let (mut adapter, mut peer) = coding(&fx, &fx.work());
    let created = call(
        &mut adapter,
        "skills.create",
        json!({"scope":"project","name":"bound","description":"Bound","instruction":"Body."}),
    )
    .await
    .unwrap();
    assert_eq!(created["propagation"], json!({"skills":"next_rebuild"}));
    assert_eq!(created["skill"]["tier"], "project-agents");
    assert!(fx.tier_root(".agents").join("bound").join("SKILL.md").is_file());
    let list = call(&mut adapter, "skills.list", json!({"scope":"project"}))
        .await
        .unwrap();
    assert_eq!(list["project_workdir"], fx.workdir());
    assert_eq!(rows(&list), vec![("project-agents".into(), "bound".into())]);
    let removed = call(
        &mut adapter,
        "skills.remove",
        json!({"tier":"project-agents","name":"bound"}),
    )
    .await
    .unwrap();
    assert_eq!(removed["propagation"], json!({"skills":"next_rebuild"}));
    drop(adapter);
    let mut sent = Vec::new();
    peer.read_to_end(&mut sent).await.unwrap();
    assert!(sent.is_empty(), "the adapter sent {} bytes", sent.len());
}
