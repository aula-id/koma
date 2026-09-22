//! `koma run` — thin headless client on the **default session-daemon** path.
//!
//! ```text
//! ensure+attach  →  [optional setup]  →  SubmitInput  →  [optional --once wait]  →  Detach
//! ```
//!
//! Setup covers model/mode, current DRSS settings, and session-local extensions.
//! Every setup request is checked against daemon readback before submitting.
//! `--status --session ID` inspects/configures a session without an inference call.
//! No standalone/`--local`. Reuses [`attach_session_headless`].
//!
//! Exit: 0 ok · 1 error · 2 `--once` timeout · 3 approval-parked

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

use crate::app::resolve::find_model_entry_by_slug;
use crate::app::runtime::client::connect::{attach_session_headless, Connection};
use crate::cli::RunCli;
use crate::ipc::proto::{ClientRequest, DaemonEvent, RunState};
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
    if let Some(error) = &cli.error {
        bail!("{error}");
    }
    let prompt = if cli.status {
        anyhow::ensure!(
            cli.prompt.is_none() && cli.prompt_file.is_none(),
            "--status does not submit a prompt"
        );
        anyhow::ensure!(
            cli.session.as_deref().is_some_and(|s| !s.trim().is_empty()),
            "--status requires --session ID"
        );
        anyhow::ensure!(!cli.once, "--status cannot be combined with --once");
        None
    } else {
        let prompt = resolve_prompt(&cli)?;
        anyhow::ensure!(!prompt.trim().is_empty(), "prompt is empty");
        Some(prompt)
    };
    let system_extra = resolve_system(&cli)?;
    let workdir = resolve_workdir(cli.workdir.as_deref())?;
    let session_id = cli
        .session
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if cli.status {
        anyhow::ensure!(
            crate::model::session_registry::get(&session_id)?.is_some(),
            "session '{session_id}' does not exist"
        );
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("tokio runtime")?;
    let conn = attach_session_headless(&rt.handle().clone(), &session_id, workdir.as_deref())
        .with_context(|| format!("attach session {session_id}"))?;
    // Always detach, including a rejected setup. Never abandon a submitted daemon turn.
    let result = run_attached(&conn, &cli, prompt, system_extra);
    finish(&conn, &rt);
    result
}

fn has_setup(cli: &RunCli, system_extra: &Option<String>) -> bool {
    cli.name.is_some()
        || cli.model.is_some()
        || cli.effort.is_some()
        || cli.mode.is_some()
        || cli.security.is_some()
        || cli.max_tokens.is_some()
        || cli.short_send.is_some()
        || cli.context_window_limit.is_some()
        || cli.context_model_alias.is_some()
        || system_extra.is_some()
        || !cli.extensions.is_empty()
        || !cli.unload_extensions.is_empty()
}

fn run_attached(
    conn: &Connection,
    cli: &RunCli,
    prompt: Option<String>,
    system_extra: Option<String>,
) -> Result<i32> {
    let mut state = request_state(conn, Duration::from_secs(10))?;
    if state.awaiting_approval && (prompt.is_some() || has_setup(cli, &system_extra)) {
        eprintln!("session awaiting approval; not submitting");
        return Ok(EXIT_APPROVAL);
    }
    if has_setup(cli, &system_extra) {
        anyhow::ensure!(
            !state.working,
            "session is working; finish the current turn before changing run settings"
        );
        state = apply_run_setup(conn, cli, state, system_extra.as_deref())?;
    }
    if let Some(prompt) = prompt {
        conn.req_tx
            .send(ClientRequest::SubmitInput { text: prompt })
            .context("SubmitInput")?;
        // Ordered readback also observes any daemon rejection before reporting success.
        state = request_state(conn, Duration::from_secs(10))?;
        print_state(&state, false);
        if cli.once {
            return wait_once(conn, Duration::from_secs(cli.timeout_sec.max(1)), state);
        }
    } else {
        print_state(&state, true);
    }
    Ok(EXIT_OK)
}

