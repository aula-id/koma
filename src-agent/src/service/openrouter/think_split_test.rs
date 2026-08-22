#![allow(clippy::unwrap_used, clippy::expect_used)]
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
