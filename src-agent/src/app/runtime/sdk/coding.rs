//! The v1 coding paths: binding by `create`/`resume`, the bound session
//! methods, `configure` and turn results.
use super::{
    binding::Binding, configure, connection::Connection, methods::HandlerFuture, protocol::*,
    Adapter,
};
use crate::ipc::proto::{ClientRequest as C, DaemonEvent as E};
use serde_json::{json, Value};

pub fn handle_bind(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(bind(r, adapter))
}
pub fn handle_session(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(session(r, adapter))
}
pub fn handle_configure(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(configure_call(r.params, &mut adapter.binding))
}

pub fn handle_sessions_info(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(sessions_info(r, adapter))
}

/// The result of `create`, `resume` and `state`. Per-session facts that are
/// not part of the v1 state object go beside `state` here, never inside it.
pub fn state_result(connection: &Connection, state: State) -> Value {
    json!({"state":state,"session_info":connection.session_info()})
}

/// `sessions.info`: the adapter's latest `session_info`, with no IPC.
async fn sessions_info(r: Request, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let _: serde_json::Map<String, Value> = new_params(r.params)?;
    let c = adapter
        .binding
        .coding_mut()
        .ok_or_else(|| RpcError::reason(Reason::WrongBinding, "no session bound"))?;
    Ok(json!({"session_info":c.session_info()}))
}

/// `create` and `resume`. Runs only while unbound; the route table refuses
/// both once a binding is held.
async fn bind(r: Request, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let mut name = None;
    let (id, cwd, resume, policy, model, external) = if r.method == "create" {
        let p: Create = params(r.params)?;
        if let Some(value) = &p.name {
            name = Some(super::host::sessions::session_name(Some(value))?);
        }
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
            let m = crate::app::resolve::find_model_entry_by_slug(&config, &settings, &id, None)
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
        crate::app::runtime::manage::ensure_daemon_running(
            &spawn_id,
            resume,
            workdir.as_deref().map(std::path::Path::new),
        )
    })
    .await
    .map_err(RpcError::unknown)?
    .map_err(RpcError::unknown)?;
    let mut c = Connection::attach(id, cwd, adapter.events.clone())
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
    let warning = match &name {
        Some(name) => {
            let secrets = super::host::config::daemon_secrets(adapter).await;
            super::host::sessions::name_created(&mut c, name, secrets.as_deref()).await?
        }
        None => None,
    };
    let state = c.state().await?;
    c.ready = true;
    c.publish();
    let mut result = state_result(&c, state);
    if let Some(warning) = warning {
        result["warnings"] = json!([warning]);
    }
    adapter.binding = Binding::Coding(Box::new(c));
    Ok(result)
}
/// The bound session methods and `detach`. Unbound `detach` still gets the v1
/// error.
async fn session(r: Request, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let c = adapter
        .binding
        .coding_mut()
        .ok_or_else(|| RpcError::new("invalid_state", "no session bound"))?;
    match r.method.as_str() {
        "state" => {
            let _: serde_json::Map<String, Value> = params(r.params)?;
            let state = c.state().await?;
            Ok(state_result(c, state))
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
            let s = c
                .projection
                .session
                .as_ref()
                .ok_or_else(|| RpcError::new("invalid_state", "no active session"))?;
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
async fn configure_call(params: Value, binding: &mut Binding) -> Result<Value, RpcError> {
    configure::check(&params).map_err(|message| RpcError::new("invalid_request", message))?;
    if let Some(connection) = binding.coding_mut() {
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
pub fn turn_result(session_id: &str, e: E) -> Result<Value, RpcError> {
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
