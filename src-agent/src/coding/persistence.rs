use super::{Operation, Request};
use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Backup {
    pub window_id: String,
    pub path: String,
    pub revision: u64,
    pub content: String,
    pub saved_content: Option<String>,
    pub fingerprint: String,
    #[serde(default)]
    pub view: Value,
}

fn connect() -> Result<Connection> {
    let dir = crate::model::store::base_dir()?.join("coding");
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let db = Connection::open(dir.join("state.sqlite3"))?;
    initialize(&db)?;
    Ok(db)
}

fn initialize(db: &Connection) -> Result<()> {
    db.busy_timeout(std::time::Duration::from_secs(5))?;
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
        CREATE TABLE IF NOT EXISTS coding_backups (
            host TEXT NOT NULL, root TEXT NOT NULL, path TEXT NOT NULL,
            window_id TEXT NOT NULL, revision INTEGER NOT NULL, body TEXT NOT NULL,
            updated INTEGER NOT NULL, PRIMARY KEY(host,root,path,window_id));
        CREATE TABLE IF NOT EXISTS coding_history (
            id INTEGER PRIMARY KEY, host TEXT NOT NULL, root TEXT NOT NULL,
            path TEXT NOT NULL, content TEXT NOT NULL, reason TEXT NOT NULL,
            created INTEGER NOT NULL, bytes INTEGER NOT NULL);
        CREATE INDEX IF NOT EXISTS coding_history_document ON coding_history(host,root,path,created);")?;
    Ok(())
}

pub(super) fn execute(request: &Request) -> Result<Value> {
    execute_on(&connect()?, request)
}

