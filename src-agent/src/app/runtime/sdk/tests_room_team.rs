//! Tests for the room reads and the team methods: a fixture `room.sqlite`
//! built with `Shelf::open`, scratch template and session layers, and a fake
//! room whose `models` and `sdk_reload` answers come from the daemon's own
//! `ModelBook` over the same layers. Nothing here starts a real daemon or
//! touches koma's data dir.
use super::binding::BindingState;
use super::host::config::ConfigStore;
use super::protocol::{Request, RpcError};
use super::room::RoomEnv;
use super::*;
use crate::ipc::frame::{read_frame_from, write_frame_to, FrameReader};
use crate::ipc::proto::{DaemonEvent, DaemonFrame, SessionStatus};
use crate::ipc::split_stream;
use crate::model::app_config::{ApiType, AppConfig, ModelEntry, ModelRole, ProviderConn};
use crate::model::session_registry::RegRowFullCreated;
use crate::model::settings::Settings;
use crate::model::usage::query::{
    GroupBy, LedgerState, RoleKind, UsageGroup, UsageQuery, UsageReport, UsageTotals,
};
use crate::room::daemon::models::ModelBook;
use crate::room::registry::RegistryHandle;
use crate::room::shelf::Shelf;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

const PLANTED_KEY: &str = "sk-team-planted-0123456789";
const PARENT_ID: &str = "parent-0001";
const BASE_RED: &str = include_str!("../../../../../src-misc/room/roles/red/role.toml");

fn room_id() -> String {
    crate::room::room_session_id(PARENT_ID)
}

/// A fresh short directory under the system temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let dir = std::env::temp_dir().join(format!("kt-{}", &id[..12]));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn sock_path(dir: &Path) -> PathBuf {
    #[cfg(unix)]
    {
        dir.join("room.sock")
    }
    #[cfg(windows)]
    {
        let _ = dir;
        PathBuf::from(format!(
            r"\\.\pipe\koma-team-test-{}",
            uuid::Uuid::new_v4().simple()
        ))
    }
}

fn config() -> AppConfig {
    AppConfig {
        providers: vec![ProviderConn {
            uuid: "p-1".into(),
            name: "local".into(),
            api_type: ApiType::OpenAiCompatible,
            endpoint: "http://127.0.0.1:9/v1".into(),
            api_key: PLANTED_KEY.into(),
            ext_id: None,
        }],
        models: vec![
            ModelEntry {
                uuid: "m-1".into(),
                name: "main".into(),
                model_id: "main-id".into(),
                provider_uuid: "p-1".into(),
                roles: vec![ModelRole::Main],
                ..ModelEntry::default()
            },
            ModelEntry {
                uuid: "m-2".into(),
                name: "fast".into(),
                model_id: "fast-id".into(),
                provider_uuid: "p-1".into(),
                ..ModelEntry::default()
            },
        ],
        ..AppConfig::default()
    }
}

fn write_config(dir: &Path) {
    let secret = crate::model::secret_box::key_for_write().unwrap();
    let bytes = config().to_disk_bytes(&secret).unwrap();
    crate::model::secret_box::write_private(&dir.join("config.json"), &bytes).unwrap();
}

// ── the fake room ───────────────────────────────────────────────────────

#[derive(Clone)]
struct Script {
    features: Vec<String>,
    /// The next this many `sdk_reload`s report a warning about this avatar.
    reject: Option<(String, usize)>,
}

impl Default for Script {
    fn default() -> Self {
        Self {
            features: vec!["sdk_user".into(), "sdk_reload".into()],
            reject: None,
        }
    }
}

enum Cmd {
    Frame(Value),
    Close,
}

struct FakeRoom {
    received: Arc<Mutex<Vec<Value>>>,
    current: Arc<Mutex<Option<UnboundedSender<Cmd>>>>,
    accept: Option<tokio::task::JoinHandle<()>>,
}

fn tag(value: &Value) -> String {
    match value {
        Value::String(tag) => tag.clone(),
        Value::Object(map) => map.keys().next().cloned().unwrap_or_default(),
        _ => String::new(),
    }
}

async fn write_value(write: &mut crate::ipc::IpcWriteHalf, value: &Value) -> std::io::Result<()> {
    write_frame_to(write, &serde_json::to_vec(value).unwrap()).await
}

