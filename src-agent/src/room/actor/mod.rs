//! The room actor: one state machine that owns the room and connects its
//! parts.
//!
//! Everything arrives as an [`ActorMsg`] on one channel, so the actor is a
//! single-threaded loop with no locks of its own: the user and the control
//! channel from the driver, a periodic [`ActorMsg::Tick`], events from the
//! running lanes, and the results of the tools those lanes call. The room's
//! visible output leaves as a [`RoomEvent`].
//!
//! What the actor does:
//! - keeps the Chief's chat lane and the specialist and clerk work lanes;
//! - starts a lane for each [`Effect::StartLane`] the coordinator returns, and
//!   steers or shows the others;
//! - folds each lane's events into the digest and the swim-lane timeline, and
//!   offers a probe's publishes to the review queue;
//! - on every tick runs the reconciler (sync, then wake or check in the Chief)
//!   and the clerk (audit, then a pass when one is due);
//! - drains the Chief's inbox into one chat turn when the Chief is idle, so an
//!   unread user message always leads.
//!
//! The Chief has the chat lane the user talks to; specialists and the clerk run
//! work lanes. Per-specialist chat lanes are a later enhancement.

mod message;
mod router;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::task::{AbortHandle, JoinHandle};

pub use message::{ActorMsg, RoomEvent, StatusSnapshot};
#[allow(unused_imports)]
pub use router::{BuiltinHost, LaneRef, NoBuiltins, RoomToolRouter};

use crate::room::clerk::{AuditConfig, AuditInput, Clerk, Tick as ClerkTick};
use crate::room::lane::{
    run_lane, ControlAction, ControlState, Digest, Inbox, InboxItem, LaneConfig, LaneEvent,
    LaneInput, LaneKind, ModelClient, Timeline,
};
use crate::room::registry::RegistryHandle;
use crate::room::shelf::Shelf;
use crate::room::transition::{LaneRunState, Observation};
use crate::room::trajectory::gate;
use crate::room::work::{Coordinator, Effect, Reconciler};

/// Limits and cadences for a room.
#[derive(Debug, Clone, Copy)]
pub struct RoomConfig {
    pub chat_max_steps: usize,
    pub work_max_steps: usize,
    pub audit: AuditConfig,
}

impl Default for RoomConfig {
    fn default() -> Self {
        Self { chat_max_steps: 12, work_max_steps: 60, audit: AuditConfig::default() }
    }
}

/// A running lane: how to steer it, how to stop it, and what the reconciler
/// should read about it.
struct LaneHandle {
    kind: LaneKind,
    assignment_id: Option<String>,
    is_chief: bool,
    inject: UnboundedSender<LaneInput>,
    abort: AbortHandle,
    run: LaneRunState,
    /// Stopped but not proven idle, so the reconciler reads it as uncertain.
    unsettled: bool,
    ended: bool,
}

/// A started room: send it [`ActorMsg`]s and drain its [`RoomEvent`]s. The
/// caller drives [`ActorMsg::Tick`] on a cadence (the daemon uses a timer;
/// tests tick by hand).
pub struct RoomHandle {
    pub to_actor: UnboundedSender<ActorMsg>,
    pub events: UnboundedReceiver<RoomEvent>,
    pub task: JoinHandle<()>,
}

/// Start a room actor on its own task.
pub fn spawn_room(
    coordinator: Arc<Coordinator>,
    shelf: Arc<Shelf>,
    registry: Arc<RegistryHandle>,
    model: Arc<dyn ModelClient>,
    builtins: Arc<dyn BuiltinHost>,
    config: RoomConfig,
) -> RoomHandle {
    let (to_actor, rx) = unbounded_channel();
    let (out, events) = unbounded_channel();
    let actor = RoomActor {
        coordinator,
        shelf,
        registry,
        model,
        builtins,
        clerk: Arc::new(Mutex::new(Clerk::new())),
        reconciler: Reconciler::new(),
        inbox: Inbox::new(),
        control: ControlState::default(),
        digest: Digest::new(),
        timeline: Timeline::new(),
        lanes: BTreeMap::new(),
        assignment_lane: BTreeMap::new(),
        seen_nonces: HashSet::new(),
        chief_busy: false,
        to_actor: to_actor.clone(),
        out,
        config,
    };
    let task = tokio::spawn(actor.run(rx));
    RoomHandle { to_actor, events, task }
}

