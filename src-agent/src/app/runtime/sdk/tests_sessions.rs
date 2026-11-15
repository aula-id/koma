//! Tests for the `sessions.*` methods and `create{name}`. A fake host plays
//! the registry, the run dir and the daemons; session dirs live in a
//! directory of each test's own. A bound session is played by a peer on a
//! socket pair.
use super::host::sessions::{self, SessionHost};
use super::protocol::{Request, RpcError};
use super::*;
use crate::ipc::proto::SessionStatus;
use crate::model::session_registry::RegRowFullCreated;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const HASH: &str = "0123456789abcdef0123456789abcdef";
const CODING_A: &str = "11111111-1111-4111-8111-111111111111";
const CODING_B: &str = "22222222-2222-4222-8222-222222222222";
const CODING_C: &str = "33333333-3333-4333-8333-333333333333";
/// A PID that no process holds.
const DEAD_PID: u32 = 4_000_000_000;

/// A fresh directory under the system temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("koma-ses-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The registry, run dir and daemons of one test.
struct Fake {
    root: TempDir,
    rows: Mutex<Vec<RegRowFullCreated>>,
    live: Mutex<Vec<SessionStatus>>,
    /// Ids whose socket accepts.
    alive: Mutex<HashSet<String>>,
    /// From this `daemon_alive` call on, every id reads as alive.
    alive_from_call: Option<usize>,
    alive_calls: AtomicUsize,
    /// Whether `stop` kills the daemon.
    stop_kills: bool,
    stop_delay: Duration,
    /// `delete` drops the row but fails before removing the dir.
    fail_delete: bool,
    set_names: Mutex<Vec<(String, String)>>,
    deletes: Mutex<Vec<PathBuf>>,
    stops: Mutex<Vec<String>>,
}

impl Fake {
    fn new() -> Self {
        Self {
            root: TempDir::new(),
            rows: Mutex::new(Vec::new()),
            live: Mutex::new(Vec::new()),
            alive: Mutex::new(HashSet::new()),
            alive_from_call: None,
            alive_calls: AtomicUsize::new(0),
            stop_kills: true,
            stop_delay: Duration::ZERO,
            fail_delete: false,
            set_names: Mutex::new(Vec::new()),
            deletes: Mutex::new(Vec::new()),
            stops: Mutex::new(Vec::new()),
        }
    }

    fn dir(&self, id: &str) -> PathBuf {
        self.root.0.join("sessions").join(HASH).join(id)
    }

    /// A registered session with a directory of its own.
    fn add(&self, id: &str, updated_at: i64) -> PathBuf {
        let dir = self.dir(id);
        std::fs::create_dir_all(&dir).unwrap();
        self.rows.lock().unwrap().push(RegRowFullCreated {
            uuid: id.into(),
            pwd_hash: HASH.into(),
            name: format!("name-{id}"),
            workdir: "/work".into(),
            created_at: updated_at - 10,
            updated_at,
        });
        dir
    }

    fn add_room(&self, parent: &str, updated_at: i64) -> String {
        let id = crate::room::room_session_id(parent);
        let dir = self.add(&id, updated_at);
        std::fs::write(crate::room::room_db_path(&dir), b"").unwrap();
        id
    }

    fn set_live(&self, id: &str, working: bool) {
        self.live.lock().unwrap().push(SessionStatus {
            session_id: id.into(),
            name: "probe-name".into(),
            pwd: String::new(),
            working,
        });
        self.alive.lock().unwrap().insert(id.into());
    }

    fn lock(&self, id: &str, pid: u32) -> PathBuf {
        let path = self.dir(id).join("session.lock");
        std::fs::write(&path, pid.to_string()).unwrap();
        path
    }
}

