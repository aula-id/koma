//! Retained SSH channels are independent of the foreground chat attachment.
use super::Request;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin};
use std::sync::{mpsc, Arc, Mutex, OnceLock};

#[derive(Clone)]
struct Remote {
    target: crate::remote::RemoteTarget,
    password: Option<String>,
    executable: String,
}

static REMOTES: OnceLock<Mutex<HashMap<String, Remote>>> = OnceLock::new();
static CONNECTIONS: OnceLock<Mutex<HashMap<String, Arc<Mutex<Connection>>>>> = OnceLock::new();

pub(crate) fn remember_remote(
    id: &str,
    target: &crate::remote::RemoteTarget,
    password: Option<&str>,
    executable: &str,
) {
    if let Ok(mut remotes) = REMOTES.get_or_init(Default::default).lock() {
        remotes.insert(
            id.to_string(),
            Remote {
                target: target.clone(),
                password: password.map(str::to_owned),
                executable: executable.into(),
            },
        );
    }
}

struct Connection {
    ready: bool,
    child: Child,
    input: ChildStdin,
    replies: mpsc::Receiver<Result<Value, String>>,
    _auth: Option<crate::remote::auth::SshAuth>,
}
impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn connect(remote: Remote) -> anyhow::Result<Connection> {
    let auth = remote
        .password
        .map(crate::remote::auth::SshAuth::from_password)
        .transpose()?;
    let mut child = crate::remote::ssh::coding_worker_command(
        &remote.target,
        auth.as_ref(),
        &remote.executable,
    )?
    .stdin(std::process::Stdio::piped())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::null())
    .spawn()?;
    let input = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("Coding SSH stdin unavailable"))?;
    let output = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("Coding SSH stdout unavailable"))?;
    let (tx, replies) = mpsc::sync_channel(8);
    std::thread::spawn(move || {
        let mut output = BufReader::new(output);
        loop {
            let result = (|| -> anyhow::Result<Value> {
                let mut line = Vec::new();
                let n = output
                    .by_ref()
                    .take(48 * 1024 * 1024 + 1)
                    .read_until(b'\n', &mut line)?;
                anyhow::ensure!(
                    n > 0 && n <= 48 * 1024 * 1024 && line.last() == Some(&b'\n'),
                    "Coding connection closed or incompatible; update remote Koma"
                );
                Ok(serde_json::from_slice(&line)?)
            })()
            .map_err(|e| format!("{e:#}"));
            if let Ok(value) = &result {
                if value.get("k").and_then(Value::as_str) == Some("CodingEvent") {
                    super::event(value.clone());
                    continue;
                }
            }
            let failed = result.is_err();
            if tx.send(result).is_err() || failed {
                break;
            }
        }
    });
    Ok(Connection {
        ready: false,
        child,
        input,
        replies,
        _auth: auth,
    })
}

pub(super) fn request(request: &Request) -> Result<Value, String> {
    let id = &request.workspace.host_id;
    let connections = CONNECTIONS.get_or_init(Default::default);
    let connection = {
        let mut map = connections
            .lock()
            .map_err(|_| "Coding connection lock failed")?;
        if let Some(connection) = map.get(id) {
            Arc::clone(connection)
        } else {
            let remote = REMOTES
                .get_or_init(Default::default)
                .lock()
                .map_err(|_| "Coding host lock failed")?
                .get(id)
                .cloned()
                .ok_or("Connect to this host before opening its coding workspace")?;
            let connection = Arc::new(Mutex::new(
                connect(remote).map_err(|e| format!("Coding SSH: {e:#}"))?,
            ));
            map.insert(id.clone(), Arc::clone(&connection));
            connection
        }
    };
    let result = (|| -> anyhow::Result<Value> {
        let mut connection = connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Coding connection lock failed"))?;
        serde_json::to_writer(&mut connection.input, request)?;
        connection.input.write_all(b"\n")?;
        connection.input.flush()?;
        let reply = connection
            .replies
            .recv_timeout(std::time::Duration::from_secs(25))
            .map_err(|_| anyhow::anyhow!("Coding SSH timed out; operation outcome may be unknown"))?
            .map_err(anyhow::Error::msg)?;
        anyhow::ensure!(
            reply.get("id").and_then(Value::as_str) == Some(&request.id),
            "Coding reply identity mismatch"
        );
        if !connection.ready {
            connection.ready = true;
            // A replacement daemon has no document snapshots. Reopen mounted
            // buffers after negotiation, without replaying the original request.
            super::event(
                serde_json::json!({"k":"CodingEvent","workspace":request.workspace,"event":{"k":"LspTransportConnected"}}),
            );
        }
        Ok(reply)
    })();
    if result.is_err() {
        if let Ok(mut map) = connections.lock() {
            if map
                .get(id)
                .is_some_and(|current| Arc::ptr_eq(current, &connection))
            {
                map.remove(id);
            }
        }
    }
    // Never retry a failed write: its outcome may be unknown.
    let reply = result.map_err(|e| format!("{e:#}"))?;
    // An application error does not invalidate a healthy language connection.
    if let Some(error) = reply.get("error").and_then(Value::as_str) {
        return Err(error.to_owned());
    }
    Ok(reply.get("result").cloned().unwrap_or(Value::Null))
}

pub(super) fn shutdown() {
    if let Some(connections) = CONNECTIONS.get() {
        if let Ok(mut connections) = connections.lock() {
            connections.clear();
        }
    }
}
