//! The reader task of a room binding and the room status it caches.
//!
//! The task owns the read half for the whole binding and never stops reading:
//! the room daemon's per-connection channel is unbounded, so a reader that paused
//! would let it grow without end. Direct answers (`models`, `sdk_ack`, `closed`)
//! go to the single pending-request slot. History frames go to the ring and, once
//! the binding is answered, to the event queue; deltas go to the queue only;
//! statuses replace the cached status. Every frame is scrubbed before it is kept.
use super::connection;
use super::ring::{self, Ring, RoomLine, Wire};
use super::scrub::{self, SharedScrubber};
use crate::app::runtime::sdk::events::EventQueue;
use crate::app::runtime::sdk::host::config::ConfigStore;
use crate::ipc::frame::{read_frame_from, FrameReader};
use crate::ipc::IpcReadHalf;
use crate::room::actor::{EventInfo, LaneInfo};
use crate::room::registry::RegistryHandle;
use crate::room::shelf::Shelf;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tokio::sync::{oneshot, watch};

/// No status for this long on an open socket gives a `stalled` notice. The
/// room's ticker runs every second.
pub const STALL_AFTER: Duration = Duration::from_secs(10);
/// Status text fields (`digest`, `timeline`, `trajectory_outline`) are cut here.
pub const STATUS_TEXT_BYTES: usize = 256 * 1024;
/// Each text field of a lane or a timeline event is cut here.
const FIELD_BYTES: usize = 8 * 1024;
pub const MAX_PAUSED: usize = 64;
pub const MAX_LANES: usize = 128;
pub const MAX_EVENTS: usize = 60;
/// Notices that arrive before the binding is answered are kept, up to this many.
const EARLY_NOTICES: usize = 16;

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// The room's status as the SDK reports it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RoomStatus {
    pub session_id: String,
    pub live: bool,
    /// False until the first status sent after the attach: a replayed status
    /// can be minutes old.
    pub fresh: bool,
    /// The age of the cached status.
    pub age_ms: Option<u64>,
    pub now_at_ms: u64,
    pub counts: StatusCounts,
    /// `first_400_items` for the room's own counts, `disk_scan` when the
    /// adapter counted the whole shelf itself.
    pub counts_basis: &'static str,
    pub paused: Vec<String>,
    pub paused_truncated: bool,
    pub lanes: Vec<LaneStatus>,
    pub lanes_truncated: bool,
    pub events: Vec<TimelineEvent>,
    pub digest: String,
    pub timeline: String,
    pub trajectory_outline: String,
    pub shelf_fingerprint: String,
    /// A text field was cut.
    pub truncated: bool,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct StatusCounts {
    pub open: u64,
    pub verified: u64,
    pub returned: u64,
    pub reports: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LaneStatus {
    pub avatar: String,
    pub display: String,
    pub alias: String,
    pub role: String,
    pub lane: Option<String>,
    pub phase: String,
    pub task: String,
    pub started_at_ms: u64,
    pub last_event_at_ms: u64,
    pub steps: u64,
    pub last_tool: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TimelineEvent {
    pub at_ms: u64,
    pub avatar: String,
    pub marker: String,
    pub label: String,
}

/// The fields of a room `status` frame the SDK reports.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct WireStatus {
    digest: String,
    timeline: String,
    open: u64,
    verified: u64,
    returned: u64,
    reports: u64,
    paused: Vec<String>,
    trajectory: String,
    lanes: Vec<LaneInfo>,
    events: Vec<EventInfo>,
}

/// The latest status frame, as received.
#[derive(Debug, Clone)]
pub struct Cached {
    status: RoomStatus,
    received_at_ms: u64,
}

fn cut(text: &mut String, max: usize, truncated: &mut bool) {
    if ring::clip(text, max) {
        *truncated = true;
    }
}

impl Cached {
    fn from_wire(session_id: &str, frame: Value, received_at_ms: u64) -> Self {
        let wire: WireStatus = serde_json::from_value(frame).unwrap_or_default();
        let mut truncated = false;
        let mut digest = wire.digest;
        let mut timeline = wire.timeline;
        let mut outline = wire.trajectory;
        cut(&mut digest, STATUS_TEXT_BYTES, &mut truncated);
        cut(&mut timeline, STATUS_TEXT_BYTES, &mut truncated);
        cut(&mut outline, STATUS_TEXT_BYTES, &mut truncated);
        let paused_truncated = wire.paused.len() > MAX_PAUSED;
        let lanes_truncated = wire.lanes.len() > MAX_LANES;
        let lanes = wire
            .lanes
            .into_iter()
            .take(MAX_LANES)
            .map(|lane| {
                let mut out = LaneStatus {
                    avatar: lane.avatar,
                    display: lane.display,
                    alias: lane.alias,
                    role: lane.role,
                    lane: Some(lane.lane).filter(|l| !l.is_empty()),
                    phase: lane.phase,
                    task: lane.task,
                    started_at_ms: lane.started_ms,
                    last_event_at_ms: lane.last_event_ms,
                    steps: lane.steps as u64,
                    last_tool: lane.last_tool,
                    error: lane.error,
                };
                for field in [
                    &mut out.display,
                    &mut out.task,
                    &mut out.last_tool,
                    &mut out.error,
                ] {
                    cut(field, FIELD_BYTES, &mut truncated);
                }
                out
            })
            .collect();
        let skip = wire.events.len().saturating_sub(MAX_EVENTS);
        let events = wire
            .events
            .into_iter()
            .skip(skip)
            .map(|event| {
                let mut label = event.label;
                cut(&mut label, FIELD_BYTES, &mut truncated);
                TimelineEvent {
                    at_ms: event.at_ms,
                    avatar: event.avatar,
                    marker: event.marker,
                    label,
                }
            })
            .collect();
        Self {
            status: RoomStatus {
                session_id: session_id.to_string(),
                live: true,
                fresh: false,
                age_ms: None,
                now_at_ms: 0,
                counts: StatusCounts {
                    open: wire.open,
                    verified: wire.verified,
                    returned: wire.returned,
                    reports: wire.reports,
                },
                counts_basis: "first_400_items",
                paused: wire.paused.into_iter().take(MAX_PAUSED).collect(),
                paused_truncated,
                lanes,
                lanes_truncated,
                events,
                digest,
                timeline,
                trajectory_outline: outline,
                shelf_fingerprint: String::new(),
                truncated,
            },
            received_at_ms,
        }
    }
}

/// What a pending request waits for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Want {
    Models,
    Ack(String),
    Closed,
}

