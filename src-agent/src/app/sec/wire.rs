//! Child-process wire protocol for the security daemon: spawn + handshake, the
//! writer/reader/stderr-drain tasks, and their small parsing/bookkeeping
//! helpers. Split out of [`super`] (the `sec` module) for file size — pure
//! code motion, no behaviour change. The whole module stays under the
//! parent's `#![allow(dead_code)]` umbrella (mirrored here) until the
//! tool-system/`/security` cockpit wiring lands.
//!
//! [`Connected`] (+ its fields) and the four task-entry functions
//! (`spawn_and_handshake`, `writer_task`, `reader_task`, `stderr_drain_task`)
//! are bumped to `pub(super)` — [`super::SecDaemonManager::start`] destructures
//! `Connected` and spawns all three tasks directly. `parse_tools` and
//! `mark_stopped` stay private (only called from within this file).
#![allow(dead_code)]

use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use super::{PendingMap, SecDaemonManager, SecToolDesc};

/// The product of a successful spawn + handshake: the live child, its split I/O
/// halves, and the tools it advertised. `stdout` is a `BufReader` wrapping the raw
/// child stdout so any bytes already buffered during the handshake read are not lost
/// when the reader task takes over.
pub(super) struct Connected {
    pub(super) child: tokio::process::Child,
    pub(super) stdin: tokio::process::ChildStdin,
    pub(super) stdout: BufReader<tokio::process::ChildStdout>,
    pub(super) stderr: tokio::process::ChildStderr,
    pub(super) tools: Vec<SecToolDesc>,
}

/// Spawn `python -m koma_sec_daemon --token <token>` (stdio piped), perform the
/// newline-delimited-JSON handshake, and return the live child + I/O + advertised
/// tools.
///
/// The handshake writes `{"v":1,"token":"<token>"}\n` to the child's stdin, then
/// reads the FIRST line of its stdout, which must parse as `{"ok":true,"tools":[…]}`.
/// Any spawn/IO/parse failure — or an `{"ok":false}` reply — is returned as `Err`.
pub(super) async fn spawn_and_handshake(token: &str) -> Result<Connected, String> {
    let python = crate::security::venv_python()
        .map_err(|e| format!("cannot locate security venv python: {e}"))?;
    let dir = crate::security::security_dir()
        .map_err(|e| format!("cannot locate security dir: {e}"))?;

    let mut cmd = tokio::process::Command::new(&python);
    cmd.arg("-m")
        .arg("koma_sec_daemon")
        .arg("--token")
        .arg(token)
        .current_dir(&dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    // No console flash on Windows — see `tool::shell::no_console_window_tokio`'s
    // docs for the `FreeConsole()` causal chain this guards against.
    crate::tool::shell::no_console_window_tokio(&mut cmd);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn koma_sec_daemon failed: {e}"))?;

    // Take ownership of the piped I/O halves. All three were requested above, so
    // these are present; a missing handle is a hard error (child gets dropped/reaped).
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "child stdin not piped".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "child stdout not piped".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "child stderr not piped".to_string())?;

    // Write the handshake frame.
    let hello = serde_json::json!({ "v": 1, "token": token }).to_string();
    stdin
        .write_all(format!("{hello}\n").as_bytes())
        .await
        .map_err(|e| format!("handshake write failed: {e}"))?;
    stdin
        .flush()
        .await
        .map_err(|e| format!("handshake flush failed: {e}"))?;

    // Read the first stdout line = the handshake reply.
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let n = reader
        .read_line(&mut line)
        .await
        .map_err(|e| format!("handshake read failed: {e}"))?;
    if n == 0 {
        return Err("daemon closed stdout before handshake reply".to_string());
    }

    let reply: serde_json::Value = serde_json::from_str(line.trim())
        .map_err(|e| format!("handshake reply was not valid JSON: {e}"))?;

    if reply.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        let err = reply
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("handshake rejected");
        return Err(format!("daemon rejected handshake: {err}"));
    }

    let tools = parse_tools(reply.get("tools"));

    // Pass the SAME BufReader into Connected so the reader task continues from where
    // the handshake left off. Using into_inner() here would DISCARD any bytes the
    // BufReader already buffered past the first line — currently safe only because the
    // daemon never sends a second line unsolicited, but wrong in principle.
    Ok(Connected {
        child,
        stdin,
        stdout: reader,
        stderr,
        tools,
    })
}

