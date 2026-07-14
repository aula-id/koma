//! Core message types shared across the whole application.
//!
//! `Role` and `ChatMessage` are the atoms of every conversation. They flow
//! upward through `Conversation` → `Session` → the OpenRouter wire format
//! (`dto/openrouter.rs`), and are persisted verbatim to `messages.json` on
//! disk. Keeping them in a separate module avoids circular imports between the
//! model layer and the wire-format layer.

// Re-exports below preserve the original flat-file public API; some names have no
// in-crate consumer yet, so silence the unused-import lint for the whole facade.
#![allow(unused_imports)]

mod attachment;
mod message;
mod role;
mod tool;

pub use attachment::Attachment;
pub use message::{merge_reasoning_details, ChatMessage, ReasoningDetail};
pub use role::{Role, BASH_NUDGE_MARK, CACHE_SPLIT_MARK, EXT_PROMPT_MARK, PLAN_NUDGE_MARK, SHELL_MARK};
pub use tool::{extract_text_tool_calls, sanitize_tool_arguments, strip_ansi, strip_tool_call_tags, FunctionCall, ToolCall};

// ---------------------------------------------------------------------------
// Reasoning-tag wire escaping
// ---------------------------------------------------------------------------
//
// DATA on the wire can contain literal reasoning delimiters — e.g. a `git log`
// of koma's OWN commit messages, which mention `<think>` / `</think>`. Sent raw,
// the model (or koma's receive-side `ThinkSplit` in
// `service::openrouter::think_split`) can mistake that DATA for a real reasoning
// block and mis-peel it.
//
// The fix is a TRANSIENT, whitelist-only escape applied to OUTBOUND content only:
// each whitelisted tag's `<`/`>` is entity-encoded (`&lt;`/`&gt;`) on the wire
// COPY so it can't act as a delimiter, while every OTHER `<`/`>` (generics,
// `a < b`, real markup) is left untouched. Persistence keeps the REAL tags: the
// store is never escaped, and any escaped tag a model echoes back is DECODED
// before it is saved (see `stream::final_answer` + `stream::turn`) so
// `messages.json` / sqlite hold verbatim `<think>`.
//
// Whitelist == the tags `think_split.rs` `TAG_PAIRS` actually peels (`think`,
// `thinking`, `thought`; open OR close). KEEP THE TWO IN SYNC: only a tag that
// ThinkSplit can mis-peel needs escaping, and escaping anything else would be a
// silent wire mutation.
// raw form:    <think> </think> <thinking> </thinking> <thought> </thought>
// entity form: &lt;think&gt; etc.

use regex::Regex;
use std::sync::OnceLock;

/// Matcher for the RAW whitelisted reasoning tags — `<think>` / `</think>` /
/// `<thinking>` / `</thinking>` / `<thought>` / `</thought>` — case-insensitive,
/// open OR close, tolerant of interior whitespace (`< / think >`). The whole tag
/// is a single match containing exactly one `<` and one `>`.
fn reasoning_tag_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)<\s*/?\s*(?:think|thinking|thought)\s*>")
            .expect("reasoning tag regex must be valid")
    })
}

/// Matcher for the ENTITY-ENCODED form of the whitelisted reasoning tags
/// (`&lt;think&gt;`, `&lt;/think&gt;`, …) — the reverse of [`reasoning_tag_re`],
/// used to decode a model's echoed-back escaped tag before persistence.
fn reasoning_entity_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)&lt;\s*/?\s*(?:think|thinking|thought)\s*&gt;")
            .expect("reasoning entity regex must be valid")
    })
}

/// Escape whitelisted reasoning tags to HTML entities so they can't act as
/// delimiters on the wire. Only the tag's `<`/`>` are entity-encoded; all other
/// `<`/`>` (generics, comparisons, real markup) are untouched.
///
/// Returns `Cow::Borrowed` (a cheap no-op) when the input has no whitelisted tag —
/// the overwhelmingly common case.
///
/// KNOWN LIMITATION: this is a text-level, non-syntax-aware whitelist, so a
/// generic type argument named EXACTLY `Think`/`Thinking`/`Thought` (e.g.
/// `Vec<Think>`) would also be escaped. Accepted trade-off — vanishingly rare
/// in practice, and far cheaper than parsing the wire content as code.
pub fn escape_reasoning_tags(s: &str) -> std::borrow::Cow<'_, str> {
    reasoning_tag_re().replace_all(s, |caps: &regex::Captures| {
        // The whole match is a single tag: exactly one leading `<` and one trailing
        // `>`, with only whitespace / `/` / the keyword between them. Encode just the
        // angle brackets; the interior (including its original case) rides verbatim.
        caps[0].replace('<', "&lt;").replace('>', "&gt;")
    })
}

