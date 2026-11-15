//! Selected-session reducer. A sequence gap invalidates deltas until a full snapshot.
use super::dto::{DaemonInfo, Limits, SessionInfo};
use super::host::usage::SessionUsage;
use super::protocol::{Approval, Gate, Message, PendingApproval, Plan, Policy, State, Turn};
use crate::app::resolve::RoleResolution;
use crate::ipc::proto::{DaemonEvent, DaemonFrame, SessionSnapshot, StateDelta, TurnState};

pub struct Reconcile {
    pub id: String,
    pub session: Option<SessionSnapshot>,
    pub policy: Option<Policy>,
    last: Option<u64>,
    pub syncing: bool,
    pub external_approvals: bool,
    revision: u64,
    resync: u64,
    previous: Option<State>,
    /// Facts that go beside `state` in `session_info`. They never enter the
    /// projected `State`, so they never emit an event or bump `revision`.
    pub info: Tracked,
}

/// What the adapter last saw of the bound session beyond the v1 `state`.
#[derive(Debug, Clone, Default)]
pub struct Tracked {
    pub name: String,
    /// As the daemon stores it: `""` is the model default.
    pub effort: String,
    pub routes: Vec<RoleResolution>,
    pub max_output_tokens: u32,
    pub context_window_limit: u64,
    pub context_model_alias: String,
    pub effective_window: Option<u64>,
    pub daemon: DaemonInfo,
    /// Spend of the session as its latest snapshot reported it.
    pub usage: Option<SessionUsage>,
    /// The daemon's in-memory catalogue from the latest snapshot: provider
    /// uuids with names, and model uuids. Never any key.
    pub providers: Vec<(String, String)>,
    pub models: Vec<String>,
    /// Adapter clock when the newest contributing frame arrived.
    pub observed_at_ms: u64,
}

impl Tracked {
    fn observe(&mut self) {
        self.observed_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or_default()
            .max(self.observed_at_ms);
    }

    fn snapshot(&mut self, s: &SessionSnapshot) {
        self.name = s.name.clone();
        self.effort = s.effort.clone();
        if !s.model_routes.is_empty() {
            self.routes = s.model_routes.clone();
        }
        self.effective_window = s.context_usage.as_ref().map(|u| u.effective_window);
        self.usage = Some(SessionUsage::from_snapshot(s));
        self.observe();
    }

    fn catalogue(&mut self, g: &crate::ipc::proto::GlobalSnapshot) {
        self.providers = g
            .providers
            .iter()
            .map(|p| (p.uuid.clone(), p.name.clone()))
            .collect();
        self.models = g.config_models.iter().map(|m| m.uuid.clone()).collect();
    }

    fn run_state(&mut self, s: &crate::ipc::proto::RunState) {
        self.name = s.name.clone();
        self.effort = s.effort.clone();
        if !s.model_routes.is_empty() {
            self.routes = s.model_routes.clone();
        }
        self.max_output_tokens = s.max_output_tokens;
        self.context_window_limit = s.context_window_limit;
        self.context_model_alias = s.context_model_alias.clone();
        self.observe();
    }

    /// The session's Main route, if the daemon reported one.
    pub fn main(&self) -> Option<&RoleResolution> {
        self.routes
            .iter()
            .find(|r| r.role == crate::model::app_config::ModelRole::Main)
    }

