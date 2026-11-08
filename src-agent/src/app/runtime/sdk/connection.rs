//! Explicit SDK attach: protocol/capability negotiation, never fingerprint restart.
use super::{
    protocol::{RpcError, State},
    reconcile::Reconcile,
};
use crate::ipc::{
    client,
    frame::FrameReader,
    proto::{ClientRequest, DaemonEvent, DaemonFrame, RunState},
    IpcStream,
};
use anyhow::{Context, Result};
use std::time::Duration;
use tokio::sync::watch;
pub const BUDGET: Duration = Duration::from_secs(15);
pub struct Connection {
    stream: IpcStream,
    reader: FrameReader,
    pub projection: Reconcile,
    pub ready: bool,
    pub version: String,
    pub capabilities: Vec<String>,
    seq: u64,
    resync_needed: bool,
    events: watch::Sender<Option<State>>,
}
impl Connection {
    pub async fn attach(
        id: String,
        cwd: Option<String>,
        events: watch::Sender<Option<State>>,
    ) -> Result<Self> {
        tokio::time::timeout(BUDGET, async {
            let stream = client::connect_default(&id).await?;
            let mut c = Self {
                stream,
                reader: FrameReader::new(),
                projection: Reconcile::new(id.clone()),
                ready: false,
                version: String::new(),
                capabilities: vec![],
                seq: 0,
                resync_needed: false,
                events,
            };
            c.send(&ClientRequest::Attach {
                foreground_id: Some(id),
                cwd,
            })
            .await?;
            let f = c.receive().await?;
            match f.event {
                DaemonEvent::Hello {
                    version,
                    protocol,
                    capabilities,
                } => {
                    anyhow::ensure!(
                        protocol == crate::ipc::proto::IPC_PROTOCOL_VERSION,
                        "unsupported daemon protocol {protocol}"
                    );
                    c.version = version;
                    c.capabilities = capabilities;
                }
                _ => anyhow::bail!("daemon did not send Hello first"),
            }
            for cap in [
                "submit_turn",
                "interrupt_turn",
                "session_policy",
                "session_policy_readback",
                "enqueue_follow_up",
                "interrupt_and_submit",
                "turn_attachments",
            ] {
                anyhow::ensure!(
                    c.capabilities.iter().any(|x| x == cap),
                    "daemon lacks {cap}"
                );
            }
            while c.projection.session.is_none() {
                c.receive().await?;
                c.flush_resync().await?;
            }
            Ok(c)
        })
        .await
        .context("daemon attach timeout")?
    }
    pub async fn send(&mut self, r: &ClientRequest) -> Result<()> {
        client::send_frame(&mut self.stream, &serde_json::to_vec(r)?).await?;
        Ok(())
    }
    pub async fn receive(&mut self) -> Result<DaemonFrame> {
        let bytes = client::recv_frame(&mut self.stream, &mut self.reader).await?;
        let f: DaemonFrame = serde_json::from_slice(&bytes)?;
        if self.projection.apply(&f) {
            self.resync_needed = true;
        }
        self.publish();
        Ok(f)
    }
    /// Called only after the cancellation-safe receive branch has won select.
    pub async fn flush_resync(&mut self) -> Result<()> {
        if self.resync_needed {
            tokio::time::timeout(BUDGET, self.send(&ClientRequest::Resync)).await??;
            self.resync_needed = false;
        }
        Ok(())
    }
    pub fn publish(&mut self) {
        if !self.ready {
            return;
        }
        if let Some(s) = self.projection.state() {
            self.events.send_if_modified(|old| {
                if old.as_ref() == Some(&s) {
                    false
                } else {
                    *old = Some(s);
                    true
                }
            });
        }
    }
    /// Only one request is in flight on IPC, avoiding uncorrelated Ack ambiguity.
    /// Any failure after beginning a write is delivery unknown; callers must close.
    pub async fn rpc(
        &mut self,
        r: ClientRequest,
        accept: impl Fn(&DaemonEvent) -> bool,
    ) -> Result<DaemonEvent, RpcError> {
        let result = tokio::time::timeout(BUDGET, async {
            self.send(&r).await?;
            loop {
                let f = self.receive().await?;
                self.flush_resync().await?;
                if let DaemonEvent::Error(e) = f.event {
                    return Ok::<_, anyhow::Error>(DaemonEvent::Error(e));
                }
                if accept(&f.event) {
                    return Ok(f.event);
                }
            }
        })
        .await;
        match result {
            Ok(Ok(DaemonEvent::Error(e))) => Err(RpcError::new("invalid_state", e)),
            Ok(Ok(e)) => Ok(e),
            Ok(Err(e)) => Err(RpcError::unknown(e)),
            Err(e) => Err(RpcError::unknown(e)),
        }
    }
    pub async fn ack(&mut self, r: ClientRequest) -> Result<(), RpcError> {
        self.rpc(r, |e| matches!(e, DaemonEvent::Ack)).await?;
        Ok(())
    }
    /// Live `configure`. The daemon acks after it saves; the reply is read back
    /// from that file so the public result does not need a new daemon event.
    pub async fn configure(
        &mut self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, RpcError> {
        let event = self
            .rpc(
                ClientRequest::SdkConfigure {
                    params: params.clone(),
                },
                |event| matches!(event, DaemonEvent::Ack),
            )
            .await
            .map_err(map_configure_error)?;
        if !matches!(event, DaemonEvent::Ack) {
            return Err(RpcError::new("unavailable", "configure did not ack"));
        }
        let config = crate::model::app_config::AppConfig::try_load().map_err(|error| {
            RpcError::new("unavailable", format!("configure save failed: {error}"))
        })?;
        super::configure::reply(&config, &params)
            .map_err(|message| RpcError::new("unavailable", message))
    }
    pub async fn readback(&mut self) -> Result<RunState, RpcError> {
        self.seq += 1;
        let seq = self.seq;
        let e = self
            .rpc(
                ClientRequest::GetRunState { req_seq: seq },
                |e| matches!(e,DaemonEvent::RunState{req_seq,..} if *req_seq==seq),
            )
            .await?;
        if let DaemonEvent::RunState { state, .. } = e {
            self.projection
                .readback(&state)
                .map_err(RpcError::unknown)?;
            self.publish();
            Ok(state)
        } else {
            let message = "Run state reply was not a run state.".to_string();
            crate::model::notifications::record(
                Some(self.projection.id.clone()),
                crate::model::notifications::Entry::new(message.clone(), "error", "sdk"),
            );
            Err(RpcError::new("unavailable", message))
        }
    }
    pub async fn state(&mut self) -> Result<State, RpcError> {
        self.readback().await?;
        // Force an ordered authoritative snapshot, including pending tool gate/history.
        self.rpc(ClientRequest::Resync, |e| {
            matches!(e, DaemonEvent::Snapshot(_))
        })
        .await?;
        self.projection.state().ok_or_else(|| {
            RpcError::new(
                "unavailable",
                "selected session has no authoritative snapshot",
            )
        })
    }
}

fn map_configure_error(error: RpcError) -> RpcError {
    if error.delivery == "unknown" {
        return error;
    }
    if let Some(rest) = error.message.strip_prefix("configure save failed") {
        return RpcError::new("unavailable", format!("configure save failed{rest}"));
    }
    if let Some(rest) = error.message.strip_prefix("configure: ") {
        return RpcError::new("invalid_request", rest.to_string());
    }
    error
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::ipc::proto::turn::SessionPolicy;
    use crate::ipc::proto::{ApprovalPolicy, PlanPolicy, TurnDisposition};
    async fn peer_frame(stream: &mut IpcStream, frame: DaemonFrame) {
        let bytes = serde_json::to_vec(&frame).unwrap();
        client::send_frame(stream, &bytes).await.unwrap();
    }
    fn connection(stream: IpcStream) -> Connection {
        let (events, _) = watch::channel(None);
        Connection {
            stream,
            reader: FrameReader::new(),
            projection: Reconcile::new("session".into()),
            ready: false,
            version: String::new(),
            capabilities: vec![],
            seq: 0,
            resync_needed: false,
            events,
        }
    }
    #[tokio::test]
    async fn correlation_does_not_consume_other_turn_result() {
        let (a, mut b) = tokio::net::UnixStream::pair().unwrap();
        let mut c = connection(a);
        let peer = tokio::spawn(async move {
            let bytes = client::recv_frame(&mut b, &mut FrameReader::new())
                .await
                .unwrap();
            let request: ClientRequest = serde_json::from_slice(&bytes).unwrap();
            assert!(
                matches!(request,ClientRequest::SubmitTurnWithAttachments{ref request_id,ref attachments,..} if request_id=="owned" && attachments.is_empty())
            );
            for (seq, id) in [(1, "other"), (2, "owned")] {
                peer_frame(
                    &mut b,
                    DaemonFrame {
                        seq,
                        event: DaemonEvent::TurnResult {
                            request_id: id.into(),
                            disposition: TurnDisposition::Started,
                            turn_id: Some(id.into()),
                            reason: None,
                        },
                    },
                )
                .await;
            }
        });
        let result = c
            .rpc(
                ClientRequest::SubmitTurnWithAttachments {
                    request_id: "owned".into(),
                    text: "hello".into(),
                    attachments: vec![],
                },
                |e| matches!(e,DaemonEvent::TurnResult{request_id,..} if request_id=="owned"),
            )
            .await
            .unwrap();
        assert!(matches!(result,DaemonEvent::TurnResult{turn_id:Some(id),..} if id=="owned"));
        peer.await.unwrap();
    }
    #[tokio::test]
    async fn disconnect_after_owned_write_is_unknown() {
        let (a, mut b) = tokio::net::UnixStream::pair().unwrap();
        let mut c = connection(a);
        let peer = tokio::spawn(async move {
            client::recv_frame(&mut b, &mut FrameReader::new())
                .await
                .unwrap();
        });
        let error = c
            .ack(ClientRequest::SetApprovalChannel { enabled: false })
            .await
            .unwrap_err();
        assert_eq!(error.delivery, "unknown");
        peer.await.unwrap();
    }
    #[tokio::test]
    async fn policy_readback_missing_is_not_silently_defaulted() {
        for policy in [
            None,
            Some(SessionPolicy {
                plan: PlanPolicy::Forbid,
                approval: ApprovalPolicy::Reject,
            }),
        ] {
            let (a, mut b) = tokio::net::UnixStream::pair().unwrap();
            let mut c = connection(a);
            let expected = policy.clone();
            let peer = tokio::spawn(async move {
                let bytes = client::recv_frame(&mut b, &mut FrameReader::new())
                    .await
                    .unwrap();
                let request: ClientRequest = serde_json::from_slice(&bytes).unwrap();
                let ClientRequest::GetRunState { req_seq } = request else {
                    panic!("expected readback")
                };
                peer_frame(
                    &mut b,
                    DaemonFrame {
                        seq: 1,
                        event: DaemonEvent::RunState {
                            req_seq,
                            state: RunState {
                                session_id: "session".into(),
                                policy,
                                mode: "auto".into(),
                                ..RunState::default()
                            },
                        },
                    },
                )
                .await;
            });
            let result = c.readback().await;
            assert_eq!(result.is_ok(), expected.is_some());
            peer.await.unwrap();
        }
    }
}
