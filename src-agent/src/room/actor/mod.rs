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
//! - keeps a chat lane and a work lane for each avatar, so a specialist can
//!   answer the user while its work lane is running;
//! - starts a lane for each [`Effect::StartLane`] the coordinator returns, and
//!   steers or shows the others;
//! - folds each lane's events into the digest and the swim-lane timeline, and
//!   offers a probe's publishes to the review queue;
//! - on every tick runs the reconciler (sync, then wake or check in the Chief)
//!   and the clerk (audit, then a pass when one is due);
//! - drains the Chief's inbox into one chat turn when the Chief is idle, so an
//!   unread user message always leads.
//!
//! A bare user line goes to the Chief. `@avatar` opens that specialist's chat
//! lane. The clerk has no chat lane: it only speaks to the Chief.

mod message;
mod router;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::task::{AbortHandle, JoinHandle};

pub use message::{ActorMsg, EventInfo, LaneInfo, LanePart, RoomEvent, StatusSnapshot};
#[allow(unused_imports)]
pub use router::{BuiltinHost, LaneRef, NoBuiltins, RoomToolRouter};

use crate::room::clerk::{AuditConfig, AuditInput, Clerk, Tick as ClerkTick};
use crate::room::lane::{
    run_lane, ControlAction, ControlState, Digest, Inbox, InboxItem, LaneConfig, LaneEvent,
    LaneInput, LaneKind, ModelClient, Timeline, UserImage,
};
use crate::room::registry::{Registry, RegistryHandle};
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
        Self {
            chat_max_steps: 60,
            work_max_steps: 60,
            audit: AuditConfig::default(),
        }
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
    started_ms: u64,
    last_event_ms: u64,
    steps: usize,
    last_tool: Option<String>,
}

/// Chat and work for one avatar are different lanes. The map key keeps them
/// from displacing each other.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Slot {
    avatar: String,
    kind: LaneKind,
}

impl Slot {
    fn new(avatar: impl Into<String>, kind: LaneKind) -> Self {
        Self { avatar: avatar.into(), kind }
    }
}

/// Where a typed line goes.
enum Addressed {
    /// The Chief's inbox, text unchanged.
    Chief(String),
    /// A specialist chat lane.
    Avatar { id: String, body: String },
    /// `@clerk` — the clerk does not take user chat.
    Clerk,
}

fn addressed(registry: &Registry, text: &str) -> Addressed {
    let Some(rest) = text.trim().strip_prefix('@') else {
        return Addressed::Chief(text.to_string());
    };
    let (name, body) = match rest.split_once(char::is_whitespace) {
        Some((name, body)) => (name, body.trim()),
        None => (rest, ""),
    };
    if name.is_empty() || body.is_empty() {
        return Addressed::Chief(text.to_string());
    }
    if name.eq_ignore_ascii_case("clerk") {
        return Addressed::Clerk;
    }
    if name == "chief" {
        return Addressed::Chief(body.to_string());
    }
    if registry.avatar(name).is_some() {
        return Addressed::Avatar { id: name.to_string(), body: body.to_string() };
    }
    Addressed::Chief(text.to_string())
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
    spawn_room_in(coordinator, shelf, registry, model, builtins, config, None)
}

/// Like [`spawn_room`], keeping each lane's conversation under `lanes_dir`
/// (`<avatar>.<chat|work>.jsonl`) so it survives turns and restarts.
pub fn spawn_room_in(
    coordinator: Arc<Coordinator>,
    shelf: Arc<Shelf>,
    registry: Arc<RegistryHandle>,
    model: Arc<dyn ModelClient>,
    builtins: Arc<dyn BuiltinHost>,
    config: RoomConfig,
    lanes_dir: Option<std::path::PathBuf>,
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
        chief_busy: false,
        resuming: false,
        turn_images: Vec::new(),
        chief_images: Vec::new(),
        lanes_dir,
        to_actor: to_actor.clone(),
        out,
        config,
    };
    let task = tokio::spawn(actor.run(rx));
    RoomHandle { to_actor, events, task }
}

