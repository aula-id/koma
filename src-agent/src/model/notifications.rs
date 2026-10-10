//! Saved notification history. One ordered worker owns all disk operations.
use anyhow::{anyhow, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        mpsc::{self, Receiver, Sender},
        OnceLock,
    },
    time::{SystemTime, UNIX_EPOCH},
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
            for job in rx {
                let result = resolve(&job.request.session)
                    .and_then(|path| execute(path, &job.request.operation));
                let (entries, error) = match result {
                    Ok(entries) => (entries, None),
                    Err(e) => {
                        eprintln!("notification history: {e}");
                        (vec![], Some(e.to_string()))
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
        let row = super::session_registry::get(id)?
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
fn execute(path: PathBuf, operation: &Operation) -> Result<Vec<Entry>> {
    let mut conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS notifications (id TEXT PRIMARY KEY, timestamp INTEGER NOT NULL, severity TEXT NOT NULL, source TEXT NOT NULL, message TEXT NOT NULL, read INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE IF NOT EXISTS notification_events (id TEXT PRIMARY KEY, timestamp INTEGER NOT NULL);
        INSERT OR IGNORE INTO notification_events SELECT id,timestamp FROM notifications;")?;
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
}