impl FakeRoom {
    /// A room over the layers at `template` and `overlay`, as a daemon would
    /// build them at start.
    fn start(path: PathBuf, script: Script, template: PathBuf, overlay: PathBuf) -> Self {
        let listener = crate::ipc::server::bind(&path).unwrap();
        let received = Arc::new(Mutex::new(Vec::new()));
        let current: Arc<Mutex<Option<UnboundedSender<Cmd>>>> = Arc::new(Mutex::new(None));
        let registry =
            Arc::new(RegistryHandle::with_layers(Some(template), overlay.clone()).unwrap());
        let book = Arc::new(ModelBook::new(
            &config(),
            &Settings::default(),
            Arc::clone(&registry),
            overlay,
        ));
        let script = Arc::new(Mutex::new(script));
        let accept = {
            let (received, current) = (received.clone(), current.clone());
            tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    let (received, current, script, book, registry) = (
                        received.clone(),
                        current.clone(),
                        script.clone(),
                        book.clone(),
                        registry.clone(),
                    );
                    tokio::spawn(async move {
                        let _ = serve(stream, received, current, script, book, registry).await;
                    });
                }
            })
        };
        Self {
            received,
            current,
            accept: Some(accept),
        }
    }

    fn received(&self, wanted: &str) -> usize {
        self.received
            .lock()
            .unwrap()
            .iter()
            .filter(|v| tag(v) == wanted)
            .count()
    }

    fn push(&self, frame: Value) {
        if let Some(tx) = self.current.lock().unwrap().as_ref() {
            let _ = tx.send(Cmd::Frame(frame));
        }
    }

    /// Another client takes the stream; the daemon keeps running.
    fn take_over(&self) {
        if let Some(tx) = self.current.lock().unwrap().take() {
            let _ = tx.send(Cmd::Close);
        }
    }

    /// The daemon goes away.
    async fn exit(&mut self) {
        if let Some(accept) = self.accept.take() {
            accept.abort();
            let _ = accept.await;
        }
        self.take_over();
    }
}

impl Drop for FakeRoom {
    fn drop(&mut self) {
        if let Some(accept) = self.accept.take() {
            accept.abort();
        }
        self.take_over();
    }
}

async fn serve(
    stream: crate::ipc::IpcStream,
    received: Arc<Mutex<Vec<Value>>>,
    current: Arc<Mutex<Option<UnboundedSender<Cmd>>>>,
    script: Arc<Mutex<Script>>,
    book: Arc<ModelBook>,
    registry: Arc<RegistryHandle>,
) -> std::io::Result<()> {
    let (mut read, mut write) = split_stream(stream);
    let mut frames = FrameReader::new();
    let first: Value = serde_json::from_slice(&read_frame_from(&mut read, &mut frames).await?)?;
    received.lock().unwrap().push(first.clone());
    if first == json!("Status") {
        let frame = DaemonFrame {
            seq: 1,
            event: DaemonEvent::Status(SessionStatus {
                session_id: room_id(),
                name: "secure".into(),
                pwd: String::new(),
                working: false,
            }),
        };
        return write_value(&mut write, &serde_json::to_value(frame)?).await;
    }
    if first != json!("hello") {
        return Ok(());
    }
    let features = script.lock().unwrap().features.clone();
    let (tx, mut rx) = unbounded_channel();
    *current.lock().unwrap() = Some(tx);
    let hello = json!({"hello": {"kind": "secure", "replayed": 0, "features": features,
        "version": "test-build", "template_revision": registry.template_revision()}});
    write_value(&mut write, &hello).await?;
    loop {
        tokio::select! {
            incoming = read_frame_from(&mut read, &mut frames) => {
                let Ok(bytes) = incoming else { break };
                let value: Value = serde_json::from_slice(&bytes)?;
                received.lock().unwrap().push(value.clone());
                match tag(&value).as_str() {
                    "models" => {
                        let answer = json!({"models": {"choices": book.choices(), "lanes": book.lanes()}});
                        write_value(&mut write, &answer).await?;
                    }
                    "quit" => {
                        write_value(&mut write, &json!("closed")).await?;
                        break;
                    }
                    "sdk_reload" => {
                        let id = value["sdk_reload"]["id"].clone();
                        let (version, mut warnings) = book.reload_registry();
                        {
                            let mut script = script.lock().unwrap();
                            if let Some((avatar, left)) = script.reject.as_mut() {
                                if *left > 0 {
                                    *left -= 1;
                                    warnings.push(format!("avatars.toml: avatar '{avatar}' uses unknown role 'ghost'; ignored"));
                                }
                            }
                        }
                        let ack = json!({"sdk_ack": {"id": id, "ok": true, "version": version, "warnings": warnings}});
                        write_value(&mut write, &ack).await?;
                    }
                    _ => {}
                }
            }
            cmd = rx.recv() => match cmd {
                Some(Cmd::Frame(frame)) => write_value(&mut write, &frame).await?,
                Some(Cmd::Close) | None => break,
            }
        }
    }
    Ok(())
}

// ── the adapter under test ──────────────────────────────────────────────

struct Rig {
    dir: TempDir,
    adapter: Adapter,
    sock: PathBuf,
}

fn row(uuid: &str) -> RegRowFullCreated {
    RegRowFullCreated {
        uuid: uuid.into(),
        pwd_hash: "bucket".into(),
        name: String::new(),
        workdir: "/work".into(),
        created_at: 10,
        updated_at: 20,
    }
}

fn usage_report(query: &UsageQuery) -> UsageReport {
    let totals = |calls: i64, cost: f64| UsageTotals {
        calls,
        tokens_in: calls * 100,
        tokens_cached: 0,
        tokens_out: calls * 10,
        cost,
    };
    assert_eq!(query.group_by, GroupBy::Role);
    assert_eq!(query.session_id.as_deref(), Some(room_id().as_str()));
    UsageReport {
        from_at: 0,
        to_at: 1,
        ledger: LedgerState::Ok,
        totals: totals(5, 0.5),
        groups: Some(vec![
            UsageGroup {
                key: "main".into(),
                totals: totals(2, 0.2),
                role_kind: Some(RoleKind::Main),
                agent: None,
            },
            UsageGroup {
                key: "sub:red-web".into(),
                totals: totals(3, 0.3),
                role_kind: Some(RoleKind::Sub),
                agent: Some("red-web".into()),
            },
        ]),
        groups_truncated: false,
        buckets: None,
    }
}

