//! The line ring of one room binding and the mapping from room wire frames to
//! public `RoomLine`s.
//!
//! Each attach starts an epoch with a fresh ring numbered from `line_seq` 1. The
//! ring holds the daemon's replay plus the live lines and the adapter's own echoes
//! that follow; nothing older is loaded. It keeps the newest [`RING_LINES`] lines
//! or [`RING_BYTES`] bytes, whichever is smaller, and survives into ROOM(closed).
//! `line_seq` is never compared with the envelope `seq` of events.
use crate::room::registry::Registry;
use serde::Serialize;
use serde_json::Value;
use std::collections::VecDeque;

pub const RING_LINES: usize = 8000;
pub const RING_BYTES: usize = 64 * 1024 * 1024;
/// A `room.history` page and the attach `tail` stop at this many bytes.
pub const PAGE_BYTES: usize = 8 * 1024 * 1024;
/// Text fields of a line are cut at this many bytes, with `truncated:true`.
pub const LINE_TEXT_BYTES: usize = 1024 * 1024;
pub const HISTORY_LINES: usize = 200;
pub const TAIL_LINES: usize = 200;

/// One line of a room's conversation. Its id within a binding is
/// `(epoch, line_seq)`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RoomLine {
    pub line_seq: u64,
    /// `chief`, `lane_chat`, `visible`, `user`, `shell`, `reasoning`,
    /// `tool_call`, `tool_result`, or `other` for a part kind this adapter does
    /// not know.
    pub kind: &'static str,
    pub author: String,
    pub display: Option<String>,
    pub alias: Option<String>,
    pub lane: Option<String>,
    pub audience: Option<String>,
    pub text: Option<String>,
    pub call_id: Option<String>,
    pub tool: Option<String>,
    pub args: Option<String>,
    pub ok: Option<bool>,
    pub result: Option<String>,
    pub replay: bool,
    /// `room` for a line the daemon sent, `adapter_echo` for the adapter's
    /// copy of a line it sent itself.
    pub origin: &'static str,
    pub received_at_ms: u64,
    pub truncated: bool,
}

impl RoomLine {
    fn new(kind: &'static str, author: &str) -> Self {
        Self {
            line_seq: 0,
            kind,
            author: author.to_string(),
            display: None,
            alias: None,
            lane: None,
            audience: None,
            text: None,
            call_id: None,
            tool: None,
            args: None,
            ok: None,
            result: None,
            replay: false,
            origin: "room",
            received_at_ms: 0,
            truncated: false,
        }
    }

    /// The adapter's copy of what it sent: the room never echoes the sender's
    /// own line live.
    pub fn echo(text: &str) -> Self {
        let mut line = Self::new("user", "you");
        line.origin = "adapter_echo";
        line.audience = Some("user".into());
        line.text = Some(text.to_string());
        line.clip();
        line
    }

    /// Cut every text field to [`LINE_TEXT_BYTES`].
    fn clip(&mut self) {
        for text in [&mut self.text, &mut self.args, &mut self.result]
            .into_iter()
            .flatten()
        {
            if clip(text, LINE_TEXT_BYTES) {
                self.truncated = true;
            }
        }
    }
}

/// Cut `text` to at most `max` bytes on a char boundary; whether it was cut.
pub fn clip(text: &mut String, max: usize) -> bool {
    if text.len() <= max {
        return false;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    true
}

/// What one frame from the room socket is.
#[derive(Debug, Clone, PartialEq)]
pub enum Wire {
    /// A history frame: `chief`, `lane_chat`, `visible`, or a part that is not
    /// a delta.
    Line(Box<RoomLine>),
    /// A `text_delta` or `reasoning_delta` part. Never history.
    Delta {
        author: String,
        lane: String,
        kind: &'static str,
        text: String,
    },
    Status(Value),
    /// Direct answers: `models`, `sdk_ack` and `closed`.
    Models(Value),
    Ack(Value),
    Closed,
    Notice(String),
    /// Frames the adapter never asks for (`pasted`, `browse`, `item`) and kinds
    /// it does not know.
    Ignored,
}

fn str_of(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn opt_str(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Classify one frame. `registry` resolves `display` and `alias` of avatar
/// authors.
pub fn classify(frame: Value, registry: Option<&Registry>) -> Wire {
    let (tag, body) = match frame {
        Value::String(tag) => (tag, Value::Null),
        Value::Object(map) if map.len() == 1 => match map.into_iter().next() {
            Some((tag, body)) => (tag, body),
            None => return Wire::Ignored,
        },
        _ => return Wire::Ignored,
    };
    let mut line = match tag.as_str() {
        "closed" => return Wire::Closed,
        "status" => return Wire::Status(body),
        "models" => return Wire::Models(body),
        "sdk_ack" => return Wire::Ack(body),
        "notice" => return Wire::Notice(str_of(&body, "text")),
        "chief" => {
            let mut line = RoomLine::new("chief", "chief");
            line.lane = Some("chat".into());
            line.text = Some(str_of(&body, "text"));
            line
        }
        "lane_chat" => {
            let mut line = RoomLine::new("lane_chat", &str_of(&body, "avatar"));
            line.lane = Some("chat".into());
            line.text = Some(str_of(&body, "text"));
            line
        }
        "visible" => {
            let author = str_of(&body, "author");
            let kind = match author.as_str() {
                "you" => "user",
                "shell" => "shell",
                _ => "visible",
            };
            let mut line = RoomLine::new(kind, &author);
            line.audience = opt_str(&body, "audience");
            line.text = Some(str_of(&body, "text"));
            line
        }
        "part" => match part(&body) {
            Ok(line) => line,
            Err(delta) => return delta,
        },
        _ => return Wire::Ignored,
    };
    if let Some(registry) = registry {
        if registry.avatars.contains_key(&line.author) {
            line.display = Some(registry.display_of(&line.author).to_string());
            line.alias = Some(registry.alias_of(&line.author));
        }
    }
    line.clip();
    Wire::Line(Box::new(line))
}

/// A `part` frame as a line, or as a delta.
fn part(body: &Value) -> Result<RoomLine, Wire> {
    let author = str_of(body, "author");
    let lane = str_of(body, "lane");
    let piece = body.get("part").cloned().unwrap_or(Value::Null);
    let kind = str_of(&piece, "kind");
    let delta = |kind: &'static str| Wire::Delta {
        author: author.clone(),
        lane: lane.clone(),
        kind,
        text: str_of(&piece, "text"),
    };
    let mut line = match kind.as_str() {
        "text_delta" => return Err(delta("text")),
        "reasoning_delta" => return Err(delta("reasoning")),
        "reasoning" => {
            let mut line = RoomLine::new("reasoning", &author);
            line.text = Some(str_of(&piece, "text"));
            line
        }
        "tool_call" => {
            let mut line = RoomLine::new("tool_call", &author);
            line.call_id = opt_str(&piece, "id");
            line.tool = opt_str(&piece, "name");
            line.args = opt_str(&piece, "args");
            line
        }
        "tool_result" => {
            let mut line = RoomLine::new("tool_result", &author);
            line.call_id = opt_str(&piece, "id");
            line.ok = piece.get("ok").and_then(Value::as_bool);
            line.result = opt_str(&piece, "result");
            line
        }
        _ => {
            let mut line = RoomLine::new("other", &author);
            line.text = Some(piece.to_string());
            line
        }
    };
    line.lane = Some(lane).filter(|l| !l.is_empty());
    line.audience = opt_str(body, "audience");
    Ok(line)
}

struct Entry {
    line: RoomLine,
    bytes: usize,
}

/// One page of the ring, ascending.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub lines: Vec<RoomLine>,
    pub has_more_before: bool,
    pub has_more_after: bool,
}

/// Which lines a page may hold.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub kinds: Option<Vec<String>>,
    pub author: Option<String>,
}

