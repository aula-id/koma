//! `koma host-agent` — one process per remote host.
//!
//! SSH starts this once (`bash -ilc` once). It multiplexes control, the
//! session-daemon bridge, fs/git/linker, and ptys over that single stdio.
//! Agent exit closes the tunnel only. It never sends QuitDaemon.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;

use crate::ipc::frame::{self, FrameReader};

const CH_CONTROL: u8 = 1;
const CH_SESSION: u8 = 2;
const CH_FS: u8 = 3;
const CH_GIT: u8 = 4;
const CH_LINKER: u8 = 5;
const CH_PTY: u8 = 6;
const PTY_DATA: u8 = 1;
const PTY_RESIZE: u8 = 3;
const PTY_CLOSE: u8 = 4;
const PTY_EXIT: u8 = 5;

fn encode(channel: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + body.len());
    out.push(channel);
    out.extend_from_slice(body);
    out
}
fn decode(frame: &[u8]) -> Option<(u8, &[u8])> {
    let (ch, body) = frame.split_first()?;
    Some((*ch, body))
}
fn encode_pty(id: &str, op: u8, payload: &[u8]) -> Vec<u8> {
    let id_bytes = id.as_bytes();
    let len = id_bytes.len().min(255);
    let mut out = Vec::with_capacity(2 + len + payload.len());
    out.push(len as u8);
    out.extend_from_slice(&id_bytes[..len]);
    out.push(op);
    out.extend_from_slice(payload);
    out
}
fn decode_pty(body: &[u8]) -> Option<(&str, u8, &[u8])> {
    let len = *body.first()? as usize;
    if body.len() < 2 + len {
        return None;
    }
    let id = std::str::from_utf8(&body[1..1 + len]).ok()?;
    let op = body[1 + len];
    Some((id, op, &body[2 + len..]))
}
fn decode_resize(payload: &[u8]) -> Option<(u16, u16)> {
    if payload.len() < 4 {
        return None;
    }
    Some((
        u16::from_be_bytes([payload[0], payload[1]]),
        u16::from_be_bytes([payload[2], payload[3]]),
    ))
}

/// Entry point for `koma host-agent`.
pub fn run_host_agent(_opts: crate::cli::Opts) -> Result<()> {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(serve())
}

async fn serve() -> Result<()> {
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let writer = out_tx.clone();
    tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(frame) = out_rx.recv().await {
            if frame::write_frame_to(&mut stdout, &frame).await.is_err() {
                break;
            }
        }
    });

    let hello = serde_json::json!({"op":"hello","v":1});
    let _ = writer.send(encode(CH_CONTROL, &serde_json::to_vec(&hello)?));

    let mut stdin = tokio::io::stdin();
    let mut reader = FrameReader::new();
    let mut pipes: HashMap<u8, mpsc::UnboundedSender<Vec<u8>>> = HashMap::new();
    let mut session_in: Option<mpsc::UnboundedSender<Vec<u8>>> = None;
    let ptys: Arc<Mutex<HashMap<String, PtySlot>>> = Arc::new(Mutex::new(HashMap::new()));

    loop {
        let frame = match frame::read_frame_from(&mut stdin, &mut reader).await {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        let Some((ch, body)) = decode(&frame) else {
            continue;
        };
        match ch {
            CH_CONTROL => {
                handle_control(
                    body,
                    &writer,
                    &mut session_in,
                    &ptys,
                )
                .await;
            }
            CH_FS | CH_GIT | CH_LINKER => {
                if let Some(tx) = ensure_service(&mut pipes, ch, &writer).await {
                    let _ = tx.send(body.to_vec());
                }
            }
            CH_SESSION => {
                if let Some(tx) = &session_in {
                    let _ = tx.send(body.to_vec());
                }
            }
            CH_PTY => handle_pty_in(body, &ptys, &writer),
            _ => {}
        }
    }
}

