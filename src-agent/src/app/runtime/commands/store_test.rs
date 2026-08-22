#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[test]
fn strip_markdown_headers_strips_leading_hashes_only() {
    // The `## Sub heading` line has NO blank line before its following prose in the
    // source, so the fix must INSERT one (a heading with an already-blank line after
    // it, like `# Title`, is left with just that one blank — no double blank).
    let md = "# Title\n\nSome body text.\n## Sub heading\nMore body, #not-a-header inline.";
    let out = strip_markdown_headers(md);
    assert_eq!(
        out,
        "Title\n\nSome body text.\nSub heading\n\nMore body, #not-a-header inline."
    );
}

/// Regression test for the live-test-caught glue bug: a heading immediately
/// followed by body text with no blank line between them (`"# Hello World\nA
/// minimal reference..."`) used to strip to a single `\n`-separated string that the
/// view's naive char-wrap rendered as "Hello WorldA minimal reference..." — glued
/// together with no visible break at all. The fix forces a paragraph break (blank
/// line) after every stripped heading, so the heading always reads as its own line.
#[test]
fn strip_markdown_headers_separates_heading_glued_to_body() {
    let md = "# Hello World\nA minimal reference to something.";
    let out = strip_markdown_headers(md);
    assert_eq!(out, "Hello World\n\nA minimal reference to something.");
}

/// A heading that is the LAST line (nothing to separate it from) must not grow a
/// trailing blank line.
#[test]
fn strip_markdown_headers_trailing_heading_has_no_trailing_blank() {
    let md = "Some body.\n# Trailing Heading";
    let out = strip_markdown_headers(md);
    assert_eq!(out, "Some body.\nTrailing Heading");
}
