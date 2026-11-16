//! The discrete event queue between the room reader and the stdout writer.
//!
//! Room events are queued here and written by the single stdout writer, which
//! numbers them with the same envelope `seq` as the coding `state` events. The
//! queue is bounded: consecutive deltas of one author, lane and kind are merged,
//! and when it is full the queued lines and deltas are dropped and replaced by one
//! `room.gap` (the lines stay in the room's ring). `room.status` is a latest-wins
//! slot. Only one epoch is accepted at a time, so nothing of an older binding is
//! written after a rebind. While an attach is being answered the queue is held,
//! and the writer releases it once that attach response is out: every
//! activation has its own generation, and only the response that carries it
//! releases the hold. A coding binding never
//! queues anything here, so its events are written exactly as before.
use serde_json::{json, Map, Value};
use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard};
use tokio::sync::Notify;

/// Queued items past this count, or past [`QUEUE_BYTES`], turn the queued lines
/// and deltas into one `room.gap`.
pub const QUEUE_ITEMS: usize = 1024;
pub const QUEUE_BYTES: usize = 16 * 1024 * 1024;
/// A merged delta never grows past this; the next piece starts a new delta.
const DELTA_TEXT_BYTES: usize = 1024 * 1024;

enum Item {
    Line {
        line_seq: u64,
        body: Value,
        bytes: usize,
    },
    Delta {
        author: String,
        lane: String,
        kind: String,
        text: String,
    },
    Gap {
        after_line_seq: u64,
        dropped_lines: u64,
        dropped_deltas: u64,
    },
    Other {
        event: &'static str,
        body: Value,
        bytes: usize,
    },
}

impl Item {
    fn bytes(&self) -> usize {
        match self {
            Item::Line { bytes, .. } | Item::Other { bytes, .. } => *bytes,
            Item::Delta {
                author, lane, text, ..
            } => author.len() + lane.len() + text.len() + 64,
            Item::Gap { .. } => 64,
        }
    }
}

#[derive(Default)]
struct Inner {
    /// The epoch whose events are accepted. `None` while no room is bound.
    accepting: Option<String>,
    /// The epoch of the queued items, kept after `accepting` is cleared so
    /// what is still queued is written with its own epoch.
    label: String,
    items: VecDeque<Item>,
    bytes: usize,
    status: Option<Value>,
    held: bool,
    /// Counts activations, so a hold is released only by its own response.
    generation: u64,
    /// The hold of the latest activation, until the response it belongs to
    /// takes it.
    unclaimed: Option<u64>,
    /// The `line_seq` of the last line the client has: from the attach result,
    /// then from each written `room.line`.
    last_line: u64,
}

impl Inner {
    fn accepts(&self, epoch: &str) -> bool {
        self.accepting.as_deref() == Some(epoch)
    }

    fn push(&mut self, item: Item) {
        self.bytes += item.bytes();
        self.items.push_back(item);
    }

    /// Drop the queued lines and deltas when one more item of `incoming`
    /// bytes would pass a bound, and queue one gap for them in their place.
    fn make_room(&mut self, incoming: usize) {
        if self.items.len() < QUEUE_ITEMS && self.bytes + incoming <= QUEUE_BYTES {
            return;
        }
        let (mut lines, mut deltas) = (0u64, 0u64);
        self.items.retain(|item| match item {
            Item::Line { .. } => {
                lines += 1;
                false
            }
            Item::Delta { .. } => {
                deltas += 1;
                false
            }
            Item::Gap {
                dropped_lines,
                dropped_deltas,
                ..
            } => {
                lines += dropped_lines;
                deltas += dropped_deltas;
                false
            }
            Item::Other { .. } => true,
        });
        self.bytes = self.items.iter().map(Item::bytes).sum();
        let gap = Item::Gap {
            after_line_seq: self.last_line,
            dropped_lines: lines,
            dropped_deltas: deltas,
        };
        self.push(gap);
    }
}

/// See the module documentation.
#[derive(Default)]
pub struct EventQueue {
    inner: Mutex<Inner>,
    notify: Notify,
}

impl EventQueue {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Start accepting `epoch`. Everything queued for an earlier epoch is
    /// discarded, and nothing is written until the response being answered
    /// (the one that claims this hold with [`EventQueue::claim_hold`]) is
    /// out. `last_line_seq` is the newest line the attach result already
    /// carries.
    pub fn activate(&self, epoch: &str, last_line_seq: u64) {
        let mut inner = self.lock();
        let generation = inner.generation + 1;
        *inner = Inner {
            accepting: Some(epoch.to_string()),
            label: epoch.to_string(),
            held: true,
            generation,
            unclaimed: Some(generation),
            last_line: last_line_seq,
            ..Inner::default()
        };
    }

    /// The hold set while the current request was answered, if any. The
    /// response frame carries it to the writer, which passes it to
    /// [`EventQueue::response_written`] once that frame is out.
    pub fn claim_hold(&self) -> Option<u64> {
        self.lock().unclaimed.take()
    }