async fn handle_control(
    body: &[u8],
    out: &mpsc::UnboundedSender<Vec<u8>>,
    session_in: &mut Option<mpsc::UnboundedSender<Vec<u8>>>,
    ptys: &Arc<Mutex<HashMap<String, PtySlot>>>,
) {
    let value: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => {
            reply_err(out, &e.to_string());
            return;
        }
    };
    let op = value.get("op").and_then(|v| v.as_str()).unwrap_or("");
    match op {
        "listSessions" => {
            let json = list_sessions_local();
            let rep = serde_json::json!({"op":"sessions","json": json});
            send_control(out, &rep);
        }
        "expandHome" => {
            let path = value.get("path").and_then(|v| v.as_str()).unwrap_or("~");
            let expanded = expand_home_local(path);
            let rep = serde_json::json!({"op":"expanded","path": expanded});
            send_control(out, &rep);
        }
        "listDirs" => {
            let path = value.get("path").and_then(|v| v.as_str()).unwrap_or("/");
            let path = expand_home_local(path);
            match list_dirs_local(&path) {
                Ok(dirs) => {
                    let rep = serde_json::json!({"op":"dirs","path": path, "dirs": dirs});
                    send_control(out, &rep);
                }
                Err(e) => reply_err(out, &e),
            }
        }
        "openSession" => {
            let id = value.get("id").and_then(|v| v.as_str()).unwrap_or("");
            if id.is_empty() || id.contains('\0') {
                reply_err(out, "invalid session id");
                return;
            }
            let cwd = value
                .get("cwd")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            match open_session(id, cwd.as_deref(), out).await {
                Ok(tx) => {
                    *session_in = Some(tx);
                    let rep = serde_json::json!({"op":"session","id": id, "ok": true});
                    send_control(out, &rep);
                }
                Err(e) => reply_err(out, &format!("{e:#}")),
            }
        }
        "ptyOpen" => {
            let id = value.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let cwd = value.get("cwd").and_then(|v| v.as_str()).map(str::to_string);
            if id.is_empty() {
                reply_err(out, "invalid pty id");
                return;
            }
            if let Err(e) = open_pty(id, cwd.as_deref(), ptys, out) {
                reply_err(out, &format!("{e:#}"));
            }
        }
        _ => reply_err(out, &format!("unknown control op {op}")),
    }
}

fn send_control(out: &mpsc::UnboundedSender<Vec<u8>>, value: &serde_json::Value) {
    if let Ok(bytes) = serde_json::to_vec(value) {
        let _ = out.send(encode(CH_CONTROL, &bytes));
    }
}

fn reply_err(out: &mpsc::UnboundedSender<Vec<u8>>, error: &str) {
    send_control(out, &serde_json::json!({"op":"err","error": error}));
}

fn list_sessions_local() -> String {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => return format!("{{\"error\":\"{e}\"}}"),
    };
    match std::process::Command::new(exe)
        .arg("sessions")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    {
        Ok(out) => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        Err(e) => format!("{{\"error\":\"{e}\"}}"),
    }
}

fn expand_home_local(path: &str) -> String {
    let path = path.trim();
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    if path.is_empty() || path == "~" {
        return home;
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return format!(
            "{}/{}",
            home.trim_end_matches('/'),
            rest.trim_start_matches('/')
        );
    }
    path.to_string()
}

fn list_dirs_local(path: &str) -> Result<Vec<String>, String> {
    const MAX_DIRS: usize = 200;
    let dir = std::fs::read_dir(path).map_err(|e| format!("{e}"))?;
    let mut dirs = Vec::new();
    for entry in dir.flatten() {
        if dirs.len() >= MAX_DIRS {
            break;
        }
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if ft.is_dir() {
            if let Some(s) = entry.path().to_str() {
                dirs.push(s.to_string());
            }
        }
    }
    dirs.sort();
    Ok(dirs)
}

async fn open_session(
    id: &str,
    cwd: Option<&str>,
    out: &mpsc::UnboundedSender<Vec<u8>>,
) -> Result<mpsc::UnboundedSender<Vec<u8>>> {
    let (local, remote) = tokio::io::duplex(1024 * 1024);
    let (bridge_r, bridge_w) = tokio::io::split(remote);
    let id_owned = id.to_string();
    let cwd_owned = cwd.map(Path::new).map(|p| p.to_path_buf());
    tokio::spawn(async move {
        let _ = super::stdio_bridge::run_stdio_sock_bridge(
            &id_owned,
            cwd_owned.as_deref(),
            bridge_r,
            bridge_w,
        )
        .await;
    });

    let (in_tx, mut in_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let (mut read_half, mut write_half) = tokio::io::split(local);
    let out_frames = out.clone();
    tokio::spawn(async move {
        let mut reader = FrameReader::new();
        loop {
            tokio::select! {
                body = in_rx.recv() => {
                    let Some(body) = body else { break };
                    if frame::write_frame_to(&mut write_half, &body).await.is_err() {
                        break;
                    }
                }
                incoming = frame::read_frame_from(&mut read_half, &mut reader) => {
                    match incoming {
                        Ok(payload) => {
                            if out_frames.send(encode(CH_SESSION, &payload)).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            }
        }
    });
    Ok(in_tx)
}

struct PtySlot {
    writer: Box<dyn std::io::Write + Send>,
    master: Box<dyn portable_pty::MasterPty + Send>,
}

fn open_pty(
    id: &str,
    cwd: Option<&str>,
    ptys: &Arc<Mutex<HashMap<String, PtySlot>>>,
    out: &mpsc::UnboundedSender<Vec<u8>>,
) -> Result<()> {
    let shell = if cfg!(target_os = "windows") {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
    };
    let mut cmd = portable_pty::CommandBuilder::new(shell);
    if !cfg!(target_os = "windows") {
        cmd.arg("-l");
    }
    if let Some(cwd) = cwd {
        cmd.cwd(cwd);
    }
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    let pair = portable_pty::native_pty_system().openpty(portable_pty::PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    })?;
    let child = pair.slave.spawn_command(cmd)?;
    let mut writer = pair.master.take_writer()?;
    writer.flush()?;
    let reader = pair.master.try_clone_reader()?;
    let id_owned = id.to_string();
    let out_frames = out.clone();
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 8192];
        loop {
            match std::io::Read::read(&mut reader, &mut buf) {
                Ok(0) | Err(_) => {
                    let _ = out_frames.send(encode(
                        CH_PTY,
                        &encode_pty(&id_owned, PTY_EXIT, &[]),
                    ));
                    break;
                }
                Ok(n) => {
                    let _ = out_frames.send(encode(
                        CH_PTY,
                        &encode_pty(&id_owned, PTY_DATA, &buf[..n]),
                    ));
                }
            }
        }
        drop(child);
    });
    if let Ok(mut g) = ptys.lock() {
        g.insert(
            id.to_string(),
            PtySlot {
                writer,
                master: pair.master,
            },
        );
    }
    Ok(())
}