fn rig() -> Rig {
    let dir = TempDir::new();
    let config_dir = dir.0.join("c");
    std::fs::create_dir_all(&config_dir).unwrap();
    write_config(&config_dir);
    let root = dir.0.join("s");
    let room = root.join("bucket").join(room_id());
    std::fs::create_dir_all(&room).unwrap();
    std::fs::create_dir_all(root.join("bucket").join(PARENT_ID)).unwrap();
    fixture(&Shelf::open(&crate::room::room_db_path(&room)).unwrap());
    let sock = sock_path(&dir.0);
    let (events, _) = watch::channel(None);
    let mut adapter = Adapter::new(events);
    adapter.config = ConfigStore::at(&config_dir);
    let socks = sock.clone();
    let template = dir.0.join("t");
    adapter.room_env = RoomEnv {
        sock_path: Arc::new(move |_| Ok(socks.clone())),
        sessions: Arc::new(|| Ok(vec![row(&room_id()), row(PARENT_ID)])),
        session_dir: Arc::new(move |hash, id| Ok(root.join(hash).join(id))),
        ensure_running: Arc::new(|_, _, _, _| Ok(false)),
        start_log: Arc::new(|_| anyhow::bail!("no start log")),
        stop: Arc::new(|_| true),
        template_dir: Arc::new(move || Some(template.clone())),
        usage: Arc::new(|query| Ok(usage_report(query))),
    };
    Rig { dir, adapter, sock }
}

impl Rig {
    fn template(&self) -> PathBuf {
        self.dir.0.join("t")
    }

    fn session(&self) -> PathBuf {
        self.dir.0.join("s").join("bucket").join(room_id()).join("room")
    }

    fn write(&self, layer: &Path, name: &str, text: &str) {
        let path = layer.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn start(&self, script: Script) -> FakeRoom {
        FakeRoom::start(self.sock.clone(), script, self.template(), self.session())
    }

    async fn call(&mut self, method: &str, params: Value) -> Result<Value, RpcError> {
        let result = dispatch(
            Request {
                id: method.into(),
                method: method.into(),
                params,
            },
            &mut self.adapter,
        )
        .await;
        self.adapter.queue.response_written();
        let text = match &result {
            Ok(value) => value.to_string(),
            Err(e) => serde_json::to_string(e).unwrap(),
        };
        assert!(!text.contains(PLANTED_KEY), "{method} leaked the key: {text}");
        result
    }

    async fn ok(&mut self, method: &str, params: Value) -> Value {
        match self.call(method, params).await {
            Ok(value) => value,
            Err(e) => panic!("{method}: {} {:?}", e.message, e.extra()),
        }
    }

    async fn refused(&mut self, method: &str, params: Value) -> RpcError {
        match self.call(method, params).await {
            Ok(value) => panic!("{method} should fail: {value}"),
            Err(e) => e,
        }
    }

    async fn attach(&mut self) -> Value {
        self.ok("room.attach", json!({"session_id": room_id(), "wait_ms": 0}))
            .await
    }

    fn state(&self) -> BindingState {
        self.adapter.binding.state()
    }
}

fn reason(error: &RpcError) -> Option<&'static str> {
    error.extra().and_then(|extra| extra.reason)
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..500 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}");
}

fn fixture(shelf: &Shelf) {
    let allow: Vec<String> = ["put_claim", "put_trajectory", "put_report", "put", "put_asset"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let call = |tool: &str, args: Value| {
        shelf
            .call_as(&allow, tool, &args)
            .unwrap_or_else(|e| panic!("{tool}: {e}"))
    };
    call("put_claim", json!({"key": "claims/web/ssrf", "vulnclass": "ssrf",
        "name": "SSRF in fetch", "severity": "high", "summary": format!("found {PLANTED_KEY}"),
        "host": "app", "poc": "curl", "expected_revision": 0}));
    call("put_asset", json!({"key": "assets/app", "host": "app", "asset_type": "host",
        "title": "App server"}));
    call("put_trajectory", json!({"key": "trajectory/g1", "goal_key": "g1",
        "nodes": [{"id": "a", "label": "Map the app"},
                  {"id": "b", "label": "Try SSRF", "resource_key": "claims/web/ssrf"}],
        "edges": [{"from": "a", "to": "b", "rel": "leads_to"}]}));
    call("put_report", json!({"key": "reports/final",
        "markdown": format!("# Final\n\nThe key {PLANTED_KEY} was exposed."),
        "finding_key": "findings/x", "title": "Final report", "audience": "manager",
        "expected_revision": 0}));
    call("put", json!({"key": "notes/plan", "content": "first line\nsecond",
        "expected_revision": 0}));
    let spend = json!({"kind": "spend", "goal": "g1", "calls": 3, "prompt_tokens": 30,
        "completion_tokens": 3, "cost": 0.3, "by_avatar": {
            "chief": {"calls": 1, "prompt_tokens": 10, "completion_tokens": 1, "cost": 0.1},
            "red-web": {"calls": 2, "prompt_tokens": 20, "completion_tokens": 2, "cost": 0.2}}});
    shelf
        .call("put", &json!({"key": "notes/spend/g1", "content": spend.to_string(),
            "expected_revision": 0}))
        .unwrap();
}

fn status_frame(avatar: &str, phase: &str, error: &str) -> Value {
    json!({"status": {"digest": "", "timeline": "", "open": 0, "verified": 0,
        "returned": 0, "reports": 0, "paused": [], "shelf": "", "trajectory": "",
        "lanes": [{"avatar": avatar, "display": avatar, "alias": "XX", "role": "red",
            "lane": "work", "phase": phase, "task": "scan", "started_ms": 1,
            "last_event_ms": 2, "steps": 3, "last_tool": "", "error": error}],
        "events": [], "now_ms": 5}})
}

fn avatar<'a>(team: &'a Value, id: &str) -> &'a Value {
    team["avatars"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == id)
        .unwrap_or_else(|| panic!("no avatar {id}"))
}

