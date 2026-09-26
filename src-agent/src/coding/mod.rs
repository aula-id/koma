//! Coding operations are owned by the GUI, independently of the chat session.
//! All operations carry their host and root; UI selection is never a routing key.

mod language;
pub(crate) mod persistence;
mod transport;
mod workspace;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{mpsc, Arc, Mutex, OnceLock};

pub(crate) use transport::remember_remote;
static SHUTTING_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub(crate) fn shutdown() {
    SHUTTING_DOWN.store(true, std::sync::atomic::Ordering::Release);
    transport::shutdown();
    language::shutdown();
}

static EVENTS: OnceLock<Arc<dyn Fn(String) + Send + Sync>> = OnceLock::new();
pub(super) fn event(value: Value) {
    if let Some(push) = EVENTS.get() {
        push(value.to_string());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceRef {
    pub host_id: String,
    pub root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Request {
    pub id: String,
    pub workspace: WorkspaceRef,
    #[serde(flatten)]
    pub operation: Operation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub(crate) enum Operation {
    Hello,
    Paths {
        query: String,
    },
    Read {
        path: String,
    },
    Inspect {
        paths: Vec<String>,
    },
    Save {
        path: String,
        content: String,
        fingerprint: String,
    },
    ReplacePreview {
        options: Value,
    },
    ConfigRead,
    ConfigEnsure,
    ConfigWrite {
        config: Value,
    },
    Lsp {
        body: Value,
    },
    LspQuery {
        path: String,
        method: String,
        params: Value,
    },
    Backup {
        document: persistence::Backup,
    },
    Backups,
    BackupRead {
        window_id: String,
        path: String,
        revision: u64,
    },
    ForgetBackup {
        window_id: String,
        path: String,
        revision: u64,
    },
    Checkpoint {
        path: String,
        content: String,
        reason: String,
    },
    History {
        path: String,
    },
    HistoryRead {
        checkpoint: i64,
    },
}

impl Operation {
    fn local_metadata(&self) -> bool {
        matches!(
            self,
            Self::Backup { .. }
                | Self::BackupRead { .. }
                | Self::Backups
                | Self::ForgetBackup { .. }
                | Self::Checkpoint { .. }
                | Self::History { .. }
                | Self::HistoryRead { .. }
        )
    }
}

pub(crate) struct Service {
    queue: mpsc::SyncSender<Request>,
    language_queue: mpsc::SyncSender<Request>,
}

impl Service {
    pub fn new(push: impl Fn(String) + Send + Sync + 'static) -> Self {
        let push: Arc<dyn Fn(String) + Send + Sync> = Arc::new(push);
        let _ = EVENTS.set(Arc::clone(&push));
        let (queue, receiver) = mpsc::sync_channel::<Request>(128);
        let receiver = Arc::new(Mutex::new(receiver));
        for _ in 0..4 {
            let receiver = Arc::clone(&receiver);
            let push = Arc::clone(&push);
            std::thread::spawn(move || loop {
                let request = match receiver.lock() {
                    Ok(rx) => match rx.recv() {
                        Ok(r) => r,
                        Err(_) => break,
                    },
                    Err(_) => break,
                };
                let result =
                    if request.operation.local_metadata() || request.workspace.host_id == "local" {
                        execute(&request)
                    } else {
                        transport::request(&request)
                    };
                let envelope = match result {
                    Ok(value) => {
                        json!({"k":"CodingReply", "id":request.id, "workspace":request.workspace, "result":value})
                    }
                    Err(error) => {
                        json!({"k":"CodingReply", "id":request.id, "workspace":request.workspace, "error":error})
                    }
                };
                push(envelope.to_string());
            });
        }
        // Submission order is significant for didOpen/didChange/didClose.
        // Never let the general I/O pool reorder language messages.
        let (language_queue, language_rx) = mpsc::sync_channel::<Request>(128);
        std::thread::spawn(move || {
            for request in language_rx {
                let result = if request.workspace.host_id == "local" {
                    execute(&request)
                } else {
                    transport::request(&request)
                };
                let envelope = match result {
                    Ok(value) => {
                        json!({"k":"CodingReply","id":request.id,"workspace":request.workspace,"result":value})
                    }
                    Err(error) => {
                        json!({"k":"CodingReply","id":request.id,"workspace":request.workspace,"error":error})
                    }
                };
                push(envelope.to_string());
            }
        });
        Self {
            queue,
            language_queue,
        }
    }

    /// A full queue is reported to the caller instead of blocking WebView's UI thread.
    pub fn submit(&self, request: Request) -> Result<(), String> {
        let queue = if matches!(request.operation, Operation::Lsp { .. }) {
            &self.language_queue
        } else {
            &self.queue
        };
        queue
            .try_send(request)
            .map_err(|_| "Coding service is busy; retry the operation".into())
    }
}

fn execute(request: &Request) -> Result<Value, String> {
    if SHUTTING_DOWN.load(std::sync::atomic::Ordering::Acquire) {
        return Err("Coding service is shutting down".into());
    }
    if request.id.len() > 200
        || request.workspace.host_id.len() > 200
        || request.workspace.root.len() > 32768
    {
        return Err("Invalid coding request identity".into());
    }
    if request.operation.local_metadata() {
        return persistence::execute(request).map_err(|e| format!("{e:#}"));
    }
    workspace::execute(request).map_err(|e| format!("{e:#}"))
}

/// Private SSH service. The protocol is versioned JSON lines with bounded frames;
/// the service never mixes diagnostic logs into stdout.
pub(crate) fn worker_main() -> anyhow::Result<()> {
    use std::io::{BufRead, Read, Write};
    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            language::shutdown();
        }
    }
    let _cleanup = Cleanup;
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let output = Arc::new(Mutex::new(std::io::stdout()));
    let events_output = Arc::clone(&output);
    let _ = EVENTS.set(Arc::new(move |line| {
        if let Ok(mut stdout) = events_output.lock() {
            let _ = writeln!(stdout, "{line}");
            let _ = stdout.flush();
        }
    }));
    loop {
        let mut line = Vec::new();
        let n = std::io::Read::by_ref(&mut input)
            .take(32 * 1024 * 1024 + 1)
            .read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        anyhow::ensure!(
            n <= 32 * 1024 * 1024 && line.last() == Some(&b'\n'),
            "Coding frame exceeds limit"
        );
        let request: Request = serde_json::from_slice(&line)?;
        let response = match execute(&request) {
            Ok(value) => json!({"id":request.id, "result":value}),
            Err(error) => json!({"id":request.id, "error":error}),
        };
        let mut output = output
            .lock()
            .map_err(|_| anyhow::anyhow!("Coding output lock failed"))?;
        serde_json::to_writer(&mut *output, &response)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
    Ok(())
}
