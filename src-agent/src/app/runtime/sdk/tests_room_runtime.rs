//! Tests for the room binding: a scripted fake room on a scratch socket, the
//! event queue, the line ring and the scrubber. Nothing here starts a real
//! daemon or touches koma's data dir.
use super::binding::BindingState;
use super::events::{EventQueue, QUEUE_ITEMS};
use super::host::config::ConfigStore;
use super::protocol::{Request, RpcError};
use super::room::ring::{self, Filter, Ring, RoomLine};
use super::room::scrub::Scrubber;
use super::room::RoomEnv;
use super::*;
use crate::app::runtime::manage::RoomStartError;
use crate::ipc::frame::{read_frame_from, write_frame_to, FrameReader};
use crate::ipc::proto::{DaemonEvent, DaemonFrame, SessionStatus};
use crate::ipc::split_stream;
use crate::model::app_config::{ApiType, AppConfig, ModelEntry, ModelRole, ProviderConn};
use crate::model::session_registry::RegRowFullCreated;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

const PLANTED_KEY: &str = "sk-room-planted-0123456789";
const LATER_KEY: &str = "sk-room-added-later-0123456789";
const ROOM_ID: &str = "0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f";

/// A fresh short directory under the system temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let dir = std::env::temp_dir().join(format!("kr-{}", &id[..12]));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn sock_path(dir: &Path, name: &str) -> PathBuf {
    #[cfg(unix)]
    {
        dir.join(format!("{name}.sock"))
    }
    #[cfg(windows)]
    {
        let _ = dir;
        PathBuf::from(format!(
            r"\\.\pipe\koma-test-{name}-{}",
            uuid::Uuid::new_v4().simple()
        ))
    }
}

fn write_config(dir: &Path, key: &str) {
    let config = AppConfig {
        providers: vec![ProviderConn {
            uuid: "p-1".into(),
            name: "local".into(),
            api_type: ApiType::OpenAiCompatible,
            endpoint: "http://127.0.0.1:9/v1".into(),
            api_key: key.into(),
            ext_id: None,
        }],
        models: vec![ModelEntry {
            uuid: "m-1".into(),
            name: "main".into(),
            model_id: "main-id".into(),
            provider_uuid: "p-1".into(),
            roles: vec![ModelRole::Main],
            ..ModelEntry::default()
        }],
        ..AppConfig::default()
    };
    let secret = crate::model::secret_box::key_for_write().unwrap();
    let bytes = config.to_disk_bytes(&secret).unwrap();
    crate::model::secret_box::write_private(&dir.join("config.json"), &bytes).unwrap();
}

// ── the fake room ───────────────────────────────────────────────────────

#[derive(Clone)]
struct Script {
    features: Vec<String>,
    replay: Vec<Value>,
    /// Sent right after the replay, as live frames.
    live: Vec<Value>,
    lanes: Vec<&'static str>,
    ack_ok: bool,
    /// Written right after the answer to a send, as a room that replies at
    /// once does.
    reply: Option<Value>,
}

impl Default for Script {
    fn default() -> Self {
        Self {
            features: vec!["sdk_user".into(), "sdk_reload".into()],
            replay: Vec::new(),
            live: Vec::new(),
            lanes: vec!["chief", "red-web", "clerk"],
            ack_ok: true,
            reply: None,
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

async fn write_value(write: &mut crate::ipc::IpcWriteHalf, value: &Value) -> std::io::Result<()> {
    write_frame_to(write, &serde_json::to_vec(value).unwrap()).await
}

fn tag(value: &Value) -> String {
    match value {
        Value::String(tag) => tag.clone(),
        Value::Object(map) => map.keys().next().cloned().unwrap_or_default(),
        _ => String::new(),
    }
}

impl FakeRoom {
    fn start(path: PathBuf, script: Script) -> Self {
        let listener = crate::ipc::server::bind(&path).unwrap();
        let received = Arc::new(Mutex::new(Vec::new()));
        let current: Arc<Mutex<Option<UnboundedSender<Cmd>>>> = Arc::new(Mutex::new(None));
        let script = Arc::new(Mutex::new(script));
        let accept = {
            let (received, current, script) = (received.clone(), current.clone(), script.clone());
            tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    let (received, current, script) =
                        (received.clone(), current.clone(), script.clone());
                    tokio::spawn(async move {
                        let _ = serve(stream, received, current, script).await;
                    });
                }
            })
        };
        let _ = path;
        Self {
            received,
            current,
            accept: Some(accept),
        }
    }

    fn push(&self, frame: Value) {
        if let Some(tx) = self.current.lock().unwrap().as_ref() {
            let _ = tx.send(Cmd::Frame(frame));
        }
    }

    fn received_tags(&self) -> Vec<String> {
        self.received.lock().unwrap().iter().map(tag).collect()
    }

    fn received(&self, wanted: &str) -> Vec<Value> {
        self.received
            .lock()
            .unwrap()
            .iter()
            .filter(|v| tag(v) == wanted)
            .cloned()
            .collect()
    }

    /// Another client takes the stream: the subscribed connection closes
    /// with no frame, and the room keeps answering status probes.
    fn take_over(&self) {
        if let Some(tx) = self.current.lock().unwrap().take() {
            let _ = tx.send(Cmd::Close);
        }
    }