fn lane<'a>(team: &'a Value, id: &str, name: &str) -> &'a Value {
    avatar(team, id)["lanes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["lane"] == name)
        .unwrap_or_else(|| panic!("no {name} lane on {id}"))
}

// ── reads ───────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shelf_item_trajectory_and_reports_are_read_from_disk_live_and_closed() {
    let mut rig = rig();
    let fake = rig.start(Script::default());
    rig.attach().await;

    let all = rig.ok("room.shelf.list", json!({})).await;
    assert_eq!(all["total"], 6);
    assert!(all["next_cursor"].is_null());
    let claim = all["rows"].as_array().unwrap().iter().find(|r| r["key"] == "claims/web/ssrf").unwrap();
    assert_eq!((claim["kind"].as_str(), claim["title"].as_str()), (Some("claim"), Some("SSRF in fetch")));
    let page = rig.ok("room.shelf.list", json!({"limit": 2})).await;
    assert_eq!(page["rows"].as_array().unwrap().len(), 2);
    let next = rig.ok("room.shelf.list", json!({"limit": 5, "cursor": page["next_cursor"]})).await;
    assert_eq!(next["rows"].as_array().unwrap().len(), 4);
    assert_eq!(next["rows"][0]["key"], all["rows"][2]["key"]);
    assert_eq!(next["fingerprint"], all["fingerprint"]);
    let notes = rig.ok("room.shelf.list", json!({"prefix": "notes/"})).await;
    assert_eq!(notes["total"], 2);
    let e = rig.refused("room.shelf.list", json!({"cursor": "%%%"})).await;
    assert_eq!(reason(&e), Some("bad_cursor"));
    let e = rig.refused("room.shelf.list", json!({"limit": 501})).await;
    assert_eq!(reason(&e), Some("out_of_range"));

    let item = rig.ok("room.item", json!({"key": "claims/web/ssrf"})).await;
    assert_eq!(item["json"]["vulnclass"], "ssrf");
    assert_eq!(item["truncated"], false);
    assert!(item["content"].as_str().unwrap().contains("[redacted]"));
    assert!(item["json"]["summary"].as_str().unwrap().contains("[redacted]"));
    let plain = rig.ok("room.item", json!({"key": "notes/plan"})).await;
    assert!(plain["json"].is_null());
    assert_eq!(plain["title"], "first line");
    let e = rig.refused("room.item", json!({"key": "notes/none"})).await;
    assert_eq!((e.code, reason(&e)), ("not_found", Some("not_found")));
    let e = rig.refused("room.item", json!({"key": "/bad/"})).await;
    assert_eq!(reason(&e), Some("invalid_name"));

    let trajectory = rig.ok("room.trajectory", json!({})).await;
    let goal = &trajectory["goals"][0];
    assert_eq!(goal["goal_key"], "g1");
    assert_eq!(goal["nodes_truncated"], false);
    assert!(goal["edges"].as_array().unwrap().iter().any(|e| e["from"] == "a" && e["to"] == "b"));
    let same = rig.ok("room.trajectory", json!({"goal": "trajectory/g1"})).await;
    assert_eq!(same["goals"][0]["goal_key"], "g1");
    let none = rig.ok("room.trajectory", json!({"goal": "other"})).await;
    assert_eq!(none["goals"], json!([]));

    let reports = rig.ok("room.reports", json!({})).await;
    assert_eq!(reports["reports"][0]["key"], "reports/final");
    assert!(reports["reports"][0]["markdown"].is_null());
    let full = rig.ok("room.reports", json!({"include_markdown": true})).await;
    let markdown = full["reports"][0]["markdown"].as_str().unwrap();
    assert!(markdown.starts_with("# Final") && markdown.contains("[redacted]"));

    // A lane error that quotes the key is scrubbed in the status too.
    fake.push(status_frame("red-web", "running", &format!("401 for {PLANTED_KEY}")));
    until("the cached status", || {
        rig.adapter.binding.room().is_some_and(|r| r.cached_status().is_some())
    })
    .await;
    let status = rig.ok("room.status", json!({})).await;
    assert!(status["lanes"][0]["error"].as_str().unwrap().contains("[redacted]"));

    fake.take_over();
    until("ROOM(closed)", || rig.state() == BindingState::RoomClosed).await;
    let closed = rig.ok("room.shelf.list", json!({})).await;
    assert_eq!(closed["rows"], all["rows"]);
    rig.ok("room.item", json!({"key": "reports/final"})).await;
    rig.ok("room.trajectory", json!({})).await;
    rig.ok("room.reports", json!({"include_markdown": true})).await;

    rig.ok("room.close", json!({})).await;
    let e = rig.refused("room.shelf.list", json!({})).await;
    assert_eq!(reason(&e), Some("wrong_binding"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spend_reads_the_goal_notes_and_maps_ledger_roles_to_avatars() {
    let mut rig = rig();
    let _fake = rig.start(Script::default());
    rig.attach().await;
    let spend = rig.ok("room.spend", json!({})).await;
    let goal = &spend["goals"][0];
    assert_eq!(goal["goal_key"], "g1");
    assert_eq!(goal["total"]["calls"], 3);
    assert_eq!(goal["by_avatar"][1], json!({"avatar": "red-web", "calls": 2,
        "prompt_tokens": 20, "completion_tokens": 2, "cost": 0.2}));
    assert_eq!(spend["ledger"]["ledger"], "ok");
    assert_eq!(spend["ledger"]["totals"]["calls"], 5);
    let avatars: Vec<&str> = spend["ledger"]["by_avatar"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["avatar"].as_str().unwrap())
        .collect();
    assert_eq!(avatars, ["chief", "red-web"]);
    let other = rig.ok("room.spend", json!({"goal": "trajectory/other"})).await;
    assert_eq!(other["goals"], json!([]));
    let one = rig.ok("room.spend", json!({"goal": "g1"})).await;
    assert_eq!(one["goals"].as_array().unwrap().len(), 1);
}

// ── team views ──────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn team_fields_name_the_layer_they_come_from() {
    let mut rig = rig();
    rig.write(&rig.template(), "avatars.toml", r#"
[[avatar]]
id = "red-web"
role = "red"
display = "Template Web"

[[avatar]]
id = "red-api"
role = "red"
work = { provider = "p-1", model = "m-2", effort = "high" }
"#);
    rig.write(&rig.session(), "avatars.toml", r#"
[[avatar]]
id = "red-web"
role = "red"
alias = "SW"

[[avatar]]
id = "red-x"
role = "red"

[[avatar]]
id = "red-api"
role = "red"
work = { effort = "low" }
"#);
    let _fake = rig.start(Script::default());
    rig.attach().await;
    let team = rig.ok("room.team.get", json!({})).await;
    assert_eq!(team["target"], "room");
    assert_eq!(team["choices_source"], "room_daemon");
    assert_eq!(team["template_pending_restart"], false);
    assert_eq!(team["layer_present"], true);
    let web = avatar(&team, "red-web");
    assert_eq!(web["defined_in"], "base");
    assert_eq!(web["removable"], true);
    assert_eq!(web["display"], json!({"value": "Template Web", "origin": "template"}));
    assert_eq!(web["alias"], json!({"value": "SW", "origin": "session"}));
    assert_eq!(web["display_effective"], "Template Web");
    let api = lane(&team, "red-api", "work");
    assert_eq!(api["model"], json!({"value": "m-2", "origin": "template"}));
    assert_eq!(api["provider_uuid"], json!({"value": "p-1", "origin": "template"}));
    assert_eq!(api["effort"], json!({"value": "low", "origin": "session"}));
    assert_eq!(api["model_label"], "fast");
    assert_eq!(api["resolves"], true);
    let chat = lane(&team, "red-api", "chat");
    assert_eq!(chat["model"], json!({"value": null, "origin": "default"}));
    assert!(chat["model_label"].is_null());
    let x = avatar(&team, "red-x");
    assert_eq!((x["defined_in"].as_str(), x["removable"].as_bool()), (Some("session"), Some(true)));
    assert_eq!(x["display"], json!({"value": "Lab Red", "origin": "role"}));
    assert_eq!(x["alias"]["origin"], "default");
    let chief = avatar(&team, "chief");
    assert_eq!(chief["alias"], json!({"value": "CF", "origin": "base"}));
    assert_eq!(chief["removable"], false);
    assert_eq!(chief["lanes"].as_array().unwrap().len(), 1);
    assert_eq!(chief["lanes"][0]["lane"], "chat");
    assert_eq!(avatar(&team, "clerk")["lanes"][0]["lane"], "work");
    assert!(team["choices"].as_array().unwrap().iter().any(|c| c["id"] == "m-2"
        && c["provider_uuid"] == "p-1" && c["provider_name"] == "local"));

    let template = rig.ok("room.team.get", json!({"target": "template"})).await;
    assert_eq!(template["target"], "template");
    assert_eq!(template["choices_source"], "config");
    assert_eq!(avatar(&template, "red-web")["alias"], json!({"value": "LW", "origin": "base"}));
    assert!(template["avatars"].as_array().unwrap().iter().all(|a| a["id"] != "red-x"));
    assert_eq!(template["revision"], team["template_revision"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn roles_show_effective_values_and_the_layer_whose_role_file_defines_them() {
    let mut rig = rig();
    let narrowed = BASE_RED
        .replace("display = \"Lab Red\"", "display = \"Narrow Red\"")
        .replace(
            "shelf_allow = [\"get\", \"list\", \"put_claim\", \"put_evidence\", \"put_asset\", \"put_scope\", \"put_trajectory\"]",
            "shelf_allow = [\"get\", \"list\"]\nskills_allow = [\"recon-*\"]",
        );
    assert_ne!(narrowed, BASE_RED, "the base red role file changed shape");
    rig.write(&rig.session(), "roles/red/role.toml", &narrowed);
    // Widening is dropped whole, so the role stays as the base has it.
    rig.write(&rig.template(), "roles/reviewer/role.toml", "id = \"reviewer\"\ndisplay = \"Wide\"\nshelf_allow = [\"get\", \"put_report\"]\n");
    let _fake = rig.start(Script::default());
    rig.attach().await;
    let team = rig.ok("room.team.get", json!({"target": "room"})).await;
    let role = |id: &str| {
        team["roles"].as_array().unwrap().iter().find(|r| r["id"] == id).unwrap().clone()
    };
    let red = role("red");
    assert_eq!(red["display"], "Narrow Red");
    assert_eq!(red["shelf_allow"], json!(["get", "list"]));
    assert_eq!(red["skills_allow"], json!(["recon-*"]));
    assert_eq!(red["defined_in"], "session");
    assert_eq!(red["chat"], json!({"provider_uuid": null, "model": null, "effort": null}));
    let reviewer = role("reviewer");
    assert_eq!((reviewer["display"].as_str(), reviewer["defined_in"].as_str()), (Some("Reviewer"), Some("base")));
    assert!(reviewer["skills_allow"].is_null());
    assert!(team["warnings"].as_array().unwrap().iter().any(|w| w["layer"] == "template"
        && w["path"] == "roles/reviewer/role.toml"));
    assert_eq!(avatar(&team, "red-web")["display_effective"], "Lab Red Web");
    assert_eq!(avatar(&team, "red")["display_effective"], "Narrow Red");
}

// ── team writes ─────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_template_write_applies_at_the_next_start_in_any_binding() {
    let mut rig = rig();
    let written = rig
        .ok("room.avatars.upsert", json!({"target": "template", "id": "red-cli", "role": "red", "alias": "rc"}))
        .await;
    assert_eq!(written["applies"], "next_start");
    assert_eq!(written["propagation"], json!({"room": "not_live"}));
    let cli = avatar(&written["team"], "red-cli");
    assert_eq!((cli["defined_in"].as_str(), cli["alias_effective"].as_str()), (Some("template"), Some("RC")));
    let text = std::fs::read_to_string(rig.template().join("avatars.toml")).unwrap();
    assert!(text.contains("red-cli"));
    assert!(rig.template().join(".sdk-lock").is_file());
    assert!(!rig.template().join(".avatars.toml.sdk-tmp").exists());

    let e = rig.refused("room.team.get", json!({"target": "room"})).await;
    assert_eq!(reason(&e), Some("wrong_binding"));
    let extra = e.extra().unwrap();
    assert_eq!(extra.binding, Some("unbound"));
    assert_eq!(extra.allowed.clone().unwrap(), ["room_live", "room_closed"]);
    assert_eq!(e.message, "room.team.get requires binding room_live or room_closed; this adapter is unbound");

    // The room started now reads the template; the room view says so.
    let _fake = rig.start(Script::default());
    let attached = rig.attach().await;
    let frozen = attached["daemon"]["template_revision"].clone();
    let team = rig.ok("room.team.get", json!({})).await;
    assert_eq!(team["template_revision"], frozen);
    assert_eq!(team["template_pending_restart"], false);
    rig.ok("room.avatars.upsert", json!({"target": "template", "id": "red-cli", "display": "CLI"})).await;
    let team = rig.ok("room.team.get", json!({})).await;
    assert_eq!(team["template_pending_restart"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_live_room_with_reload_takes_the_write_live_and_keeps_the_display() {
    let mut rig = rig();
    let fake = rig.start(Script::default());
    rig.attach().await;
    let written = rig
        .ok("room.models.set", json!({"target": "room", "avatar": "red-web", "lane": "work", "model": "m-2"}))
        .await;
    assert_eq!(written["applies"], "live");
    assert_eq!(written["propagation"]["room"], "reloaded");
    assert_eq!(fake.received("sdk_reload"), 1);
    assert_eq!(fake.received("set_model"), 0);
    let web = avatar(&written["team"], "red-web");
    assert_eq!(web["display_effective"], "Lab Red Web");
    assert_eq!(web["alias_effective"], "LW");
    let work = lane(&written["team"], "red-web", "work");
    assert_eq!(work["model"], json!({"value": "m-2", "origin": "session"}));
    assert_eq!(work["provider_uuid"]["value"], "p-1");
    let text = std::fs::read_to_string(rig.session().join("avatars.toml")).unwrap();
    assert!(text.contains("m-2") && text.contains("p-1"));

    let added = rig
        .ok("room.avatars.upsert", json!({"target": "room", "id": "red-mobile", "role": "red", "alias": "RM"}))
        .await;
    assert_eq!(added["applies"], "live");
    let roster = rig.adapter.binding.room().unwrap().roster().to_vec();
    assert!(roster.iter().any(|a| a == "red-mobile"), "{roster:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_live_room_without_reload_gets_a_file_only_write_and_no_room_frame() {
    let mut rig = rig();
    let fake = rig.start(Script {
        features: vec!["sdk_user".into()],
        ..Script::default()
    });
    rig.attach().await;
    let written = rig
        .ok("room.models.set", json!({"target": "room", "avatar": "red-web", "lane": "work", "effort": "high"}))
        .await;
    assert_eq!(written["applies"], "next_reload");
    assert_eq!(written["propagation"]["room"], "not_supported");
    assert_eq!(lane(&written["team"], "red-web", "work")["effort"], json!({"value": "high", "origin": "session"}));
    assert_eq!(fake.received("sdk_reload"), 0);
    assert_eq!(fake.received("set_model"), 0);
    assert!(std::fs::read_to_string(rig.session().join("avatars.toml")).unwrap().contains("high"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_closed_binding_writes_the_file_and_says_when_it_applies() {
    let mut rig = rig();
    let mut fake = rig.start(Script::default());
    rig.attach().await;
    fake.take_over();
    until("ROOM(closed)", || rig.state() == BindingState::RoomClosed).await;
    let written = rig
        .ok("room.avatars.upsert", json!({"target": "room", "id": "red-ops", "role": "red"}))
        .await;
    assert_eq!(written["applies"], "next_reload");
    assert_eq!(written["propagation"]["room"], "not_live");
    assert_eq!(written["team"]["choices_source"], "config");
    let e = rig.refused("room.send", json!({"text": "hi"})).await;
    assert_eq!(reason(&e), Some("room_closed"));

    fake.exit().await;
    let written = rig
        .ok("room.avatars.remove", json!({"target": "room", "id": "red-ops"}))
        .await;
    assert_eq!(written["applies"], "next_start");
    assert!(written["reverted_to"].is_null());
    assert!(!rig.session().join("avatars.toml").exists(), "an empty layer file is removed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_change_the_room_rejects_is_rolled_back() {
    let mut rig = rig();
    let before = "[[avatar]]\nid = \"red-web\"\nrole = \"red\"\nalias = \"QQ\"\n";
    rig.write(&rig.session(), "avatars.toml", before);
    let fake = rig.start(Script {
        reject: Some(("red-web".into(), 1)),
        ..Script::default()
    });
    rig.attach().await;
    let e = rig
        .refused("room.avatars.upsert", json!({"target": "room", "id": "red-web", "display": "New"}))
        .await;
    assert_eq!((e.code, reason(&e), e.delivery), ("invalid_state", Some("rejected_by_room"), "not_sent"));
    let detail = e.extra().unwrap().detail.clone().unwrap();
    assert_eq!(detail["warnings"][0]["path"], "avatars.toml");
    assert_eq!(std::fs::read_to_string(rig.session().join("avatars.toml")).unwrap(), before);
    assert_eq!(fake.received("sdk_reload"), 2);
    let team = rig.ok("room.team.get", json!({})).await;
    assert_eq!(avatar(&team, "red-web")["alias"]["value"], "QQ");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn null_resets_a_field_to_the_layer_below_and_remove_reverts_the_entry() {
    let mut rig = rig();
    rig.write(&rig.template(), "avatars.toml", "[[avatar]]\nid = \"red-web\"\nrole = \"red\"\ndisplay = \"T Web\"\n");
    let _fake = rig.start(Script::default());
    rig.attach().await;
    let set = rig
        .ok("room.avatars.upsert", json!({"target": "room", "id": "red-web", "display": "S Web"}))
        .await;
    assert_eq!(avatar(&set["team"], "red-web")["display"], json!({"value": "S Web", "origin": "session"}));
    let revision = set["team"]["revision"].clone();
    let reset = rig
        .ok("room.avatars.upsert", json!({"target": "room", "id": "red-web", "display": null, "if_revision": revision}))
        .await;
    assert_eq!(avatar(&reset["team"], "red-web")["display"], json!({"value": "T Web", "origin": "template"}));
    assert!(!rig.session().join("avatars.toml").exists(), "a bare entry over a lower avatar is dropped");
    let e = rig
        .refused("room.avatars.upsert", json!({"target": "room", "id": "red-web", "alias": "ZZ", "if_revision": revision}))
        .await;
    assert_eq!(reason(&e), Some("stale"));

    rig.ok("room.models.set", json!({"target": "room", "avatar": "red", "lane": "chat", "model": "m-2", "effort": "high"})).await;
    let model_only = rig
        .ok("room.models.set", json!({"target": "room", "avatar": "red", "lane": "chat", "model": "m-1"}))
        .await;
    let chat = lane(&model_only["team"], "red", "chat");
    assert_eq!((chat["model"]["value"].as_str(), chat["effort"]["value"].as_str()), (Some("m-1"), Some("high")));
    let cleared = rig
        .ok("room.models.set", json!({"target": "room", "avatar": "red", "lane": "chat", "model": null}))
        .await;
    assert_eq!(lane(&cleared["team"], "red", "chat")["model"], json!({"value": null, "origin": "default"}));
    assert_eq!(lane(&cleared["team"], "red", "chat")["effort"]["origin"], "default");

    rig.ok("room.avatars.upsert", json!({"target": "room", "id": "red-web", "alias": "ZZ"})).await;
    let removed = rig
        .ok("room.avatars.remove", json!({"target": "room", "id": "red-web"}))
        .await;
    assert_eq!(removed["reverted_to"], "template");
    assert_eq!(avatar(&removed["team"], "red-web")["alias"]["value"], "LW");
    let removed = rig
        .ok("room.avatars.remove", json!({"target": "template", "id": "red-web"}))
        .await;
    assert_eq!(removed["reverted_to"], "base");
    let e = rig.refused("room.avatars.remove", json!({"target": "room", "id": "red-web"})).await;
    assert_eq!(reason(&e), Some("unknown_avatar"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn team_writes_check_everything_before_writing() {
    let mut rig = rig();
    let fake = rig.start(Script::default());
    rig.attach().await;
    let cases = [
        (json!({"avatar": "red", "lane": "chat", "model": "m-2"}), "room.models.set", "target_required"),
        (json!({"target": "room", "avatar": "red", "lane": "chat"}), "room.models.set", "missing_params"),
        (json!({"target": "room", "avatar": "chief", "lane": "work", "model": "m-2"}), "room.models.set", "no_such_lane"),
        (json!({"target": "room", "avatar": "clerk", "lane": "chat", "effort": "low"}), "room.models.set", "no_such_lane"),
        (json!({"target": "room", "avatar": "red", "lane": "chat", "model": "nope"}), "room.models.set", "unknown_model"),
        (json!({"target": "room", "avatar": "ghost", "lane": "chat", "effort": "low"}), "room.models.set", "unknown_avatar"),
        (json!({"target": "room", "avatar": "red", "lane": "chat", "effort": "two words"}), "room.models.set", "out_of_range"),
        (json!({"target": "room", "id": "Bad Id", "role": "red"}), "room.avatars.upsert", "invalid_name"),
        (json!({"target": "room", "id": "red-new"}), "room.avatars.upsert", "missing_params"),
        (json!({"target": "room", "id": "red-new", "role": "ghost"}), "room.avatars.upsert", "unknown_role"),
        (json!({"target": "room", "id": "red-new", "role": "chief"}), "room.avatars.upsert", "reserved_id"),
        (json!({"target": "room", "id": "red-web", "role": "blue"}), "room.avatars.upsert", "role_immutable"),
        (json!({"target": "room", "id": "red-web"}), "room.avatars.upsert", "missing_params"),
        (json!({"target": "room", "id": "red-web", "alias": "TOOLONG"}), "room.avatars.upsert", "out_of_range"),
        (json!({"target": "room", "id": "red-web", "display": "a\u{1}b"}), "room.avatars.upsert", "invalid_name"),
        (json!({"target": "room", "id": "red-web"}), "room.avatars.remove", "unknown_avatar"),
        (json!({"target": "elsewhere", "id": "red-web"}), "room.avatars.remove", "out_of_range"),
    ];
    for (params, method, want) in cases {
        let e = rig.refused(method, params.clone()).await;
        assert_eq!((reason(&e), e.delivery), (Some(want), "not_sent"), "{method} {params}");
    }
    assert!(!rig.session().join("avatars.toml").exists(), "a refused write wrote nothing");
    assert_eq!(fake.received("sdk_reload"), 0);

    rig.write(&rig.session(), "avatars.toml", "[[avatar]\nbroken");
    let e = rig
        .refused("room.avatars.upsert", json!({"target": "room", "id": "red-web", "alias": "AB"}))
        .await;
    assert_eq!(reason(&e), Some("layer_rejected"));
    assert_eq!(std::fs::read_to_string(rig.session().join("avatars.toml")).unwrap(), "[[avatar]\nbroken");
    std::fs::remove_file(rig.session().join("avatars.toml")).unwrap();

    rig.ok("room.avatars.upsert", json!({"target": "room", "id": "red-ops", "role": "red"})).await;
    fake.push(status_frame("red-ops", "running", ""));
    until("the running lane", || {
        rig.adapter.binding.room().and_then(|r| r.cached_status()).is_some_and(|s| s.lanes.iter().any(|l| l.avatar == "red-ops"))
    })
    .await;
    let e = rig.refused("room.avatars.remove", json!({"target": "room", "id": "red-ops"})).await;
    assert_eq!(reason(&e), Some("lane_running"));
}

#[test]
fn hello_advertises_the_room_reads_team_methods_and_their_limits() {
    let hello = protocol::hello("/bin/koma");
    for method in [
        "room.shelf.list", "room.item", "room.trajectory", "room.reports", "room.spend",
        "room.team.get", "room.models.set", "room.avatars.upsert", "room.avatars.remove",
    ] {
        assert!(hello["methods"].as_array().unwrap().iter().any(|m| m == method), "{method}");
    }
    assert!(hello["features"].as_array().unwrap().iter().any(|f| f == "room.template"));
    let list = &hello["limits"]["list"];
    assert_eq!((list["shelf_rows"].as_u64(), list["reports"].as_u64()), (Some(500), Some(100)));
    assert_eq!(list["trajectory_goals"], 200);
    assert_eq!(list["trajectory_nodes"], 2000);
    assert_eq!(list["trajectory_edges"], 8000);
    assert_eq!(hello["limits"]["room"]["item_bytes"], 16 * 1024 * 1024);
}