pub struct Pending {
    pub want: Want,
    pub reply: oneshot::Sender<Value>,
    /// For an `sdk_user` send: the text of the adapter's echo line. The
    /// reader numbers it when the accepting `sdk_ack` arrives, before it
    /// reads the next frame, so the echo precedes the room's reply to it.
    /// The ack is handed over with `echo_line_seq` set.
    pub echo: Option<String>,
}

/// The mutable state of one binding, shared by the reader and the handlers.
/// The lock is never held across an await.
pub struct Shared {
    pub ring: Ring,
    pub status: Option<Cached>,
    pub fresh: bool,
    /// False once the stream ended or a write failed: ROOM(closed).
    pub live: bool,
    /// Set by `room.close` and `detach`: the end of the stream is expected and
    /// announces nothing.
    pub closing: bool,
    /// `room.closed` was queued (or deliberately not, while closing).
    pub announced: bool,
    /// History frames of the attach replay still to come.
    pub replay_left: usize,
    /// Lines among the replayed frames.
    pub replayed_lines: usize,
    /// False until the binding is answered: the attach result carries what
    /// arrived before.
    pub emitting: bool,
    early_notices: Vec<(&'static str, String)>,
    pub pending: Option<Pending>,
    stalled: bool,
    /// The last `room.status` queued, without its clock fields.
    emitted_status: Option<Value>,
}

/// Everything one room binding shares with its reader task.
pub struct Core {
    pub session_id: String,
    pub epoch: String,
    pub sock_path: PathBuf,
    pub queue: Arc<EventQueue>,
    pub scrub: SharedScrubber,
    pub store: ConfigStore,
    pub registry: Option<Arc<RegistryHandle>>,
    pub shelf: Option<Arc<Shelf>>,
    state: Mutex<Shared>,
    /// Bumped on every change a waiting handler may care about.
    changed: watch::Sender<u64>,
}

impl Core {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: String,
        epoch: String,
        sock_path: PathBuf,
        queue: Arc<EventQueue>,
        scrub: SharedScrubber,
        store: ConfigStore,
        registry: Option<Arc<RegistryHandle>>,
        shelf: Option<Arc<Shelf>>,
        replayed: usize,
    ) -> Self {
        Self {
            session_id,
            epoch,
            sock_path,
            queue,
            scrub,
            store,
            registry,
            shelf,
            state: Mutex::new(Shared {
                ring: Ring::new(),
                status: None,
                fresh: false,
                live: true,
                closing: false,
                announced: false,
                replay_left: replayed,
                replayed_lines: 0,
                emitting: false,
                early_notices: Vec::new(),
                pending: None,
                stalled: false,
                emitted_status: None,
            }),
            changed: watch::channel(0).0,
        }
    }