fn apply_run_setup(
    conn: &Connection,
    cli: &RunCli,
    mut state: RunState,
    system_extra: Option<&str>,
) -> Result<RunState> {
    // Resolve/check the entire requested selection before changing any settings.
    for id in cli.extensions.iter().chain(&cli.unload_extensions) {
        let ext = state
            .extensions
            .iter()
            .find(|e| &e.id == id)
            .ok_or_else(|| anyhow::anyhow!("extension '{id}' is not installed"))?;
        if cli.extensions.contains(id) {
            anyhow::ensure!(ext.enabled, "extension '{id}' is disabled");
        } else {
            anyhow::ensure!(
                ext.activation != "global",
                "extension '{id}' is global; set it to on-demand in /extension first"
            );
        }
    }
    let model = cli.model.as_deref().map(resolve_model_ref).transpose()?;
    let mode = cli.mode.as_deref();
    if let Some(name) = &cli.name {
        state = apply_request(conn, ClientRequest::RenameSession { name: name.clone() })?;
        anyhow::ensure!(
            state.name == name.trim(),
            "daemon did not apply the requested session name"
        );
    }
    if let Some(enabled) = cli
        .security
        .or_else(|| (mode == Some("yolo")).then_some(true))
    {
        state = apply_request(conn, ClientRequest::SetSecurityEnabled { enabled })?;
        if enabled {
            state = wait_for_state(conn, state, "security daemon startup", |s| {
                s.security_running
            })?;
        }
        anyhow::ensure!(
            state.security_enabled == enabled,
            "daemon did not apply security setting"
        );
    }
    if let Some(uuid) = model {
        let expected = AppConfig::load()
            .models
            .iter()
            .find(|m| m.uuid == uuid)
            .map(|m| m.model_id.clone());
        state = apply_request(
            conn,
            ClientRequest::SetSessionMain {
                model_uuid: Some(uuid),
            },
        )?;
        anyhow::ensure!(
            expected.as_deref() == Some(state.model.as_str()),
            "daemon did not apply the requested model"
        );
    }
    if let Some(effort) = &cli.effort {
        let effort = effort.trim();
        state = apply_request(
            conn,
            ClientRequest::SetEffort {
                effort: effort.into(),
            },
        )?;
        anyhow::ensure!(
            state.effort == if effort == "default" { "" } else { effort },
            "daemon did not apply the requested effort"
        );
    }
    if let Some(mode) = mode {
        if mode == "yolo" {
            state = apply_request(conn, ClientRequest::SetYoloArmed { armed: true })?;
            anyhow::ensure!(state.yolo_armed, "daemon refused to arm yolo");
        }
        state = apply_request(conn, ClientRequest::SetMode { mode: mode.into() })?;
        anyhow::ensure!(
            state.mode == mode,
            "daemon did not enter requested mode '{mode}' (current: {})",
            state.mode
        );
    }
    if let Some(text) = system_extra {
        state = apply_request(
            conn,
            ClientRequest::SetSessionSystem {
                text: text.to_string(),
            },
        )?;
        anyhow::ensure!(
            state.system_extra,
            "daemon did not apply session system text"
        );
    }
    if cli.max_tokens.is_some()
        || cli.short_send.is_some()
        || cli.context_window_limit.is_some()
        || cli.context_model_alias.is_some()
    {
        state = apply_request(
            conn,
            ClientRequest::SetSessionPrefs {
                short_send: cli.short_send,
                sliding_cache: None,
                bash_saving: None,
                coding_autosave: None,
                internet_mode: None,
                workdir: None,
                subagent_max_turns: None,
                short_send_engage_n: None,
                short_send_tail_n: None,
                max_output_tokens: cli.max_tokens,
                context_window_limit: cli.context_window_limit,
                context_model_alias: cli.context_model_alias.clone(),
            },
        )?;
        anyhow::ensure!(
            cli.short_send.is_none_or(|v| state.short_send == v),
            "daemon did not apply short-send setting"
        );
        anyhow::ensure!(
            cli.max_tokens.is_none_or(|v| state.max_output_tokens == v),
            "daemon did not apply reply limit"
        );
        anyhow::ensure!(
            cli.context_window_limit
                .is_none_or(|v| state.context_window_limit == v),
            "daemon did not apply context limit"
        );
        anyhow::ensure!(
            cli.context_model_alias
                .as_ref()
                .is_none_or(|v| &state.context_model_alias == v),
            "daemon did not apply context alias"
        );
    }
    if !cli.extensions.is_empty() || !cli.unload_extensions.is_empty() {
        state = apply_request(
            conn,
            ClientRequest::SetSessionExtensions {
                load: cli.extensions.clone(),
                unload: cli.unload_extensions.clone(),
            },
        )?;
        for id in &cli.extensions {
            anyhow::ensure!(
                state.extensions.iter().any(|e| &e.id == id && e.active),
                "daemon did not activate extension '{id}'"
            );
        }
        for id in &cli.unload_extensions {
            anyhow::ensure!(
                !state.extensions.iter().any(|e| &e.id == id && e.active),
                "daemon did not unload extension '{id}'"
            );
        }
        state = wait_for_state(conn, state, "extension startup", |s| {
            s.extensions
                .iter()
                .filter(|e| cli.extensions.contains(&e.id) && e.kind == "daemon")
                .all(|e| e.running)
        })?;
    }
    Ok(state)
}

fn apply_request(conn: &Connection, request: ClientRequest) -> Result<RunState> {
    conn.req_tx.send(request).context("send run setup")?;
    request_state(conn, Duration::from_secs(10))
}

