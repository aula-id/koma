//! Headless SSH lifecycle for the OpenTUI client. Only framed data crosses these
//! pipes. No terminal ownership and no credentials in argv.
use super::{auth, bootstrap, sessions, ssh, RemoteTarget};
use crate::ipc::frame::{read_frame_from, write_frame_to, FrameReader};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncWriteExt, BufReader};

#[derive(Deserialize)]
pub(crate) enum Control {
    Password {
        value: String,
    },
    Usage(crate::model::usage::reader::Request),
    Directories {
        path: String,
    },
    Sessions,
    Open {
        session_id: Option<String>,
        cwd: Option<String>,
    },
    Kill {
        session_id: String,
    },
    Delete {
        session_id: String,
    },
    Frame(Value),
    Disconnect,
    Cancel,
}
#[derive(Serialize)]
pub(crate) enum Event {
    Progress { stage: String },
    AuthRequired { error: Option<String> },
    Sessions(sessions::DiscoveredSessions),
    Directories { path: String, entries: Vec<String> },
    Connected { session_id: String },
    Daemon(Value),
    Usage(Box<crate::model::usage::reader::Response>),
    UsageError { request_id: u64, message: String },
    Error { message: String },
    Disconnected,
}
type Output = std::sync::Arc<tokio::sync::Mutex<tokio::io::Stdout>>;
async fn emit(out: &mut Output, event: Event) -> Result<()> {
    let mut out = out.lock().await;
    write_frame_to(&mut *out, &serde_json::to_vec(&event)?).await?;
    out.flush().await?;
    Ok(())
}

