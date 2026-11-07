//! The one place untrusted text is framed before a model sees it.
//!
//! Everything a target controls — a response body, corpus text, tool output read
//! back from a target — is data, never instruction. A tool result is the classic
//! injection channel: the model reads `ignore previous instructions` inside what
//! looks like a normal tool reply.
//!
//! Tags alone do not survive this. A body containing the closing fence closes the
//! wrapper, and everything after it reads as trusted. So the framing is STRUCTURAL:
//!
//! 1. The body is newline-padded, so a fence-shaped first or last line cannot join
//!    the boundary.
//! 2. The body is NOT escaped. Escaping rewrites bytes a repro payload has to keep
//!    intact, so the fence is placed where a closing tag is harmless instead of
//!    being stripped.
//! 3. The machine-readable header (origin, verdict, handle) sits ABOVE the fence and
//!    outside the untrusted region, so a body forging a verdict cannot win.
//!
//! This mirrors `koma_sec_daemon/untrusted.py`, which is the daemon's copy. The
//! parent gets its own because a tool handler the daemon never touches — or a
//! handler that returns a raw string — must still be framed, and the parent is
//! where the model-facing string is finally assembled.

/// Where the body came from. Closed, so a caller cannot invent an origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Response,
    Corpus,
    ToolOutput,
    Source,
}

impl Origin {
    #[must_use]
    fn tag(self) -> &'static str {
        match self {
            Self::Response => "response",
            Self::Corpus => "corpus",
            Self::ToolOutput => "tool-output",
            Self::Source => "source",
        }
    }
}

pub const FENCE_OPEN: &str = "<<<UNTRUSTED";
pub const FENCE_CLOSE: &str = "UNTRUSTED>>>";

/// Frame `body` as untrusted data, with trusted metadata above the fence.
///
/// `hint` describes the SHAPE ("json array of 12 objects") so the model can decide
/// how to read the body without learning it from the body.
#[must_use]
pub fn wrap(body: &str, origin: Origin, hint: &str) -> String {
    let mut header = format!("[{}]", origin.tag());
    if !hint.is_empty() {
        header.push(' ');
        header.push_str(hint);
    }
    if let Some(note) = skim(body) {
        header.push_str(" — ");
        header.push_str(&note);
    }
    // The padding is the whole trick: a body starting or ending with a fence token
    // can no longer sit flush against the real fence.
    format!("{header}\n{FENCE_OPEN}\n{body}\n{FENCE_CLOSE}")
}

/// One short sentence when the body looks like it is trying something, else "".
///
/// Deliberately crude. It raises suspicion; it does not parse, because taint
/// reasoning that depends on parsing attacker text is another way to be wrong.
#[must_use]
pub fn skim(body: &str) -> Option<String> {
    let upper = body.to_ascii_uppercase();
    // Both spellings: the fence is a bare token, so a body carrying the natural XML
    // form would otherwise slip through on exactly the case that matters.
    let mut notes = Vec::new();
    if [FENCE_OPEN, FENCE_CLOSE, "<UNTRUSTED>", "</UNTRUSTED>"]
        .iter()
        .any(|t| upper.contains(t))
    {
        notes.push("body contains a fence token");
    }
    let lowered = body.to_ascii_lowercase();
    for marker in ["ignore previous", "ignore the above", "system:", "you are now"] {
        if lowered.contains(marker) {
            notes.push("body looks instructional");
            break;
        }
    }
    if notes.is_empty() {
        return None;
    }
    Some(format!("skim: {}", notes.join("; ")))
}

/// Frame a tool's output for the model.
///
/// The verdict and handle ride in the header, outside the fence: they are the
/// fields a finding is built on, so they must stay trusted even when the body is
/// hostile.
#[must_use]
pub fn frame_result(body: &str, verdict: &str, handle: Option<&str>, origin: Origin) -> String {
    let mut hint = format!("verdict={verdict}");
    if let Some(h) = handle {
        hint.push_str(&format!(" handle={h}"));
    }
    wrap(body, origin, &hint)
}

#[cfg(test)]
#[path = "untrusted_test.rs"]
mod tests;

