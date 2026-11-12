//! A lane's durable conversation.
//!
//! Every lane is one long conversation, not a fresh one per turn. The model
//! history of a lane is kept here as JSON lines (one [`ChatMessage`] per line)
//! so a Chief turn sees the turns before it, and a restart picks up where the
//! room left off. The system prompt is never stored: it is rebuilt from the
//! registry on every run, so a prompt edit applies to the next turn.
//!
//! Each lane has its own directory holding `messages.jsonl` (this record) and
//! `messages.sqlite`, the same archive a coding session keeps. The archive is
//! what DRSS shapes requests from and what `message_find` and `message_load`
//! read, so a lane recalls its own history exactly.
//!
//! The file is the full record. What a run sends to the model is a window over
//! the end of it ([`Transcript::window`]) until request shaping takes over.

use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::PathBuf;

use crate::dto::chat::{ChatMessage, Role};

/// Rough characters-per-token, the same cheap estimate used elsewhere for budgets.
const CHARS_PER_TOKEN: usize = 4;

/// The default window a run reads from the end of its transcript (about 100k tokens).
pub const DEFAULT_WINDOW_CHARS: usize = 100_000 * CHARS_PER_TOKEN;

/// An image the user shared, as the room holds it: the number its `[Image #n]`
/// marker carries in the user's text, and where the file is.
#[derive(Debug, Clone, PartialEq)]
pub struct UserImage {
    pub n: usize,
    pub path: PathBuf,
}

#[derive(Debug, Default)]
pub struct Transcript {
    path: Option<PathBuf>,
    /// The lane directory, when archiving (`messages.sqlite` lives here).
    dir: Option<PathBuf>,
    messages: Vec<ChatMessage>,
}

/// The directory of one lane under the room's `lanes/` root.
pub fn lane_dir(root: &std::path::Path, avatar: &str, kind: crate::room::registry::LaneKind) -> PathBuf {
    use crate::room::registry::LaneKind;
    let kind = match kind {
        LaneKind::Chat => "chat",
        LaneKind::Work => "work",
    };
    root.join(format!("{avatar}.{kind}"))
}

impl Transcript {
    /// A transcript that lives only in memory (tests, the clerk's one-shot audits).
    pub fn memory() -> Self {
        Self::default()
    }

    /// Open the transcript at `path`, loading what is there. A missing file is an
    /// empty transcript. Unreadable lines are skipped, and an unfinished tool
    /// round (a crash between a model's tool calls and their results) is cut off
    /// so the next request never carries a call without its answer.
    pub fn open_dir(dir: PathBuf) -> Self {
        let mut transcript = Self::open(dir.join("messages.jsonl"));
        transcript.dir = Some(dir);
        transcript
    }

    fn open(path: PathBuf) -> Self {
        let mut messages: Vec<ChatMessage> = std::fs::read_to_string(&path)
            .map(|text| {
                text.lines()
                    .filter(|line| !line.trim().is_empty())
                    .filter_map(|line| serde_json::from_str::<ChatMessage>(line).ok())
                    .filter(|m| m.role != Role::System)
                    .collect()
            })
            .unwrap_or_default();
        let kept = complete_prefix_len(&messages);
        let repaired = kept < messages.len();
        messages.truncate(kept);
        let mut transcript = Self { path: Some(path), dir: None, messages };
        if repaired {
            transcript.rewrite();
        }
        transcript
    }

    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    /// The lane directory holding the archive, when this transcript archives.
    pub fn archive_dir(&self) -> Option<&std::path::Path> {
        self.dir.as_deref()
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Add a message and append it to the file. Persistence is best effort: a
    /// disk error must not stop a lane mid-turn.
    pub fn push(&mut self, message: ChatMessage) {
        if let Some(path) = &self.path {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(line) = serde_json::to_string(&message) {
                if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
                    let _ = writeln!(file, "{line}");
                }
            }
        }
        if let Some(dir) = &self.dir {
            if message.role != Role::System {
                let _ = crate::model::msglog::append(dir, message.role, &message.content, message.reasoning.as_deref(), None);
            }
        }
        self.messages.push(message);
    }