pub(crate) fn run(
    target: RemoteTarget,
    host_id: Option<String>,
    requested: Option<String>,
    new_session: bool,
) -> Result<()> {
    tokio::runtime::Runtime::new()?.block_on(run_async(target, host_id, requested, new_session))
}
async fn run_async(
    target: RemoteTarget,
    host_id: Option<String>,
    requested: Option<String>,
    new_session: bool,
) -> Result<()> {
    let mut input = BufReader::new(tokio::io::stdin());
    let mut reader = FrameReader::new();
    let mut output = std::sync::Arc::new(tokio::sync::Mutex::new(tokio::io::stdout()));
    emit(
        &mut output,
        Event::Progress {
            stage: "authenticating".into(),
        },
    )
    .await?;
    let mut resolved = auth::resolve_ssh_auth(
        &target,
        host_id.as_deref(),
        None,
        auth::InteractivePassword::None,
    )
    .ok();
    loop {
        if let Some(ref r) = resolved {
            emit(
                &mut output,
                Event::Progress {
                    stage: "bootstrapping".into(),
                },
            )
            .await?;
            // Bootstrap owns provisioning/version checks. Its structured stages are
            // forwarded from a worker so stdout remains exclusively framed.
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let bootstrap_target = target.clone();
            let password = r.password.clone();
            let task = tokio::task::spawn_blocking(move || {
                let auth = password.map(auth::SshAuth::new).transpose()?;
                bootstrap::ensure_koma_compatible_progress(
                    &bootstrap_target,
                    auth.as_ref(),
                    |stage| {
                        let _ = tx.send(format!("{stage:?}"));
                    },
                )
            });
            while let Some(stage) = rx.recv().await {
                emit(&mut output, Event::Progress { stage }).await?;
            }
            match task.await? {
                Ok(_) => {
                    auth::remember_password(r);
                    break;
                }
                Err(error) if ssh::is_auth_rejection(&error) => {
                    auth::forget_stored_password(r);
                    emit(
                        &mut output,
                        Event::AuthRequired {
                            error: Some(format!("{error:#}")),
                        },
                    )
                    .await?;
                }
                // Not a credentials problem: keep the saved password and show
                // the real error instead of asking to log in again.
                Err(error) => {
                    emit(
                        &mut output,
                        Event::Error {
                            message: format!("remote bootstrap failed: {error:#}"),
                        },
                    )
                    .await?;
                    return Ok(());
                }
            }
            resolved = None;
        } else {
            emit(&mut output, Event::AuthRequired { error: None }).await?;
        }
        let bytes = read_frame_from(&mut input, &mut reader).await?;
        match serde_json::from_slice::<Control>(&bytes)? {
            Control::Password { value } => {
                resolved = Some(auth::resolve_ssh_auth(
                    &target,
                    host_id.as_deref(),
                    Some(&value),
                    auth::InteractivePassword::None,
                )?);
            }
            Control::Cancel => return Ok(()),
            _ => continue,
        }
    }
    let resolved = resolved.context("SSH authentication was cancelled")?;
    let auth = resolved.auth.as_ref();
    // Dedicated task and SSH process: no daemon frames or discovery work on this pipe.
    let (usage_tx, mut usage_rx) =
        tokio::sync::mpsc::channel::<crate::model::usage::reader::Request>(1);
    let usage_target = target.clone();
    let usage_password = resolved.password.clone();
    let mut usage_output = output.clone();
    let usage_task = tokio::spawn(async move {
        let usage_auth = usage_password.map(auth::SshAuth::new).transpose();
        let mut lane: Option<ssh::SshSession> = None;
        let mut decoder = FrameReader::new();
        while let Some(request) = usage_rx.recv().await {
            let id = request.request_id;
            let operation = async {
                let auth = usage_auth.as_ref().map_err(|e| anyhow::anyhow!("{e:#}"))?;
                if lane.is_none() {
                    let path = ssh::find_koma(&usage_target, auth.as_ref())?;
                    let mut child = ssh::connect_command(
                        &usage_target,
                        auth.as_ref(),
                        &path,
                        &["remote-usage"],
                    )?;
                    if let Some(mut stderr) = child.child.stderr.take() {
                        tokio::spawn(async move {
                            let _ = tokio::io::copy(&mut stderr, &mut tokio::io::sink()).await;
                        });
                    }
                    lane = Some(child);
                    decoder = FrameReader::new();
                }
                let child = lane.as_mut().context("usage ssh session was not started")?;
                write_frame_to(&mut child.stdin, &serde_json::to_vec(&request)?).await?;
                let bytes = read_frame_from(&mut child.stdout, &mut decoder).await?;
                Ok(serde_json::from_slice(&bytes)?)
            };
            let result: Result<crate::model::usage::reader::Response> =
                match tokio::time::timeout(std::time::Duration::from_secs(4), operation).await {
                    Ok(result) => result,
                    Err(_) => Err(anyhow::anyhow!("Remote usage reader timed out")),
                };
            let event = match result {
                Ok(response) => Event::Usage(Box::new(response)),
                Err(error) => {
                    lane = None;
                    Event::UsageError {
                        request_id: id,
                        message: format!("{error:#}"),
                    }
                }
            };
            if emit(&mut usage_output, event).await.is_err() {
                break;
            }
        }
    });
    let _usage_cleanup = scopeguard::guard(usage_task, |task| task.abort());
    let mut bridge: Option<ssh::SshSession> = None;
    let mut daemon_reader = FrameReader::new();
    if let Some(session_id) = requested {
        bridge = Some(open(&target, auth, &session_id, None, &mut output).await?);
    } else if new_session {
        directories(&target, auth, "/".into(), &mut output).await?;
    } else {
        discovery(&target, auth, &mut output).await?;
    }
    loop {
        let bytes = if let Some(session) = &mut bridge {
            tokio::select! {
                command = read_frame_from(&mut input, &mut reader) => command?,
                frame = read_frame_from(&mut session.stdout, &mut daemon_reader) => {
                    match frame {
                        Ok(bytes) => { emit(&mut output, Event::Daemon(serde_json::from_slice(&bytes)?)).await?; }
                        Err(error) => { emit(&mut output, Event::Error { message: format!("SSH connection lost: {error}") }).await?; bridge = None; }
                    }
                    continue;
                }
            }
        } else {
            read_frame_from(&mut input, &mut reader).await?
        };
        let command: Control = serde_json::from_slice(&bytes)?;
        let result: Result<()> = match command {
            Control::Cancel => break,
            Control::Usage(request) => {
                let id = request.request_id;
                if usage_tx.try_send(request).is_err() {
                    emit(
                        &mut output,
                        Event::UsageError {
                            request_id: id,
                            message: "Remote usage reader busy".into(),
                        },
                    )
                    .await?;
                }
                Ok(())
            }
            Control::Password { .. } => Ok(()),
            Control::Frame(request) => {
                if let Some(session) = &mut bridge {
                    // Validate the vocabulary before forwarding, sharing the same
                    // frame encoder and daemon event path as local sockets.
                    let _: crate::ipc::proto::ClientRequest =
                        serde_json::from_value(request.clone())?;
                    write_frame_to(&mut session.stdin, &serde_json::to_vec(&request)?).await?;
                }
                Ok(())
            }
            Control::Disconnect => {
                if let Some(mut session) = bridge.take() {
                    let _ = write_frame_to(
                        &mut session.stdin,
                        &serde_json::to_vec(&crate::ipc::proto::ClientRequest::Detach)?,
                    )
                    .await;
                    let _ = session.stdin.shutdown().await;
                    crate::app::runtime::stdio_bridge::reap_bridge_child(&mut session.child).await;
                }
                daemon_reader = FrameReader::new();
                emit(&mut output, Event::Disconnected).await
            }
            Control::Sessions => discovery(&target, auth, &mut output).await,
            Control::Directories { path } => directories(&target, auth, path, &mut output).await,
            Control::Open { session_id, cwd } => {
                if let Some(mut old) = bridge.take() {
                    let _ = old.child.kill().await;
                }
                daemon_reader = FrameReader::new();
                let id = session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                match open(&target, auth, &id, cwd.as_deref(), &mut output).await {
                    Ok(session) => {
                        bridge = Some(session);
                        Ok(())
                    }
                    Err(error) => Err(error),
                }
            }
            Control::Kill { session_id } => {
                match sessions::kill_session_over_ssh(&target, auth, &session_id) {
                    Ok(()) => discovery(&target, auth, &mut output).await,
                    Err(error) => Err(error),
                }
            }
            Control::Delete { session_id } => {
                match sessions::delete_session_over_ssh(&target, auth, &session_id) {
                    Ok(()) => discovery(&target, auth, &mut output).await,
                    Err(error) => Err(error),
                }
            }
        };
        if let Err(error) = result {
            emit(
                &mut output,
                Event::Error {
                    message: format!("{error:#}"),
                },
            )
            .await?;
        }
    }
    if let Some(mut session) = bridge {
        let _ = session.child.kill().await;
    }
    Ok(())
}
async fn open(
    target: &RemoteTarget,
    auth: Option<&auth::SshAuth>,
    id: &str,
    cwd: Option<&str>,
    out: &mut Output,
) -> Result<ssh::SshSession> {
    emit(
        out,
        Event::Progress {
            stage: "connecting".into(),
        },
    )
    .await?;
    let path = ssh::find_koma(target, auth)?;
    let mut session = ssh::connect(target, id, auth, cwd, &path)?;
    if let Some(mut stderr) = session.child.stderr.take() {
        tokio::spawn(async move {
            let _ = tokio::io::copy(&mut stderr, &mut tokio::io::sink()).await;
        });
    }
    emit(
        out,
        Event::Connected {
            session_id: id.into(),
        },
    )
    .await?;
    Ok(session)
}
async fn discovery(
    target: &RemoteTarget,
    auth: Option<&auth::SshAuth>,
    out: &mut Output,
) -> Result<()> {
    emit(
        out,
        Event::Sessions(sessions::list_sessions_over_ssh(target, auth)?),
    )
    .await
}
async fn directories(
    target: &RemoteTarget,
    auth: Option<&auth::SshAuth>,
    path: String,
    out: &mut Output,
) -> Result<()> {
    let entries = ssh::list_dirs(target, &path, auth)?;
    emit(out, Event::Directories { path, entries }).await
}