    /// Stop accepting events. What is already queued is still written, before
    /// the next response.
    pub fn deactivate(&self) {
        self.lock().accepting = None;
    }

    /// Queue a `room.line`. `body` is the whole event body, `epoch` included.
    pub fn push_line(&self, epoch: &str, line_seq: u64, body: Value) {
        let mut inner = self.lock();
        if !inner.accepts(epoch) {
            return;
        }
        let bytes = serde_json::to_vec(&body).map_or(0, |b| b.len());
        inner.make_room(bytes);
        inner.push(Item::Line {
            line_seq,
            body,
            bytes,
        });
        drop(inner);
        self.notify.notify_one();
    }

    /// Queue a `room.delta`, merged into the newest queued delta of the same
    /// author, lane and kind when no line, gap or other event follows it.
    pub fn push_delta(&self, epoch: &str, author: &str, lane: &str, kind: &str, text: &str) {
        let mut inner = self.lock();
        if !inner.accepts(epoch) {
            return;
        }
        let mut merged = false;
        for item in inner.items.iter_mut().rev() {
            match item {
                Item::Delta {
                    author: a,
                    lane: l,
                    kind: k,
                    text: t,
                } => {
                    if a == author && l == lane && k == kind {
                        if t.len() + text.len() <= DELTA_TEXT_BYTES {
                            t.push_str(text);
                            merged = true;
                        }
                        break;
                    }
                }
                _ => break,
            }
        }
        if merged {
            inner.bytes += text.len();
        } else {
            let item = Item::Delta {
                author: author.to_string(),
                lane: lane.to_string(),
                kind: kind.to_string(),
                text: text.to_string(),
            };
            inner.make_room(item.bytes());
            inner.push(item);
        }
        drop(inner);
        self.notify.notify_one();
    }

    /// Queue an event that is never dropped or merged (`room.notice`,
    /// `room.closed`).
    pub fn push(&self, epoch: &str, event: &'static str, body: Value) {
        let mut inner = self.lock();
        if !inner.accepts(epoch) {
            return;
        }
        let bytes = serde_json::to_vec(&body).map_or(0, |b| b.len());
        inner.push(Item::Other { event, body, bytes });
        drop(inner);
        self.notify.notify_one();
    }

    /// Replace the pending `room.status`: only the latest is written.
    pub fn set_status(&self, epoch: &str, body: Value) {
        let mut inner = self.lock();
        if !inner.accepts(epoch) {
            return;
        }
        inner.status = Some(body);
        drop(inner);
        self.notify.notify_one();
    }

    /// Wait until something may be ready.
    pub async fn ready(&self) {
        self.notify.notified().await;
    }

    /// The frames ready to be written, numbered from `seq`. Empty while held.
    pub fn take_ready(&self, seq: &mut u64) -> Vec<Value> {
        let mut inner = self.lock();
        if inner.held {
            return Vec::new();
        }
        let label = inner.label.clone();
        let mut frames = Vec::new();
        while let Some(item) = inner.items.pop_front() {
            let (event, body) = match item {
                Item::Line { line_seq, body, .. } => {
                    inner.last_line = inner.last_line.max(line_seq);
                    ("room.line", body)
                }
                Item::Delta {
                    author,
                    lane,
                    kind,
                    text,
                } => (
                    "room.delta",
                    json!({"epoch": label, "author": author, "lane": lane, "kind": kind, "text": text}),
                ),
                Item::Gap {
                    after_line_seq,
                    dropped_lines,
                    dropped_deltas,
                } => (
                    "room.gap",
                    json!({"epoch": label, "after_line_seq": after_line_seq,
                           "dropped_lines": dropped_lines, "dropped_deltas": dropped_deltas}),
                ),
                Item::Other { event, body, .. } => (event, body),
            };
            *seq += 1;
            frames.push(frame(event, *seq, body));
        }
        inner.bytes = 0;
        if let Some(body) = inner.status.take() {
            *seq += 1;
            frames.push(frame("room.status", *seq, body));
        }
        frames
    }

    /// The response that claimed hold `generation` was written: the queue
    /// may now be written, unless a later activation holds it again.
    pub fn response_written(&self, generation: u64) {
        let mut inner = self.lock();
        if inner.held && inner.generation == generation {
            inner.held = false;
            let pending = !inner.items.is_empty() || inner.status.is_some();
            drop(inner);
            if pending {
                self.notify.notify_one();
            }
        }
    }
}

/// What the writer does once a response is out, for tests that dispatch
/// without the writer.
#[cfg(test)]
pub fn release_after_response(queue: &EventQueue) {
    if let Some(generation) = queue.claim_hold() {
        queue.response_written(generation);
    }
}

/// `{"type":"event","event":<event>,"seq":<seq>, ...body}`.
fn frame(event: &str, seq: u64, body: Value) -> Value {
    let mut out = Map::new();
    out.insert("type".into(), json!("event"));
    out.insert("event".into(), json!(event));
    out.insert("seq".into(), json!(seq));
    if let Value::Object(fields) = body {
        for (key, value) in fields {
            out.insert(key, value);
        }
    }
    Value::Object(out)
}