    /// A user turn that carries `images`. Each file is copied into this lane's own
    /// `images/` directory (so the lane's wire reads it like any attachment, and
    /// `load_image` can name it), and the `[Image #n]` markers in `text` are
    /// renumbered to the lane's. An image the text never mentions gets a marker
    /// on its own line. A lane without a directory is told the path instead.
    pub fn user_turn(&self, text: &str, images: &[UserImage]) -> ChatMessage {
        if images.is_empty() {
            return ChatMessage::new(Role::User, text);
        }
        let Some(dir) = &self.dir else {
            let mut text = text.to_string();
            for image in images {
                text.push_str(&format!("\n[Image #{}: {}]", image.n, image.path.display()));
            }
            return ChatMessage::new(Role::User, text);
        };
        let images_dir = dir.join("images");
        let mut text = text.to_string();
        let mut attachments = Vec::new();
        let mut markers = Vec::new();
        for image in images {
            let Ok(bytes) = std::fs::read(&image.path) else { continue };
            let name = image.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "image.png".into());
            let Ok((attachment, marker)) = crate::model::attachment::ingest_image_bytes(&images_dir, &name, &bytes) else { continue };
            let old = format!("[Image #{}]", image.n);
            if text.contains(&old) {
                // Park the new marker so a later renumbering cannot rewrite it.
                text = text.replace(&old, &format!("\u{0}{}\u{0}", markers.len()));
            } else {
                text.push_str(&format!("\n\u{0}{}\u{0}", markers.len()));
            }
            markers.push(marker);
            attachments.push(attachment);
        }
        for (i, marker) in markers.iter().enumerate() {
            text = text.replace(&format!("\u{0}{i}\u{0}"), marker);
        }
        ChatMessage::new(Role::User, text).with_attachments(attachments)
    }

    /// The tail of the conversation that fits `max_chars`, cut so it starts on a
    /// user turn (never in the middle of a tool round). When even the last turn
    /// is larger than the budget, that turn is still returned whole.
    pub fn window(&self, max_chars: usize) -> Vec<ChatMessage> {
        let mut total = 0usize;
        let mut start = self.messages.len();
        for (i, message) in self.messages.iter().enumerate().rev() {
            total += message_chars(message);
            if total > max_chars && start < self.messages.len() {
                break;
            }
            start = i;
        }
        // Move forward to the first user turn at or after `start`.
        let from = (start..self.messages.len())
            .find(|&i| self.messages[i].role == Role::User)
            .unwrap_or(start);
        self.messages[from..].to_vec()
    }

    fn rewrite(&mut self) {
        let Some(path) = &self.path else { return };
        let mut text = String::new();
        for message in &self.messages {
            if let Ok(line) = serde_json::to_string(message) {
                text.push_str(&line);
                text.push('\n');
            }
        }
        let _ = std::fs::write(path, text);
    }
}

fn message_chars(message: &ChatMessage) -> usize {
    let calls: usize = message
        .tool_calls
        .as_ref()
        .map(|calls| calls.iter().map(|c| c.function.name.len() + c.function.arguments.len()).sum())
        .unwrap_or(0);
    message.content.len() + calls
}