    /// The daemon goes away: the listener closes, then the stream.
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
) -> std::io::Result<()> {
    let (mut read, mut write) = split_stream(stream);
    let mut frames = FrameReader::new();
    let first: Value = serde_json::from_slice(&read_frame_from(&mut read, &mut frames).await?)?;
    received.lock().unwrap().push(first.clone());
    if first == json!("Status") {
        let frame = DaemonFrame {
            seq: 1,
            event: DaemonEvent::Status(SessionStatus {
                session_id: ROOM_ID.into(),
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
    let script = script.lock().unwrap().clone();
    let (tx, mut rx) = unbounded_channel();
    *current.lock().unwrap() = Some(tx);
    let hello = json!({"hello": {"kind": "secure", "replayed": script.replay.len(),
        "features": script.features, "version": "test-build"}});
    write_value(&mut write, &hello).await?;
    for frame in script.replay.iter().chain(script.live.iter()) {
        write_value(&mut write, frame).await?;
    }
    loop {
        tokio::select! {
            incoming = read_frame_from(&mut read, &mut frames) => {
                let Ok(bytes) = incoming else { break };
                let value: Value = serde_json::from_slice(&bytes)?;
                received.lock().unwrap().push(value.clone());
                match tag(&value).as_str() {
                    "models" => {
                        let lanes: Vec<Value> = script.lanes.iter()
                            .map(|a| json!({"avatar": a, "display": a, "lane": "chat", "model": "session default"}))
                            .collect();
                        write_value(&mut write, &json!({"models": {"choices": [], "lanes": lanes}})).await?;
                    }
                    "quit" => {
                        write_value(&mut write, &json!("closed")).await?;
                        break;
                    }
                    "sdk_user" => {
                        let id = value["sdk_user"]["id"].clone();
                        let count = value["sdk_user"]["attachments"].as_array().map_or(0, Vec::len);
                        let ack = if script.ack_ok {
                            let inserts: Vec<String> = (1..=count).map(|n| format!("[Image #{n}]")).collect();
                            json!({"sdk_ack": {"id": id, "ok": true, "inserts": inserts}})
                        } else {
                            json!({"sdk_ack": {"id": id, "ok": false, "code": "image_unreadable",
                                "error": format!("could not read the image with key {PLANTED_KEY}")}})
                        };
                        write_value(&mut write, &ack).await?;
                        if let Some(reply) = &script.reply {
                            write_value(&mut write, reply).await?;
                        }
                    }
                    "user" => {
                        if let Some(reply) = &script.reply {
                            write_value(&mut write, reply).await?;
                        }
                    }
                    "sdk_reload" => {
                        let id = value["sdk_reload"]["id"].clone();
                        write_value(&mut write, &json!({"sdk_ack": {"id": id, "ok": true, "version": 3, "warnings": ["w1"]}})).await?;
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

fn status_frame(open: u64, now_ms: u64) -> Value {
    json!({"status": {"digest": "digest text", "timeline": "", "open": open, "verified": 0,
        "returned": 0, "reports": 0, "paused": [], "shelf": "", "trajectory": "",
        "lanes": [{"avatar": "red-web", "display": "Red Web", "alias": "RW", "role": "red",
            "lane": "work", "phase": "running", "task": "scan", "started_ms": 1,
            "last_event_ms": 2, "steps": 3, "last_tool": "", "error": ""}],
        "events": [], "now_ms": now_ms}})
}

fn chief(text: &str) -> Value {
    json!({"chief": {"text": text}})
}

// ── the adapter under test ──────────────────────────────────────────────

struct Rig {
    _dir: TempDir,
    _keep: Vec<TempDir>,
    root: PathBuf,
    config_dir: PathBuf,
    adapter: Adapter,
    seq: u64,
}

fn row(uuid: &str, workdir: &str) -> RegRowFullCreated {
    RegRowFullCreated {
        uuid: uuid.into(),
        pwd_hash: "bucket".into(),
        name: String::new(),
        workdir: workdir.into(),
        created_at: 10,
        updated_at: 20,
    }
}

/// An adapter whose room hooks point at `socks` (id to socket path) and a
/// registry of `rows`, with session dirs under the rig's own directory.
fn rig(socks: HashMap<String, PathBuf>, rows: Vec<RegRowFullCreated>) -> Rig {
    let dir = TempDir::new();
    let root = dir.0.join("s");
    let config_dir = dir.0.join("c");
    std::fs::create_dir_all(&config_dir).unwrap();
    write_config(&config_dir, PLANTED_KEY);
    for row in &rows {
        std::fs::create_dir_all(root.join(&row.pwd_hash).join(&row.uuid)).unwrap();
    }
    let (events, _) = watch::channel(None);
    let mut adapter = Adapter::new(events);
    adapter.config = ConfigStore::at(&config_dir);
    let dead = dir.0.join("dead.sock");
    let session_root = root.clone();
    adapter.room_env = RoomEnv {
        sock_path: Arc::new(move |id| Ok(socks.get(id).cloned().unwrap_or_else(|| dead.clone()))),
        sessions: Arc::new(move || Ok(rows.clone())),
        session_dir: Arc::new(move |hash, id| Ok(session_root.join(hash).join(id))),
        ensure_running: Arc::new(|_, _, _, _| Ok(false)),
        start_log: Arc::new(|_| anyhow::bail!("no start log")),
        stop: Arc::new(|_| true),
        template_dir: Arc::new(|| None),
        usage: Arc::new(|_| {
            Err(crate::model::usage::query::UsageError::Unreadable(
                "no ledger in tests".into(),
            ))
        }),
    };
    Rig {
        _dir: dir,
        _keep: Vec::new(),
        root,
        config_dir,
        adapter,
        seq: 0,
    }
}

impl Rig {
    fn room_dir(&self, id: &str) -> PathBuf {
        self.root.join("bucket").join(id)
    }

    /// Make `id` a room session: a shelf database in its dir.
    fn make_room(&self, id: &str) {
        let dir = self.room_dir(id);
        std::fs::create_dir_all(&dir).unwrap();
        crate::room::shelf::Shelf::open(&crate::room::room_db_path(&dir)).unwrap();
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
        // As the writer does once a response is out.
        crate::app::runtime::sdk::events::release_after_response(&self.adapter.queue);
        result
    }

    /// The events the writer would write now.
    fn events(&mut self) -> Vec<Value> {
        self.adapter.queue.take_ready(&mut self.seq)
    }

    fn state(&self) -> BindingState {
        self.adapter.binding.state()
    }
}

fn single_room(script: Script) -> (Rig, FakeRoom) {
    let probe = TempDir::new();
    let path = sock_path(&probe.0, "room");
    let fake = FakeRoom::start(path.clone(), script);
    let mut rig = rig(
        HashMap::from([(ROOM_ID.to_string(), path)]),
        vec![row(ROOM_ID, "/work")],
    );
    rig._keep.push(probe);
    rig.make_room(ROOM_ID);
    (rig, fake)
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

fn line_events(events: &[Value]) -> Vec<&Value> {
    events
        .iter()
        .filter(|e| e["event"] == "room.line")
        .collect()
}

async fn attach(rig: &mut Rig, wait_ms: u64) -> Value {
    rig.call(
        "room.attach",
        json!({"session_id": ROOM_ID, "tail": 200, "wait_ms": wait_ms}),
    )
    .await
    .unwrap()
}

// ── binding and replay ──────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_status_led_replay_counts_lines_and_the_live_status_makes_it_fresh() {
    let (mut rig, fake) = single_room(Script {
        replay: vec![status_frame(1, 5), chief("old one"), chief("old two")],
        live: vec![status_frame(2, 6)],
        ..Script::default()
    });
    let result = attach(&mut rig, 2000).await;
    assert_eq!(rig.state(), BindingState::RoomLive);
    assert_eq!(result["history"]["replayed"], 2);
    assert_eq!(result["history"]["count"], 2);
    assert_eq!(result["history"]["first_line_seq"], 1);
    assert_eq!(result["history"]["last_line_seq"], 2);
    assert_eq!(result["tail"].as_array().unwrap().len(), 2);
    assert_eq!(result["tail"][0]["text"], "old one");
    assert_eq!(result["tail"][0]["replay"], true);
    assert_eq!(result["tail"][0]["kind"], "chief");
    assert_eq!(result["status"]["fresh"], true);
    assert_eq!(result["status"]["counts"]["open"], 2);
    assert_eq!(result["status"]["counts_basis"], "first_400_items");
    assert_eq!(result["status"]["lanes"][0]["started_at_ms"], 1);
    assert_eq!(result["started"], false);
    assert_eq!(result["room"]["bound"], true);
    assert_eq!(result["daemon"]["version"], "test-build");
    assert_eq!(
        result["daemon"]["features"],
        json!(["sdk_user", "sdk_reload"])
    );
    assert_eq!(result["roster"], json!(["chief", "red-web", "clerk"]));
    assert!(uuid::Uuid::parse_str(result["epoch"].as_str().unwrap()).is_ok());
    drop(fake);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_replay_without_status_is_all_lines_and_stays_unfresh_without_a_live_status() {
    let (mut rig, _fake) = single_room(Script {
        replay: vec![chief("a"), chief("b"), chief("c")],
        ..Script::default()
    });
    let result = attach(&mut rig, 100).await;
    assert_eq!(result["history"]["replayed"], 3);
    assert!(result["status"].is_null());
    let status = rig.call("room.status", json!({})).await.unwrap();
    assert_eq!(status["fresh"], false);
    assert_eq!(status["live"], true);
    assert!(status["shelf_fingerprint"]
        .as_str()
        .is_some_and(|f| !f.is_empty()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_event_precedes_the_attach_response_and_line_seq_is_not_the_envelope_seq() {
    let (mut rig, fake) = single_room(Script {
        replay: vec![chief("replayed")],
        ..Script::default()
    });
    // Envelope numbers already used by earlier events.
    rig.seq = 40;
    let attached = dispatch(
        Request {
            id: "a".into(),
            method: "room.attach".into(),
            params: json!({"session_id": ROOM_ID, "wait_ms": 0}),
        },
        &mut rig.adapter,
    )
    .await
    .unwrap();
    fake.push(chief("live"));
    tokio::time::sleep(Duration::from_millis(200)).await;
    // Held until the response is written.
    assert!(rig.events().is_empty(), "nothing before the response");
    crate::app::runtime::sdk::events::release_after_response(&rig.adapter.queue);
    let mut events = Vec::new();
    until("the live line event", || {
        events.extend(rig.adapter.queue.take_ready(&mut rig.seq));
        !line_events(&events).is_empty()
    })
    .await;
    let lines = line_events(&events);
    assert_eq!(lines[0]["line"]["line_seq"], 2);
    assert_eq!(lines[0]["line"]["text"], "live");
    assert_eq!(lines[0]["epoch"], attached["epoch"]);
    assert!(lines[0]["seq"].as_u64().unwrap() > 40);
    assert!(events
        .iter()
        .all(|e| e.get("state").is_none() && e.get("revision").is_none()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_take_back_discards_the_old_epoch_and_starts_a_fresh_ring() {
    let (mut rig, fake) = single_room(Script {
        replay: vec![chief("one")],
        ..Script::default()
    });
    let first = attach(&mut rig, 0).await;
    fake.take_over();
    until("room(closed)", || rig.state() == BindingState::RoomClosed).await;
    // The closed binding keeps its ring.
    let history = rig
        .call("room.history", json!({"epoch": first["epoch"]}))
        .await
        .unwrap();
    assert_eq!(history["lines"][0]["text"], "one");
    // Take it back before the old events are written.
    let second = attach(&mut rig, 0).await;
    assert_ne!(first["epoch"], second["epoch"]);
    assert_eq!(second["history"]["first_line_seq"], 1);
    let events = rig.events();
    assert!(
        events.iter().all(|e| e["epoch"] == second["epoch"]),
        "{events:?}"
    );
    let stale = rig
        .call("room.history", json!({"epoch": first["epoch"]}))
        .await
        .unwrap_err();
    assert_eq!(reason(&stale), Some("epoch_changed"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_takeover_and_a_daemon_exit_are_told_apart() {
    let (mut rig, mut fake) = single_room(Script::default());
    attach(&mut rig, 0).await;
    rig.events();
    fake.take_over();
    let mut events = Vec::new();
    until("room.closed", || {
        events.extend(rig.events());
        events.iter().any(|e| e["event"] == "room.closed")
    })
    .await;
    let closed = events.iter().find(|e| e["event"] == "room.closed").unwrap();
    assert_eq!(closed["reason"], "taken_over");
    assert_eq!(closed["session_id"], ROOM_ID);
    assert_eq!(rig.state(), BindingState::RoomClosed);

    let second = attach(&mut rig, 0).await;
    rig.events();
    fake.exit().await;
    let mut events = Vec::new();
    until("room.closed", || {
        events.extend(rig.events());
        events.iter().any(|e| e["event"] == "room.closed")
    })
    .await;
    let closed = events.iter().find(|e| e["event"] == "room.closed").unwrap();
    assert_eq!(closed["reason"], "daemon_exit");
    assert_eq!(closed["epoch"], second["epoch"]);
    // ROOM(closed): socket writes are E4, reads keep working.
    let send = rig
        .call("room.send", json!({"text": "hi"}))
        .await
        .unwrap_err();
    assert_eq!(reason(&send), Some("room_closed"));
    assert_eq!(send.message, "the room connection is closed");
    let status = rig.call("room.status", json!({})).await.unwrap();
    assert_eq!(status["live"], false);
    assert_eq!(status["counts_basis"], "disk_scan");
    assert_eq!(status["lanes"], json!([]));
    // A take-back of a room that is gone is `not_running`.
    let gone = rig
        .call("room.attach", json!({"session_id": ROOM_ID}))
        .await
        .unwrap_err();
    assert_eq!(reason(&gone), Some("not_running"));
    assert_eq!(rig.state(), BindingState::RoomClosed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_status_that_only_ticks_is_not_emitted_again() {
    let (mut rig, fake) = single_room(Script::default());
    attach(&mut rig, 0).await;
    rig.events();
    let statuses = |events: &[Value]| {
        events
            .iter()
            .filter(|e| e["event"] == "room.status")
            .count()
    };
    fake.push(status_frame(1, 100));
    let mut events = Vec::new();
    until("the first status", || {
        events.extend(rig.events());
        statuses(&events) == 1
    })
    .await;
    fake.push(status_frame(1, 200));
    fake.push(chief("marker"));
    until("the marker line", || {
        events.extend(rig.events());
        !line_events(&events).is_empty()
    })
    .await;
    assert_eq!(statuses(&events), 1, "a clock-only change is not emitted");
    fake.push(status_frame(7, 300));
    until("the changed status", || {
        events.extend(rig.events());
        statuses(&events) == 2
    })
    .await;
    let last = events
        .iter()
        .rfind(|e| e["event"] == "room.status")
        .unwrap();
    assert_eq!(last["status"]["counts"]["open"], 7);
    assert_eq!(last["status"]["fresh"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deltas_and_parts_map_to_events_and_lines() {
    let (mut rig, fake) = single_room(Script::default());
    attach(&mut rig, 0).await;
    rig.events();
    let part = |part: Value| json!({"part": {"author": "red-web", "lane": "work", "part": part, "audience": "team"}});
    fake.push(part(json!({"kind": "text_delta", "text": "Hel"})));
    fake.push(part(json!({"kind": "text_delta", "text": "lo"})));
    fake.push(part(
        json!({"kind": "tool_call", "id": "c1", "name": "shell", "args": "{}"}),
    ));
    fake.push(part(
        json!({"kind": "tool_result", "id": "c1", "ok": false, "result": "error: x"}),
    ));
    fake.push(part(json!({"kind": "brand_new", "x": 1})));
    fake.push(json!({"visible": {"author": "shell", "text": "$ echo hi\nhi"}}));
    fake.push(json!({"notice": {"text": "unsolicited"}}));
    let mut events = Vec::new();
    until("all frames", || {
        events.extend(rig.events());
        events.iter().any(|e| e["event"] == "room.notice")
    })
    .await;
    let deltas: Vec<&Value> = events
        .iter()
        .filter(|e| e["event"] == "room.delta")
        .collect();
    let text: String = deltas.iter().map(|d| d["text"].as_str().unwrap()).collect();
    assert_eq!(text, "Hello");
    assert_eq!(deltas[0]["lane"], "work");
    assert_eq!(deltas[0]["kind"], "text");
    let lines = line_events(&events);
    let kinds: Vec<&str> = lines
        .iter()
        .map(|l| l["line"]["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["tool_call", "tool_result", "other", "shell"]);
    assert_eq!(lines[0]["line"]["call_id"], "c1");
    assert_eq!(lines[0]["line"]["tool"], "shell");
    assert_eq!(lines[1]["line"]["ok"], false);
    assert_eq!(lines[1]["line"]["audience"], "team");
    assert!(lines[2]["line"]["text"]
        .as_str()
        .unwrap()
        .contains("brand_new"));
    let seqs: Vec<u64> = lines
        .iter()
        .map(|l| l["line"]["line_seq"].as_u64().unwrap())
        .collect();
    assert_eq!(seqs, [1, 2, 3, 4]);
    let notice = events.iter().find(|e| e["event"] == "room.notice").unwrap();
    assert_eq!(notice["code"], "room");
    assert_eq!(notice["text"], "unsolicited");
    // Deltas are never history.
    let history = rig
        .call(
            "room.history",
            json!({"epoch": lines[0]["epoch"], "kinds": ["shell"]}),
        )
        .await
        .unwrap();
    assert_eq!(history["lines"].as_array().unwrap().len(), 1);
    assert_eq!(history["first_line_seq"], 1);
    assert_eq!(history["last_line_seq"], 4);
}

// ── sending ─────────────────────────────────────────────────────────────

fn image(dir: &Path) -> String {
    let path = dir.join("shot.png");
    std::fs::write(&path, b"\x89PNG\r\n\x1a\n").unwrap();
    path.to_string_lossy().into_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdk_user_sends_one_frame_with_or_without_attachments_and_echoes_first() {
    let (mut rig, fake) = single_room(Script::default());
    attach(&mut rig, 0).await;
    rig.events();
    let plain = rig
        .call("room.send", json!({"text": "hello", "to": "red-web"}))
        .await
        .unwrap();
    assert_eq!(plain["acked"], true);
    assert_eq!(plain["sent"], true);
    let events = rig.events();
    let echo = line_events(&events)[0];
    assert_eq!(echo["line"]["origin"], "adapter_echo");
    assert_eq!(echo["line"]["kind"], "user");
    assert_eq!(echo["line"]["line_seq"], plain["echo_line_seq"]);
    assert_eq!(echo["line"]["text"], "@red-web hello");
    let path = image(&rig.config_dir);
    let with = rig
        .call(
            "room.send",
            json!({"text": "look", "attachments": [{"kind": "image_file", "path": path}]}),
        )
        .await
        .unwrap();
    assert_eq!(with["inserts"], json!(["[Image #1]"]));
    let sent = fake.received("sdk_user");
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0]["sdk_user"]["text"], "@red-web hello");
    assert_eq!(sent[1]["sdk_user"]["attachments"][0]["kind"], "image_file");
    let shell = rig
        .call("room.send", json!({"text": "echo hi", "shell": true}))
        .await
        .unwrap();
    assert_eq!(shell["acked"], true);
    assert_eq!(fake.received("sdk_user")[2]["sdk_user"]["text"], "!echo hi");
    assert!(fake.received("user").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reply_written_right_after_the_send_comes_after_its_echo() {
    // The room's answer to a send is the sync point: its echo is numbered
    // before the reply that follows the ack on the stream.
    let (mut rig, _fake) = single_room(Script {
        reply: Some(chief("the clerk only speaks to the chief")),
        ..Script::default()
    });
    attach(&mut rig, 0).await;
    rig.events();
    let mut echoes = Vec::new();
    for n in 0..20 {
        let sent = rig
            .call("room.send", json!({"text": format!("@clerk hello {n}")}))
            .await
            .unwrap();
        echoes.push(sent["echo_line_seq"].as_u64().unwrap());
    }
    let mut events = Vec::new();
    until("every reply", || {
        events.extend(rig.events());
        line_events(&events).len() >= 40
    })
    .await;
    let lines: Vec<(u64, String)> = line_events(&events)
        .iter()
        .map(|e| {
            (
                e["line"]["line_seq"].as_u64().unwrap(),
                e["line"]["origin"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    for (n, pair) in lines.chunks(2).enumerate() {
        assert_eq!(pair[0].1, "adapter_echo", "{lines:?}");
        assert_eq!(pair[0].0, echoes[n]);
        assert_eq!(pair[1].1, "room", "{lines:?}");
        assert!(pair[1].0 > pair[0].0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_older_room_gets_plain_user_frames_and_refuses_attachments_before_writing() {
    let (mut rig, fake) = single_room(Script {
        features: Vec::new(),
        ..Script::default()
    });
    attach(&mut rig, 0).await;
    let sent = rig.call("room.send", json!({"text": "hi"})).await.unwrap();
    assert_eq!(sent["acked"], false);
    assert_eq!(sent["inserts"], json!([]));
    let path = image(&rig.config_dir);
    let before = fake.received_tags().len();
    let refused = rig
        .call(
            "room.send",
            json!({"text": "look", "attachments": [{"kind": "image_file", "path": path}]}),
        )
        .await
        .unwrap_err();
    assert_eq!(reason(&refused), Some("peer_unsupported"));
    assert_eq!(refused.delivery, "not_sent");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(fake.received_tags().len(), before, "nothing was written");
    assert_eq!(fake.received("user")[0], json!({"user": {"text": "hi"}}));
    let reload = rig
        .adapter
        .binding
        .room_mut()
        .unwrap()
        .reload()
        .await
        .unwrap_err();
    assert_eq!(reason(&reload), Some("peer_unsupported"));
    assert!(fake.received("sdk_reload").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_and_control_params_are_checked_against_the_roster() {
    let (mut rig, fake) = single_room(Script::default());
    attach(&mut rig, 0).await;
    let cases = [
        ("room.send", json!({"text": "  "}), "out_of_range"),
        (
            "room.send",
            json!({"text": "x", "to": "clerk"}),
            "clerk_not_addressable",
        ),
        (
            "room.send",
            json!({"text": "x", "to": "nobody"}),
            "unknown_avatar",
        ),
        (
            "room.send",
            json!({"text": "x", "shell": true, "to": "chief"}),
            "conflicting_params",
        ),
        (
            "room.control",
            json!({"action": "redirect", "avatar": "red-web"}),
            "note_required",
        ),
        (
            "room.control",
            json!({"action": "pause", "avatar": "red-web", "note": "n"}),
            "conflicting_params",
        ),
        (
            "room.control",
            json!({"action": "pause", "avatar": "chief"}),
            "no_work_lane",
        ),
        (
            "room.control",
            json!({"action": "pause", "avatar": "ghost"}),
            "unknown_avatar",
        ),
        (
            "room.control",
            json!({"action": "explode", "avatar": "red-web"}),
            "out_of_range",
        ),
    ];
    for (method, params, expected) in cases {
        let error = rig.call(method, params.clone()).await.unwrap_err();
        assert_eq!(reason(&error), Some(expected), "{method} {params}");
        assert_eq!(error.delivery, "not_sent");
    }
    rig.call(
        "room.control",
        json!({"action": "redirect", "avatar": "red-web", "note": "look at /login"}),
    )
    .await
    .unwrap();
    rig.call("room.interrupt", json!({})).await.unwrap();
    until("control and interrupt", || {
        !fake.received("interrupt").is_empty() && !fake.received("control").is_empty()
    })
    .await;
    assert_eq!(
        fake.received("control")[0],
        json!({"control": {"command": {"redirect": {"avatar": "red-web", "note": "look at /login"}}}})
    );
    assert!(fake.received("sdk_user").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_attachment_is_not_sent_and_its_text_is_scrubbed() {
    let (mut rig, _fake) = single_room(Script {
        ack_ok: false,
        ..Script::default()
    });
    attach(&mut rig, 0).await;
    let path = image(&rig.config_dir);
    let error = rig
        .call(
            "room.send",
            json!({"text": "look", "attachments": [{"kind": "image_file", "path": path}]}),
        )
        .await
        .unwrap_err();
    assert_eq!(reason(&error), Some("attachment_refused"));
    assert_eq!(error.code, "invalid_request");
    assert!(!error.message.contains(PLANTED_KEY), "{}", error.message);
    assert!(error.message.contains("[redacted]"));
    assert_eq!(rig.state(), BindingState::RoomLive);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_advertised_frames_are_ever_sent() {
    let (mut rig, fake) = single_room(Script::default());
    attach(&mut rig, 0).await;
    rig.call("room.send", json!({"text": "hi"})).await.unwrap();
    rig.call("room.interrupt", json!({})).await.unwrap();
    rig.adapter
        .binding
        .room_mut()
        .unwrap()
        .reload()
        .await
        .unwrap();
    rig.call("room.close", json!({})).await.unwrap();
    let allowed = [
        "hello",
        "models",
        "user",
        "sdk_user",
        "sdk_reload",
        "interrupt",
        "control",
        "quit",
    ];
    let tags = fake.received_tags();
    assert!(
        tags.iter().all(|t| allowed.contains(&t.as_str())),
        "{tags:?}"
    );
    for never in ["paste", "set_model", "open", "browse", "paste_image"] {
        assert!(!tags.iter().any(|t| t == never));
    }
}

// ── close, detach, reload ───────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_quits_unbinds_and_announces_nothing() {
    let (mut rig, fake) = single_room(Script::default());
    attach(&mut rig, 0).await;
    rig.events();
    let closed = rig.call("room.close", json!({})).await.unwrap();
    assert_eq!(closed, json!({"closed": true, "stopped": false}));
    assert_eq!(rig.state(), BindingState::Unbound);
    assert_eq!(fake.received("quit").len(), 1);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(rig.events().iter().all(|e| e["event"] != "room.closed"));
    let again = rig.call("room.close", json!({})).await.unwrap_err();
    assert_eq!(reason(&again), Some("wrong_binding"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_with_stop_reports_a_room_that_is_still_running() {
    let (mut rig, _fake) = single_room(Script::default());
    rig.adapter.room_env.stop = Arc::new(|_| false);
    attach(&mut rig, 0).await;
    let error = rig
        .call("room.close", json!({"stop": true}))
        .await
        .unwrap_err();
    assert_eq!(reason(&error), Some("still_running"));
    assert_eq!(error.delivery, "unknown");
    assert!(!error.ends_process(false));
    assert_eq!(rig.state(), BindingState::Unbound);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn detach_in_a_room_sends_quit_and_ends() {
    let (mut rig, fake) = single_room(Script::default());
    attach(&mut rig, 0).await;
    let mut seen = std::collections::HashSet::new();
    let (frame, end) = respond(
        Request {
            id: "d".into(),
            method: "detach".into(),
            params: json!({}),
        },
        &mut seen,
        &mut rig.adapter,
    )
    .await
    .unwrap();
    assert!(end);
    assert_eq!(frame["result"], json!({"detached": true}));
    assert_eq!(fake.received("quit").len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reload_answers_with_the_version_and_warnings() {
    let (mut rig, fake) = single_room(Script::default());
    attach(&mut rig, 0).await;
    let room = rig.adapter.binding.room_mut().unwrap();
    let reloaded = room.reload().await.unwrap();
    assert!(reloaded.ok);
    assert_eq!(reloaded.version, Some(3));
    assert_eq!(reloaded.warnings, vec!["w1".to_string()]);
    assert_eq!(fake.received("sdk_reload").len(), 1);
    assert_eq!(room.roster(), ["chief", "red-web", "clerk"]);
    assert_eq!(room.session_dir(), rig.root.join("bucket").join(ROOM_ID));
    assert!(room.shelf().is_ok());
    assert!(room.registry().is_some());
}

// ── secrets ─────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn configured_secrets_never_reach_results_or_events_even_when_added_later() {
    let (mut rig, fake) = single_room(Script {
        replay: vec![chief(&format!("replayed {PLANTED_KEY}"))],
        ..Script::default()
    });
    let result = attach(&mut rig, 0).await;
    let text = result.to_string();
    assert!(!text.contains(PLANTED_KEY), "{text}");
    assert!(text.contains("[redacted]"));
    rig.events();
    // A key added by another process after the bind is covered too.
    write_config(&rig.config_dir, LATER_KEY);
    fake.push(chief(&format!(
        "lane error with {LATER_KEY} at https://bob:pw@h.example/x"
    )));
    fake.push(status_frame(0, 1));
    let mut events = Vec::new();
    until("the line", || {
        events.extend(rig.events());
        !line_events(&events).is_empty()
    })
    .await;
    let history = rig
        .call("room.history", json!({"epoch": result["epoch"]}))
        .await
        .unwrap();
    for out in [serde_json::to_string(&events).unwrap(), history.to_string()] {
        assert!(!out.contains(LATER_KEY), "{out}");
        assert!(!out.contains(PLANTED_KEY));
        assert!(!out.contains("bob:pw"));
    }
    // Text that only looks like a credential is evidence and stays.
    let mut scrubber = Scrubber::new();
    scrubber.refresh(&rig.adapter.config);
    assert_eq!(
        scrubber.text("found sk-target-abcdef123456"),
        "found sk-target-abcdef123456"
    );
    assert_eq!(scrubber.text(LATER_KEY), "[redacted]");
}

// ── open and list ───────────────────────────────────────────────────────

const PARENT: &str = "11111111-2222-3333-4444-555555555555";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_pre_checks_refuse_before_any_spawn() {
    let room_id = crate::room::room_session_id(PARENT);
    let spawned = Arc::new(AtomicBool::new(false));
    let hook = |rig: &mut Rig, spawned: &Arc<AtomicBool>| {
        let spawned = spawned.clone();
        rig.adapter.room_env.ensure_running = Arc::new(move |_, _, _, _| {
            spawned.store(true, Ordering::SeqCst);
            Ok(true)
        });
    };
    let mut rig = rig(HashMap::new(), Vec::new());
    hook(&mut rig, &spawned);
    let error = rig
        .call("room.open", json!({"parent_session_id": PARENT}))
        .await
        .unwrap_err();
    assert_eq!(reason(&error), Some("not_found"));

    let mut rig = self::rig(HashMap::new(), vec![row(PARENT, "/work")]);
    hook(&mut rig, &spawned);
    rig.make_room(PARENT);
    let error = rig
        .call("room.open", json!({"parent_session_id": PARENT}))
        .await
        .unwrap_err();
    assert_eq!(reason(&error), Some("parent_is_room"));

    let mut rig = self::rig(
        HashMap::new(),
        vec![row(PARENT, "/work"), row(&room_id, "/work")],
    );
    hook(&mut rig, &spawned);
    std::fs::write(rig.room_dir(&room_id).join("messages.json"), b"[]").unwrap();
    let error = rig
        .call("room.open", json!({"parent_session_id": PARENT}))
        .await
        .unwrap_err();
    assert_eq!(reason(&error), Some("room_id_in_use"));

    let mut rig = self::rig(HashMap::new(), vec![row(PARENT, "/work")]);
    hook(&mut rig, &spawned);
    let empty = rig.config_dir.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    rig.adapter.config = ConfigStore::at(&empty);
    let error = rig
        .call("room.open", json!({"parent_session_id": PARENT}))
        .await
        .unwrap_err();
    assert_eq!(reason(&error), Some("no_main_route"));

    let error = rig
        .call(
            "room.open",
            json!({"parent_session_id": PARENT, "tail": 201}),
        )
        .await
        .unwrap_err();
    assert_eq!(reason(&error), Some("out_of_range"));
    assert!(!spawned.load(Ordering::SeqCst));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_home_without_a_registry_has_no_rooms_and_creates_nothing() {
    let home = TempDir::new();
    for data in [home.0.join("no-data-dir"), home.0.join("empty")] {
        if data.ends_with("empty") {
            std::fs::create_dir_all(&data).unwrap();
        }
        let mut rig = rig(HashMap::new(), Vec::new());
        let spawned = Arc::new(AtomicBool::new(false));
        let hook = spawned.clone();
        rig.adapter.room_env.ensure_running = Arc::new(move |_, _, _, _| {
            hook.store(true, Ordering::SeqCst);
            Ok(true)
        });
        let registry = data.join("session.sqlite");
        rig.adapter.room_env.sessions =
            Arc::new(move || crate::model::session_registry::list_all_full_at(&registry));
        let listed = rig.call("room.list", json!({})).await.unwrap();
        assert_eq!(listed, json!({"rooms": [], "next_cursor": null}));
        let error = rig
            .call("room.open", json!({"parent_session_id": PARENT}))
            .await
            .unwrap_err();
        assert_eq!(reason(&error), Some("not_found"));
        let error = rig
            .call("room.attach", json!({"session_id": ROOM_ID, "wait_ms": 0}))
            .await
            .unwrap_err();
        assert_eq!(reason(&error), Some("not_running"));
        assert!(!spawned.load(Ordering::SeqCst));
        if data.ends_with("empty") {
            assert_eq!(std::fs::read_dir(&data).unwrap().count(), 0);
        } else {
            assert!(!data.exists());
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_room_that_does_not_start_reports_a_scrubbed_log_tail() {
    let mut rig = rig(HashMap::new(), vec![row(PARENT, "/work")]);
    let log = rig.config_dir.join("start.log");
    std::fs::write(&log, format!("boot\nprovider rejected key {PLANTED_KEY}\n")).unwrap();
    rig.adapter.room_env.start_log = Arc::new(move |_| Ok(log.clone()));
    rig.adapter.room_env.ensure_running = Arc::new(|_, _, _, wait| {
        Err(RoomStartError::NotAccepting {
            pid: 1,
            waited: wait,
        })
    });
    let error = rig
        .call(
            "room.open",
            json!({"parent_session_id": PARENT, "wait_ms": 0}),
        )
        .await
        .unwrap_err();
    assert_eq!(reason(&error), Some("room_start_failed"));
    assert_eq!(error.delivery, "unknown");
    assert!(!error.ends_process(false));
    let text = serde_json::to_string(&error).unwrap();
    assert!(!text.contains(PLANTED_KEY), "{text}");
    assert!(text.contains("provider rejected key [redacted]"));
    assert_eq!(rig.state(), BindingState::Unbound);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_spawns_then_attaches_and_list_finds_the_parent() {
    let room_id = crate::room::room_session_id(PARENT);
    let probe = TempDir::new();
    let path = sock_path(&probe.0, "open");
    let started = Arc::new(AtomicBool::new(false));
    let mut rig = rig(
        HashMap::new(),
        vec![row(PARENT, "/work"), row(&room_id, "/work")],
    );
    rig.make_room(&room_id);
    let fake = Arc::new(Mutex::new(None::<FakeRoom>));
    {
        let (path, started) = (path.clone(), started.clone());
        let dead = probe.0.join("none.sock");
        rig.adapter.room_env.sock_path = Arc::new(move |_| {
            Ok(if started.load(Ordering::SeqCst) {
                path.clone()
            } else {
                dead.clone()
            })
        });
    }
    {
        let (path, started, fake) = (path.clone(), started.clone(), fake.clone());
        let handle = tokio::runtime::Handle::current();
        rig.adapter.room_env.ensure_running = Arc::new(move |id, parent, _, _| {
            assert_eq!(parent, PARENT);
            assert_eq!(id, crate::room::room_session_id(PARENT));
            let _guard = handle.enter();
            *fake.lock().unwrap() = Some(FakeRoom::start(path.clone(), Script::default()));
            started.store(true, Ordering::SeqCst);
            Ok(true)
        });
    }
    let opened = rig
        .call("room.open", json!({"parent_session_id": PARENT}))
        .await
        .unwrap();
    assert_eq!(opened["started"], true);
    assert_eq!(opened["room"]["session_id"], room_id.as_str());
    assert_eq!(opened["room"]["parent_session_id"], PARENT);
    assert_eq!(rig.state(), BindingState::RoomLive);
    let listed = rig
        .call("room.list", json!({"parent_session_id": PARENT}))
        .await
        .unwrap();
    assert_eq!(listed["rooms"].as_array().unwrap().len(), 1);
    assert_eq!(listed["rooms"][0]["bound"], true);
    assert_eq!(listed["rooms"][0]["live"], true);
    assert!(listed["next_cursor"].is_null());
    let page = rig
        .call("room.list", json!({"limit": 1, "cursor": "%%"}))
        .await
        .unwrap_err();
    assert_eq!(reason(&page), Some("bad_cursor"));
    // Opening again while bound and live is refused.
    let again = rig
        .call("room.open", json!({"parent_session_id": PARENT}))
        .await
        .unwrap_err();
    assert_eq!(reason(&again), Some("already_bound"));
    drop(fake.lock().unwrap().take());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_socket_that_is_not_a_room_is_refused() {
    let dir = TempDir::new();
    let path = sock_path(&dir.0, "coding");
    let listener = crate::ipc::server::bind(&path).unwrap();
    // Closes every connection after its first frame, like a coding daemon
    // that does not understand `hello`.
    let accept = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let (mut read, _write) = split_stream(stream);
            let _ = read_frame_from(&mut read, &mut FrameReader::new()).await;
        }
    });
    let mut rig = rig(
        HashMap::from([(ROOM_ID.to_string(), path)]),
        vec![row(ROOM_ID, "/work")],
    );
    rig.make_room(ROOM_ID);
    let error = rig
        .call("room.attach", json!({"session_id": ROOM_ID}))
        .await
        .unwrap_err();
    assert_eq!(reason(&error), Some("not_a_room"));
    assert_eq!(rig.state(), BindingState::Unbound);
    accept.abort();
}

// ── the queue, the ring and the scrubber on their own ───────────────────

fn line_body(epoch: &str, seq: u64) -> Value {
    json!({"epoch": epoch, "line": {"line_seq": seq}})
}

#[test]
fn the_queue_merges_deltas_per_author_lane_and_kind() {
    let queue = EventQueue::new();
    queue.activate("e", 0);
    crate::app::runtime::sdk::events::release_after_response(&queue);
    queue.push_delta("e", "a", "chat", "text", "He");
    queue.push_delta("e", "b", "chat", "text", "x");
    queue.push_delta("e", "a", "chat", "text", "llo");
    queue.push_delta("e", "a", "chat", "reasoning", "hm");
    queue.push_line("e", 1, line_body("e", 1));
    queue.push_delta("e", "a", "chat", "text", "!");
    let mut seq = 0;
    let frames = queue.take_ready(&mut seq);
    let texts: Vec<(&str, &str)> = frames
        .iter()
        .map(|f| {
            (
                f["event"].as_str().unwrap(),
                f["text"].as_str().unwrap_or(""),
            )
        })
        .collect();
    assert_eq!(
        texts,
        [
            ("room.delta", "Hello"),
            ("room.delta", "x"),
            ("room.delta", "hm"),
            ("room.line", ""),
            ("room.delta", "!"),
        ]
    );
    let seqs: Vec<u64> = frames.iter().map(|f| f["seq"].as_u64().unwrap()).collect();
    assert_eq!(seqs, [1, 2, 3, 4, 5]);
}

#[test]
fn an_overflowing_queue_drops_lines_and_deltas_for_one_gap() {
    let queue = EventQueue::new();
    queue.activate("e", 3);
    crate::app::runtime::sdk::events::release_after_response(&queue);
    queue.push(
        "e",
        "room.notice",
        json!({"epoch": "e", "code": "room", "text": "kept"}),
    );
    queue.push_delta("e", "a", "chat", "text", "d");
    for n in 0..(QUEUE_ITEMS as u64 + 10) {
        queue.push_line("e", 4 + n, line_body("e", 4 + n));
    }
    let mut seq = 0;
    let frames = queue.take_ready(&mut seq);
    let gaps: Vec<&Value> = frames.iter().filter(|f| f["event"] == "room.gap").collect();
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0]["after_line_seq"], 3);
    assert_eq!(gaps[0]["dropped_deltas"], 1);
    assert_eq!(gaps[0]["epoch"], "e");
    let dropped = gaps[0]["dropped_lines"].as_u64().unwrap();
    let delivered = frames.iter().filter(|f| f["event"] == "room.line").count() as u64;
    assert_eq!(dropped + delivered, QUEUE_ITEMS as u64 + 10);
    assert!(
        frames.iter().any(|f| f["text"] == "kept"),
        "notices are never dropped"
    );
    assert!(frames.len() <= QUEUE_ITEMS + 1);
}

#[test]
fn the_queue_holds_until_a_response_and_accepts_one_epoch() {
    let queue = EventQueue::new();
    queue.activate("old", 0);
    crate::app::runtime::sdk::events::release_after_response(&queue);
    queue.push_line("old", 1, line_body("old", 1));
    queue.activate("new", 5);
    queue.push_line("old", 2, line_body("old", 2));
    queue.push_line("new", 6, line_body("new", 6));
    queue.set_status("new", json!({"epoch": "new", "status": {"n": 1}}));
    queue.set_status("new", json!({"epoch": "new", "status": {"n": 2}}));
    let mut seq = 0;
    assert!(queue.take_ready(&mut seq).is_empty(), "held");
    crate::app::runtime::sdk::events::release_after_response(&queue);
    let frames = queue.take_ready(&mut seq);
    assert_eq!(frames.len(), 2);
    assert!(frames.iter().all(|f| f["epoch"] == "new"));
    assert_eq!(frames[1]["event"], "room.status");
    assert_eq!(frames[1]["status"]["n"], 2, "latest wins");
    queue.deactivate();
    queue.push_line("new", 7, line_body("new", 7));
    assert!(queue.take_ready(&mut seq).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_earlier_response_still_being_written_does_not_release_an_attach_hold() {
    use tokio::io::AsyncBufReadExt;
    let queue = std::sync::Arc::new(EventQueue::new());
    let (out, read) = tokio::io::duplex(256);
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    let (_states_tx, states) = tokio::sync::watch::channel(None);
    let writer = tokio::spawn(writer(out, rx, states, std::sync::Arc::clone(&queue)));
    // A large earlier response is still being written while the attach is
    // answered: the reader is slow and the pipe holds only 256 bytes.
    let big = json!({"type": "response", "id": "history", "result": "x".repeat(64 * 1024)});
    tx.send((big, None)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    queue.activate("e", 0);
    queue.push_line("e", 1, line_body("e", 1));
    let hold = queue.claim_hold();
    assert!(hold.is_some());
    let attach = json!({"type": "response", "id": "attach", "result": {}});
    tx.send((attach, hold)).await.unwrap();
    let mut lines = tokio::io::BufReader::new(read).lines();
    let mut order = Vec::new();
    while order.len() < 3 {
        let line = lines.next_line().await.unwrap().unwrap();
        let frame: Value = serde_json::from_str(&line).unwrap();
        order.push(frame["id"].as_str().map_or_else(
            || frame["event"].as_str().unwrap().to_string(),
            str::to_string,
        ));
    }
    assert_eq!(order, ["history", "attach", "room.line"]);
    drop(tx);
    writer.await.unwrap().unwrap();
}

fn text_line(text: &str) -> RoomLine {
    match ring::classify(chief(text), None) {
        ring::Wire::Line(line) => *line,
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_ring_pages_by_exclusive_line_seq_bounds() {
    let mut ring = Ring::new();
    for n in 1..=10 {
        ring.push(text_line(&format!("l{n}")), n <= 3, n);
    }
    let all = Filter::default();
    let newest = ring.page(None, None, 3, &all);
    let seqs = |page: &ring::Page| page.lines.iter().map(|l| l.line_seq).collect::<Vec<_>>();
    assert_eq!(seqs(&newest), [8, 9, 10]);
    assert!(newest.has_more_before && !newest.has_more_after);
    let after = ring.page(Some(8), None, 5, &all);
    assert_eq!(seqs(&after), [9, 10]);
    assert!(after.has_more_before && !after.has_more_after);
    let before = ring.page(None, Some(4), 2, &all);
    assert_eq!(seqs(&before), [2, 3]);
    assert!(before.has_more_before && before.has_more_after);
    assert!(!ring.page(None, None, 1, &all).lines[0].replay);
}

#[test]
fn the_ring_evicts_the_oldest_and_long_text_is_cut() {
    let mut ring = Ring::new();
    for n in 0..(ring::RING_LINES + 5) {
        ring.push(text_line("x"), false, n as u64);
    }
    assert_eq!(ring.len(), ring::RING_LINES);
    assert_eq!(ring.first_line_seq(), Some(6));
    let long = text_line(&"é".repeat(ring::LINE_TEXT_BYTES));
    assert!(long.truncated);
    assert!(long.text.unwrap().len() <= ring::LINE_TEXT_BYTES);
}

#[test]
fn the_scrubber_rereads_the_value_set_when_the_config_changes() {
    let dir = TempDir::new();
    write_config(&dir.0, PLANTED_KEY);
    let store = ConfigStore::at(&dir.0);
    let mut scrubber = Scrubber::new();
    scrubber.refresh(&store);
    assert_eq!(scrubber.text(PLANTED_KEY), "[redacted]");
    write_config(&dir.0, LATER_KEY);
    scrubber.refresh(&store);
    assert_eq!(scrubber.text(LATER_KEY), "[redacted]");
    let mut value = json!({"a": [format!("x {LATER_KEY}")], LATER_KEY: 1});
    scrubber.value(&mut value);
    assert_eq!(value, json!({"a": ["x [redacted]"], "[redacted]": 1}));
}

#[test]
fn room_routes_and_hello_lists() {
    let gate = |method: &str, state: BindingState| methods::find(method).unwrap().check(state);
    for method in ["room.open", "room.attach"] {
        assert!(gate(method, BindingState::Unbound).is_ok());
        assert!(gate(method, BindingState::RoomClosed).is_ok());
        let e3 = gate(method, BindingState::RoomLive).unwrap_err();
        assert_eq!(e3.message, "session already bound");
        assert_eq!(
            reason(&gate(method, BindingState::Coding).unwrap_err()),
            Some("already_bound")
        );
    }
    for method in ["room.send", "room.interrupt", "room.control"] {
        assert!(gate(method, BindingState::RoomLive).is_ok());
        assert_eq!(
            reason(&gate(method, BindingState::RoomClosed).unwrap_err()),
            Some("room_closed")
        );
        assert_eq!(
            reason(&gate(method, BindingState::Unbound).unwrap_err()),
            Some("wrong_binding")
        );
    }
    for method in ["room.close", "room.status", "room.history"] {
        assert!(gate(method, BindingState::RoomLive).is_ok());
        assert!(gate(method, BindingState::RoomClosed).is_ok());
        assert!(gate(method, BindingState::Coding).is_err());
    }
    for state in BindingState::ALL {
        assert!(gate("room.list", state).is_ok());
    }
    let hello = protocol::hello("/k");
    for event in [
        "room.line",
        "room.delta",
        "room.status",
        "room.notice",
        "room.closed",
        "room.gap",
    ] {
        assert!(hello["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e == event));
    }
    for feature in ["bindings.room", "events.room"] {
        assert!(hello["features"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == feature));
    }
    assert_eq!(hello["limits"]["room"]["ring_lines"], 8000);
    assert_eq!(hello["limits"]["list"]["rooms"], 500);
    assert_eq!(hello["limits"]["time"]["room_start_ms"], 30000);
    assert_eq!(hello["capabilities"].as_array().unwrap().len(), 10);
}