/// Parse the `tools` array from a handshake reply into [`SecToolDesc`]s. A missing or
/// non-array value yields an empty list; malformed entries degrade field-by-field
/// (missing strings → empty, missing `risk` → false) rather than failing the whole
/// handshake.
fn parse_tools(value: Option<&serde_json::Value>) -> Vec<SecToolDesc> {
    let arr = match value.and_then(serde_json::Value::as_array) {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .filter_map(|t| {
            // A tool with no name is unusable; skip it.
            let name = t.get("name").and_then(serde_json::Value::as_str)?.to_string();
            Some(SecToolDesc {
                name,
                description: t
                    .get("description")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                parameters: t
                    .get("parameters")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({})),
                risk: t
                    .get("risk")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                compute: t
                    .get("compute")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                domain: t
                    .get("domain")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect()
}

/// Owns the child's stdin and forwards every frame received on `rx` to it. Exits when
/// the channel closes (all senders dropped — i.e. [`SecDaemonManager::stop`] cleared
/// the `writer`) or on the first write error (child gone), closing stdin on the way
/// out so the child observes EOF.
pub(super) async fn writer_task(mut stdin: tokio::process::ChildStdin, mut rx: mpsc::UnboundedReceiver<String>) {
    while let Some(frame) = rx.recv().await {
        if stdin.write_all(frame.as_bytes()).await.is_err() {
            break;
        }
        if stdin.flush().await.is_err() {
            break;
        }
    }
    // Explicitly close stdin so the child sees EOF promptly (drop would do this too).
    let _ = stdin.shutdown().await;
}

/// Mark the daemon as stopped, but ONLY if the generation still matches the one this
/// reader was started under. This prevents a fresh `start()` — which would have bumped
/// the generation — from being clobbered by an EOF event from the previous child's
/// reader task racing the new start.
///
/// On match: sets `running = false`, clears `tools`, and drops `writer` (closing the
/// writer task and therefore stdin). The lock is held synchronously and dropped
/// before returning; no `.await` occurs here.
fn mark_stopped(mgr: &SecDaemonManager, gen: u64) {
    let mut guard = mgr.inner.lock().unwrap_or_else(|p| p.into_inner());
    if guard.generation != gen {
        // A stop/restart already superseded this child; leave the new state alone.
        return;
    }
    guard.running = false;
    guard.tools.clear();
    // Drop the writer sender → writer task ends → stdin closes → child sees EOF.
    guard.writer = None;
}

/// Owns the child's stdout and dispatches replies. Each line is parsed as a JSON
/// frame; a frame carrying an `id` fulfils (and removes) the matching `oneshot` from
/// `pending` with `Ok(result)` / `Err(error)`. Frames without an `id` (or for an
/// unknown id) are logged and ignored. Exits when stdout reaches EOF (child gone),
/// at which point every still-pending caller is failed so none hang.
///
/// `mgr` and `gen` are used to mark the daemon stopped on EOF (generation-guarded
/// so a superseding `start()` is never clobbered by this stale reader).
pub(super) async fn reader_task(
    stdout: BufReader<tokio::process::ChildStdout>,
    pending: PendingMap,
    mgr: Arc<SecDaemonManager>,
    gen: u64,
) {
    // `stdout` is the same BufReader used for the handshake read, so any bytes
    // buffered past the handshake line are not lost.
    let mut lines = stdout.lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let frame: serde_json::Value = match serde_json::from_str(trimmed) {
                    Ok(v) => v,
                    Err(e) => {
                        crate::model::store::append_global_error_log(
                            "security",
                            &format!("ignoring unparseable daemon frame: {e}"),
                        );
                        continue;
                    }
                };
                // Only id-bearing frames map to a pending call.
                let id = match frame.get("id").and_then(serde_json::Value::as_u64) {
                    Some(id) => id,
                    None => {
                        // Non-reply frame (e.g. a log/event); nothing to fulfil.
                        continue;
                    }
                };
                let tx = {
                    let mut map = pending.lock().unwrap_or_else(|p| p.into_inner());
                    map.remove(&id)
                };
                let Some(tx) = tx else {
                    // No waiter for this id (timed out + removed, or duplicate reply).
                    continue;
                };
                let result = if frame.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
                    Ok(frame
                        .get("result")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .to_string())
                } else {
                    Err(frame
                        .get("error")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("tool reported an error")
                        .to_string())
                };
                // The receiver may have gone away (caller timed out between our remove
                // and now); ignore the send error in that case.
                let _ = tx.send(result);
            }
            // EOF or read error: the child closed stdout. Stop reading and fail any
            // stragglers, then mark the manager stopped so stale tools stop advertising.
            Ok(None) => break,
            Err(e) => {
                crate::model::store::append_global_error_log(
                    "security",
                    &format!("error reading daemon stdout: {e}"),
                );
                break;
            }
        }
    }

    // Child is gone: fail every remaining waiter so no execute_blocking hangs out the
    // full CALL_TIMEOUT.
    {
        let mut map = pending.lock().unwrap_or_else(|p| p.into_inner());
        for (_id, tx) in map.drain() {
            let _ = tx.send(Err("security daemon closed connection".to_string()));
        }
    }

    // Flip the manager to stopped (generation-guarded) so is_running() returns false
    // and tool_names()/tool_defs() stop advertising dead tools after a daemon crash.
    mark_stopped(&mgr, gen);
}

/// Reads the child's stderr to EOF and discards it, so a chatty child never blocks on
/// a full stderr pipe. Errors are ignored — this is a best-effort drain.
pub(super) async fn stderr_drain_task(mut stderr: tokio::process::ChildStderr) {
    let mut buf = Vec::new();
    let _ = stderr.read_to_end(&mut buf).await;
}
