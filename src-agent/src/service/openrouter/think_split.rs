//! Token-by-token `<think>…</think>` splitter for streaming responses.
//!
//! Some backends embed reasoning inside `delta.content` as a leading
//! `<think>` (or `<thinking>` / `<thought>`) block rather than using a
//! separate `delta.reasoning` field.  [`ThinkSplit`] peels that block out of
//! the content stream and routes it to the reasoning channel, handling the
//! case where tag markers are split across SSE chunks.
//!
//! On tool-call continuation turns some vLLM/local backends peel the opener
//! and the reasoning into `delta.reasoning` but still leak a bare, orphaned
//! *closing* tag (e.g. `</think>`) at the very start of `delta.content`.  That
//! stray closer is swallowed here instead of surfacing as visible content.
//! Tag matching is case-insensitive.
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

/// (opener, closer) tag pairs, tried in order, matched case-insensitively.
const TAG_PAIRS: [(&str, &str); 3] = [
    ("<think>", "</think>"),
    ("<thinking>", "</thinking>"),
    ("<thought>", "</thought>"),
];

/// State machine that splits a leading `<think>…</think>` block out of the
/// content stream, routing it to the reasoning channel and the rest to the
/// content channel.
pub struct ThinkSplit {
    buf: String,
    phase: Phase,
}

