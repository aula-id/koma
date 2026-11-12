//! `koma --daemon --kind secure`: the room process.
//!
//! It takes the same session id, lock, cwd, and Main-route credentials as the
//! coding daemon, then runs the room until a signal. It does not enter the
//! coding loop and it does not bind `run/<id>.sock` (the room protocol is a
//! later phase).

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Result};

use super::assemble::RoomSession;
use super::selftest::run_secure_selftest;
use crate::app::resolve::resolve_role;
use crate::app::runtime::{
    build_client, build_startup, build_tool_ctx, install_daemon_session, install_daemon_signals,
    shutdown_runtime,
};
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
    let model = Arc::new(ProviderModelClient::new(
        client,
        route,
        settings.max_output_tokens,
        context_window,
    ));
    let idx = state.rest.foreground;
    let builtins: Arc<dyn BuiltinHost> =
        Arc::new(CtxBuiltins::from_ctx(build_tool_ctx(&state, idx)));

    let shutting_down = install_daemon_signals(&handle);
    let pid_path = crate::model::store::daemon_pid_path(&session_id).ok();
    if pid_path.is_some() {
        let _ = crate::model::store::write_daemon_pid(&session_id);
    }

    let session = {
        let _enter = handle.enter();
        RoomSession::open(&session_dir, model, builtins, RoomConfig::default())
            .map_err(|e| anyhow::anyhow!(e))
    };
    let session = match session {
        Ok(session) => session,
        Err(e) => {
            shutdown_runtime(&mut state, rt);
            if let Some(path) = pid_path {
                let _ = std::fs::remove_file(path);
            }
            return Err(e);
        }
    };
    while !shutting_down.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = session.room.to_actor.send(ActorMsg::Shutdown);
    drop(session);

    shutdown_runtime(&mut state, rt);
    if let Some(path) = pid_path {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}