/// Reverse of [`escape_reasoning_tags`]: decode the entity form of whitelisted tags
/// back to raw `<think>` etc. Used before persisting model output so storage keeps
/// real tags. Returns `Cow::Borrowed` when there is nothing to decode.
///
/// KNOWN LIMITATION: a literal HTML-entity `&lt;think&gt;` already present in a
/// model's raw output (not produced by our own [`escape_reasoning_tags`]) is
/// indistinguishable from an echoed-back escape and will also be decoded to
/// `<think>`. Accepted as cosmetic/rare. The entity form is kept as-is rather
/// than swapped for an opaque sentinel so the tags stay legible to the model
/// when it reasons about koma's own think-handling code (e.g. this very file).
pub fn unescape_reasoning_tags(s: &str) -> std::borrow::Cow<'_, str> {
    reasoning_entity_re().replace_all(s, |caps: &regex::Captures| {
        caps[0].replace("&lt;", "<").replace("&gt;", ">")
    })
}

#[cfg(test)]
mod escape_tests {
    use super::{escape_reasoning_tags, unescape_reasoning_tags};

    #[test]
    fn escape_unescape_roundtrip() {
        let escaped = escape_reasoning_tags("<think>foo</think>");
        assert_eq!(escaped, "&lt;think&gt;foo&lt;/think&gt;");
        assert_eq!(unescape_reasoning_tags(&escaped), "<think>foo</think>");
    }

    #[test]
    fn all_whitelist_variants_roundtrip() {
        for raw in [
            "<think>", "</think>",
            "<thinking>", "</thinking>",
            "<thought>", "</thought>",
        ] {
            let esc = escape_reasoning_tags(raw);
            // Fully escaped: entity brackets present, raw brackets gone.
            assert!(
                esc.contains("&lt;") && esc.contains("&gt;")
                    && !esc.contains('<') && !esc.contains('>'),
                "tag not fully escaped: {esc}"
            );
            assert_eq!(unescape_reasoning_tags(&esc), raw);
        }
    }

    #[test]
    fn non_reasoning_angles_untouched() {
        // Generics, comparisons, and unrelated markup survive BOTH directions.
        for s in ["Vec<String>", "a < b", "<div>", "if x > 0", "Vec<Vec<u8>>", "x <= y"] {
            assert_eq!(escape_reasoning_tags(s), s);
            assert_eq!(unescape_reasoning_tags(s), s);
        }
    }

    #[test]
    fn escape_is_case_insensitive() {
        assert_eq!(escape_reasoning_tags("<THINK>"), "&lt;THINK&gt;");
        assert_eq!(escape_reasoning_tags("</Thinking>"), "&lt;/Thinking&gt;");
        // Decoding is case-insensitive too and roundtrips the original case.
        assert_eq!(unescape_reasoning_tags("&lt;THINK&gt;"), "<THINK>");
    }

    #[test]
    fn only_whitelisted_keywords_match() {
        // `<reason>` is NOT in the ThinkSplit whitelist → left as-is by both.
        assert_eq!(escape_reasoning_tags("<reason>x</reason>"), "<reason>x</reason>");
        // All three whitelisted keywords DO escape.
        assert_eq!(escape_reasoning_tags("<thought>"), "&lt;thought&gt;");
    }

    #[test]
    fn no_match_returns_borrowed() {
        assert!(matches!(
            escape_reasoning_tags("plain text, no tags"),
            std::borrow::Cow::Borrowed(_)
        ));
        assert!(matches!(
            unescape_reasoning_tags("plain text, no tags"),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    #[test]
    fn mixed_content_escapes_only_the_tag() {
        let s = "commit msg mentions <think> and code has Vec<String> too";
        let escaped = escape_reasoning_tags(s);
        assert_eq!(
            escaped,
            "commit msg mentions &lt;think&gt; and code has Vec<String> too"
        );
        // Decoding restores the real tag and leaves Vec<String> untouched.
        assert_eq!(unescape_reasoning_tags(&escaped), s);
    }
}
