//! `koma run` — thin headless client on the **default session-daemon** path.
//!
//! ```text
//! ensure+attach  →  [optional setup]  →  SubmitInput  →  [optional --once wait]  →  Detach
//! ```
//!
//! Optional setup (before submit): `--security`, `--model` (Main only), `--effort`, `--mode`.
//! No standalone/`--local`. Reuses [`attach_session_headless`].
//!
//! Exit: 0 ok · 1 error · 2 `--once` timeout · 3 approval-parked

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

use crate::app::resolve::find_model_entry_by_slug;
use crate::app::runtime::client::connect::{attach_session_headless, Connection};
use crate::cli::RunCli;
use crate::ipc::proto::{ClientRequest, DaemonEvent};
use crate::model::app_config::AppConfig;
use crate::model::settings::Settings;

const EXIT_OK: i32 = 0;
const EXIT_ERR: i32 = 1;
const EXIT_TIMEOUT: i32 = 2;
const EXIT_APPROVAL: i32 = 3;

/// `main` entry.
pub fn run_cli(cli: RunCli) -> i32 {
    match run_inner(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            EXIT_ERR
        }
    }
}

fn run_inner(cli: RunCli) -> Result<i32> {
    let prompt = resolve_prompt(&cli)?;
    if prompt.trim().is_empty() {
        bail!("prompt is empty");
    }
    let workdir = resolve_workdir(cli.workdir.as_deref())?;
    let session_id = cli
        .session
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    if let Some(name) = cli.name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let _ = crate::model::session_registry::set_name(&session_id, name);
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("tokio runtime")?;
    let handle = rt.handle().clone();

    let conn = attach_session_headless(&handle, &session_id, workdir.as_deref())
        .with_context(|| format!("attach session {session_id}"))?;

    if let Some(name) = cli.name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let _ = conn.req_tx.send(ClientRequest::RenameSession {
            name: name.to_string(),
        });
    }

    // Brief drain so Attach snapshot lands before setup/submit (non-fatal if slow).
    let mut working = false;
    let mut awaiting_approval = false;
    drain_snapshot(
        &conn,
        Duration::from_secs(5),
        &mut working,
        &mut awaiting_approval,
    );

    if awaiting_approval {
        eprintln!("session awaiting approval; not submitting");
        finish(&conn, &rt);
        return Ok(EXIT_APPROVAL);
    }

    apply_run_setup(&conn, &cli)?;

    // Let daemon apply setup before the turn starts.
    std::thread::sleep(Duration::from_millis(150));
    drain_quiet(&conn, Duration::from_millis(400));

    conn.req_tx
        .send(ClientRequest::SubmitInput { text: prompt })
        .context("SubmitInput")?;

    println!("session_id={session_id}");
    if let Some(n) = cli.name.as_deref() {
        println!("name={n}");
    }
    if let Some(ref w) = workdir {
        println!("workdir={}", w.display());
    }
    if let Some(ref m) = cli.model {
        println!("model={m}");
    }
    if let Some(ref e) = cli.effort {
        println!("effort={e}");
    }
    if let Some(ref m) = cli.mode {
        println!("mode={m}");
    }
    if let Some(s) = cli.security {
        println!("security={}", if s { "on" } else { "off" });
    }

    if !cli.once {
        finish(&conn, &rt);
        return Ok(EXIT_OK);
    }

    let code = wait_once(&conn, Duration::from_secs(cli.timeout_sec.max(1)))?;
    finish(&conn, &rt);
    Ok(code)
}

/// Apply optional `--security` / `--model` / `--effort` / `--mode` before submit.
///
/// Order: security → model → effort → mode (yolo arms after security is up).
fn apply_run_setup(conn: &Connection, cli: &RunCli) -> Result<()> {
    let mode_l = cli
        .mode
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase());
    let want_yolo = mode_l.as_deref() == Some("yolo");

    // Security: explicit flag, or auto-on when entering yolo.
    let security_on = match cli.security {
        Some(v) => Some(v),
        None if want_yolo => Some(true),
        None => None,
    };
    if let Some(enabled) = security_on {
        conn.req_tx
            .send(ClientRequest::SetSecurityEnabled { enabled })
            .context("SetSecurityEnabled")?;
        if enabled {
            // Give the sec manager a moment to come up before arming yolo.
            std::thread::sleep(Duration::from_millis(400));
            drain_quiet(conn, Duration::from_millis(200));
        }
    }

    if let Some(raw) = cli.model.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let uuid = resolve_model_ref(raw)
            .with_context(|| format!("--model {raw}: no matching catalogue entry"))?;
        conn.req_tx
            .send(ClientRequest::SetSessionMain {
                model_uuid: Some(uuid),
            })
            .context("SetSessionMain")?;
    }

    if let Some(effort) = cli
        .effort
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        conn.req_tx
            .send(ClientRequest::SetEffort {
                effort: effort.to_string(),
            })
            .context("SetEffort")?;
    }

    if let Some(mode) = mode_l {
        if mode == "yolo" {
            // Arm Layer-1 YOLO (refused daemon-side if security not running).
            conn.req_tx
                .send(ClientRequest::SetYoloArmed { armed: true })
                .context("SetYoloArmed")?;
            std::thread::sleep(Duration::from_millis(100));
        }
        conn.req_tx
            .send(ClientRequest::SetMode { mode })
            .context("SetMode")?;
    }

    Ok(())
}

