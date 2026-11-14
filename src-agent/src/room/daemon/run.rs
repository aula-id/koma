//! `koma --daemon --kind secure`: the room process.
//!
//! It takes the same session id, lock, cwd, and Main-route credentials as the
//! coding daemon, binds `run/<id>.sock`, and runs the room until a signal. It
//! does not enter the coding loop.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Result};
use tokio::sync::mpsc::unbounded_channel;

use super::assemble::RoomSession;
use super::models::ModelBook;
use super::selftest::run_secure_selftest;
use crate::app::resolve::resolve_role;
use crate::app::runtime::{
    build_client, build_startup, build_tool_ctx, install_daemon_session, install_daemon_signals,
    shutdown_runtime,
};
use crate::ipc::proto::SessionStatus;
use crate::model::app_config::ModelRole;
use crate::room::actor::{ActorMsg, BuiltinHost, RoomConfig};
use crate::room::daemon::builtins::CtxBuiltins;
use crate::room::daemon::model_client::ProviderModelClient;

pub fn run_secure_daemon(opts: crate::cli::Opts) -> Result<()> {
    if opts.daemon_selftest {
        match run_secure_selftest() {
            Ok(()) => {
                println!("koma secure-daemon-selftest: OK");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("koma secure-daemon-selftest: FAIL: {e}");
                std::process::exit(1);
            }
        }
    }

    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    let session_id = opts.session.clone().ok_or_else(|| {
        anyhow::anyhow!("`koma --daemon --kind secure` requires `--session <id>`")
    })?;

    let (rt, handle, mut state, mut client) = build_startup(&opts)?;
    install_daemon_session(&mut state, &mut client, &handle, &session_id);

    let opened = state
        .rest
        .fg()
        .session
        .as_ref()
        .map(|s| (s.path.clone(), s.settings.clone()));
    let Some((session_dir, settings)) = opened else {
        let status = state.rest.fg().status.clone();
        shutdown_runtime(&mut state, rt);
        bail!("secure daemon: session did not open ({status})");
    };

    // The room is its own session beside the chat it was opened from. It uses that chat's settings,
    // so the model picked there is the one the room starts with.
    let usage_pwd_hash = state.rest.fg().session.as_ref().map(|s| s.pwd_hash.clone()).unwrap_or_default();
    let settings = inherited_settings(&usage_pwd_hash, &session_id).unwrap_or(settings);
    if let Ok(Some(row)) = crate::model::session_registry::get(&session_id) {
        if row.name.trim().is_empty() {
            let _ = crate::model::session_registry::set_name(&session_id, "secure room");
        }
    }

    let route =
        resolve_role(&state.rest.config, &settings, ModelRole::Main).filter(|r| r.is_usable());
    let Some(route) = route else {
        shutdown_runtime(&mut state, rt);
        bail!("secure daemon: this session has no usable Main route");
    };
    let client = match client {
        Some(c) => c,
        None => build_client(),
    };

    let context_window =
        (settings.context_window_limit > 0).then_some(settings.context_window_limit);
    let model = Arc::new(
        ProviderModelClient::new(
            client,
            route,
            state.rest.config.clone(),
            settings.clone(),
            settings.max_output_tokens,
            context_window,
        )
        .with_usage(session_id.clone(), usage_pwd_hash),
    );
    let idx = state.rest.foreground;
    let mut tool_ctx = build_tool_ctx(&state, idx);
    // The room reads the same skill folders the chat does; each role sees its own `skills_allow` of them.
    tool_ctx.skill_registry = Some(crate::model::skill::SkillRegistry::load(
        Some(&tool_ctx.workspace),
        &state.rest.config.extra_skill_roots,
    ));
    tool_ctx.active_skill_names = None;
    // The proof check may read only where the lanes may work.
    let mut oracle_roots = tool_ctx.workspaces.clone();
    if !tool_ctx.workspace.as_os_str().is_empty() && !oracle_roots.contains(&tool_ctx.workspace) {
        oracle_roots.push(tool_ctx.workspace.clone());
    }
    let builtins: Arc<dyn BuiltinHost> = Arc::new(CtxBuiltins::from_ctx(tool_ctx));

    let shutting_down = install_daemon_signals(&handle);
    let pid_path = crate::model::store::daemon_pid_path(&session_id).ok();
    if pid_path.is_some() {
        let _ = crate::model::store::write_daemon_pid(&session_id);
    }

    let probe = session_status(&state, &session_id);
    let sock_path = crate::model::store::daemon_sock_path(&session_id)?;
    let (session, serve) = {
        let _enter = handle.enter();
        let session = match RoomSession::open(&session_dir, model, builtins, RoomConfig { oracle_roots, ..RoomConfig::default() }) {
            Ok(session) => session,
            Err(e) => {
                drop(_enter);
                shutdown_runtime(&mut state, rt);
                if let Some(path) = &pid_path {
                    let _ = std::fs::remove_file(path);
                }
                return Err(anyhow::anyhow!(e));
            }
        };
        let listener = match crate::ipc::server::bind(&sock_path) {
            Ok(listener) => listener,
            Err(e) => {
                drop(_enter);
                shutdown_runtime(&mut state, rt);
                if let Some(path) = &pid_path {
                    let _ = std::fs::remove_file(path);
                }
                return Err(e.into());
            }
        };
        let mut session = session;
        let (dummy_tx, dummy_rx) = unbounded_channel();
        drop(dummy_tx);
        let events = std::mem::replace(&mut session.room.events, dummy_rx);
        let to_actor = session.room.to_actor.clone();
        let book = Arc::new(ModelBook::new(
            &state.rest.config,
            &settings,
            Arc::clone(&session.registry),
            crate::room::registry::overlay_dir_for(&session_dir),
        ));
        let serve = tokio::spawn(super::serve::run(
            listener,
            to_actor,
            events,
            probe,
            Arc::clone(&shutting_down),
            book,
            Arc::new(super::stage::Staging::new(&session_dir)),
        ));
        (session, serve)
    };
    while !shutting_down.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(200));
    }
    serve.abort();
    let _ = session.room.to_actor.send(ActorMsg::Shutdown);
    drop(session);

    shutdown_runtime(&mut state, rt);
    #[cfg(unix)]
    let _ = std::fs::remove_file(&sock_path);
    if let Some(path) = pid_path {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

/// The settings of the chat session this room was opened from (`KOMA_ROOM_PARENT`), when it exists.
fn inherited_settings(pwd_hash: &str, own_id: &str) -> Option<crate::model::settings::Settings> {
    let parent = std::env::var("KOMA_ROOM_PARENT").ok().filter(|p| !p.is_empty() && p != own_id)?;
    let dir = crate::model::store::session_dir(pwd_hash, &parent).ok()?;
    crate::model::session::Session::load(&dir).ok().map(|s| s.settings)
}

fn session_status(state: &crate::app::state::AppState, session_id: &str) -> SessionStatus {
    let rt = state.rest.fg();
    let name = rt.session.as_ref().map(|s| s.name.clone()).unwrap_or_default();
    let pwd = rt.session.as_ref().map(|_| rt.effective_cwd().display().to_string()).unwrap_or_default();
    SessionStatus {
        session_id: session_id.to_string(),
        name: if name.is_empty() { "secure".to_string() } else { name },
        pwd,
        working: false,
    }
}