fn execute_on(db: &Connection, request: &Request) -> Result<Value> {
    let w = &request.workspace;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as i64;
    match &request.operation {
        Operation::Backup { document } => {
            anyhow::ensure!(
                document.content.len() <= 20 * 1024 * 1024,
                "Recovery document exceeds limit"
            );
            anyhow::ensure!(
                document.revision <= i64::MAX as u64,
                "Invalid recovery revision"
            );
            anyhow::ensure!(
                document.window_id.len() <= 200 && document.path.len() <= 32768,
                "Invalid recovery identity"
            );
            let body = serde_json::to_string(document)?;
            anyhow::ensure!(
                body.len() <= 24 * 1024 * 1024,
                "Recovery snapshot exceeds limit"
            );
            db.execute(
                "INSERT INTO coding_backups(host,root,path,window_id,revision,body,updated)
                VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(host,root,path,window_id) DO UPDATE SET
                revision=excluded.revision,body=excluded.body,updated=excluded.updated
                WHERE excluded.revision > coding_backups.revision",
                params![
                    w.host_id,
                    w.root,
                    document.path,
                    document.window_id,
                    document.revision,
                    body,
                    now
                ],
            )?;
            Ok(json!({"revision":document.revision}))
        }
        Operation::Backups => {
            // List metadata only; loading many full drafts would exhaust the
            // WebView and exceed the transport frame limit.
            let mut stmt = db.prepare(
                "SELECT path,window_id,revision,updated FROM coding_backups
                WHERE host=?1 AND root=?2 AND COALESCE(json_extract(body,'$.view.discarded'),0)=0
                ORDER BY updated DESC LIMIT 201",
            )?;
            let rows = stmt.query_map(params![w.host_id, w.root], |r| {
                Ok(json!({
                "path":r.get::<_,String>(0)?, "windowId":r.get::<_,String>(1)?,
                "revision":r.get::<_,i64>(2)?, "updated":r.get::<_,i64>(3)?}))
            })?;
            let mut documents = rows.collect::<Result<Vec<_>, _>>()?;
            let truncated = documents.len() > 200;
            documents.truncate(200);
            Ok(json!({"documents":documents,"truncated":truncated}))
        }
        Operation::BackupRead {
            window_id,
            path,
            revision,
        } => {
            let body: String = db
                .query_row(
                    "SELECT body FROM coding_backups
                WHERE host=?1 AND root=?2 AND window_id=?3 AND path=?4 AND revision=?5
                AND COALESCE(json_extract(body,'$.view.discarded'),0)=0",
                    params![w.host_id, w.root, window_id, path, revision],
                    |r| r.get(0),
                )
                .context("Recovery entry changed or was discarded; refresh the list")?;
            Ok(serde_json::from_str(&body)?)
        }
        Operation::ForgetBackup {
            window_id,
            path,
            revision,
        } => {
            // Keep a revision tombstone, so a delayed backup cannot resurrect a
            // document after Save/Discard acknowledged a newer revision.
            let document = Backup {
                window_id: window_id.clone(),
                path: path.clone(),
                revision: *revision,
                content: String::new(),
                saved_content: Some(String::new()),
                fingerprint: String::new(),
                view: json!({"discarded":true}),
            };
            let body = serde_json::to_string(&document)?;
            db.execute(
                "INSERT INTO coding_backups(host,root,path,window_id,revision,body,updated)
                VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(host,root,path,window_id) DO UPDATE SET
                revision=excluded.revision,body=excluded.body,updated=excluded.updated
                WHERE excluded.revision >= coding_backups.revision",
                params![w.host_id, w.root, path, window_id, revision, body, now],
            )?;
            Ok(Value::Null)
        }
        Operation::Checkpoint {
            path,
            content,
            reason,
        } => {
            anyhow::ensure!(
                content.len() <= 20 * 1024 * 1024 && reason.len() <= 200,
                "History checkpoint exceeds limit"
            );
            db.execute("INSERT INTO coding_history(host,root,path,content,reason,created,bytes) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![w.host_id,w.root,path,content,reason,now,content.len()])?;
            let id = db.last_insert_rowid();
            db.execute(
                "DELETE FROM coding_history WHERE host=?1 AND created < ?2",
                params![w.host_id, now - 30 * 86400 * 1000],
            )?;
            db.execute(
                "DELETE FROM coding_history WHERE id IN (
                SELECT id FROM (SELECT id,SUM(bytes) OVER(ORDER BY created DESC,id DESC) AS total
                FROM coding_history WHERE host=?1) WHERE total > 524288000)",
                params![w.host_id],
            )?;
            Ok(json!({"id":id}))
        }
        Operation::History { path } => {
            let mut stmt = db.prepare("SELECT id,reason,created,bytes FROM coding_history WHERE host=?1 AND root=?2 AND path=?3 ORDER BY created DESC,id DESC LIMIT 200")?;
            let rows = stmt.query_map(params![w.host_id,w.root,path], |r| Ok(json!({
                "id":r.get::<_,i64>(0)?,"reason":r.get::<_,String>(1)?,"created":r.get::<_,i64>(2)?,"bytes":r.get::<_,i64>(3)?})))?;
            Ok(json!(rows.collect::<Result<Vec<_>, _>>()?))
        }
        Operation::HistoryRead { checkpoint } => {
            let (path, content): (String, String) = db.query_row(
                "SELECT path,content FROM coding_history WHERE id=?1 AND host=?2 AND root=?3",
                params![checkpoint, w.host_id, w.root],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            Ok(json!({"path":path,"content":content}))
        }
        _ => anyhow::bail!("Unsupported persistence operation"),
    }
}

pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("File has no parent directory")?;
    let tmp = parent.join(format!(".koma-write-{}", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)?;
        if let Ok(meta) = path.metadata() {
            anyhow::ensure!(!meta.permissions().readonly(), "File is read-only");
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                anyhow::ensure!(
                    meta.nlink() <= 1,
                    "Atomic save would break hard links; save to a separate file"
                );
            }
            file.set_permissions(meta.permissions())?;
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)?;
        #[cfg(unix)]
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding::WorkspaceRef;

    fn request(host: &str, operation: Operation) -> Request {
        Request {
            id: "test".into(),
            workspace: WorkspaceRef {
                host_id: host.into(),
                root: "/workspace".into(),
            },
            operation,
        }
    }
    fn backup(revision: u64, content: &str) -> Operation {
        Operation::Backup {
            document: Backup {
                window_id: "window".into(),
                path: "main.rs".into(),
                revision,
                content: content.into(),
                saved_content: Some("original".into()),
                fingerprint: "fingerprint".into(),
                view: Value::Null,
            },
        }
    }
    #[test]
    fn late_drafts_cannot_resurrect_discarded_content() {
        let db = Connection::open_in_memory().unwrap();
        initialize(&db).unwrap();
        execute_on(&db, &request("local", backup(2, "newer"))).unwrap();
        execute_on(&db, &request("local", backup(1, "older"))).unwrap();
        let read = Operation::BackupRead {
            window_id: "window".into(),
            path: "main.rs".into(),
            revision: 2,
        };
        assert_eq!(
            execute_on(&db, &request("local", read.clone())).unwrap()["content"],
            "newer"
        );
        assert!(execute_on(&db, &request("ssh-other", read.clone())).is_err());
        execute_on(
            &db,
            &request(
                "local",
                Operation::ForgetBackup {
                    window_id: "window".into(),
                    path: "main.rs".into(),
                    revision: 3,
                },
            ),
        )
        .unwrap();
        execute_on(&db, &request("local", backup(2, "late"))).unwrap();
        assert!(execute_on(&db, &request("local", read)).is_err());
        let rows = execute_on(&db, &request("local", Operation::Backups)).unwrap();
        assert_eq!(rows["documents"], json!([]));
        execute_on(&db, &request("local", backup(4, "new edit"))).unwrap();
        assert_eq!(
            execute_on(&db, &request("local", Operation::Backups)).unwrap()["documents"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
    #[test]
    fn history_is_host_scoped_and_drafts_are_not_history_evictions() {
        let db = Connection::open_in_memory().unwrap();
        initialize(&db).unwrap();
        execute_on(&db, &request("local", backup(1, "unsaved"))).unwrap();
        let row = execute_on(
            &db,
            &request(
                "local",
                Operation::Checkpoint {
                    path: "main.rs".into(),
                    content: "saved".into(),
                    reason: "Saved".into(),
                },
            ),
        )
        .unwrap();
        let read = Operation::HistoryRead {
            checkpoint: row["id"].as_i64().unwrap(),
        };
        assert_eq!(
            execute_on(&db, &request("local", read.clone())).unwrap()["content"],
            "saved"
        );
        assert!(execute_on(&db, &request("other", read)).is_err());
        assert_eq!(
            execute_on(&db, &request("local", Operation::Backups)).unwrap()["documents"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
}