    pub fn limits(&self) -> Limits {
        Limits {
            max_output_tokens: self.max_output_tokens,
            context_window_limit: self.context_window_limit,
            context_model_alias: self.context_model_alias.clone(),
            effective_window: self.effective_window,
        }
    }
}
impl Reconcile {
    pub fn new(id: String) -> Self {
        Self {
            id,
            session: None,
            policy: None,
            last: None,
            syncing: false,
            external_approvals: false,
            revision: 0,
            resync: 0,
            previous: None,
            info: Tracked::default(),
        }
    }
    /// Returns true exactly when a new resync request is needed.
    pub fn apply(&mut self, f: &DaemonFrame) -> bool {
        let gap = self.last.is_some_and(|n| f.seq != n + 1);
        self.last = Some(f.seq);
        let request = gap && !self.syncing;
        if gap {
            self.syncing = true;
        }
        if let DaemonEvent::Snapshot(s) = &f.event {
            if self.syncing {
                self.resync += 1;
            }
            self.session = s.sessions.iter().find(|s| s.id == self.id).cloned();
            if let Some(session) = &self.session {
                self.info.snapshot(session);
            }
            self.info.catalogue(&s.global);
            self.syncing = false;
            return false;
        }
        if self.syncing {
            return request;
        }
        if let (Some(s), DaemonEvent::Delta(d)) = (&mut self.session, &f.event) {
            match d {
                StateDelta::TokenAppended { session_id, text } if session_id == &self.id => {
                    s.streaming.get_or_insert_with(String::new).push_str(text)
                }
                StateDelta::SessionStatusChanged {
                    session_id,
                    working,
                    ..
                } if session_id == &self.id => s.working = *working,
                StateDelta::SessionAdded(snap) if snap.id == self.id => {
                    *s = *snap.clone();
                    self.info.snapshot(snap);
                }
                _ => {}
            }
        }
        false
    }
    pub fn state(&mut self) -> Option<State> {
        if self.syncing {
            return None;
        }
        let s = self.session.as_ref()?;
        let call = s.pending_tool_calls.get(s.tool_idx);
        let kind = if !s.awaiting_approval {
            None
        } else {
            Some(match call.map(|c| c.function.name.as_str()) {
                Some("plan_ready") => "plan",
                Some("mission_ready") => "mission",
                Some(_) if call.is_some_and(|c| !c.id.is_empty()) => "tool",
                _ => "unknown",
            })
        };
        let pending_approval = if kind == Some("tool") {
            call.map(|c| PendingApproval {
                call_id: c.id.clone(),
                reason: s.approval_reason.clone(),
            })
        } else {
            None
        };
        let gate = kind.map(|kind| Gate {
            kind,
            reason: s.approval_reason.clone(),
            supported: kind == "tool" && self.external_approvals,
        });
        let mut state = State {
            revision: self.revision,
            external_approvals: self.external_approvals,
            pending_approval,
            gate,
            resync: self.resync,
            session_id: s.id.clone(),
            cwd: s.cwd.clone(),
            model_id: s.resolved_model_id.clone(),
            policy: self.policy.clone()?,
            turn: s.turn.as_ref().map(|t| Turn {
                id: t.id.clone(),
                state: match t.state {
                    TurnState::Running => "running",
                    TurnState::Finished => "finished",
                    TurnState::Interrupted => "interrupted",
                    TurnState::Failed => "failed",
                },
                error: t.error.clone(),
            }),
            working: s.working,
            awaiting_approval: s.awaiting_approval,
            approval_reason: s.approval_reason.clone(),
            pending_count: s.pending_steer.len(),
            messages: s
                .messages
                .iter()
                .map(|m| Message {
                    role: match m.role {
                        crate::dto::chat::Role::System => "system",
                        crate::dto::chat::Role::User => "user",
                        crate::dto::chat::Role::Assistant => "assistant",
                        crate::dto::chat::Role::Tool => "tool",
                    },
                    content: m.content.clone(),
                })
                .collect(),
            streaming: s.streaming.clone(),
        };
        if self.previous.as_ref() != Some(&state) {
            self.revision += 1;
            state.revision = self.revision;
            self.previous = Some(state.clone());
        }
        Some(state)
    }
    pub fn readback(&mut self, s: &crate::ipc::proto::RunState) -> anyhow::Result<()> {
        anyhow::ensure!(s.session_id == self.id, "daemon returned another session");
        let p = s
            .policy
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("daemon omitted policy readback"))?;
        self.policy = Some(Policy {
            plan: match p.plan {
                crate::ipc::proto::PlanPolicy::Allow => Plan::Allow,
                crate::ipc::proto::PlanPolicy::Forbid => Plan::Forbid,
            },
            approval: match p.approval {
                crate::ipc::proto::ApprovalPolicy::Default => Approval::Default,
                crate::ipc::proto::ApprovalPolicy::Reject => Approval::Reject,
            },
        });
        // RunState confirms policy only. Turn/output/gate fields must remain from
        // the same selected-session snapshot until an authoritative update arrives.
        // Its name, effort, routes and limits feed `session_info` only.
        self.info.run_state(s);
        Ok(())
    }
    /// The `session_info` sibling of `state`. Provider names come from the
    /// daemon's in-memory catalogue, then from `names`.
    pub fn session_info(&self, names: impl Fn(&str) -> Option<String>) -> SessionInfo {
        let info = &self.info;
        let lookup = |uuid: &str| {
            info.providers
                .iter()
                .find(|(id, _)| id == uuid)
                .map(|(_, name)| name.clone())
                .or_else(|| names(uuid))
        };
        SessionInfo {
            name: info.name.clone(),
            effort: public_effort(&info.effort),
            routes: super::host::redact::role_routes(&info.routes, lookup),
            limits: info.limits(),
            usage: info
                .usage
                .as_ref()
                .and_then(|u| serde_json::to_value(u).ok()),
            daemon: info.daemon.clone(),
            observed_at_ms: info.observed_at_ms,
        }
    }
}

/// The daemon stores the model default as `""`; every public output spells it
/// `"default"`.
pub fn public_effort(stored: &str) -> String {
    if stored.is_empty() {
        "default".to_string()
    } else {
        stored.to_string()
    }
}