/// Resolve `--model` the same way agent/manifest `model:` slugs do:
/// [`find_model_entry_by_slug`] — case-insensitive match on `model_id` | `name` | `uuid`.
///
/// Returns the **global catalogue uuid** `SetSessionMain` expects (clones that entry,
/// including its `provider_uuid` / router). No new identifier formats.
fn resolve_model_ref(raw: &str) -> Result<String> {
    let cfg = AppConfig::load();
    // Fresh headless attach: no session overrides yet; empty settings matches spawn-time
    // slug resolution with `preferred_provider_uuids: None`.
    let settings = Settings::default();
    let needle = raw.trim();
    if needle.is_empty() {
        bail!("empty model ref");
    }
    let entry = find_model_entry_by_slug(&cfg, &settings, needle, None).ok_or_else(|| {
        anyhow::anyhow!(
            "no catalogue match for {needle:?} (slug = model_id | name | uuid, same as agent model:)"
        )
    })?;
    // SetSessionMain looks up the GLOBAL catalogue by uuid. Prefer source_uuid when
    // present (session clone pointing at a global); globals have source_uuid = None.
    Ok(entry
        .source_uuid
        .clone()
        .unwrap_or_else(|| entry.uuid.clone()))
}

fn finish(conn: &Connection, rt: &tokio::runtime::Runtime) {
    let _ = conn.req_tx.send(ClientRequest::Detach);
    // Let writer flush Detach before dropping the runtime (kills writer task).
    std::thread::sleep(Duration::from_millis(50));
    let _ = rt;
}

fn resolve_prompt(cli: &RunCli) -> Result<String> {
    match (&cli.prompt, &cli.prompt_file) {
        (Some(p), None) => Ok(p.clone()),
        (None, Some(path)) => {
            let path = expand_user(path);
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))
        }
        (Some(_), Some(_)) => bail!("pass only one of --prompt or --prompt-file"),
        (None, None) => bail!("missing --prompt or --prompt-file"),
    }
}

fn resolve_workdir(raw: Option<&str>) -> Result<Option<PathBuf>> {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let path = expand_user(raw);
    let canon = path
        .canonicalize()
        .with_context(|| format!("workdir missing: {}", path.display()))?;
    if !canon.is_dir() {
        bail!("workdir not a directory: {}", canon.display());
    }
    Ok(Some(canon))
}

fn expand_user(p: &str) -> PathBuf {
    if p == "~" {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(p));
    }
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(p)
}

fn note_frame(ev: &DaemonEvent, working: &mut bool, awaiting_approval: &mut bool) {
    match ev {
        DaemonEvent::Snapshot(state) => {
            let s = state
                .foreground_id
                .as_ref()
                .and_then(|fg| state.sessions.iter().find(|s| s.id == *fg))
                .or_else(|| state.sessions.first());
            if let Some(s) = s {
                *working = s.working || s.waiting;
                *awaiting_approval = s.awaiting_approval;
            }
        }
        DaemonEvent::Delta(crate::ipc::proto::StateDelta::SessionStatusChanged {
            working: w,
            ..
        }) => {
            *working = *w;
        }
        DaemonEvent::Status(st) => {
            *working = st.working;
        }
        _ => {}
    }
}

fn drain_snapshot(
    conn: &Connection,
    budget: Duration,
    working: &mut bool,
    awaiting_approval: &mut bool,
) {
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        match conn.frame_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(frame) => {
                let is_snap = matches!(frame.event, DaemonEvent::Snapshot(_));
                note_frame(&frame.event, working, awaiting_approval);
                if is_snap {
                    return;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn drain_quiet(conn: &Connection, budget: Duration) {
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        match conn.frame_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Wait until busy→idle (or timeout / approval). Polls `Status` as a heartbeat.
fn wait_once(conn: &Connection, timeout: Duration) -> Result<i32> {
    let deadline = Instant::now() + timeout;
    let grace = Instant::now() + Duration::from_secs(2);
    let mut working = false;
    let mut saw_busy = false;
    let mut awaiting_approval = false;
    let mut last_poll = Instant::now()
        .checked_sub(Duration::from_secs(2))
        .unwrap_or_else(Instant::now);

    loop {
        if awaiting_approval {
            eprintln!("parked on approval (daemon still running)");
            return Ok(EXIT_APPROVAL);
        }
        if saw_busy && !working {
            println!("status=idle");
            return Ok(EXIT_OK);
        }
        // Fast-finish / never-started after short grace.
        if !saw_busy && !working && Instant::now() >= grace {
            println!("status=idle");
            return Ok(EXIT_OK);
        }
        if Instant::now() >= deadline {
            eprintln!("timeout (daemon still running)");
            return Ok(EXIT_TIMEOUT);
        }

        if last_poll.elapsed() >= Duration::from_secs(1) {
            let _ = conn.req_tx.send(ClientRequest::Status);
            last_poll = Instant::now();
        }

        match conn.frame_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(frame) => {
                note_frame(&frame.event, &mut working, &mut awaiting_approval);
                if working {
                    saw_busy = true;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                bail!("daemon disconnected while waiting");
            }
        }
    }
}

#[cfg(test)]
mod resolve_model_tests {
    use super::*;

    #[test]
    fn empty_ref_errors() {
        let err = resolve_model_ref("   ").unwrap_err();
        assert!(err.to_string().contains("empty"));
    }
}