/// How many leading messages form complete tool rounds: an assistant message
/// with tool calls needs a tool result for every call id right after it.
fn complete_prefix_len(messages: &[ChatMessage]) -> usize {
    let mut i = 0;
    while i < messages.len() {
        let Some(calls) = messages[i].tool_calls.as_ref().filter(|c| !c.is_empty()) else {
            i += 1;
            continue;
        };
        let mut wanted: HashSet<&str> = calls.iter().map(|c| c.id.as_str()).collect();
        let mut j = i + 1;
        while j < messages.len() && messages[j].role == Role::Tool {
            if let Some(id) = messages[j].tool_call_id.as_deref() {
                wanted.remove(id);
            }
            j += 1;
        }
        if !wanted.is_empty() {
            return i;
        }
        i = j;
    }
    messages.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::chat::{FunctionCall, ToolCall};

    fn call(id: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            kind: "function".into(),
            function: FunctionCall { name: "bash".into(), arguments: "{}".into() },
        }
    }

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("koma-transcript-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_reopened_transcript_returns_the_same_history() {
        let path = dir("reopen").join("chief.chat.jsonl");
        let mut t = Transcript::open(path.clone());
        t.push(ChatMessage::new(Role::User, "hello"));
        t.push(ChatMessage::assistant_with_tools("running".to_string(), vec![call("c1")]));
        t.push(ChatMessage::tool_result("c1".to_string(), "ok".to_string()));
        t.push(ChatMessage::new(Role::Assistant, "done"));
        let again = Transcript::open(path);
        assert_eq!(again.messages(), t.messages());
    }

    #[test]
    fn an_unfinished_tool_round_is_cut_and_the_file_repaired() {
        let path = dir("repair").join("red.work.jsonl");
        let mut t = Transcript::open(path.clone());
        t.push(ChatMessage::new(Role::User, "go"));
        t.push(ChatMessage::assistant_with_tools(String::new(), vec![call("a"), call("b")]));
        t.push(ChatMessage::tool_result("a".to_string(), "ok".to_string()));
        let reopened = Transcript::open(path.clone());
        assert_eq!(reopened.messages().len(), 1, "the half-answered round is dropped");
        t = Transcript::open(path);
        t.push(ChatMessage::new(Role::User, "again"));
        assert_eq!(t.messages().len(), 2);
    }

    #[test]
    fn the_window_starts_on_a_user_turn_and_keeps_the_latest_turn() {
        let mut t = Transcript::memory();
        for n in 0..6 {
            t.push(ChatMessage::new(Role::User, format!("question {n} {}", "x".repeat(100))));
            t.push(ChatMessage::new(Role::Assistant, format!("answer {n} {}", "y".repeat(100))));
        }
        let window = t.window(500);
        assert_eq!(window.first().map(|m| m.role.clone()), Some(Role::User));
        assert!(window.last().unwrap().content.starts_with("answer 5"));
        assert!(window.len() < t.messages().len());
        let tiny = t.window(1);
        assert!(tiny.iter().any(|m| m.content.starts_with("answer 5")), "the last turn is never lost");
    }

    #[test]
    fn a_user_turn_copies_images_into_the_lane_and_renumbers_the_markers() {
        let root = dir("images");
        let src = root.join("shared");
        std::fs::create_dir_all(&src).unwrap();
        let png: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, 0x49, 0x48, 0x44, 0x52, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0,
            0x1F, 0x15, 0xC4, 0x89, 0, 0, 0, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0, 1, 0, 0, 5, 0, 1, 0x0D, 0x0A, 0x2D, 0xB4, 0, 0, 0, 0,
            0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        std::fs::write(src.join("01-a.png"), png).unwrap();
        std::fs::write(src.join("02-b.png"), png).unwrap();
        let lane = Transcript::open_dir(root.join("red.chat"));
        let first = lane.user_turn("see [Image #2] and [Image #1]", &[UserImage { n: 1, path: src.join("01-a.png") }, UserImage { n: 2, path: src.join("02-b.png") }]);
        assert_eq!(first.attachments.len(), 2);
        assert!(first.content.contains("[Image #2] and [Image #1]") || first.content.contains("[Image #1]"));
        for a in &first.attachments {
            assert!(root.join("red.chat").join(&a.rel_path).is_file(), "the copy lives in the lane's directory");
        }
        let bare = lane.user_turn("look", &[UserImage { n: 7, path: src.join("01-a.png") }]);
        assert!(bare.content.starts_with("look\n[Image #"), "an unmentioned image gets its own marker: {}", bare.content);
        assert_eq!(bare.attachments.len(), 1);
        let none = Transcript::memory().user_turn("hi", &[UserImage { n: 1, path: src.join("01-a.png") }]);
        assert!(none.content.contains("01-a.png") && none.attachments.is_empty());
    }

    #[test]
    fn system_messages_are_never_stored() {
        let path = dir("system").join("x.jsonl");
        let mut t = Transcript::open(path.clone());
        t.push(ChatMessage::new(Role::System, "prompt"));
        t.push(ChatMessage::new(Role::User, "hi"));
        assert_eq!(Transcript::open(path).messages().len(), 1);
    }
}