struct RoomActor {
    coordinator: Arc<Coordinator>,
    shelf: Arc<Shelf>,
    registry: Arc<RegistryHandle>,
    model: Arc<dyn ModelClient>,
    builtins: Arc<dyn BuiltinHost>,
    clerk: Arc<Mutex<Clerk>>,
    reconciler: Reconciler,
    inbox: Inbox,
    control: ControlState,
    digest: Digest,
    timeline: Timeline,
    lanes: BTreeMap<String, LaneHandle>,
    assignment_lane: BTreeMap<String, String>,
    seen_nonces: HashSet<String>,
    chief_busy: bool,
    to_actor: UnboundedSender<ActorMsg>,
    out: UnboundedSender<RoomEvent>,
    config: RoomConfig,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

impl RoomActor {
    async fn run(mut self, mut rx: UnboundedReceiver<ActorMsg>) {
        while let Some(msg) = rx.recv().await {
            match msg {
                ActorMsg::User(text) => self.on_user(text),
                ActorMsg::Control(cmd) => self.on_control(cmd),
                ActorMsg::Tick => self.on_tick(),
                ActorMsg::LaneEvent { avatar, event } => self.on_lane_event(&avatar, event),
                ActorMsg::Effects(effects) => self.apply_effects(effects),
                ActorMsg::Flag(item) => {
                    self.inbox.push(item);
                    self.maybe_start_chief();
                }
                ActorMsg::MessageUser { from, text } => {
                    let _ = self.out.send(if from == "chief" {
                        RoomEvent::Chief(text)
                    } else {
                        RoomEvent::LaneChat { avatar: from, text }
                    });
                }
                ActorMsg::Shutdown => break,
            }
        }
        for lane in self.lanes.values() {
            lane.abort.abort();
        }
        let _ = self.out.send(RoomEvent::Closed);
    }

    fn emit(&self, event: RoomEvent) {
        let _ = self.out.send(event);
    }

    // ---- user and control ----

    fn on_user(&mut self, text: String) {
        if self.chief_busy {
            // Preempt the running Chief turn: it sees the message at its next boundary.
            if let Some(chief) = self.lanes.get("chief") {
                let _ = chief.inject.send(LaneInput::Steer(format!("The user says: {text}")));
            }
            return;
        }
        self.inbox.push(InboxItem::User(text));
        self.maybe_start_chief();
    }

    fn on_control(&mut self, cmd: crate::room::lane::ControlCommand) {
        let running: std::collections::BTreeSet<String> =
            self.lanes.iter().filter(|(_, l)| !l.ended && !l.is_chief).map(|(a, _)| a.clone()).collect();
        match self.control.apply(cmd, &running) {
            ControlAction::Pause(avatar) => self.inject(&avatar, LaneInput::Pause),
            ControlAction::Resume(avatar) => self.inject(&avatar, LaneInput::Resume),
            ControlAction::Stop(avatar) => self.stop_lane(&avatar),
            ControlAction::Inject { avatar, text } => self.inject(&avatar, LaneInput::Steer(text)),
            ControlAction::Noop(reason) => self.emit(RoomEvent::Visible { author: "room".into(), text: reason }),
        }
    }

    fn inject(&self, avatar: &str, input: LaneInput) {
        if let Some(lane) = self.lanes.get(avatar) {
            let _ = lane.inject.send(input);
        }
    }

    fn stop_lane(&mut self, avatar: &str) {
        if let Some(lane) = self.lanes.get_mut(avatar) {
            lane.abort.abort();
            lane.unsettled = true;
            lane.ended = true;
            let _ = lane.inject.send(LaneInput::Stop);
        }
    }

    // ---- lane events ----

    fn on_lane_event(&mut self, avatar: &str, event: LaneEvent) {
        let now = now_ms();
        self.digest.observe_at(avatar, &event, now);
        self.timeline.record(avatar, &event, now);
        let is_chief = self.lanes.get(avatar).is_some_and(|l| l.is_chief);
        match event {
            LaneEvent::Chat(text) if is_chief => self.emit(RoomEvent::Chief(text)),
            LaneEvent::Chat(text) => self.emit(RoomEvent::LaneChat { avatar: avatar.into(), text }),
            LaneEvent::Finished { report } => self.lane_ended(avatar, LaneRunState::Completed, is_chief, Some(report)),
            LaneEvent::Failed { error } => {
                self.emit(RoomEvent::Visible { author: avatar.into(), text: format!("lane failed: {error}") });
                self.lane_ended(avatar, LaneRunState::Failed, is_chief, None);
            }
            _ => {}
        }
    }