fn handle_pty_in(
    body: &[u8],
    ptys: &Arc<Mutex<HashMap<String, PtySlot>>>,
    out: &mpsc::UnboundedSender<Vec<u8>>,
) {
    let Some((id, op, payload)) = decode_pty(body) else {
        return;
    };
    let mut g = match ptys.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    match op {
        PTY_DATA => {
            if let Some(slot) = g.get_mut(id) {
                let _ = slot.writer.write_all(payload);
                let _ = slot.writer.flush();
            }
        }
        PTY_RESIZE => {
            if let (Some(slot), Some((cols, rows))) = (g.get_mut(id), decode_resize(payload)) {
                let _ = slot.master.resize(portable_pty::PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                });
            }
        }
        PTY_CLOSE => {
            g.remove(id);
            let _ = out.send(encode(CH_PTY, &encode_pty(id, PTY_EXIT, &[])));
        }
        _ => {}
    }
}

async fn ensure_service(
    pipes: &mut HashMap<u8, mpsc::UnboundedSender<Vec<u8>>>,
    channel: u8,
    out: &mpsc::UnboundedSender<Vec<u8>>,
) -> Option<mpsc::UnboundedSender<Vec<u8>>> {
    if let Some(tx) = pipes.get(&channel) {
        return Some(tx.clone());
    }
    let argv: &[&str] = match channel {
        CH_FS => &["remote-fs"],
        CH_GIT => &["remote-git"],
        CH_LINKER => &["remote-linker"],
        _ => return None,
    };
    match spawn_service(argv, channel, out).await {
        Ok(tx) => {
            pipes.insert(channel, tx.clone());
            Some(tx)
        }
        Err(e) => {
            reply_err(out, &format!("service {} failed: {e:#}", argv[0]));
            None
        }
    }
}

async fn spawn_service(
    argv: &[&str],
    channel: u8,
    out: &mpsc::UnboundedSender<Vec<u8>>,
) -> Result<mpsc::UnboundedSender<Vec<u8>>> {
    let exe = std::env::current_exe()?;
    let mut cmd = Command::new(exe);
    cmd.args(argv)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child: Child = cmd.spawn()?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("service stdin missing"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("service stdout missing"))?;
    let (tx, rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let out_frames = out.clone();
    tokio::spawn(async move {
        let _child = child;
        service_bridge(stdin, stdout, rx, channel, out_frames).await;
    });
    Ok(tx)
}

async fn service_bridge(
    mut stdin: ChildStdin,
    mut stdout: tokio::process::ChildStdout,
    mut rx: mpsc::UnboundedReceiver<Vec<u8>>,
    channel: u8,
    out: mpsc::UnboundedSender<Vec<u8>>,
) {
    let mut reader = FrameReader::new();
    loop {
        tokio::select! {
            body = rx.recv() => {
                let Some(body) = body else { break };
                if frame::write_frame_to(&mut stdin, &body).await.is_err() {
                    break;
                }
            }
            incoming = frame::read_frame_from(&mut stdout, &mut reader) => {
                match incoming {
                    Ok(payload) => {
                        if out.send(encode(channel, &payload)).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        }
    }
    let _ = stdin.shutdown().await;
}
