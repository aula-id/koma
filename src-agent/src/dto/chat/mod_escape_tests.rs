#![allow(clippy::unwrap_used, clippy::expect_used)]
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
        "<think>",
        "</think>",
        "<thinking>",
        "</thinking>",
        "<thought>",
        "</thought>",
    ] {
        let esc = escape_reasoning_tags(raw);
        // Fully escaped: entity brackets present, raw brackets gone.
        assert!(
            esc.contains("&lt;")
                && esc.contains("&gt;")
                && !esc.contains('<')
                && !esc.contains('>'),
            "tag not fully escaped: {esc}"
        );
        assert_eq!(unescape_reasoning_tags(&esc), raw);
    }
}

#[test]
fn non_reasoning_angles_untouched() {
    // Generics, comparisons, and unrelated markup survive BOTH directions.
    for s in [
        "Vec<String>",
        "a < b",
        "<div>",
        "if x > 0",
        "Vec<Vec<u8>>",
        "x <= y",
    ] {
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
    assert_eq!(
        escape_reasoning_tags("<reason>x</reason>"),
        "<reason>x</reason>"
    );
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