impl ThinkSplit {
    /// Create a fresh instance (one per stream call / per turn).
    pub fn new() -> Self {
        ThinkSplit {
            buf: String::new(),
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
                    // Skip leading Unicode whitespace: a stray whitespace character
                    // (e.g. NBSP `\u{00A0}`) before the opener must not latch
                    // Passthrough. `ws_len` is a
                    // BYTE offset (index of the first non-whitespace char); it is
                    // only *consumed* once a tag matches, so a genuine
                    // leading-whitespace answer still survives into Passthrough.
                    let ws_len = self
                        .buf
                        .find(|c: char| !c.is_whitespace())
                        .unwrap_or(self.buf.len());
                    let t = &self.buf[ws_len..];
                    if t.is_empty() {
                        // Only whitespace so far — wait for more.
                        break;
                    }
                    // (1) Full opener match (case-insensitive) — enter Inside.
                    let opener = TAG_PAIRS
                        .iter()
                        .enumerate()
                        .find(|&(_, &(open, _))| starts_with_ci(t, open));
                    if let Some((i, &(open, _))) = opener {
                        // Consume leading whitespace + opener, enter Inside.
                        self.buf.drain(..ws_len + open.len());
                        self.phase = Phase::Inside(i);
                        continue;
                    }
                    // (2) `t` is a strict prefix of some opener — it could still
                    // become one once more bytes arrive; wait.
                    if TAG_PAIRS.iter().any(|&(open, _)| is_ci_prefix(t, open)) {
                        break;
                    }
                    // (3) Leading ORPHAN closer: some vLLM/local backends peel the
                    // opener + reasoning into the dedicated reasoning field but leak
                    // the bare close tag into content. There is nothing to route to
                    // Reasoning — swallow the tag and passthrough the remainder.
                    //
                    // Caveat: content whose very first bytes literally spell a closer
                    // tag (e.g. a model answering about `</think>` itself) is
                    // indistinguishable from a leaked orphan closer and will be
                    // swallowed here. Accepted trade-off — vanishingly rare, and the
                    // alternative (leaking stray close tags) is worse.
                    if let Some(&(_, close)) = TAG_PAIRS
                        .iter()
                        .find(|&&(_, close)| starts_with_ci(t, close))
                    {
                        self.buf.drain(..ws_len + close.len());
                        self.phase = Phase::Passthrough;
                        continue;
                    }
                    // (4) A closer split across SSE chunks (`</`, `</thi`,
                    // `</think` …). Hold back — never leak a partial orphan closer
                    // as visible content.
                    if TAG_PAIRS.iter().any(|&(_, close)| is_ci_prefix(t, close)) {
                        break;
                    }
                    // (5) Genuine content — passthrough.
                    // DO NOT discard anything; leading whitespace is real content.
                    self.phase = Phase::Passthrough;
                    continue;
                }

                Phase::Inside(i) => {
                    let closer = TAG_PAIRS[*i].1;
                    if let Some(p) = find_ci(&self.buf, closer) {
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

/// Case-insensitive ASCII prefix test: does `buf` begin with `tag`?  Tags are
/// always ASCII think-markers, so a byte compare is sufficient (and byte
/// slicing avoids the char-boundary panics that `str` slicing could hit).
fn starts_with_ci(buf: &str, tag: &str) -> bool {
    let (bb, tb) = (buf.as_bytes(), tag.as_bytes());
    bb.len() >= tb.len() && bb[..tb.len()].eq_ignore_ascii_case(tb)
}

/// Case-insensitive ASCII: is `buf` a *proper* prefix of `tag` — strictly
/// shorter, with every byte matching ignoring case?  Used to hold an
/// opener/closer that is split across SSE chunks.
fn is_ci_prefix(buf: &str, tag: &str) -> bool {
    let (bb, tb) = (buf.as_bytes(), tag.as_bytes());
    bb.len() < tb.len() && tb[..bb.len()].eq_ignore_ascii_case(bb)
}

/// Case-insensitive ASCII substring search — byte index of the first position
/// where `needle` matches `haystack` ignoring ASCII case (`needle` is ASCII).
fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    let (hb, nb) = (haystack.as_bytes(), needle.as_bytes());
    if nb.is_empty() {
        return Some(0);
    }
    if hb.len() < nb.len() {
        return None;
    }
    (0..=hb.len() - nb.len()).find(|&i| hb[i..i + nb.len()].eq_ignore_ascii_case(nb))
}

/// Return the byte length of the longest suffix of `buf` that equals a
/// *proper* prefix of `needle` (length < `needle.len()`), matched
/// case-insensitively.  Returns 0 if no such suffix exists.
///
/// Example: `buf = "...abc</thin"`, `needle = "</think>"` → returns 6
/// (the suffix `"</thin"` matches the proper prefix `"</thin"` of `"</think>"`).
fn prefix_holdback(buf: &str, needle: &str) -> usize {
    let (bb, nb) = (buf.as_bytes(), needle.as_bytes());
    let max_k = bb.len().min(nb.len() - 1);
    for k in (1..=max_k).rev() {
        if bb[bb.len() - k..].eq_ignore_ascii_case(&nb[..k]) {
            return k;
        }
    }
    0
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Feed `chunks` through a fresh splitter (push each, then finish) and
    /// return the concatenated `(reasoning, content)` streams.
    fn run(chunks: &[&str]) -> (String, String) {
        fn drain(emits: Vec<Emit>, reasoning: &mut String, content: &mut String) {
            for e in emits {
                match e {
                    Emit::Reasoning(s) => reasoning.push_str(&s),
                    Emit::Content(s) => content.push_str(&s),
                }
            }
        }
        let mut ts = ThinkSplit::new();
        let (mut reasoning, mut content) = (String::new(), String::new());
        for chunk in chunks {
            drain(ts.push(chunk), &mut reasoning, &mut content);
        }
        drain(ts.finish(), &mut reasoning, &mut content);
        (reasoning, content)
    }

    #[test]
    fn standard_inline_block() {
        // Shape 1: full inline block, reasoning field empty.
        let (reasoning, content) = run(&["<think>reason</think>answer"]);
        assert_eq!(reasoning, "reason");
        assert_eq!(content, "answer");
        assert!(
            !content.contains("think>"),
            "tag leaked into content: {content:?}"
        );
    }

    #[test]
    fn leading_orphan_close_is_stripped() {
        // Shape 2: bare orphan closer at the start of content.
        let (reasoning, content) = run(&["</think>answer"]);
        assert_eq!(reasoning, "");
        assert_eq!(content, "answer");
        assert!(
            !content.contains("think>"),
            "orphan closer leaked: {content:?}"
        );
        assert!(!content.contains("</"), "orphan closer leaked: {content:?}");
    }

    #[test]
    fn orphan_close_split_across_chunks() {
        // The orphan closer is split across two SSE chunks.
        let (reasoning, content) = run(&["</thi", "nk>hi"]);
        assert_eq!(reasoning, "");
        assert_eq!(content, "hi");
        assert!(
            !content.contains("think>"),
            "split closer leaked: {content:?}"
        );
        assert!(
            !content.contains("</thi"),
            "split closer leaked: {content:?}"
        );
    }

    #[test]
    fn mid_answer_think_is_literal() {
        // Invariant: once Passthrough is latched by real content, a later
        // `<think>` stays literal — it is NOT captured as reasoning.
        let (reasoning, content) = run(&["answer ", "<think>x"]);
        assert_eq!(reasoning, "");
        assert_eq!(content, "answer <think>x");
    }

    #[test]
    fn leading_whitespace_then_think() {
        // Leading whitespace before the opener is skipped, not leaked.
        let (reasoning, content) = run(&["\n  <think>r</think>a"]);
        assert_eq!(reasoning, "r");
        assert_eq!(content, "a");
    }

    #[test]
    fn orphan_close_case_insensitive() {
        // Upper-case orphan closer is still stripped.
        let (reasoning, content) = run(&["</THINK>hey"]);
        assert_eq!(reasoning, "");
        assert_eq!(content, "hey");
        assert!(
            !content.to_ascii_lowercase().contains("think>"),
            "orphan closer leaked: {content:?}"
        );
    }

    #[test]
    fn angle_slash_non_closer_passes_through() {
        // "</" is a prefix of every closer, so it can be held back waiting to
        // see whether it completes one — but once the bytes diverge from all
        // known closers, the held (and subsequent) bytes must be released as
        // ordinary content, not dropped or held forever.
        let (reasoning, content) = run(&["</di", "v>hello"]);
        assert_eq!(reasoning, "");
        assert_eq!(content, "</div>hello");
    }

    #[test]
    fn multibyte_content_around_tags_no_panic() {
        // Multi-byte UTF-8 reasoning content butting up against a closer tag
        // that is itself split across chunks must not panic on a byte slice
        // that lands mid-character.
        let (reasoning, content) = run(&["<think>abc🎉", "</th", "ink>done"]);
        assert_eq!(reasoning, "abc🎉");
        assert_eq!(content, "done");
    }
}