impl Filter {
    fn matches(&self, line: &RoomLine) -> bool {
        self.kinds
            .as_ref()
            .is_none_or(|kinds| kinds.iter().any(|k| k == line.kind))
            && self.author.as_ref().is_none_or(|a| *a == line.author)
    }
}

#[derive(Default)]
pub struct Ring {
    entries: VecDeque<Entry>,
    next_seq: u64,
    bytes: usize,
}

impl Ring {
    pub fn new() -> Self {
        Self {
            next_seq: 1,
            ..Self::default()
        }
    }

    /// Number `line`, keep it, and return the numbered copy.
    pub fn push(&mut self, mut line: RoomLine, replay: bool, received_at_ms: u64) -> RoomLine {
        line.line_seq = self.next_seq;
        self.next_seq += 1;
        line.replay = replay;
        line.received_at_ms = received_at_ms;
        let bytes = serde_json::to_vec(&line).map_or(0, |b| b.len());
        self.entries.push_back(Entry {
            line: line.clone(),
            bytes,
        });
        self.bytes += bytes;
        while self.entries.len() > RING_LINES || (self.bytes > RING_BYTES && self.entries.len() > 1)
        {
            if let Some(old) = self.entries.pop_front() {
                self.bytes -= old.bytes;
            }
        }
        line
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn first_line_seq(&self) -> Option<u64> {
        self.entries.front().map(|e| e.line.line_seq)
    }

    pub fn last_line_seq(&self) -> Option<u64> {
        self.entries.back().map(|e| e.line.line_seq)
    }

    /// Lines after `after`, or before `before`, or the newest ones, that
    /// match `filter`: at most `limit` lines and [`PAGE_BYTES`] bytes. The
    /// bounds are exclusive.
    pub fn page(
        &self,
        after: Option<u64>,
        before: Option<u64>,
        limit: usize,
        filter: &Filter,
    ) -> Page {
        let matching = || self.entries.iter().filter(|e| filter.matches(&e.line));
        let mut lines = Vec::new();
        let mut bytes = 0;
        let fits = |bytes: usize, entry: &Entry, taken: usize| {
            taken < limit && (taken == 0 || bytes + entry.bytes <= PAGE_BYTES)
        };
        if let Some(after) = after {
            let mut rest = matching().filter(|e| e.line.line_seq > after).peekable();
            while let Some(entry) = rest.peek() {
                if !fits(bytes, entry, lines.len()) {
                    break;
                }
                bytes += entry.bytes;
                lines.push(entry.line.clone());
                rest.next();
            }
            let has_more_after = rest.peek().is_some();
            return Page {
                lines,
                has_more_before: matching().any(|e| e.line.line_seq <= after),
                has_more_after,
            };
        }
        let bound = before.unwrap_or(u64::MAX);
        let mut rest = matching()
            .rev()
            .filter(|e| e.line.line_seq < bound)
            .peekable();
        while let Some(entry) = rest.peek() {
            if !fits(bytes, entry, lines.len()) {
                break;
            }
            bytes += entry.bytes;
            lines.push(entry.line.clone());
            rest.next();
        }
        let has_more_before = rest.peek().is_some();
        lines.reverse();
        Page {
            lines,
            has_more_before,
            has_more_after: before.is_some_and(|b| matching().any(|e| e.line.line_seq >= b)),
        }
    }
}