    pub fn lock(&self) -> MutexGuard<'_, Shared> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    fn bump(&self) {
        self.changed.send_modify(|n| *n = n.wrapping_add(1));
    }

    pub fn live(&self) -> bool {
        self.lock().live
    }

    /// The live status built from the cached frame, or `None` before the
    /// first one.
    pub fn cached_status(&self) -> Option<RoomStatus> {
        let state = self.lock();
        status_of(&state, now_ms())
    }

    /// Queue a `room.notice`. Before the binding is answered it is kept and
    /// queued right after the answer.
    pub fn notice(&self, code: &'static str, text: String) {
        let mut state = self.lock();
        if state.emitting {
            self.queue.push(
                &self.epoch,
                "room.notice",
                json!({"epoch": self.epoch, "code": code, "text": text}),
            );
        } else if state.early_notices.len() < EARLY_NOTICES {
            state.early_notices.push((code, text));
        }
    }

    /// Start writing events: called once the attach result is built, under
    /// the same lock, so every line is either in the result or an event.
    pub fn start_emitting(&self, state: &mut Shared) {
        state.emitting = true;
        let last = state.ring.last_line_seq().unwrap_or(0);
        self.queue.activate(&self.epoch, last);
        for (code, text) in std::mem::take(&mut state.early_notices) {
            self.queue.push(
                &self.epoch,
                "room.notice",
                json!({"epoch": self.epoch, "code": code, "text": text}),
            );
        }
        if let Some(status) = status_of(state, now_ms()) {
            self.queue_status(state, status);
        }
    }

    /// Queue `status` unless only its clock fields changed since the last one.
    fn queue_status(&self, state: &mut Shared, status: RoomStatus) {
        let Ok(full) = serde_json::to_value(&status) else {
            return;
        };
        let mut key = full.clone();
        if let Some(fields) = key.as_object_mut() {
            fields.remove("now_at_ms");
            fields.remove("age_ms");
        }
        if state.emitted_status.as_ref() == Some(&key) {
            return;
        }
        // Scrubbed at ingest, like every frame.
        state.emitted_status = Some(key);
        self.queue
            .set_status(&self.epoch, json!({"epoch": self.epoch, "status": full}));
    }

    /// Keep the adapter's own copy of a line it sent and queue it.
    pub fn echo(&self, text: &str) -> u64 {
        let mut line = RoomLine::echo(text);
        scrub::lock(&self.scrub).line(&mut line);
        let mut state = self.lock();
        let line = state.ring.push(line, false, now_ms());
        if state.emitting {
            self.queue_line(&line);
        }
        line.line_seq
    }

    fn queue_line(&self, line: &RoomLine) {
        if let Ok(body) = serde_json::to_value(line) {
            self.queue.push_line(
                &self.epoch,
                line.line_seq,
                json!({"epoch": self.epoch, "line": body}),
            );
        }
    }

    /// The stream is down: no more socket writes. Nothing is announced yet.
    fn mark_down(&self) -> bool {
        let mut state = self.lock();
        let was = state.live;
        state.live = false;
        state.pending = None;
        drop(state);
        self.bump();
        was
    }

    /// Queue `room.closed{reason}` once, unless the end was asked for.
    pub fn announce(&self, reason: &'static str) {
        let mut state = self.lock();
        state.live = false;
        state.pending = None;
        if !state.announced {
            state.announced = true;
            if !state.closing {
                self.queue.push(
                    &self.epoch,
                    "room.closed",
                    json!({"epoch": self.epoch, "session_id": self.session_id, "reason": reason}),
                );
            }
        }
        drop(state);
        self.bump();
    }

    /// A socket write failed or an answer was lost: ROOM(closed) with
    /// `stream_error`.
    pub fn fail_stream(&self) {
        self.announce("stream_error");
    }

    async fn on_frame(&self, mut frame: Value) {
        let shared = Arc::clone(&self.scrub);
        let store = self.store.clone();
        let _ = tokio::task::spawn_blocking(move || scrub::lock(&shared).refresh(&store)).await;
        scrub::lock(&self.scrub).value(&mut frame);
        let registry = self.registry.as_ref().map(|r| r.current());
        match ring::classify(frame, registry.as_deref()) {
            Wire::Line(line) => {
                let mut state = self.lock();
                let replay = state.replay_left > 0;
                if replay {
                    state.replay_left -= 1;
                    state.replayed_lines += 1;
                }
                let line = state.ring.push(*line, replay, now_ms());
                if state.emitting {
                    self.queue_line(&line);
                }
                drop(state);
                self.bump();
            }
            Wire::Delta {
                author,
                lane,
                kind,
                text,
            } => {
                let state = self.lock();
                if state.emitting {
                    self.queue
                        .push_delta(&self.epoch, &author, &lane, kind, &text);
                }
            }
            Wire::Status(frame) => self.on_status(frame).await,
            Wire::Models(body) => self.answer(Want::Models, json!({"models": body})),
            Wire::Ack(body) => {
                let id = body
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                self.answer(Want::Ack(id), body);
            }
            Wire::Closed => self.answer(Want::Closed, Value::Null),
            Wire::Notice(text) => self.notice("room", text),
            Wire::Ignored => {}
        }
    }

    async fn on_status(&self, frame: Value) {
        let mut cached = Cached::from_wire(&self.session_id, frame, now_ms());
        if let Some(shelf) = &self.shelf {
            let shelf = Arc::clone(shelf);
            if let Ok(Ok(fingerprint)) =
                tokio::task::spawn_blocking(move || shelf.fingerprint()).await
            {
                cached.status.shelf_fingerprint = fingerprint;
            }
        }
        let mut state = self.lock();
        let replay = state.replay_left > 0;
        if replay {
            state.replay_left -= 1;
        } else {
            state.fresh = true;
        }
        state.status = Some(cached);
        let recovered = !replay && std::mem::take(&mut state.stalled);
        if state.emitting {
            if let Some(status) = status_of(&state, now_ms()) {
                self.queue_status(&mut state, status);
            }
        }
        drop(state);
        if recovered {
            self.notice("recovered", "the room's status resumed".into());
        }
        self.bump();
    }

    /// Hand a direct answer to the pending request that waits for it. An
    /// accepting ack of a send first puts the send's echo line in the ring.
    fn answer(&self, got: Want, mut value: Value) {
        let mut state = self.lock();
        if !state.pending.as_ref().is_some_and(|p| p.want == got) {
            return;
        }
        let Some(pending) = state.pending.take() else {
            return;
        };
        drop(state);
        if let Some(text) = &pending.echo {
            if value.get("ok").and_then(Value::as_bool) == Some(true) {
                let inserts = super::send::ack_inserts(&value);
                let line_seq = self.echo(&crate::room::daemon::stage::with_inserts(text, &inserts));
                if let Some(fields) = value.as_object_mut() {
                    fields.insert("echo_line_seq".into(), json!(line_seq));
                }
            }
        }
        let _ = pending.reply.send(value);
    }

    fn stall(&self) {
        let mut state = self.lock();
        if state.stalled || !state.live {
            return;
        }
        state.stalled = true;
        drop(state);
        self.notice(
            "stalled",
            format!("no status from the room for {} s", STALL_AFTER.as_secs()),
        );
    }
}

