//! Public JSONL adapter. Mutations are serialized through a single IPC owner.
pub(crate) mod configure;
mod connection;
mod protocol;
mod reconcile;
#[cfg(test)]
mod tests;
use crate::ipc::proto::{ClientRequest as C, DaemonEvent as E};
use anyhow::{Context, Result};
use connection::Connection;
use protocol::*;
use serde_json::{json, Value};
use tokio::{
    io::{AsyncWriteExt, BufReader},
    sync::{mpsc, watch},
};

/// The only stdout writer. Responses are bounded (one in flight); unsent complete
/// state events coalesce before sequence assignment, independently of responses.
fn event_frames(previous: &mut Option<State>, state: State, seq: &mut u64) -> Vec<Value> {
    let mut frames = Vec::new();
    if previous.as_ref() == Some(&state) {
        return frames;
    }
    let append = previous.as_ref().and_then(|old| {
        let mut same = old.clone();
        same.streaming = state.streaming.clone();
        same.revision = state.revision;
        let before = old.streaming.as_deref().unwrap_or("");
        let after = state.streaming.as_deref().unwrap_or("");
        if same == state
            && state.revision == old.revision + 1
            && after.starts_with(before)
            && after.len() > before.len()
        {
            Some((before.len(), after[before.len()..].to_string()))
        } else {
            None
        }
    });
    *seq += 1;
    if let Some((offset, text)) = append {
        frames.push(json!({"type":"event","event":"output_append","seq":*seq,"revision":state.revision,"offset":offset,"text":text}));
    } else {
        let resynced = previous
            .as_ref()
            .map_or(state.resync > 0, |old| old.resync != state.resync);
        frames.push(
            json!({"type":"event","event":"state","seq":*seq,"state":state,"resynced":resynced}),
        );
        if previous.as_ref().and_then(|s| s.turn.as_ref()) != state.turn.as_ref() {
            *seq += 1;
            frames.push(json!({"type":"event","event":"turn","seq":*seq,"revision":state.revision,"turn":state.turn}));
        }
    }
    *previous = Some(state);
    frames
}
async fn writer(
    mut responses: mpsc::Receiver<Value>,
    mut states: watch::Receiver<Option<State>>,
) -> Result<()> {
    let mut out = tokio::io::stdout();
    let mut seq = 0u64;
    let mut previous = None;
    loop {
        let frames = tokio::select! {biased;
         r=responses.recv()=>match r{Some(r)=>vec![r],None=>break},
         changed=states.changed()=>{if changed.is_err(){break}let s=states.borrow_and_update().clone();let Some(s)=s else{continue};event_frames(&mut previous,s,&mut seq)}
        };
        for frame in frames {
            let mut bytes = serde_json::to_vec(&frame)?;
            anyhow::ensure!(bytes.len() <= MAX_LINE, "outbound frame exceeds 40 MiB");
            bytes.push(b'\n');
            tokio::time::timeout(connection::BUDGET, async {
                out.write_all(&bytes).await?;
                out.flush().await
            })
            .await
            .context("stdout stalled")??;
        }
    }
    Ok(())
}
pub async fn serve() -> Result<()> {
    // Emit and flush hello before even constructing the stdin reader.
    let hello = json!({"type":"hello","protocol":1,"binary":{"path":std::env::current_exe()?.to_string_lossy(),"version":env!("CARGO_PKG_VERSION")},"diagnostics":[],"capabilities":METHODS});
    let mut stdout = tokio::io::stdout();
    stdout.write_all(&serde_json::to_vec(&hello)?).await?;
    stdout.write_all(b"\n").await?;
    stdout.flush().await?;
    drop(stdout);
    let (tx, rx) = mpsc::channel(1);
    let (events, states) = watch::channel(None);
    let mut output = tokio::spawn(writer(rx, states));
    let (input_tx, mut input_rx) = mpsc::channel(1);
    let input = tokio::spawn(async move {
        let mut stdin = BufReader::new(tokio::io::stdin());
        loop {
            let value = match line(&mut stdin).await {
                Ok(Some(b)) => {
                    serde_json::from_slice::<Request>(&b).context("malformed JSONL request")
                }
                Ok(None) => break,
                Err(e) => Err(e),
            };
            let fatal = value.is_err();
            if input_tx.send(value).await.is_err() || fatal {
                break;
            }
        }
    });
    let mut conn: Option<Connection> = None;
    let mut seen = std::collections::HashSet::new();
    let result:Result<()>=async {
 loop {
  let request=if let Some(c)=conn.as_mut(){tokio::select!{
   r=input_rx.recv()=>r,
   f=c.receive()=>{f?;c.flush_resync().await?;continue},
   w=&mut output=>{return w.context("writer task")?;}
  }}else{tokio::select!{r=input_rx.recv()=>r,w=&mut output=>{return w.context("writer task")?;}}};
  let Some(r)=request else{break};let r=r?;
  anyhow::ensure!(!r.id.is_empty(),"empty request ID");
  // Bound lifetime correlation storage; never silently forget IDs and allow reuse.
  anyhow::ensure!(seen.len()<65536,"request lifetime limit reached; detach and reconnect");
  let id=r.id.clone();let method=r.method.clone();
  let response=if !seen.insert(id.clone()){Err(RpcError::new("invalid_request","duplicate request id"))}else{dispatch(r,&mut conn,&events).await};
  let fatal=response.as_ref().err().is_some_and(|e|e.delivery=="unknown");
  let frame=match response{Ok(result)=>json!({"type":"response","id":id,"result":result}),Err(error)=>json!({"type":"response","id":id,"error":error})};
  let frame=if serde_json::to_vec(&frame)?.len()>MAX_LINE {json!({"type":"response","id":id,"error":RpcError::new("unavailable","state exceeds frame limit")})}else{frame};
  tx.send(frame).await.context("stdout writer closed")?;
  if method=="detach"||fatal{break}
 }
 Ok(())
 }.await;
    // Closing this socket is sufficient. Never send QuitDaemon, even on errors.
    drop(conn);
    input.abort();
    drop(tx);
    drop(events);
    let written = if output.is_finished() {
        Ok(())
    } else {
        match tokio::time::timeout(connection::BUDGET, &mut output).await {
            Ok(r) => r.context("writer task")?,
            Err(e) => {
                output.abort();
                Err(e.into())
            }
        }
    };
    result.and(written)
}
async fn dispatch(
    r: Request,
    conn: &mut Option<Connection>,
    events: &watch::Sender<Option<State>>,
) -> Result<Value, RpcError> {
    if !METHODS.contains(&r.method.as_str()) {
        return Err(RpcError::new("unsupported_method", "unsupported method"));
    }
    if r.method == "configure" {
        return configure_call(r.params, conn).await;
    }
    if r.method == "create" || r.method == "resume" {
        if conn.is_some() {
            return Err(RpcError::new("invalid_state", "session already bound"));
        }
        let (id, cwd, resume, policy, model, external) = if r.method == "create" {
            let p: Create = params(r.params)?;
            let path = std::path::PathBuf::from(&p.cwd);
            if !path.is_absolute() || !path.is_dir() {
                return Err(RpcError::new(
                    "invalid_request",
                    "cwd must be an absolute existing directory",
                ));
            }
            let model = if let Some(id) = p.model_id {
                let config = crate::model::app_config::AppConfig::load();
                let settings = crate::model::settings::Settings::default();
                let m =
                    crate::app::resolve::find_model_entry_by_slug(&config, &settings, &id, None)
                        .ok_or_else(|| RpcError::new("not_found", "unknown model"))?;
                Some((m.uuid.clone(), m.model_id.clone()))
            } else {
                None
            };
            (
                uuid::Uuid::new_v4().to_string(),
                Some(p.cwd),
                false,
                Some(p.policy.unwrap_or_default()),
                model,
                p.external_approvals,
            )
        } else {
            let p: Resume = params(r.params)?;
            let id = p.session_id;
            if uuid::Uuid::parse_str(&id).is_err() {
                return Err(RpcError::new("not_found", "unknown session"));
            }
            let row = tokio::task::spawn_blocking({
                let id = id.clone();
                move || crate::model::session_registry::get(&id)
            })
            .await
            .map_err(|e| RpcError::new("internal", e.to_string()))?
            .map_err(|e| RpcError::new("unavailable", e.to_string()))?
            .ok_or_else(|| RpcError::new("not_found", "unknown session"))?;
            (
                id,
                Some(row.workdir),
                true,
                p.policy,
                None,
                p.external_approvals,
            )
        };
        if policy
            .as_ref()
            .is_some_and(|p| p.approval == Approval::Default)
            && !external
        {
            return Err(RpcError::new(
                "invalid_request",
                "approval default requires external_approvals opt-in",
            ));
        }
        let spawn_id = id.clone();
        let workdir = cwd.clone();
        tokio::task::spawn_blocking(move || {
            super::manage::ensure_daemon_running(
                &spawn_id,
                resume,
                workdir.as_deref().map(std::path::Path::new),
            )
        })
        .await
        .map_err(RpcError::unknown)?
        .map_err(RpcError::unknown)?;
        let mut c = Connection::attach(id, cwd, events.clone())
            .await
            .map_err(RpcError::unknown)?;
        c.readback().await?;
        c.projection.external_approvals = external;
        // Only this connection opts into answering tool gates.
        c.ack(C::SetApprovalChannel { enabled: external }).await?;
        if let Some(p) = policy {
            c.ack(C::SetSessionPolicy {
                request_id: r.id.clone(),
                plan: match p.plan {
                    Plan::Allow => crate::ipc::proto::PlanPolicy::Allow,
                    Plan::Forbid => crate::ipc::proto::PlanPolicy::Forbid,
                },
                approval: match p.approval {
                    Approval::Default => crate::ipc::proto::ApprovalPolicy::Default,
                    Approval::Reject => crate::ipc::proto::ApprovalPolicy::Reject,
                },
            })
            .await?;
            c.readback().await?;
            if c.projection.policy.as_ref() != Some(&p) {
                return Err(RpcError::unknown("daemon did not apply policy"));
            }
        }
        if let Some((uuid, expected)) = model {
            c.ack(C::SetSessionMain {
                model_uuid: Some(uuid),
            })
            .await?;
            if c.readback().await?.model != expected {
                return Err(RpcError::unknown("daemon did not apply requested model"));
            }
        }
        let state = c.state().await?;
        c.ready = true;
        c.publish();
        *conn = Some(c);
        return Ok(json!({"state":state}));
    }
    let c = conn
        .as_mut()
        .ok_or_else(|| RpcError::new("invalid_state", "no session bound"))?;
    match r.method.as_str() {
        "state" => {
            let _: serde_json::Map<String, Value> = params(r.params)?;
            Ok(json!({"state":c.state().await?}))
        }
        "detach" => {
            let _: serde_json::Map<String, Value> = params(r.params)?;
            Ok(json!({"detached":true}))
        }
        "submit" | "enqueue" | "interrupt_and_submit" | "interrupt" => {
            let id = r.id;
            let req = match r.method.as_str() {
                "submit" | "enqueue" => {
                    let input: Input = params(r.params)?;
                    let (text, attachments) =
                        tokio::task::spawn_blocking(move || attachments(input))
                            .await
                            .map_err(|e| RpcError::new("internal", e.to_string()))??;
                    if r.method == "submit" {
                        C::SubmitTurnWithAttachments {
                            request_id: id.clone(),
                            text,
                            attachments,
                        }
                    } else {
                        C::EnqueueFollowUp {
                            request_id: id.clone(),
                            text,
                            attachments,
                        }
                    }
                }
                "interrupt" => {
                    let p: Target = params(r.params)?;
                    C::InterruptTurn {
                        request_id: id.clone(),
                        turn_id: p.turn_id,
                    }
                }
                _ => {
                    let p: Replacement = params(r.params)?;
                    let (text, attachments) =
                        tokio::task::spawn_blocking(move || attachments(p.input))
                            .await
                            .map_err(|e| RpcError::new("internal", e.to_string()))??;
                    C::InterruptAndSubmit {
                        request_id: id.clone(),
                        turn_id: p.turn_id,
                        text,
                        attachments,
                    }
                }
            };
            let event = c
                .rpc(req, |e| match e {
                    E::TurnResult { request_id, .. }
                    | E::InterruptAndSubmitResult { request_id, .. } => request_id == &id,
                    _ => false,
                })
                .await?;
            Ok(turn_result(&c.projection.id, event)?)
        }
        "approve" => {
            let p: Decision = params(r.params)?;
            let state = c.state().await?;
            if !state.external_approvals
                || !state
                    .pending_approval
                    .as_ref()
                    .is_some_and(|a| a.call_id == p.call_id)
            {
                return Err(RpcError::new(
                    "invalid_state",
                    "target call is not awaiting opted-in tool approval",
                ));
            }
            let s = c.projection.session.as_ref().ok_or_else(|| {
                RpcError::new("invalid_state", "no active session")
            })?;
            let call = s
                .pending_tool_calls
                .get(s.tool_idx)
                .ok_or_else(|| RpcError::new("invalid_state", "no pending tool approval"))?;
            if matches!(call.function.name.as_str(), "plan_ready" | "mission_ready") {
                return Err(RpcError::new(
                    "invalid_state",
                    "plan and mission decisions are not tool approvals",
                ));
            }
            c.ack(C::ApproveTool {
                approve: p.approve,
                call_id: Some(p.call_id),
            })
            .await?;
            Ok(json!({"approved":p.approve}))
        }
        other => {
            let message = format!("SDK method `{other}` is not implemented.");
            crate::model::notifications::record(
                Some(c.projection.id.clone()),
                crate::model::notifications::Entry::new(message.clone(), "error", "sdk"),
            );
            Err(RpcError::new("unsupported_method", message))
        }
    }
}
async fn configure_call(
    params: Value,
    conn: &mut Option<Connection>,
) -> Result<Value, RpcError> {
    if let Some(connection) = conn.as_mut() {
        connection.configure(params).await
    } else {
        let mut config = crate::model::app_config::AppConfig::load();
        let require_main = !configure::has_main(&config);
        let applied = configure::apply(&mut config, &params, require_main)
            .map_err(|message| RpcError::new("invalid_request", message))?;
        config.save().map_err(|error| {
            RpcError::new("unavailable", format!("configure save failed: {error}"))
        })?;
        Ok(applied.reply)
    }
}
fn turn_result(session_id: &str, e: E) -> Result<Value, RpcError> {
    let (d, id, reason, interrupted) = match e {
        E::TurnResult {
            disposition,
            turn_id,
            reason,
            ..
        } => (disposition, turn_id, reason, None),
        E::InterruptAndSubmitResult {
            disposition,
            turn_id,
            reason,
            interrupted_turn_id,
            ..
        } => (disposition, turn_id, reason, Some(interrupted_turn_id)),
        _ => {
            let message = "Turn reply was not a turn result.".to_string();
            crate::model::notifications::record(
                Some(session_id.to_string()),
                crate::model::notifications::Entry::new(message.clone(), "error", "sdk"),
            );
            return Err(RpcError::new("unavailable", message));
        }
    };
    let reason = reason.map(|message| {
        let code = if message.contains("stale_turn") {
            "stale_turn"
        } else if message.contains("queue_full") {
            "queue_full"
        } else if message.contains("busy") {
            "busy"
        } else {
            "invalid_input"
        };
        json!({"code":code,"message":message})
    });
    let disposition = match d {
        crate::ipc::proto::TurnDisposition::Started => "started",
        crate::ipc::proto::TurnDisposition::Queued => "queued",
        crate::ipc::proto::TurnDisposition::Rejected => "rejected",
        crate::ipc::proto::TurnDisposition::Interrupted => "interrupted",
    };
    let mut v = json!({"disposition":disposition,"turn_id":id,"reason":reason});
    if let Some(id) = interrupted {
        v["interrupted_turn_id"] = json!(id)
    }
    Ok(v)
}
