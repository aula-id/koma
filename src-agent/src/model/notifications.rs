//! Saved notification history. One ordered worker owns all disk operations.
use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, Receiver, Sender},
        OnceLock,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub timestamp: i64,
    pub severity: String,
    pub source: String,
    pub message: String,
    pub read: bool,
}
impl Entry {
    pub fn new(message: String, severity: &str, source: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64,
            severity: severity.into(),
            source: source.into(),
            message,
            read: false,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Operation {
    List,
    Record { entry: Entry },
    Read { id: Option<String> },
    Clear,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    pub session: Option<String>,
    pub operation: Operation,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    pub id: String,
    pub session: Option<String>,
    pub entries: Vec<Entry>,
    pub error: Option<String>,
}
struct Job {
    request: Request,
    reply: Sender<Reply>,
}
fn worker() -> &'static Sender<Job> {
    static QUEUE: OnceLock<Sender<Job>> = OnceLock::new();
    QUEUE.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::spawn(move || {
            let mut last: Option<(String, u64)> = None;
            for job in rx {
                let op = op_name(&job.request.operation);
                let mut path_for_log = None;
                let result = match guard(|| resolve(&job.request.session)) {
                    Ok(path) => {
                        path_for_log = Some(path.clone());
                        run_with_retry(&path, &job.request.operation)
                    }
                    Err(e) => Err(e),
                };
                let (entries, error) = match result {
                    Ok(entries) => (entries, None),
                    Err(e) => {
                        let msg = describe(op, &job.request.session, path_for_log.as_deref(), &e);
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        if should_log(&last, &msg, now) {
                            eprintln!("notification history: {msg}");
                            super::store::append_global_error_log("notification history", &msg);
                            last = Some((msg.clone(), now));
                        }
                        (vec![], Some(msg))
                    }
                };
                let _ = job.reply.send(Reply {
                    id: job.request.id,
                    session: job.request.session,
                    entries,
                    error,
                });
            }
        });
        tx
    })
}
pub fn request(request: Request) -> Receiver<Reply> {
    let (tx, rx) = mpsc::channel();
    let _ = worker().send(Job { request, reply: tx });
    rx
}
pub fn record(session: Option<String>, entry: Entry) {
    let _ = request(Request {
        id: entry.id.clone(),
        session,
        operation: Operation::Record { entry },
    });
}
fn resolve(session: &Option<String>) -> Result<PathBuf> {
    let dir = if let Some(id) = session {
        let row = super::session_registry::get_existing(id)?
            .ok_or_else(|| anyhow!("unknown notification session"))?;
        super::store::session_dir(&row.pwd_hash, &row.uuid)?
    } else {
        super::store::base_dir()?
    };
    // Never recreate a deleted session.
    if session.is_some() && !dir.is_dir() {
        return Err(anyhow!("notification session no longer exists"));
    }
    if session.is_none() {
        std::fs::create_dir_all(&dir)?;
    }
    Ok(dir.join("notifications.sqlite"))
}
fn op_name(op: &Operation) -> &'static str {
    match op {
        Operation::List => "list",
        Operation::Record { .. } => "record",
        Operation::Read { .. } => "read",
        Operation::Clear => "clear",
    }
}
/// Only transient I/O errors (a scanner holding a journal file) are retried;
/// BUSY/LOCKED were already waited out by the busy timeout.
fn retry_delay(code: rusqlite::ErrorCode, attempt: u32) -> Option<Duration> {
    if !matches!(
        code,
        rusqlite::ErrorCode::SystemIoFailure | rusqlite::ErrorCode::CannotOpen
    ) {
        return None;
    }
    match attempt {
        0 => Some(Duration::from_millis(50)),
        1 => Some(Duration::from_millis(200)),
        _ => None,
    }
}
fn error_code(err: &anyhow::Error) -> Option<rusqlite::ErrorCode> {
    match err.downcast_ref::<rusqlite::Error>() {
        Some(rusqlite::Error::SqliteFailure(e, _)) => Some(e.code),
        _ => None,
    }
}
fn describe(op: &str, scope: &Option<String>, path: Option<&Path>, err: &anyhow::Error) -> String {
    let scope = match scope {
        Some(id) => format!("session {id}"),
        None => "app".to_string(),
    };
    let path = path.map_or_else(|| "?".to_string(), |p| p.display().to_string());
    let mut out = format!("{op} scope={scope} path={path}: {err:#}");
    if let Some(n) = err
        .chain()
        .find_map(|c| c.downcast_ref::<std::io::Error>())
        .and_then(|e| e.raw_os_error())
    {
        out.push_str(&format!(" [os error {n}]"));
    }
    if let Some(rusqlite::Error::SqliteFailure(e, _)) = err.downcast_ref::<rusqlite::Error>() {
        out.push_str(&format!(" [sqlite extended code {}]", e.extended_code));
    }
    out
}
fn should_log(last: &Option<(String, u64)>, msg: &str, now_secs: u64) -> bool {
    match last {
        Some((m, t)) => m != msg || now_secs.saturating_sub(*t) >= 60,
        None => true,
    }
}
fn guard<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(_) => Err(anyhow!("notification worker panicked")),
    }
}
fn run_with_retry(path: &Path, op: &Operation) -> Result<Vec<Entry>> {
    let mut attempt = 0;
    loop {
        match guard(|| execute(path.to_path_buf(), op)) {
            Ok(v) => return Ok(v),
            Err(e) => match error_code(&e).and_then(|c| retry_delay(c, attempt)) {
                Some(d) => {
                    std::thread::sleep(d);
                    attempt += 1;
                }
                None => return Err(e),
            },
        }
    }
}
fn schema_ready(conn: &Connection) -> rusqlite::Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('notifications','notification_events')",
        [],
        |r| r.get(0),
    )?;
    Ok(n == 2)
}
fn execute(path: PathBuf, operation: &Operation) -> Result<Vec<Entry>> {
    let mut conn = Connection::open(&path).with_context(|| format!("open {}", path.display()))?;
    conn.busy_timeout(Duration::from_secs(5))?;
    let _ = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get::<_, String>(0));
    let _ = conn.execute_batch("PRAGMA synchronous=NORMAL;");
    if !schema_ready(&conn)? {
        conn.execute_batch("CREATE TABLE IF NOT EXISTS notifications (id TEXT PRIMARY KEY, timestamp INTEGER NOT NULL, severity TEXT NOT NULL, source TEXT NOT NULL, message TEXT NOT NULL, read INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE IF NOT EXISTS notification_events (id TEXT PRIMARY KEY, timestamp INTEGER NOT NULL);
        INSERT OR IGNORE INTO notification_events SELECT id,timestamp FROM notifications;")?;
    }
    let tx = conn.transaction()?;
    match operation {
        Operation::List => {}
        Operation::Record { entry: e } => {
            if e.id.is_empty()
                || !["info", "success", "warn", "error"].contains(&e.severity.as_str())
                || e.id.len() > 128
                || e.message.len() > 65536
                || e.source.len() > 128
            {
                return Err(anyhow!("invalid notification"));
            }
            // Bounded ID tombstones survive clearing; replays cannot restore cleared rows.
            let fresh = tx.execute(
                "INSERT OR IGNORE INTO notification_events VALUES (?1,?2)",
                params![e.id, e.timestamp],
            )?;
            if fresh > 0 {
                tx.execute(
                    "INSERT OR IGNORE INTO notifications VALUES (?1,?2,?3,?4,?5,0)",
                    params![e.id, e.timestamp, e.severity, e.source, e.message],
                )?;
            }
            tx.execute("DELETE FROM notification_events WHERE rowid NOT IN (SELECT rowid FROM notification_events ORDER BY timestamp DESC,rowid DESC LIMIT 2000)", [])?;
            tx.execute("DELETE FROM notifications WHERE rowid NOT IN (SELECT rowid FROM notifications ORDER BY timestamp DESC, rowid DESC LIMIT 500)", [])?;
        }
        Operation::Read { id } => {
            if let Some(id) = id {
                tx.execute("UPDATE notifications SET read=1 WHERE id=?1", [id])?;
            } else {
                tx.execute("UPDATE notifications SET read=1", [])?;
            }
        }
        Operation::Clear => {
            tx.execute("DELETE FROM notifications", [])?;
        }
    }
    tx.commit()?;
    let mut stmt = conn.prepare("SELECT id,timestamp,severity,source,message,read FROM notifications ORDER BY timestamp DESC,rowid DESC")?;
    let entries = stmt
        .query_map([], |r| {
            Ok(Entry {
                id: r.get(0)?,
                timestamp: r.get(1)?,
                severity: r.get(2)?,
                source: r.get(3)?,
                message: r.get(4)?,
                read: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(entries)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scopes_read_and_clear_are_independent() {
        let a =
            std::env::temp_dir().join(format!("notification-a-{}.sqlite", uuid::Uuid::new_v4()));
        let b =
            std::env::temp_dir().join(format!("notification-b-{}.sqlite", uuid::Uuid::new_v4()));
        let entry = Entry::new("session notice".into(), "warn", "test");
        execute(
            a.clone(),
            &Operation::Record {
                entry: entry.clone(),
            },
        )
        .unwrap();
        assert!(execute(b.clone(), &Operation::List).unwrap().is_empty());
        let read = execute(
            a.clone(),
            &Operation::Read {
                id: Some(entry.id.clone()),
            },
        )
        .unwrap();
        assert!(read[0].read);
        execute(
            b.clone(),
            &Operation::Record {
                entry: entry.clone(),
            },
        )
        .unwrap();
        assert!(!execute(b.clone(), &Operation::List).unwrap()[0].read);
        execute(a.clone(), &Operation::Clear).unwrap();
        assert_eq!(execute(b.clone(), &Operation::List).unwrap().len(), 1);
        std::fs::remove_file(a).unwrap();
        std::fs::remove_file(b).unwrap();
    }
    #[test]
    fn persistence_retention_and_event_identity() {
        let path =
            std::env::temp_dir().join(format!("notification-{}.sqlite", uuid::Uuid::new_v4()));
        let mut e = Entry::new("same text".into(), "info", "test");
        execute(path.clone(), &Operation::Record { entry: e.clone() }).unwrap();
        assert_eq!(
            execute(path.clone(), &Operation::Record { entry: e.clone() })
                .unwrap()
                .len(),
            1
        );
        e.id = uuid::Uuid::new_v4().to_string();
        assert_eq!(
            execute(path.clone(), &Operation::Record { entry: e.clone() })
                .unwrap()
                .len(),
            2
        );
        for n in 0..501 {
            e.id = n.to_string();
            e.timestamp = n;
            execute(path.clone(), &Operation::Record { entry: e.clone() }).unwrap();
        }
        let entries = execute(path.clone(), &Operation::Read { id: None }).unwrap();
        assert_eq!(entries.len(), 500);
        assert!(entries.iter().all(|e| e.read));
        assert!(entries.windows(2).all(|p| p[0].timestamp >= p[1].timestamp));
        assert!(execute(path.clone(), &Operation::Clear).unwrap().is_empty());
        assert!(execute(path.clone(), &Operation::Record { entry: e })
            .unwrap()
            .is_empty());
        std::fs::remove_file(path).unwrap();
    }

    fn temp_db() -> PathBuf {
        std::env::temp_dir().join(format!("notification-t-{}.sqlite", uuid::Uuid::new_v4()))
    }
    fn cleanup(p: &Path) {
        let _ = std::fs::remove_file(p);
        for ext in ["-wal", "-shm", "-journal"] {
            let _ = std::fs::remove_file(format!("{}{ext}", p.display()));
        }
    }
    #[test]
    fn retry_delay_only_for_transient_io() {
        use rusqlite::ErrorCode::*;
        for c in [SystemIoFailure, CannotOpen] {
            assert_eq!(retry_delay(c, 0), Some(Duration::from_millis(50)));
            assert_eq!(retry_delay(c, 1), Some(Duration::from_millis(200)));
            assert_eq!(retry_delay(c, 2), None);
        }
        for c in [DatabaseBusy, DatabaseLocked, Unknown] {
            assert_eq!(retry_delay(c, 0), None);
        }
    }
    #[test]
    fn describe_names_path_scope_and_os_code() {
        let err = anyhow::Error::new(std::io::Error::from_raw_os_error(32))
            .context("open C:/x/notifications.sqlite");
        let s = describe(
            "list",
            &Some("abc".into()),
            Some(Path::new("C:/x/n.sqlite")),
            &err,
        );
        assert!(s.contains("scope=session abc"), "{s}");
        assert!(s.contains("C:/x/n.sqlite"), "{s}");
        assert!(s.contains("os error 32"), "{s}");
        assert!(describe("clear", &None, None, &err).contains("scope=app"));
    }
    #[test]
    fn describe_includes_sqlite_extended_code() {
        let err = anyhow::Error::new(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(10),
            None,
        ));
        assert_eq!(error_code(&err), Some(rusqlite::ErrorCode::SystemIoFailure));
        assert!(describe("list", &None, None, &err).contains("sqlite extended code 10"));
    }
    #[test]
    fn should_log_throttles_same_message() {
        assert!(should_log(&None, "a", 100));
        let last = Some(("a".to_string(), 100));
        assert!(!should_log(&last, "a", 110));
        assert!(should_log(&last, "a", 160));
        assert!(should_log(&last, "b", 101));
    }
    #[test]
    fn guard_turns_panic_into_error() {
        let r: Result<u8> = guard(|| panic!("boom"));
        assert!(r.is_err());
        assert_eq!(guard(|| Ok(3)).unwrap(), 3);
    }
    #[test]
    fn execute_sets_wal_and_creates_schema_once() {
        let path = temp_db();
        execute(path.clone(), &Operation::List).unwrap();
        let conn = Connection::open(&path).unwrap();
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        assert!(schema_ready(&conn).unwrap());
        drop(conn);
        cleanup(&path);
    }
    #[test]
    fn legacy_db_without_events_table_is_backfilled() {
        let path = temp_db();
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE notifications (id TEXT PRIMARY KEY, timestamp INTEGER NOT NULL, severity TEXT NOT NULL, source TEXT NOT NULL, message TEXT NOT NULL, read INTEGER NOT NULL DEFAULT 0);
                INSERT INTO notifications VALUES ('old',1,'info','t','m',0);").unwrap();
        }
        assert_eq!(execute(path.clone(), &Operation::List).unwrap().len(), 1);
        assert!(execute(path.clone(), &Operation::Clear).unwrap().is_empty());
        let mut e = Entry::new("m".into(), "info", "t");
        e.id = "old".into();
        assert!(execute(path.clone(), &Operation::Record { entry: e })
            .unwrap()
            .is_empty());
        cleanup(&path);
    }
    #[test]
    fn open_failure_names_the_path() {
        let path = std::env::temp_dir()
            .join(format!("no-such-{}", uuid::Uuid::new_v4()))
            .join("n.sqlite");
        let err = execute(path.clone(), &Operation::List).unwrap_err();
        let s = describe("list", &None, Some(&path), &err);
        assert!(s.contains("n.sqlite"), "{s}");
    }
    #[test]
    fn concurrent_workers_do_not_fail() {
        let path = temp_db();
        execute(path.clone(), &Operation::List).unwrap();
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for _ in 0..40 {
                        let e = Entry::new("x".into(), "info", "t");
                        let id = e.id.clone();
                        execute(path.clone(), &Operation::Record { entry: e })?;
                        execute(path.clone(), &Operation::List)?;
                        execute(path.clone(), &Operation::Read { id: Some(id) })?;
                    }
                    Ok::<(), anyhow::Error>(())
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap().unwrap();
        }
        cleanup(&path);
    }
}