impl SessionHost for Fake {
    fn rows(&self) -> anyhow::Result<Vec<RegRowFullCreated>> {
        Ok(self.rows.lock().unwrap().clone())
    }
    fn probe(&self) -> Vec<SessionStatus> {
        self.live.lock().unwrap().clone()
    }
    fn daemon_alive(&self, id: &str) -> bool {
        let call = self.alive_calls.fetch_add(1, Ordering::SeqCst);
        self.alive_from_call.is_some_and(|from| call >= from)
            || self.alive.lock().unwrap().contains(id)
    }
    fn session_dir(&self, pwd_hash: &str, id: &str) -> anyhow::Result<PathBuf> {
        Ok(self.root.0.join("sessions").join(pwd_hash).join(id))
    }
    fn pwd_hash(&self, _workdir: &str) -> String {
        HASH.into()
    }
    fn set_name(&self, id: &str, name: &str) -> anyhow::Result<()> {
        self.set_names
            .lock()
            .unwrap()
            .push((id.into(), name.into()));
        for row in self.rows.lock().unwrap().iter_mut() {
            if row.uuid == id {
                row.name = name.into();
            }
        }
        Ok(())
    }
    fn delete(&self, dir: &Path) -> anyhow::Result<()> {
        self.deletes.lock().unwrap().push(dir.to_path_buf());
        let id = dir.file_name().unwrap().to_str().unwrap().to_string();
        self.rows.lock().unwrap().retain(|row| row.uuid != id);
        if self.fail_delete {
            anyhow::bail!("disk went away");
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
    fn stop(&self, id: &str) -> bool {
        self.stops.lock().unwrap().push(id.into());
        std::thread::sleep(self.stop_delay);
        if self.stop_kills {
            self.alive.lock().unwrap().remove(id);
            self.live.lock().unwrap().retain(|s| s.session_id != id);
        }
        self.stop_kills
    }
}

fn unbound() -> Adapter {
    let (events, _) = watch::channel(None);
    Adapter::new(events)
}

fn request(method: &str, params: Value) -> Request {
    Request {
        id: method.into(),
        method: method.into(),
        params,
    }
}

async fn list(fake: &Arc<Fake>, adapter: &mut Adapter, params: Value) -> Result<Value, RpcError> {
    sessions::list(fake.clone(), request("sessions.list", params), adapter).await
}

async fn rename(fake: &Arc<Fake>, adapter: &mut Adapter, params: Value) -> Result<Value, RpcError> {
    sessions::rename(fake.clone(), request("sessions.rename", params), adapter).await
}

async fn delete(fake: &Arc<Fake>, adapter: &mut Adapter, id: &str) -> Result<Value, RpcError> {
    let params = json!({ "session_id": id });
    sessions::delete(fake.clone(), request("sessions.delete", params), adapter).await
}

async fn stop(fake: &Arc<Fake>, adapter: &mut Adapter, id: &str) -> Result<Value, RpcError> {
    let params = json!({ "session_id": id });
    sessions::stop(fake.clone(), request("sessions.stop", params), adapter).await
}

fn reason(error: &RpcError) -> Option<&'static str> {
    error.extra().and_then(|extra| extra.reason)
}

fn ids(result: &Value) -> Vec<String> {
    result["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["session_id"].as_str().unwrap().to_string())
        .collect()
}

fn row<'a>(result: &'a Value, id: &str) -> &'a Value {
    result["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["session_id"] == id)
        .unwrap()
}

// ── sessions.list ──────────────────────────────────────────────────────────

#[tokio::test]
async fn list_reports_kind_liveness_locks_and_room_linkage() {
    let fake = Arc::new(Fake::new());
    fake.add(CODING_A, 100);
    fake.add(CODING_B, 300);
    let room = fake.add_room(CODING_A, 200);
    fake.set_live(CODING_A, true);
    fake.set_live(&room, false);
    fake.lock(CODING_A, std::process::id());
    // A live socket nobody registered.
    let stray = "44444444-4444-4444-8444-444444444444";
    fake.set_live(stray, false);
    let mut adapter = unbound();
    let result = list(&fake, &mut adapter, json!({})).await.unwrap();
    assert_eq!(
        ids(&result),
        vec![
            stray.to_string(),
            CODING_B.into(),
            room.clone(),
            CODING_A.into()
        ]
    );
    assert!(result["next_cursor"].is_null());

    let a = row(&result, CODING_A);
    assert_eq!(a["kind"], "coding");
    assert_eq!(a["name"], format!("name-{CODING_A}"));
    assert_eq!(a["workdir"], "/work");
    assert_eq!(
        (a["created_at"].as_i64(), a["updated_at"].as_i64()),
        (Some(90), Some(100))
    );
    assert_eq!(
        (a["live"].clone(), a["working"].clone()),
        (json!(true), json!(true))
    );
    assert_eq!(a["locked"], true);
    assert_eq!(a["bound"], false);
    assert_eq!(a["room_session_id"], room);
    assert_eq!(a["room_live"], true);
    assert!(a["parent_session_id"].is_null());

    let r = row(&result, &room);
    assert_eq!(r["kind"], "room");
    assert!(r["working"].is_null());
    assert_eq!(r["parent_session_id"], CODING_A);
    assert!(r["room_session_id"].is_null());
    assert_eq!(r["name"], format!("name-{room}"));

    let b = row(&result, CODING_B);
    assert_eq!(
        (b["live"].clone(), b["working"].clone()),
        (json!(false), json!(false))
    );
    assert!(b["room_session_id"].is_null());
    assert_eq!(b["locked"], false);

    let s = row(&result, stray);
    assert!(s["name"].is_null() && s["workdir"].is_null() && s["updated_at"].is_null());
    assert_eq!(s["live"], true);
    assert_eq!(s.as_object().unwrap().len(), 13);
}

#[tokio::test]
async fn list_filters_by_kind_and_liveness() {
    let fake = Arc::new(Fake::new());
    fake.add(CODING_A, 100);
    fake.add(CODING_B, 200);
    let room = fake.add_room(CODING_A, 300);
    fake.set_live(CODING_B, false);
    let mut adapter = unbound();
    let rooms = list(&fake, &mut adapter, json!({"kind":"room"}))
        .await
        .unwrap();
    assert_eq!(ids(&rooms), vec![room.clone()]);
    let coding = list(&fake, &mut adapter, json!({"kind":"coding"}))
        .await
        .unwrap();
    assert_eq!(ids(&coding), vec![CODING_B.to_string(), CODING_A.into()]);
    let live = list(&fake, &mut adapter, json!({"live":true}))
        .await
        .unwrap();
    assert_eq!(ids(&live), vec![CODING_B.to_string()]);
    let idle = list(&fake, &mut adapter, json!({"live":false, "kind":"all"}))
        .await
        .unwrap();
    assert_eq!(ids(&idle), vec![room, CODING_A.into()]);
}

#[tokio::test]
async fn list_removes_no_lock_file_and_reads_a_stale_lock_as_unlocked() {
    let fake = Arc::new(Fake::new());
    fake.add(CODING_A, 100);
    let stale = fake.lock(CODING_A, DEAD_PID);
    let mut adapter = unbound();
    let result = list(&fake, &mut adapter, json!({})).await.unwrap();
    assert_eq!(row(&result, CODING_A)["locked"], false);
    assert!(stale.exists());
    assert!(fake.deletes.lock().unwrap().is_empty());
    assert!(fake.stops.lock().unwrap().is_empty());
}

#[tokio::test]
async fn list_params_out_of_bounds_are_refused() {
    let fake = Arc::new(Fake::new());
    let mut adapter = unbound();
    for params in [
        json!({"limit":0}),
        json!({"limit":501}),
        json!({"limit":"ten"}),
        json!({"kind":"chat"}),
        json!({"live":"yes"}),
    ] {
        let error = list(&fake, &mut adapter, params.clone()).await.unwrap_err();
        assert_eq!(
            (error.code, reason(&error)),
            ("invalid_request", Some("out_of_range")),
            "{params}"
        );
        assert_eq!(error.delivery, "not_sent");
    }
    for cursor in [
        json!("not a cursor!"),
        json!(sessions::encode_cursor(1, "../etc")),
        json!(7),
    ] {
        let error = list(&fake, &mut adapter, json!({"cursor":cursor}))
            .await
            .unwrap_err();
        assert_eq!(
            (error.code, reason(&error)),
            ("invalid_request", Some("bad_cursor"))
        );
    }
    let full = list(&fake, &mut adapter, json!({"limit":500}))
        .await
        .unwrap();
    assert_eq!(full["sessions"], json!([]));
}

#[tokio::test]
async fn cursor_pages_stay_stable_under_inserts() {
    let fake = Arc::new(Fake::new());
    let mut original = Vec::new();
    for n in 0..7 {
        let id = format!("{n:08x}-0000-4000-8000-000000000000");
        // Two rows share each timestamp, so the id breaks ties.
        fake.add(&id, 1000 - (n / 2) as i64);
        original.push(id);
    }
    let mut adapter = unbound();
    let mut seen = Vec::new();
    let mut cursor = Value::Null;
    let mut pages = 0;
    loop {
        let result = list(&fake, &mut adapter, json!({"limit":2, "cursor":cursor}))
            .await
            .unwrap();
        seen.extend(ids(&result));
        pages += 1;
        // A newer session appears between pages.
        fake.add(
            &format!("ffffff{pages:02x}-0000-4000-8000-000000000000"),
            5000,
        );
        if result["next_cursor"].is_null() {
            break;
        }
        cursor = result["next_cursor"].clone();
    }
    assert_eq!(pages, 4);
    assert_eq!(seen, original);
}

#[cfg(unix)]
#[tokio::test]
async fn list_marks_the_bound_session() {
    let fake = Arc::new(Fake::new());
    fake.add(CODING_A, 100);
    fake.add(CODING_B, 200);
    let (mut adapter, peer) = bound::adapter(CODING_A, bound::quiet());
    let result = list(&fake, &mut adapter, json!({})).await.unwrap();
    assert_eq!(row(&result, CODING_A)["bound"], true);
    assert_eq!(row(&result, CODING_B)["bound"], false);
    assert!(bound::requests(adapter, peer).await.is_empty());
}

// ── sessions.rename ────────────────────────────────────────────────────────

#[tokio::test]
async fn rename_validates_the_name_before_anything_else() {
    let fake = Arc::new(Fake::new());
    fake.add(CODING_A, 100);
    let mut adapter = unbound();
    let long = "x".repeat(sessions::MAX_NAME_CHARS + 1);
    for name in [
        json!(""),
        json!("   "),
        json!("a\u{1}b"),
        json!("tab\tname"),
        json!(long),
        json!(3),
    ] {
        let error = rename(
            &fake,
            &mut adapter,
            json!({"session_id":CODING_A, "name":name}),
        )
        .await
        .unwrap_err();
        assert_eq!(
            (error.code, reason(&error)),
            ("invalid_request", Some("invalid_name")),
            "{name}"
        );
    }
    let error = rename(&fake, &mut adapter, json!({"session_id":CODING_A}))
        .await
        .unwrap_err();
    assert_eq!(reason(&error), Some("invalid_name"));
    assert!(fake.set_names.lock().unwrap().is_empty());
    let max = "é".repeat(sessions::MAX_NAME_CHARS);
    let result = rename(
        &fake,
        &mut adapter,
        json!({"session_id":CODING_A, "name":format!(" {max} ")}),
    )
    .await
    .unwrap();
    assert_eq!(result["session"]["name"], max);
}

#[tokio::test]
async fn rename_of_an_unknown_or_malformed_id_is_not_found() {
    let fake = Arc::new(Fake::new());
    let mut adapter = unbound();
    for id in [CODING_A, "../../etc", "not-a-uuid"] {
        let error = rename(&fake, &mut adapter, json!({"session_id":id, "name":"x"}))
            .await
            .unwrap_err();
        assert_eq!(
            (error.code, reason(&error)),
            ("not_found", Some("not_found")),
            "{id}"
        );
    }
    assert!(fake.set_names.lock().unwrap().is_empty());
}

#[tokio::test]
async fn rename_of_a_session_that_is_not_live_writes_the_registry() {
    let fake = Arc::new(Fake::new());
    fake.add(CODING_A, 100);
    let mut adapter = unbound();
    let result = rename(
        &fake,
        &mut adapter,
        json!({"session_id":CODING_A, "name":"  fresh  "}),
    )
    .await
    .unwrap();
    assert_eq!(result["session"]["session_id"], CODING_A);
    assert_eq!(result["session"]["name"], "fresh");
    assert_eq!(
        *fake.set_names.lock().unwrap(),
        vec![(CODING_A.to_string(), "fresh".to_string())]
    );
}

#[tokio::test]
async fn rename_of_a_live_room_writes_the_registry() {
    let fake = Arc::new(Fake::new());
    fake.add(CODING_A, 100);
    let room = fake.add_room(CODING_A, 200);
    fake.set_live(&room, false);
    let mut adapter = unbound();
    let result = rename(
        &fake,
        &mut adapter,
        json!({"session_id":room, "name":"war room"}),
    )
    .await
    .unwrap();
    assert_eq!(result["session"]["name"], "war room");
    assert_eq!(result["session"]["kind"], "room");
    assert_eq!(result["session"]["live"], true);
}

#[tokio::test]
async fn rename_of_another_live_coding_session_is_refused() {
    let fake = Arc::new(Fake::new());
    fake.add(CODING_A, 100);
    fake.set_live(CODING_A, false);
    let mut adapter = unbound();
    let error = rename(
        &fake,
        &mut adapter,
        json!({"session_id":CODING_A, "name":"x"}),
    )
    .await
    .unwrap_err();
    assert_eq!(
        (error.code, reason(&error)),
        ("invalid_state", Some("live"))
    );
    assert_eq!(error.delivery, "not_sent");
    assert!(error
        .message
        .contains("rename it from an adapter bound to it, or stop it first"));
    assert!(fake.set_names.lock().unwrap().is_empty());
    assert_eq!(
        fake.rows().unwrap().first().map(|r| r.name.clone()),
        Some(format!("name-{CODING_A}"))
    );
    // A socket that accepts but missed the probe is live too.
    fake.live.lock().unwrap().clear();
    let error = rename(
        &fake,
        &mut adapter,
        json!({"session_id":CODING_A, "name":"x"}),
    )
    .await
    .unwrap_err();
    assert_eq!(reason(&error), Some("live"));
}

#[cfg(unix)]
#[tokio::test]
async fn rename_of_another_live_session_sends_nothing_on_the_bound_connection() {
    let fake = Arc::new(Fake::new());
    fake.add(CODING_A, 100);
    fake.add(CODING_B, 200);
    fake.set_live(CODING_B, false);
    let (mut adapter, peer) = bound::adapter(CODING_A, bound::quiet());
    let error = rename(
        &fake,
        &mut adapter,
        json!({"session_id":CODING_B, "name":"x"}),
    )
    .await
    .unwrap_err();
    assert_eq!(reason(&error), Some("live"));
    assert!(bound::requests(adapter, peer).await.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn rename_of_the_bound_session_goes_through_its_daemon() {
    let fake = Arc::new(Fake::new());
    fake.add(CODING_A, 100);
    let (mut adapter, peer) = bound::adapter(CODING_A, bound::renaming(None));
    let result = rename(
        &fake,
        &mut adapter,
        json!({"session_id":CODING_A, "name":" smoke2 "}),
    )
    .await
    .unwrap();
    assert_eq!(result["session"]["bound"], true);
    assert!(fake.set_names.lock().unwrap().is_empty());
    let info = dispatch(request("sessions.info", json!({})), &mut adapter)
        .await
        .unwrap();
    assert_eq!(info["session_info"]["name"], "smoke2");
    let seen = bound::requests(adapter, peer).await;
    assert_eq!(
        seen.first(),
        Some(&crate::ipc::proto::ClientRequest::RenameSession {
            name: "smoke2".into()
        })
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_daemon_refusal_of_the_bound_rename_is_not_sent() {
    let fake = Arc::new(Fake::new());
    fake.add(CODING_A, 100);
    let (mut adapter, peer) = bound::adapter(CODING_A, bound::renaming(Some("disk full")));
    let error = rename(
        &fake,
        &mut adapter,
        json!({"session_id":CODING_A, "name":"x"}),
    )
    .await
    .unwrap_err();
    assert_eq!(
        (error.code, reason(&error)),
        ("invalid_state", Some("daemon_refused"))
    );
    assert_eq!(error.delivery, "not_sent");
    assert!(fake.set_names.lock().unwrap().is_empty());
    bound::requests(adapter, peer).await;
}

// ── sessions.delete ────────────────────────────────────────────────────────

#[tokio::test]
async fn delete_removes_the_session_and_names_an_orphaned_room() {
    let fake = Arc::new(Fake::new());
    let dir = fake.add(CODING_A, 100);
    let room = fake.add_room(CODING_A, 200);
    let mut adapter = unbound();
    let result = delete(&fake, &mut adapter, CODING_A).await.unwrap();
    assert_eq!(
        result,
        json!({"deleted":true, "kind":"coding", "orphaned_room_session_id":room})
    );
    assert!(!dir.exists());
    assert!(fake.dir(&room).exists());
    let after = list(&fake, &mut adapter, json!({})).await.unwrap();
    assert_eq!(ids(&after), vec![room.clone()]);
    assert!(row(&after, &room)["parent_session_id"].is_null());
    // A retry finds nothing.
    let error = delete(&fake, &mut adapter, CODING_A).await.unwrap_err();
    assert_eq!(
        (error.code, reason(&error)),
        ("not_found", Some("not_found"))
    );
    let room_result = delete(&fake, &mut adapter, &room).await.unwrap();
    assert_eq!(
        room_result,
        json!({"deleted":true, "kind":"room", "orphaned_room_session_id":null})
    );
}

#[tokio::test]
async fn delete_refuses_a_live_session() {
    let fake = Arc::new(Fake::new());
    let dir = fake.add(CODING_A, 100);
    fake.set_live(CODING_A, false);
    let mut adapter = unbound();
    let error = delete(&fake, &mut adapter, CODING_A).await.unwrap_err();
    assert_eq!(
        (error.code, reason(&error)),
        ("invalid_state", Some("live"))
    );
    assert!(dir.exists());
    assert!(fake.deletes.lock().unwrap().is_empty());
}

#[tokio::test]
async fn delete_checks_liveness_before_the_lock() {
    let fake = Arc::new(Fake::new());
    fake.add(CODING_A, 100);
    fake.set_live(CODING_A, false);
    fake.lock(CODING_A, std::process::id());
    let mut adapter = unbound();
    let error = delete(&fake, &mut adapter, CODING_A).await.unwrap_err();
    assert_eq!(reason(&error), Some("live"));
}

#[tokio::test]
async fn delete_refuses_a_locked_session_and_leaves_its_lock() {
    let fake = Arc::new(Fake::new());
    let dir = fake.add(CODING_A, 100);
    let lock = fake.lock(CODING_A, std::process::id());
    let mut adapter = unbound();
    let error = delete(&fake, &mut adapter, CODING_A).await.unwrap_err();
    assert_eq!(
        (error.code, reason(&error)),
        ("invalid_state", Some("session_locked"))
    );
    assert!(dir.exists() && lock.exists());
    // A stale lock does not block, and goes with the directory.
    std::fs::write(&lock, DEAD_PID.to_string()).unwrap();
    delete(&fake, &mut adapter, CODING_A).await.unwrap();
    assert!(!dir.exists());
}

#[tokio::test]
async fn delete_re_probes_liveness_right_before_removing() {
    let mut fake = Fake::new();
    // The first probe finds nothing; the daemon is up by the second.
    fake.alive_from_call = Some(1);
    let fake = Arc::new(fake);
    let dir = fake.add(CODING_A, 100);
    let mut adapter = unbound();
    let error = delete(&fake, &mut adapter, CODING_A).await.unwrap_err();
    assert_eq!(reason(&error), Some("live"));
    assert_eq!(fake.alive_calls.load(Ordering::SeqCst), 2);
    assert!(dir.exists());
    assert!(fake.deletes.lock().unwrap().is_empty());
}

#[tokio::test]
async fn delete_failing_midway_is_an_honest_partial_delete() {
    let mut fake = Fake::new();
    fake.fail_delete = true;
    let fake = Arc::new(fake);
    fake.add(CODING_A, 100);
    let mut adapter = unbound();
    let error = delete(&fake, &mut adapter, CODING_A).await.unwrap_err();
    assert_eq!(
        (error.code, reason(&error)),
        ("unavailable", Some("partial_delete"))
    );
    assert_eq!(error.delivery, "unknown");
    assert_eq!(error.extra().and_then(|e| e.written), Some(true));
    // In-process work: the process stays.
    assert!(!error.ends_process(false));
}

#[tokio::test]
async fn delete_resolves_its_target_from_the_registry_only() {
    let fake = Arc::new(Fake::new());
    let mut adapter = unbound();
    // An id that would escape the sessions dir never reaches the host.
    for id in ["..", "../../x", "/tmp", CODING_A] {
        let error = delete(&fake, &mut adapter, id).await.unwrap_err();
        assert_eq!(error.code, "not_found", "{id}");
    }
    // A registry row whose bucket is not a hash names no directory.
    fake.rows.lock().unwrap().push(RegRowFullCreated {
        uuid: CODING_B.into(),
        pwd_hash: "../..".into(),
        name: "x".into(),
        workdir: "/work".into(),
        created_at: 1,
        updated_at: 1,
    });
    let error = delete(&fake, &mut adapter, CODING_B).await.unwrap_err();
    assert_eq!(
        (error.code, reason(&error)),
        ("unavailable", Some("io_error"))
    );
    assert!(fake.deletes.lock().unwrap().is_empty());
    let error = delete(&fake, &mut adapter, CODING_C).await.unwrap_err();
    assert_eq!(error.code, "not_found");
}

#[cfg(unix)]
#[tokio::test]
async fn delete_and_stop_refuse_the_own_binding_first() {
    let fake = Arc::new(Fake::new());
    fake.add(CODING_A, 100);
    fake.set_live(CODING_A, false);
    let (mut adapter, peer) = bound::adapter(CODING_A, bound::quiet());
    let error = delete(&fake, &mut adapter, CODING_A).await.unwrap_err();
    assert_eq!(
        (error.code, reason(&error)),
        ("invalid_state", Some("bound_self"))
    );
    let error = stop(&fake, &mut adapter, CODING_A).await.unwrap_err();
    assert_eq!(
        (error.code, reason(&error)),
        ("invalid_state", Some("bound_self"))
    );
    assert_eq!(fake.alive_calls.load(Ordering::SeqCst), 0);
    assert!(fake.stops.lock().unwrap().is_empty());
    assert!(fake.deletes.lock().unwrap().is_empty());
    assert!(bound::requests(adapter, peer).await.is_empty());
}

// ── sessions.stop ──────────────────────────────────────────────────────────

#[tokio::test]
async fn stop_of_a_session_that_is_not_live_changes_nothing() {
    let fake = Arc::new(Fake::new());
    fake.add(CODING_A, 100);
    let mut adapter = unbound();
    let result = stop(&fake, &mut adapter, CODING_A).await.unwrap();
    assert_eq!(result, json!({"stopped":true, "was_live":false}));
    assert!(fake.stops.lock().unwrap().is_empty());
    let error = stop(&fake, &mut adapter, "../x").await.unwrap_err();
    assert_eq!(error.code, "not_found");
}

#[tokio::test]
async fn stop_of_a_live_session_waits_until_it_is_gone() {
    let fake = Arc::new(Fake::new());
    fake.add(CODING_A, 100);
    fake.set_live(CODING_A, true);
    let mut adapter = unbound();
    let result = stop(&fake, &mut adapter, CODING_A).await.unwrap();
    assert_eq!(result, json!({"stopped":true, "was_live":true}));
    assert_eq!(*fake.stops.lock().unwrap(), vec![CODING_A.to_string()]);
}

#[tokio::test]
async fn a_daemon_that_survives_is_still_running_and_unknown() {
    let mut fake = Fake::new();
    fake.stop_kills = false;
    let fake = Arc::new(fake);
    fake.set_live(CODING_A, false);
    let mut adapter = unbound();
    let error = stop(&fake, &mut adapter, CODING_A).await.unwrap_err();
    assert_eq!(
        (error.code, reason(&error)),
        ("unavailable", Some("still_running"))
    );
    assert_eq!(error.delivery, "unknown");
    assert!(!error.ends_process(false));
}

#[test]
fn stop_keeps_its_time_bound() {
    let mut fake = Fake::new();
    fake.stop_delay = Duration::from_millis(400);
    let fake = Arc::new(fake);
    fake.set_live(CODING_A, false);
    let started = std::time::Instant::now();
    let error = sessions::stop_session(fake.clone(), CODING_A.into(), Duration::from_millis(50))
        .unwrap_err();
    assert!(started.elapsed() < Duration::from_millis(350));
    assert_eq!(reason(&error), Some("still_running"));
}

// ── create{name} and hello ─────────────────────────────────────────────────

#[tokio::test]
async fn create_validates_the_name_before_spawning() {
    let dir = TempDir::new();
    let mut adapter = unbound();
    for name in [
        json!(""),
        json!("bad\u{0}name"),
        json!("x".repeat(201)),
        json!(5),
    ] {
        let params = json!({"cwd": dir.0.to_string_lossy(), "name": name});
        let error = dispatch(request("create", params), &mut adapter)
            .await
            .unwrap_err();
        assert_eq!(
            (error.code, reason(&error)),
            ("invalid_request", Some("invalid_name"))
        );
        assert_eq!(error.delivery, "not_sent");
        assert_eq!(adapter.binding.state(), binding::BindingState::Unbound);
    }
}

#[test]
fn create_without_name_parses_as_before() {
    let p: Create = params(json!({"cwd":"/w"})).unwrap();
    assert!(p.name.is_none());
    let p: Create = params(json!({"cwd":"/w", "name":null})).unwrap();
    assert!(p.name.is_none());
    let p: Create = params(json!({"cwd":"/w", "name":"n"})).unwrap();
    assert_eq!(p.name, Some(json!("n")));
}

#[test]
fn hello_advertises_the_session_methods_and_bounds() {
    let hello = protocol::hello("/opt/koma/bin/koma");
    let methods = hello["methods"].as_array().unwrap();
    for method in [
        "sessions.list",
        "sessions.rename",
        "sessions.delete",
        "sessions.stop",
    ] {
        assert!(methods.contains(&json!(method)), "{method}");
    }
    assert!(hello["features"]
        .as_array()
        .unwrap()
        .contains(&json!("create.name")));
    assert_eq!(hello["limits"]["list"]["sessions"], 500);
    assert_eq!(hello["limits"]["time"]["session_stop_ms"], 8000);
    assert_eq!(hello["capabilities"].as_array().unwrap().len(), 10);
}

/// A bound coding session, played by a peer on a socket pair.
#[cfg(unix)]
mod bound {
    use super::*;
    use crate::ipc::{
        client,
        frame::FrameReader,
        proto::{ClientRequest, DaemonEvent as E, DaemonFrame, RunState},
    };

    /// What the fake daemon sends back for one request.
    pub type Script = Box<dyn FnMut(&ClientRequest, &mut String) -> Vec<E> + Send>;

    fn run_state(id: &str, name: &str) -> RunState {
        RunState {
            session_id: id.into(),
            policy: Some(crate::ipc::proto::turn::SessionPolicy {
                plan: crate::ipc::proto::PlanPolicy::Forbid,
                approval: crate::ipc::proto::ApprovalPolicy::Reject,
            }),
            name: name.into(),
            ..RunState::default()
        }
    }

    /// A daemon that is never asked anything.
    pub fn quiet() -> Script {
        Box::new(|_, _| Vec::new())
    }

    /// A daemon that answers `RenameSession` with `Ack`, or with `Error`
    /// and no change when `refuse` is set, and reports its name on readback.
    pub fn renaming(refuse: Option<&'static str>) -> Script {
        Box::new(move |request, name| match request {
            ClientRequest::RenameSession { name: new } => match refuse {
                Some(message) => vec![E::Error(message.into())],
                None => {
                    *name = new.trim().to_string();
                    vec![E::Ack]
                }
            },
            ClientRequest::GetRunState { req_seq } => vec![E::RunState {
                req_seq: *req_seq,
                state: run_state(CODING_A, name),
            }],
            _ => Vec::new(),
        })
    }

    /// An adapter bound to `id`, whose daemon follows `script`.
    pub fn adapter(
        id: &str,
        mut script: Script,
    ) -> (Adapter, tokio::task::JoinHandle<Vec<ClientRequest>>) {
        let (a, mut b) = tokio::net::UnixStream::pair().unwrap();
        let mut c = connection::Connection::for_test(a, id);
        c.projection.policy = Some(super::super::protocol::Policy::default());
        c.projection.info.name = id.to_string();
        let mut name = id.to_string();
        let peer = tokio::spawn(async move {
            let mut seq = 0u64;
            let mut seen = Vec::new();
            let mut reader = FrameReader::new();
            while let Ok(bytes) = client::recv_frame(&mut b, &mut reader).await {
                let request: ClientRequest = serde_json::from_slice(&bytes).unwrap();
                let replies = script(&request, &mut name);
                seen.push(request);
                for event in replies {
                    seq += 1;
                    let frame = serde_json::to_vec(&DaemonFrame { seq, event }).unwrap();
                    if client::send_frame(&mut b, &frame).await.is_err() {
                        return seen;
                    }
                }
            }
            seen
        });
        let mut adapter = unbound();
        adapter.binding = Binding::Coding(Box::new(c));
        (adapter, peer)
    }

    /// Drop the adapter and return every request its daemon received.
    pub async fn requests(
        adapter: Adapter,
        peer: tokio::task::JoinHandle<Vec<ClientRequest>>,
    ) -> Vec<ClientRequest> {
        drop(adapter);
        peer.await.unwrap()
    }

    #[tokio::test]
    async fn create_names_the_session_once_attached() {
        let (mut adapter, peer) = adapter(CODING_A, renaming(None));
        let c = adapter.binding.coding_mut().unwrap();
        let warning = sessions::name_created(c, "smoke", Some(&[])).await.unwrap();
        assert!(warning.is_none());
        assert_eq!(c.session_info().name, "smoke");
        requests(adapter, peer).await;
    }

    #[tokio::test]
    async fn a_refused_create_name_is_a_warning_and_the_name_stays_the_id() {
        let (mut adapter, peer) = adapter(
            CODING_A,
            renaming(Some("no foreground session for key-12345678")),
        );
        let c = adapter.binding.coding_mut().unwrap();
        let secrets = ["key-12345678".to_string()];
        let warning = sessions::name_created(c, "smoke", Some(&secrets))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(warning.code, "rename_failed");
        // The daemon's text is scrubbed of configured secrets.
        assert_eq!(warning.message, "no foreground session for [redacted]");
        assert_eq!(c.session_info().name, CODING_A);
        requests(adapter, peer).await;
    }

    #[tokio::test]
    async fn a_lost_daemon_during_the_create_name_is_unknown() {
        let (mut adapter, peer) = adapter(CODING_A, quiet());
        peer.abort();
        let c = adapter.binding.coding_mut().unwrap();
        let error = sessions::name_created(c, "smoke", Some(&[]))
            .await
            .unwrap_err();
        assert_eq!(error.delivery, "unknown");
        assert!(error.ends_process(false));
    }
}

#[tokio::test]
async fn the_real_host_is_unreachable_from_unit_tests() {
    let mut adapter = unbound();
    let error = dispatch(request("sessions.list", json!({})), &mut adapter)
        .await
        .unwrap_err();
    assert_eq!(
        (error.code, reason(&error)),
        ("unavailable", Some("io_error"))
    );
    let error = dispatch(
        request("sessions.delete", json!({"session_id": CODING_A})),
        &mut adapter,
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "unavailable");
    let stopped = dispatch(
        request("sessions.stop", json!({"session_id": CODING_A})),
        &mut adapter,
    )
    .await
    .unwrap();
    assert_eq!(stopped["was_live"], false);
}
