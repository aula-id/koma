//! Token-by-token `<think>…</think>` splitter for streaming responses.
//!
//! Some backends embed reasoning inside `delta.content` as a leading
//! `<think>` (or `<thinking>` / `<thought>`) block rather than using a
//! separate `delta.reasoning` field.  [`ThinkSplit`] peels that block out of
//! the content stream and routes it to the reasoning channel, handling the
//! case where tag markers are split across SSE chunks.
//!
//! **Key invariant:** the Pending → (Inside | Passthrough) decision is made
//! exactly once per response.  Once the phase is `Passthrough`, it stays
//! `Passthrough` for the entire stream — a `<think>` tag appearing mid-answer
//! (e.g. inside a code block) is treated as literal content, not captured.

/// Output item produced by [`ThinkSplit::push`] / [`ThinkSplit::finish`].
pub enum Emit {
    /// Text that should be routed to the reasoning/thinking channel.
    Reasoning(String),
    /// Text that should be routed to the normal content channel.
    Content(String),
}

/// Tracks where in the stream we are relative to a leading think-tag.
enum Phase {
    /// Haven't seen a non-whitespace byte yet; waiting to decide.
    Pending,
    /// Inside the think block; `usize` is the index into [`TAG_PAIRS`] so we
    /// know which closer to search for.
    Inside(usize),
    /// Past the decision point — emit everything as plain content.
    Passthrough,
}

/// (opener, closer) tag pairs, tried in order, matched case-sensitively.
const TAG_PAIRS: [(&str, &str); 3] = [
    ("<think>",    "</think>"),
    ("<thinking>", "</thinking>"),
    ("<thought>",  "</thought>"),
];

/// State machine that splits a leading `<think>…</think>` block out of the
/// content stream, routing it to the reasoning channel and the rest to the
/// content channel.
pub struct ThinkSplit {
    buf:   String,
    phase: Phase,
}

impl ThinkSplit {
    /// Create a fresh instance (one per stream call / per turn).
    pub fn new() -> Self {
        ThinkSplit {
            buf:   String::new(),
            phase: Phase::Pending,
        }
    }

    /// Feed the next `delta.content` chunk; returns zero or more [`Emit`]
    /// values that should be forwarded to the appropriate channel.
    ///
    /// Calling `push("")` is always a safe no-op.
    pub fn push(&mut self, content: &str) -> Vec<Emit> {
        if content.is_empty() {
            return Vec::new();
        }
        self.buf.push_str(content);
        let mut out = Vec::new();
        loop {
            match &self.phase {
                Phase::Pending => {
                    // Count leading ASCII-whitespace bytes.
                    let ws_len = self.buf
                        .as_bytes()
                        .iter()
                        .take_while(|&&b| b == b' ' || b == b'\t' || b == b'\r' || b == b'\n')
                        .count();
                    let t = &self.buf[ws_len..];
                    if t.is_empty() {
                        // Only whitespace so far — wait for more.
                        break;
                    }
                    // Check for a full opener match.
                    let full_match = TAG_PAIRS.iter().enumerate().find(|(_, (open, _))| {
                        t.starts_with(open)
                    });
                    if let Some((i, (open, _))) = full_match {
                        // Consume leading whitespace + opener, enter Inside.
                        self.buf.drain(..ws_len + open.len());
                        self.phase = Phase::Inside(i);
                        continue;
                    }
                    // Check whether `t` is a strict prefix of any opener (could
                    // still become one once more bytes arrive).
                    let is_prefix = TAG_PAIRS.iter().any(|(open, _)| {
                        open.starts_with(t) && t.len() < open.len()
                    });
                    if is_prefix {
                        // Ambiguous — wait for more input.
                        break;
                    }
                    // Not an opener and not a prefix of one — passthrough.
                    // DO NOT discard anything; leading whitespace is real content.
                    self.phase = Phase::Passthrough;
                    continue;
                }

                Phase::Inside(i) => {
                    let closer = TAG_PAIRS[*i].1;
                    if let Some(p) = self.buf.find(closer) {
                        // Found the closing tag; flush reasoning up to it.
                        if p > 0 {
                            out.push(Emit::Reasoning(self.buf[..p].to_string()));
                        }
                        self.buf.drain(..p + closer.len());
                        self.phase = Phase::Passthrough;
                        continue;
                    }
                    // Closer not (fully) present yet; hold back any bytes that
                    // could be the start of the closer split across chunks.
                    let hold = prefix_holdback(&self.buf, closer);
                    let safe = self.buf.len().saturating_sub(hold);
                    if safe > 0 {
                        out.push(Emit::Reasoning(self.buf[..safe].to_string()));
                        self.buf.drain(..safe);
                    }
                    break;
                }

                Phase::Passthrough => {
                    if !self.buf.is_empty() {
                        out.push(Emit::Content(std::mem::take(&mut self.buf)));
                    }
                    break;
                }
            }
        }
        out
    }

    /// Flush any remaining buffered bytes at end-of-stream.
    ///
    /// Must be called once after the SSE loop finishes (normal completion or
    /// `[DONE]`).  A leftover partial opener (e.g. `"<thi"`) flushes as
    /// `Content` — we never silently discard real content.
    pub fn finish(&mut self) -> Vec<Emit> {
        let mut out = Vec::new();
        if self.buf.is_empty() {
            return out;
        }
        match &self.phase {
            Phase::Inside(_) => {
                out.push(Emit::Reasoning(std::mem::take(&mut self.buf)));
            }
            Phase::Pending | Phase::Passthrough => {
                out.push(Emit::Content(std::mem::take(&mut self.buf)));
            }
        }
        out
    }
}

/// Return the byte length of the longest suffix of `buf` that equals a
/// *proper* prefix of `needle` (length < `needle.len()`).  Returns 0 if no
/// such suffix exists.
///
/// Example: `buf = "...abc</thin"`, `needle = "</think>"` → returns 6
/// (the suffix `"</thin"` matches the proper prefix `"</thin"` of `"</think>"`).
fn prefix_holdback(buf: &str, needle: &str) -> usize {
    let max_k = buf.len().min(needle.len() - 1);
    for k in (1..=max_k).rev() {
        if buf.ends_with(&needle[..k]) {
            return k;
        }
    }
    0
}
