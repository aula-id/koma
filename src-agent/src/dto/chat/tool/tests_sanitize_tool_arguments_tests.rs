#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

/// Parse both sides as `Value` and assert structural equality — re-serialise
/// may reorder keys / drop whitespace, so a byte compare would be wrong.
fn assert_json_eq(a: &str, b: &str) {
    let va: serde_json::Value = serde_json::from_str(a).unwrap();
    let vb: serde_json::Value = serde_json::from_str(b).unwrap();
    assert_eq!(va, vb, "left={a} right={b}");
}

#[test]
fn single_clean_object_is_preserved() {
    // The normal path MUST be a semantic no-op.
    let input = r#"{"command":"ls -la","timeout":30}"#;
    let out = sanitize_tool_arguments(input);
    assert_json_eq(&out, input);
}

#[test]
fn duplicated_object_collapses_to_one() {
    let out = sanitize_tool_arguments(r#"{"a":1}{"a":1}"#);
    assert_json_eq(&out, r#"{"a":1}"#);
}

#[test]
fn empty_then_full_keeps_full() {
    // Provider emits `{}` first, then the complete args.
    let out = sanitize_tool_arguments(r#"{}{"command":"x"}"#);
    assert_json_eq(&out, r#"{"command":"x"}"#);
}

#[test]
fn full_then_duplicate_realistic_bash_keeps_command() {
    // The real-world bug: the full bash args, then a duplicate copy.
    let one = r#"{"command":"grep -rn \"foo\" src/ | head -20"}"#;
    let input = format!("{one}{one}");
    let out = sanitize_tool_arguments(&input);
    assert_json_eq(&out, one);
    // The command must survive intact.
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        v["command"].as_str().unwrap(),
        r#"grep -rn "foo" src/ | head -20"#
    );
}

#[test]
fn incomplete_or_garbage_yields_empty_object() {
    // A truncated/partial value parses to nothing → "{}".
    assert_eq!(sanitize_tool_arguments(r#"{"command":"#), "{}");
}

#[test]
fn legit_empty_object_is_preserved() {
    // A genuinely empty argument bag stays empty (no value to upgrade to).
    assert_eq!(sanitize_tool_arguments("{}"), "{}");
}

#[test]
fn whitespace_between_two_values_handled() {
    // Newlines/spaces separating two full copies must not defeat parsing.
    let out = sanitize_tool_arguments("{\"a\":1}\n  \t {\"a\":1}");
    assert_json_eq(&out, r#"{"a":1}"#);
}

#[test]
fn empty_string_yields_empty_object() {
    assert_eq!(sanitize_tool_arguments(""), "{}");
    assert_eq!(sanitize_tool_arguments("   \n\t "), "{}");
}

#[test]
fn empty_then_full_then_duplicate_keeps_full() {
    // `{}` then the full args repeated twice: keep the last non-empty object.
    let out = sanitize_tool_arguments(r#"{}{"command":"x"}{"command":"x"}"#);
    assert_json_eq(&out, r#"{"command":"x"}"#);
}