/// The live status from the cached frame, with its clock fields for `now`.
fn status_of(state: &Shared, now: u64) -> Option<RoomStatus> {
    let cached = state.status.as_ref()?;
    let mut status = cached.status.clone();
    status.live = state.live;
    status.fresh = state.fresh;
    status.now_at_ms = now;
    status.age_ms = Some(now.saturating_sub(cached.received_at_ms));
    Some(status)
}

/// The live status as of now, or `None` before the first status.
pub fn status_now(state: &Shared) -> Option<RoomStatus> {
    status_of(state, now_ms())
}

/// A status with no room frame behind it: before the first status of a live
/// room, or in ROOM(closed), where `counts` come from a full shelf scan and
/// lanes and events are empty.
pub fn blank_status(
    session_id: &str,
    live: bool,
    age_ms: Option<u64>,
    shelf_fingerprint: String,
    counts: Option<StatusCounts>,
) -> RoomStatus {
    RoomStatus {
        session_id: session_id.to_string(),
        live,
        fresh: false,
        age_ms,
        now_at_ms: now_ms(),
        counts_basis: if counts.is_some() {
            "disk_scan"
        } else {
            "first_400_items"
        },
        counts: counts.unwrap_or_default(),
        paused: Vec::new(),
        paused_truncated: false,
        lanes: Vec::new(),
        lanes_truncated: false,
        events: Vec::new(),
        digest: String::new(),
        timeline: String::new(),
        trajectory_outline: String::new(),
        shelf_fingerprint,
        truncated: false,
    }
}

