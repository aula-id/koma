//! A lane's durable conversation.
//!
//! Every lane is one long conversation, not a fresh one per turn. The model
//! history of a lane is kept here as JSON lines (one [`ChatMessage`] per line)
//! so a Chief turn sees the turns before it, and a restart picks up where the
//! room left off. The system prompt is never stored: it is rebuilt from the
//! registry on every run, so a prompt edit applies to the next turn.
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

#[derive(Debug, Default)]
pub struct Transcript {
    path: Option<PathBuf>,
    messages: Vec<ChatMessage>,
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
    pub fn open(path: PathBuf) -> Self {
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
        let mut transcript = Self { path: Some(path), messages };
        if repaired {
            transcript.rewrite();
        }
        transcript
    }

    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
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
        self.messages.push(message);
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
    fn system_messages_are_never_stored() {
        let path = dir("system").join("x.jsonl");
        let mut t = Transcript::open(path.clone());
        t.push(ChatMessage::new(Role::System, "prompt"));
        t.push(ChatMessage::new(Role::User, "hi"));
        assert_eq!(Transcript::open(path).messages().len(), 1);
    }
}