    fn lane_ended(&mut self, avatar: &str, run: LaneRunState, is_chief: bool, report: Option<String>) {
        if let Some(lane) = self.lanes.get_mut(avatar) {
            lane.ended = true;
            lane.run = run;
        }
        if is_chief {
            self.chief_busy = false;
            self.lanes.remove("chief");
            let _ = report; // the chief's words already streamed as Chat events
            self.maybe_start_chief();
        }
    }

    // ---- effects from coordinator tool calls ----

    fn apply_effects(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::StartLane { assignment, goal } => {
                    if self.lanes.get(&assignment.avatar).is_some_and(|l| !l.ended) {
                        continue; // one lane per avatar at a time
                    }
                    let registry = self.registry.current();
                    let role = registry.role_of(&assignment.avatar);
                    let prompt = role.map(|r| r.prompt.clone()).unwrap_or_default();
                    let task = format!("Goal: {goal}\n\nYour assignment:\n{}", assignment.prompt);
                    let model = registry.lane_model(&assignment.avatar, LaneKind::Work);
                    let config = LaneConfig {
                        avatar: assignment.avatar.clone(),
                        kind: LaneKind::Work,
                        system: prompt,
                        task,
                        model,
                        max_steps: self.config.work_max_steps,
                    };
                    self.timeline.open(&assignment.avatar, assignment.kind.as_str(), now_ms());
                    self.assignment_lane.insert(assignment.id.clone(), assignment.avatar.clone());
                    self.start_lane(config, LaneRef {
                        avatar: assignment.avatar.clone(),
                        kind: LaneKind::Work,
                        assignment_id: Some(assignment.id),
                        is_chief: false,
                        is_clerk: false,
                    });
                }
                Effect::Steer { avatar, text } => self.inject(&avatar, LaneInput::Steer(text)),
                Effect::Visible { author, text, nonce } => {
                    if self.seen_nonces.insert(nonce) {
                        self.emit(RoomEvent::Visible { author, text });
                    }
                }
            }
        }
    }

    // ---- the Chief's chat turn ----

    fn maybe_start_chief(&mut self) {
        if self.chief_busy || !self.inbox.has_pending() {
            return;
        }
        let registry = self.registry.current();
        let items = self.inbox.drain();
        let task = self.build_chief_turn(items);
        let prompt = registry.role("chief").map(|r| r.prompt.clone()).unwrap_or_default();
        let model = registry.lane_model("chief", LaneKind::Chat);
        let config = LaneConfig { avatar: "chief".into(), kind: LaneKind::Chat, system: prompt, task, model, max_steps: self.config.chat_max_steps };
        self.chief_busy = true;
        self.timeline.open("chief", "chief", now_ms());
        self.start_lane(config, LaneRef { avatar: "chief".into(), kind: LaneKind::Chat, assignment_id: None, is_chief: true, is_clerk: false });
    }

    fn build_chief_turn(&self, items: Vec<InboxItem>) -> String {
        let brief = self.coordinator.active_brief();
        let registry = self.registry.current();
        let render = |template: &str| -> String {
            match &brief {
                Some((key, goal, open)) => registry
                    .render(template, &[("goal_key", key), ("goal", goal), ("open_goals", &open.join("; "))])
                    .unwrap_or_else(|_| format!("Continue goal {key}.")),
                None => "A batch needs your attention. Use group_status.".to_string(),
            }
        };
        let mut parts = Vec::new();
        for item in items {
            parts.push(match item {
                InboxItem::User(text) => text,
                InboxItem::ClerkFlag { avatar, severity, note } => {
                    format!("Clerk flag [{severity}] on {avatar}: {note}")
                }
                InboxItem::LaneResult { avatar, summary } => format!("{avatar} reported: {summary}"),
                InboxItem::Wake { .. } => render("wake"),
                InboxItem::CheckIn { .. } => render("checkin"),
            });
        }
        parts.join("\n\n")
    }

    // ---- the periodic tick ----

    fn on_tick(&mut self) {
        self.reconcile();
        self.run_clerk();
        self.maybe_start_chief();
        self.emit_status();
    }

    fn reconcile(&mut self) {
        let now = now_ms();
        let coordinator = Arc::clone(&self.coordinator);
        let observations: BTreeMap<String, Observation> = self
            .assignment_lane
            .iter()
            .map(|(id, avatar)| {
                let obs = self.lanes.get(avatar).map_or(Observation::default(), |l| Observation {
                    run: Some(l.run),
                    unsettled: l.unsettled,
                });
                (id.clone(), obs)
            })
            .collect();
        let busy: HashSet<String> = self.lanes.iter().filter(|(_, l)| !l.ended).map(|(a, _)| a.clone()).collect();

        let outcome = coordinator.reconcile(
            &mut self.reconciler,
            |id| observations.get(id).copied().unwrap_or_default(),
            |avatar| !busy.contains(avatar),
            now,
        );
        let Ok(outcome) = outcome else { return };

        for work_id in outcome.wakes {
            self.inbox.push(InboxItem::Wake { work_id });
        }
        for work_id in outcome.checkins {
            self.inbox.push(InboxItem::CheckIn { work_id });
        }
        // An assignment the reconciler settled frees its avatar's lane slot.
        for (id, status) in &outcome.status_changes {
            if !status.is_pending() {
                if let Some(avatar) = self.assignment_lane.remove(id) {
                    self.lanes.remove(&avatar);
                }
            }
        }
        self.apply_effects(outcome.effects);
    }

    fn run_clerk(&mut self) {
        let Some(work) = self.coordinator.active_work() else { return };
        if self.lanes.get("clerk").is_some_and(|l| !l.ended) {
            return;
        }
        let paused: std::collections::BTreeSet<String> = self.control.paused().cloned().collect();
        let input = AuditInput {
            shelf: &self.shelf,
            digest: &self.digest,
            assignments: &work.assignments,
            paused: &paused,
            now_ms: now_ms(),
            config: self.config.audit,
        };
        let registry = self.registry.current();
        let pass = {
            let mut clerk = self.clerk.lock().unwrap_or_else(|e| e.into_inner());
            clerk.tick(&input)
        };
        let Ok(ClerkTick::Run(pass)) = pass else { return };
        let brief = Clerk::brief(&registry, &pass, &self.digest.render()).unwrap_or_default();
        let model = registry.lane_model("clerk", LaneKind::Work);
        let config = LaneConfig {
            avatar: "clerk".into(),
            kind: LaneKind::Work,
            system: registry.role("clerk").map(|r| r.prompt.clone()).unwrap_or_default(),
            task: brief,
            model,
            max_steps: self.config.work_max_steps,
        };
        self.timeline.open("clerk", "clerk", now_ms());
        self.start_lane(config, LaneRef { avatar: "clerk".into(), kind: LaneKind::Work, assignment_id: None, is_chief: false, is_clerk: true });
    }

    fn emit_status(&self) {
        let counts = gate::count(&self.shelf.list_items(&[], true, 400).unwrap_or_default());
        self.emit(RoomEvent::Status(StatusSnapshot {
            digest: self.digest.render(),
            timeline: self.timeline.render(120),
            counts,
            paused: self.control.paused().cloned().collect(),
        }));
    }

    // ---- spawning a lane ----

    fn start_lane(&mut self, config: LaneConfig, lane: LaneRef) {
        let avatar = lane.avatar.clone();
        let (kind, is_chief, assignment_id) = (lane.kind, lane.is_chief, lane.assignment_id.clone());
        let is_clerk = lane.is_clerk;
        let registry = self.registry.current();
        let router = Arc::new(RoomToolRouter::new(
            lane,
            Arc::clone(&self.coordinator),
            Arc::clone(&self.shelf),
            Arc::clone(&self.clerk),
            registry,
            Arc::clone(&self.builtins),
            self.to_actor.clone(),
        ));
        let (inject_tx, inject_rx) = unbounded_channel();
        let (ev_tx, mut ev_rx) = unbounded_channel();
        let task = tokio::spawn(run_lane(config, Arc::clone(&self.model), router, inject_rx, ev_tx));

        // Forward the lane's events into the actor's one channel, tagged by avatar.
        let to_actor = self.to_actor.clone();
        let forward_avatar = avatar.clone();
        tokio::spawn(async move {
            while let Some(event) = ev_rx.recv().await {
                if to_actor.send(ActorMsg::LaneEvent { avatar: forward_avatar.clone(), event }).is_err() {
                    break;
                }
            }
        });
        let _ = is_clerk;
        self.lanes.insert(
            avatar,
            LaneHandle {
                kind,
                assignment_id,
                is_chief,
                inject: inject_tx,
                abort: task.abort_handle(),
                run: LaneRunState::Running,
                unsettled: false,
                ended: false,
            },
        );
    }
}