/// Read the room stream until it ends, then decide why it ended.
pub async fn run(core: Arc<Core>, mut read: IpcReadHalf, mut frames: FrameReader) {
    let mut last_status = Instant::now();
    loop {
        let stall_at = tokio::time::Instant::from_std(last_status + STALL_AFTER);
        tokio::select! {
            incoming = read_frame_from(&mut read, &mut frames) => {
                let bytes = match incoming {
                    Ok(bytes) => bytes,
                    Err(_) => break,
                };
                let Ok(frame) = serde_json::from_slice::<Value>(&bytes) else {
                    core.mark_down();
                    core.announce("stream_error");
                    return;
                };
                let is_status = matches!(&frame, Value::Object(map) if map.contains_key("status"));
                core.on_frame(frame).await;
                if is_status {
                    last_status = Instant::now();
                }
            }
            _ = tokio::time::sleep_until(stall_at) => {
                core.stall();
                // One notice per stall: the timer is armed again by the next status.
                last_status = Instant::now() + Duration::from_secs(365 * 24 * 3600);
            }
        }
    }
    if !core.mark_down() {
        return;
    }
    let closing = core.lock().closing;
    let reason = if closing {
        "daemon_exit"
    } else {
        connection::classify_end(&core.sock_path).await
    };
    core.announce(reason);
}