struct RoomActor {
    /// Where lane transcripts live; `None` keeps them in memory only.
    lanes_dir: Option<std::path::PathBuf>,
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
    lanes: BTreeMap<Slot, LaneHandle>,
    assignment_lane: BTreeMap<String, String>,
    chief_busy: bool,
    /// Lanes are being started again after the room was entered anew.
    resuming: bool,
    /// Images from the user's latest message, handed to the lanes the Chief starts this turn.
    turn_images: Vec<UserImage>,
    /// Images waiting for the Chief's next turn.
    chief_images: Vec<UserImage>,
    to_actor: UnboundedSender<ActorMsg>,
    out: UnboundedSender<RoomEvent>,
    config: RoomConfig,
}

/// A steer with the images the user shared, when there are any.
fn steer_with(text: String, images: Vec<UserImage>) -> LaneInput {
    if images.is_empty() {
        LaneInput::Steer(text)
    } else {
        LaneInput::SteerImages(text, images)
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

impl RoomActor {
    async fn run(mut self, mut rx: UnboundedReceiver<ActorMsg>) {
        self.resume_work();
        while let Some(msg) = rx.recv().await {
            match msg {
                ActorMsg::User(text) => self.on_user(text, Vec::new()),
                ActorMsg::UserWith { text, images } => self.on_user(text, images),
                ActorMsg::Control(cmd) => self.on_control(cmd),
                ActorMsg::Interrupt => self.interrupt(),
                ActorMsg::Tick => self.on_tick(),
                ActorMsg::LaneEvent { avatar, kind, event } => self.on_lane_event(&avatar, kind, event),
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
                ActorMsg::Resync => self.emit_status(),
                ActorMsg::Browse => self.emit(RoomEvent::Browse(crate::room::browse::build(&self.shelf))),
                ActorMsg::Open { key } => match crate::room::browse::item(&self.shelf, &key) {
                    Some(item) => self.emit(RoomEvent::Item(item)),
                    None => self.emit(RoomEvent::Notice(format!("{key} is not on the shelf"))),
                },
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

    /// `!cmd` runs a shell command right away, without the model, like the
    /// coding chat's `!` shortcut. It goes through the same `bash` tool.
    fn run_shell_shortcut(&self, command: String) {
        let builtins = Arc::clone(&self.builtins);
        let out = self.out.clone();
        let call = crate::dto::chat::ToolCall {
            id: "shell-shortcut".into(),
            kind: "function".into(),
            function: crate::dto::chat::FunctionCall {
                name: "bash".into(),
                arguments: serde_json::json!({ "command": command }).to_string(),
            },
        };
        tokio::task::spawn_blocking(move || {
            let result = builtins.run("chief", LaneKind::Chat, &call);
            let _ = out.send(RoomEvent::Visible { author: "shell".into(), text: format!("$ {command}\n{result}") });
        });
    }

    fn on_user(&mut self, text: String, images: Vec<UserImage>) {
        if let Some(command) = text.trim().strip_prefix('!').map(str::trim).filter(|c| !c.is_empty()) {
            self.run_shell_shortcut(command.to_string());
            return;
        }
        self.turn_images = images.clone();
        let registry = self.registry.current();
        match addressed(&registry, &text) {
            Addressed::Clerk => self.emit(RoomEvent::Visible {
                author: "room".into(),
                text: "the clerk only speaks to the chief".into(),
            }),
            Addressed::Avatar { id, body } => self.speak_to(&id, body, images),
            Addressed::Chief(text) => self.tell_chief(text, images),
        }
    }

    fn tell_chief(&mut self, text: String, images: Vec<UserImage>) {
        let slot = Slot::new("chief", LaneKind::Chat);
        if self.chief_busy {
            // Preempt the running Chief turn: it sees the message at its next boundary.
            if let Some(chief) = self.lanes.get(&slot) {
                let _ = chief.inject.send(steer_with(format!("The user says: {text}"), images));
            }
            return;
        }
        self.chief_images.extend(images);
        self.inbox.push(InboxItem::User(text));
        self.maybe_start_chief();
    }

    /// Open or steer one specialist's chat lane. Its work lane, if any, keeps running.
    fn speak_to(&mut self, avatar: &str, text: String, images: Vec<UserImage>) {
        let slot = Slot::new(avatar, LaneKind::Chat);
        if self.lanes.get(&slot).is_some_and(|lane| !lane.ended) {
            if let Some(lane) = self.lanes.get(&slot) {
                let _ = lane.inject.send(steer_with(format!("The user says: {text}"), images));
            }
            return;
        }
        let registry = self.registry.current();
        // A chat lane is the same agent with the same tools, spoken to in a friendlier register.
        let mut prompt = registry.role_of(avatar).map(|role| role.prompt.clone()).unwrap_or_default();
        if let Ok(chat) = registry.render("chat_lane", &[]) {
            prompt.push_str("\n\n");
            prompt.push_str(&chat);
        }
        let digest = self.digest.render();
        let task = if digest.is_empty() {
            text
        } else {
            format!("{digest}\n\nThe user says:\n{text}")
        };
        let model = registry.lane_model(avatar, LaneKind::Chat);
        let config = LaneConfig {
            avatar: avatar.to_string(),
            kind: LaneKind::Chat,
            system: prompt,
            task,
            task_images: images,
            model,
            max_steps: self.config.chat_max_steps,
            transcript: self.transcript_for(avatar, LaneKind::Chat),
            objective: String::new(),
        };
        self.start_lane(config, LaneRef {
            avatar: avatar.to_string(),
            kind: LaneKind::Chat,
            assignment_id: None,
            is_chief: false,
            is_clerk: false,
        });
    }

    fn on_control(&mut self, cmd: crate::room::lane::ControlCommand) {
        if let crate::room::lane::ControlCommand::Stop { avatar } = &cmd {
            let avatar = avatar.clone();
            // A stopped lane is not paused any more.
            let _ = self.control.apply(cmd, &std::collections::BTreeSet::new());
            self.stop_avatar(&avatar);
            return;
        }
        let running: std::collections::BTreeSet<String> = self
            .lanes
            .iter()
            .filter(|(slot, lane)| slot.kind == LaneKind::Work && !lane.ended && !lane.is_chief)
            .map(|(slot, _)| slot.avatar.clone())
            .collect();
        match self.control.apply(cmd, &running) {
            ControlAction::Pause(avatar) => self.inject_work(&avatar, LaneInput::Pause),
            ControlAction::Resume(avatar) => self.inject_work(&avatar, LaneInput::Resume),
            ControlAction::Stop(avatar) => self.stop_slot(&Slot::new(&avatar, LaneKind::Work)),
            ControlAction::Inject { avatar, text } => self.inject_work(&avatar, LaneInput::Steer(text)),
            ControlAction::Noop(reason) => self.emit(RoomEvent::Visible { author: "room".into(), text: reason }),
        }
    }

    fn inject_work(&self, avatar: &str, input: LaneInput) {
        if let Some(lane) = self.lanes.get(&Slot::new(avatar, LaneKind::Work)) {
            let _ = lane.inject.send(input);
        }
    }

    /// `/stop red` stops that avatar's lanes, chat and work; `/stop all` stops every lane.
    fn stop_avatar(&mut self, who: &str) {
        let slots: Vec<Slot> = self
            .lanes
            .iter()
            .filter(|(slot, lane)| !lane.ended && slot.avatar != "clerk" && (who == "all" || slot.avatar == who))
            .map(|(slot, _)| slot.clone())
            .collect();
        if slots.is_empty() {
            self.emit(RoomEvent::Visible { author: "room".into(), text: format!("{who} has nothing running to stop") });
            return;
        }
        for slot in slots {
            self.stop_slot(&slot);
        }
        self.emit_status();
    }

    /// Esc: the Chief's and the specialists' conversations stop; the work keeps running.
    fn interrupt(&mut self) {
        let slots: Vec<Slot> = self
            .lanes
            .iter()
            .filter(|(slot, lane)| slot.kind == LaneKind::Chat && !lane.ended)
            .map(|(slot, _)| slot.clone())
            .collect();
        for slot in slots {
            self.stop_slot(&slot);
        }
        self.emit_status();
    }

    fn stop_slot(&mut self, slot: &Slot) {
        let now = now_ms();
        let Some(lane) = self.lanes.get_mut(slot) else { return };
        lane.abort.abort();
        let _ = lane.inject.send(LaneInput::Stop);
        let is_chief = lane.is_chief;
        match slot.kind {
            LaneKind::Work => {
                lane.unsettled = true;
                lane.ended = true;
                self.digest.stopped(&slot.avatar, now);
                self.timeline.stopped(&slot.avatar, now);
            }
            LaneKind::Chat => {
                self.lanes.remove(slot);
                if is_chief {
                    self.chief_busy = false;
                    self.turn_images.clear();
                }
            }
        }
        let which = if slot.kind == LaneKind::Chat { "chat" } else { "work" };
        self.emit(RoomEvent::Visible { author: slot.avatar.clone(), text: format!("stopped by you ({which})") });
    }

    // ---- lane events ----

    fn on_lane_event(&mut self, avatar: &str, kind: LaneKind, event: LaneEvent) {
        let now = now_ms();
        // Chat turns must not rewrite the work digest. A finished chat would
        // otherwise look like the assignment ended.
        if kind == LaneKind::Work {
            self.digest.observe_at(avatar, &event, now);
            self.timeline.record(avatar, &event, now);
        }
        if let Some(lane) = self.lanes.get_mut(&Slot::new(avatar, kind)) {
            lane.last_event_ms = now;
            match &event {
                LaneEvent::Step(n) => lane.steps = lane.steps.max(n + 1),
                LaneEvent::ToolStarted { name } => lane.last_tool = Some(name.clone()),
                _ => {}
            }
        }
        let is_chief = self.lanes.get(&Slot::new(avatar, kind)).is_some_and(|lane| lane.is_chief);
        match event {
            LaneEvent::Reasoning(text) => self.emit_part(avatar, kind, LanePart::Reasoning { text }),
            LaneEvent::TextDelta(text) => self.emit_part(avatar, kind, LanePart::TextDelta { text }),
            LaneEvent::ReasoningDelta(text) => self.emit_part(avatar, kind, LanePart::ReasoningDelta { text }),
            LaneEvent::ToolCall { id, name, args } => self.emit_part(avatar, kind, LanePart::ToolCall { id, name, args }),
            LaneEvent::ToolResult { id, ok, result } => self.emit_part(avatar, kind, LanePart::ToolResult { id, ok, result }),
            LaneEvent::Chat(text) if is_chief => self.emit(RoomEvent::Chief(text)),
            LaneEvent::Chat(text) => self.emit(RoomEvent::LaneChat { avatar: avatar.into(), text }),
            LaneEvent::Finished { report } => {
                self.lane_ended(avatar, kind, LaneRunState::Completed, Some(report));
            }
            LaneEvent::Failed { error } => {
                self.emit(RoomEvent::Visible { author: avatar.into(), text: format!("lane failed: {error}") });
                self.lane_ended(avatar, kind, LaneRunState::Failed, None);
            }
            _ => {}
        }
    }

    /// Forward one structured part of a lane's turn. The clerk's audit passes
    /// are internal and never shown.
    fn emit_part(&self, avatar: &str, kind: LaneKind, part: LanePart) {
        if avatar == "clerk" {
            return;
        }
        let lane = if kind == LaneKind::Chat { "chat" } else { "work" };
        self.emit(RoomEvent::Part { author: avatar.into(), lane: lane.into(), part });
    }

    fn lane_ended(&mut self, avatar: &str, kind: LaneKind, run: LaneRunState, report: Option<String>) {
        let slot = Slot::new(avatar, kind);
        let is_chief = self.lanes.get(&slot).is_some_and(|lane| lane.is_chief);
        if let Some(lane) = self.lanes.get_mut(&slot) {
            lane.ended = true;
            lane.run = run;
        }
        if kind == LaneKind::Chat {
            self.lanes.remove(&slot);
            let _ = report; // the words already streamed as Chat events
            if is_chief {
                self.chief_busy = false;
                self.turn_images.clear();
                self.maybe_start_chief();
            }
        }
    }

    /// A room entered again picks its assignments up where they were: a specialist that was
    /// mid-assignment when the room closed continues its own conversation instead of being
    /// written off as failed.
    fn resume_work(&mut self) {
        let Some(work) = self.coordinator.active_work() else { return };
        let effects: Vec<Effect> = work
            .assignments
            .iter()
            .filter(|a| a.status.is_pending() && a.status != crate::room::types::AssignmentStatus::Submitted)
            .map(|a| Effect::StartLane { assignment: a.clone(), goal: work.prompt.clone() })
            .collect();
        if effects.is_empty() {
            return;
        }
        self.resuming = true;
        self.apply_effects(effects);
        self.resuming = false;
    }

    // ---- effects from coordinator tool calls ----

    fn apply_effects(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::StartLane { assignment, goal } => {
                    let slot = Slot::new(&assignment.avatar, LaneKind::Work);
                    if self.lanes.get(&slot).is_some_and(|lane| !lane.ended) {
                        continue; // one work lane per avatar; chat may already be open
                    }
                    let registry = self.registry.current();
                    let role = registry.role_of(&assignment.avatar);
                    let prompt = role.map(|r| r.prompt.clone()).unwrap_or_default();
                    let mut task = format!("Goal: {goal}\n\nYour assignment:\n{}", assignment.prompt);
                    if self.resuming {
                        task = format!(
                            "The room was closed and has been entered again. Your conversation above is intact. Carry on with this assignment from where you stopped: check the shelf for what is already published, and do not repeat finished work.\n\n{task}"
                        );
                    }
                    let model = registry.lane_model(&assignment.avatar, LaneKind::Work);
                    let config = LaneConfig {
                        avatar: assignment.avatar.clone(),
                        kind: LaneKind::Work,
                        system: prompt,
                        task,
                        task_images: self.turn_images.clone(),
                        model,
                        max_steps: self.config.work_max_steps,
                        transcript: self.transcript_for(&assignment.avatar, LaneKind::Work),
                        objective: goal.clone(),
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
                Effect::Steer { avatar, text } => self.inject_work(&avatar, LaneInput::Steer(text)),
                Effect::Visible { author, text, nonce, work_id } => {
                    // The store's unique key decides, so a line posts once even
                    // across a restart. `Ok(false)` means it was already posted. A
                    // storage error still posts: losing the line is worse than a
                    // repeat after a crash.
                    let fresh = self.coordinator.store().remember_nonce(&work_id, &nonce).unwrap_or(true);
                    if fresh {
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
        let config = LaneConfig { avatar: "chief".into(), kind: LaneKind::Chat, system: prompt, task, task_images: std::mem::take(&mut self.chief_images), model, max_steps: self.config.chat_max_steps, transcript: self.transcript_for("chief", LaneKind::Chat), objective: self.coordinator.active_brief().map(|(_, goal, _)| goal).unwrap_or_default() };
        self.chief_busy = true;
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
                let obs = self
                    .lanes
                    .get(&Slot::new(avatar, LaneKind::Work))
                    .map_or(Observation::default(), |lane| Observation {
                        run: Some(lane.run),
                        unsettled: lane.unsettled,
                    });
                (id.clone(), obs)
            })
            .collect();
        let busy: HashSet<String> = self
            .lanes
            .iter()
            .filter(|(slot, lane)| slot.kind == LaneKind::Work && !lane.ended)
            .map(|(slot, _)| slot.avatar.clone())
            .collect();

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
                    self.lanes.remove(&Slot::new(avatar, LaneKind::Work));
                }
            }
        }
        self.apply_effects(outcome.effects);
    }

    fn run_clerk(&mut self) {
        let Some(work) = self.coordinator.active_work() else { return };
        if self.lanes.get(&Slot::new("clerk", LaneKind::Work)).is_some_and(|lane| !lane.ended) {
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
            task_images: Vec::new(),
            model,
            max_steps: self.config.work_max_steps,
            // Each audit pass is independent; the clerk keeps no history.
            transcript: crate::room::lane::Transcript::memory(),
            objective: String::new(),
        };
        self.timeline.open("clerk", "clerk", now_ms());
        self.start_lane(config, LaneRef { avatar: "clerk".into(), kind: LaneKind::Work, assignment_id: None, is_chief: false, is_clerk: true });
    }

    fn emit_status(&self) {
        let items = self.shelf.list_items(&[], true, 40).unwrap_or_default();
        let counts = gate::count(&self.shelf.list_items(&[], true, 400).unwrap_or_default());
        let shelf = items.iter().map(|item| item.key.as_str()).collect::<Vec<_>>().join("\n");
        let trajectory = crate::room::trajectory::project(&self.shelf, None)
            .ok()
            .map(|views| {
                views.iter().map(crate::room::trajectory::render::outline).collect::<Vec<_>>().join("\n")
            })
            .unwrap_or_default();
        self.emit(RoomEvent::Status(StatusSnapshot {
            digest: self.digest.render(),
            timeline: self.timeline.render(120),
            counts,
            paused: self.control.paused().cloned().collect(),
            shelf,
            trajectory,
            lanes: self.lane_rows(),
            events: self
                .timeline
                .events(60)
                .into_iter()
                .map(|(at_ms, avatar, marker, label)| EventInfo { at_ms, avatar, marker: marker.to_string(), label })
                .collect(),
            now_ms: now_ms(),
        }));
    }

    /// One row per running lane, and one idle row for an avatar with none, so
    /// the table is the whole roster.
    fn lane_rows(&self) -> Vec<LaneInfo> {
        let registry = self.registry.current();
        let paused: std::collections::BTreeSet<String> = self.control.paused().cloned().collect();
        let work = self.coordinator.active_work();
        let task_of = |avatar: &str| -> String {
            let id = self.assignment_lane.iter().find(|(_, a)| a.as_str() == avatar).map(|(id, _)| id.clone());
            work.as_ref()
                .and_then(|w| w.assignments.iter().find(|a| Some(&a.id) == id.as_ref()))
                .map(|a| format!("{}: {}", a.kind.as_str(), a.prompt.lines().next().unwrap_or("").chars().take(80).collect::<String>()))
                .unwrap_or_default()
        };
        let mut rows = Vec::new();
        for avatar in registry.all_avatars() {
            let display = registry.display_of(&avatar.id).to_string();
            let mut any = false;
            for kind in [LaneKind::Chat, LaneKind::Work] {
                let Some(lane) = self.lanes.get(&Slot::new(&avatar.id, kind)) else { continue };
                any = true;
                let digest = (kind == LaneKind::Work).then(|| self.digest.status(&avatar.id)).flatten();
                let phase = if paused.contains(&avatar.id) && kind == LaneKind::Work {
                    "paused".to_string()
                } else if lane.ended {
                    digest.map_or("done", |s| s.phase.label()).to_string()
                } else {
                    digest.map_or("running", |s| s.phase.label()).to_string()
                };
                rows.push(LaneInfo {
                    avatar: avatar.id.clone(),
                    display: display.clone(),
                    role: avatar.role.clone(),
                    lane: if kind == LaneKind::Chat { "chat" } else { "work" }.into(),
                    phase,
                    task: if kind == LaneKind::Work { task_of(&avatar.id) } else if lane.is_chief { "answering you".into() } else { "talking with you".into() },
                    started_ms: lane.started_ms,
                    last_event_ms: lane.last_event_ms,
                    steps: lane.steps,
                    last_tool: lane.last_tool.clone().unwrap_or_default(),
                    error: digest.and_then(|s| s.error.clone()).unwrap_or_default(),
                });
            }
            if !any {
                rows.push(LaneInfo { avatar: avatar.id.clone(), display, role: avatar.role.clone(), phase: "idle".into(), ..Default::default() });
            }
        }
        rows
    }

    // ---- spawning a lane ----

    fn transcript_for(&self, avatar: &str, kind: LaneKind) -> crate::room::lane::Transcript {
        use crate::room::lane::{lane_dir, Transcript};
        match &self.lanes_dir {
            Some(root) => Transcript::open_dir(lane_dir(root, avatar, kind)),
            None => Transcript::memory(),
        }
    }

    fn start_lane(&mut self, config: LaneConfig, lane: LaneRef) {
        let avatar = lane.avatar.clone();
        let (kind, is_chief, assignment_id) = (lane.kind, lane.is_chief, lane.assignment_id.clone());
        let slot = Slot::new(&avatar, kind);
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
        let forward_kind = kind;
        tokio::spawn(async move {
            while let Some(event) = ev_rx.recv().await {
                if to_actor
                    .send(ActorMsg::LaneEvent {
                        avatar: forward_avatar.clone(),
                        kind: forward_kind,
                        event,
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let _ = is_clerk;
        self.lanes.insert(
            slot,
            LaneHandle {
                kind,
                assignment_id,
                is_chief,
                inject: inject_tx,
                abort: task.abort_handle(),
                run: LaneRunState::Running,
                unsettled: false,
                ended: false,
                started_ms: now_ms(),
                last_event_ms: now_ms(),
                steps: 0,
                last_tool: None,
            },
        );
    }
}