fn request_state(conn: &Connection, budget: Duration) -> Result<RunState> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);
    let req_seq = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    conn.req_tx
        .send(ClientRequest::GetRunState { req_seq })
        .context("GetRunState")?;
    let deadline = Instant::now() + budget;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!("timed out waiting for daemon run state; no setup confirmation received");
        }
        let frame = conn
            .frame_rx
            .recv_timeout(remaining)
            .context("waiting for daemon run state")?;
        match frame.event {
            DaemonEvent::RunState {
                req_seq: reply_seq,
                state,
            } if reply_seq == req_seq => return Ok(state),
            DaemonEvent::Error(error) => bail!("daemon rejected request: {error}"),
            _ => {}
        }
    }
}

fn wait_for_state(
    conn: &Connection,
    mut state: RunState,
    label: &str,
    ready: impl Fn(&RunState) -> bool,
) -> Result<RunState> {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !ready(&state) {
        anyhow::ensure!(
            Instant::now() < deadline,
            "timed out waiting for {label}; prompt was not submitted"
        );
        std::thread::sleep(Duration::from_millis(100));
        state = request_state(
            conn,
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_secs(5)),
        )?;
    }
    Ok(state)
}

fn print_state(state: &RunState, include_available: bool) {
    println!("session_id={}", state.session_id);
    println!("name={}", state.name.replace('\n', "\\n"));
    println!(
        "workdir={}",
        state
            .workdir
            .first()
            .map(String::as_str)
            .unwrap_or_default()
    );
    println!(
        "workspaces={}",
        serde_json::to_string(&state.workdir).unwrap_or_default()
    );
    println!("model={}", state.model);
    println!(
        "effort={}",
        if state.effort.is_empty() {
            "default"
        } else {
            &state.effort
        }
    );
    println!("mode={}", state.mode);
    println!(
        "security={}",
        if state.security_enabled { "on" } else { "off" }
    );
    println!("short_send={}", if state.short_send { "on" } else { "off" });
    println!("drss_active={}", state.drss_active);
    println!("max_tokens={}", state.max_output_tokens);
    println!("context_window_limit={}", state.context_window_limit);
    println!("context_model_alias={}", state.context_model_alias);
    println!(
        "system_extra={}",
        if state.system_extra { "on" } else { "off" }
    );
    let active: Vec<_> = state
        .extensions
        .iter()
        .filter(|e| e.active)
        .map(|e| &e.id)
        .collect();
    println!(
        "active_extensions={}",
        serde_json::to_string(&active).unwrap_or_default()
    );
    if include_available {
        println!(
            "extensions={}",
            serde_json::to_string(&state.extensions).unwrap_or_default()
        );
        println!(
            "status={}",
            if state.awaiting_approval {
                "approval"
            } else if state.working {
                "working"
            } else {
                "idle"
            }
        );
    }
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

fn resolve_system(cli: &RunCli) -> Result<Option<String>> {
    let text = match (&cli.system, &cli.system_file) {
        (None, None) => return Ok(None),
        (Some(_), Some(_)) => bail!("pass only one of --system or --system-file"),
        (Some(s), None) => s.clone(),
        (None, Some(path)) => {
            let path = expand_user(path);
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?
        }
    };
    let text = text.trim().to_string();
    anyhow::ensure!(!text.is_empty(), "system text is empty");
    anyhow::ensure!(
        text.chars().count() <= crate::model::session::MAX_SESSION_SYSTEM_CHARS,
        "system text exceeds {} characters",
        crate::model::session::MAX_SESSION_SYSTEM_CHARS
    );
    Ok(Some(text))
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

/// Poll daemon-authoritative state after SubmitInput has been processed.
fn wait_once(conn: &Connection, timeout: Duration, mut state: RunState) -> Result<i32> {
    let deadline = Instant::now() + timeout;
    loop {
        if state.awaiting_approval {
            eprintln!("parked on approval (daemon still running)");
            return Ok(EXIT_APPROVAL);
        }
        if !state.working {
            println!("status=idle");
            return Ok(EXIT_OK);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            eprintln!("timeout (daemon still running)");
            return Ok(EXIT_TIMEOUT);
        }
        std::thread::sleep(remaining.min(Duration::from_millis(250)));
        match request_state(
            conn,
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_secs(5)),
        ) {
            Ok(next) => state = next,
            Err(_) if Instant::now() >= deadline => return Ok(EXIT_TIMEOUT),
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
#[path = "headless_run_tests.rs"]
mod tests;

#[cfg(test)]
mod resolve_model_tests {
    use super::*;

    #[test]
    fn empty_ref_errors() {
        let err = resolve_model_ref("   ").unwrap_err();
        assert!(err.to_string().contains("empty"));
    }
}
