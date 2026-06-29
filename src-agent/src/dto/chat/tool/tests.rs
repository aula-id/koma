use super::*;

#[cfg(test)]
mod text_tool_call_tests {
    use super::*;

    #[test]
    fn single_block_object_arguments() {
        let content =
            r#"<tool_call>{"name": "read", "arguments": {"path": "ARCHITECTURE.md"}}</tool_call>"#;
        let (cleaned, calls) = extract_text_tool_calls(content);
        assert_eq!(cleaned, "");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "text_call_0");
        assert_eq!(calls[0].kind, "function");
        assert_eq!(calls[0].function.name, "read");
        assert_eq!(calls[0].function.arguments, r#"{"path":"ARCHITECTURE.md"}"#);
    }

    #[test]
    fn parameters_alias() {
        let content =
            r#"<tool_call>{"name": "list", "parameters": {"dir": "src"}}</tool_call>"#;
        let (cleaned, calls) = extract_text_tool_calls(content);
        assert_eq!(cleaned, "");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "list");
        assert_eq!(calls[0].function.arguments, r#"{"dir":"src"}"#);
    }

    #[test]
    fn arguments_already_json_string() {
        // `arguments` is itself a JSON string holding stringified JSON — used verbatim.
        let content =
            r#"<tool_call>{"name": "bash", "arguments": "{\"cmd\": \"ls\"}"}</tool_call>"#;
        let (cleaned, calls) = extract_text_tool_calls(content);
        assert_eq!(cleaned, "");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "bash");
        assert_eq!(calls[0].function.arguments, r#"{"cmd": "ls"}"#);
    }

    #[test]
    fn two_blocks_in_one_message() {
        let content = concat!(
            r#"<tool_call>{"name": "read", "arguments": {"path": "a"}}</tool_call>"#,
            "\n",
            r#"<tool_call>{"name": "read", "arguments": {"path": "b"}}</tool_call>"#,
        );
        let (cleaned, calls) = extract_text_tool_calls(content);
        assert_eq!(cleaned, "");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].id, "text_call_0");
        assert_eq!(calls[1].id, "text_call_1");
        assert_eq!(calls[0].function.arguments, r#"{"path":"a"}"#);
        assert_eq!(calls[1].function.arguments, r#"{"path":"b"}"#);
    }

    #[test]
    fn malformed_json_skipped_and_left_in_content() {
        let content = r#"<tool_call>{"name": "read", "arguments": }</tool_call>"#;
        let (cleaned, calls) = extract_text_tool_calls(content);
        assert!(calls.is_empty());
        // Skipped block stays in the content verbatim.
        assert_eq!(cleaned, content);
    }

    #[test]
    fn surrounding_prose_preserved() {
        let content = concat!(
            "Let me read that file.\n\n",
            r#"<tool_call>{"name": "read", "arguments": {"path": "x"}}</tool_call>"#,
            "\n\nDone.",
        );
        let (cleaned, calls) = extract_text_tool_calls(content);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "read");
        assert_eq!(cleaned, "Let me read that file.\n\nDone.");
    }

    #[test]
    fn no_tool_call_present() {
        let content = "Just a normal answer with no tool calls at all.";
        let (cleaned, calls) = extract_text_tool_calls(content);
        assert!(calls.is_empty());
        assert_eq!(cleaned, content);
    }

    #[test]
    fn nested_braces_in_arguments() {
        let content = concat!(
            r#"<tool_call>{"name": "edit", "arguments": {"path": "f", "#,
            r#""replace": {"from": {"a": 1}, "to": {"b": 2}}}}</tool_call>"#,
        );
        let (cleaned, calls) = extract_text_tool_calls(content);
        assert_eq!(cleaned, "");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "edit");
        // Re-parse the stringified arguments to confirm the nesting survived.
        let v: serde_json::Value =
            serde_json::from_str(&calls[0].function.arguments).unwrap();
        assert_eq!(v["replace"]["from"]["a"], 1);
        assert_eq!(v["replace"]["to"]["b"], 2);
    }

    #[test]
    fn whitespace_inside_tags_tolerated() {
        let content = "<tool_call>\n  {\n  \"name\": \"read\",\n  \"arguments\": {\"path\": \"y\"}\n  }\n</tool_call>";
        let (cleaned, calls) = extract_text_tool_calls(content);
        assert_eq!(cleaned, "");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.arguments, r#"{"path":"y"}"#);
    }

    #[test]
    fn stray_open_before_valid_block_still_parses() {
        // A stray <tool_call> with no matching close precedes a complete valid
        // block. The close of the valid block must re-anchor to the valid
        // block's own open; the stray open is left in content as text.
        let content = concat!(
            "stray <tool_call> then a real ",
            r#"<tool_call>{"name":"read","arguments":{"path":"x"}}</tool_call>"#,
        );
        let (cleaned, calls) = extract_text_tool_calls(content);
        assert_eq!(calls.len(), 1, "exactly one call must be parsed");
        assert_eq!(calls[0].function.name, "read");
        // The valid block's span is removed from content; the stray open tag
        // text may remain but the parsed block must not be present.
        assert!(
            !cleaned.contains(r#"{"name":"read""#),
            "parsed block should be stripped from content"
        );
    }

    #[test]
    fn scalar_arguments_coerced_to_empty_object() {
        // `arguments` is a bare number — not a valid argument bag.
        // Must be coerced to "{}" rather than emitting "5".
        let content = r#"<tool_call>{"name":"ping","arguments":5}</tool_call>"#;
        let (cleaned, calls) = extract_text_tool_calls(content);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "ping");
        assert_eq!(calls[0].function.arguments, "{}");
        assert_eq!(cleaned, "");
    }
    #[test]
    fn stray_close_tag_is_stripped() {
        let content = "Some prose.</tool_call> More prose.";
        let out = strip_tool_call_tags(content);
        assert!(!out.contains("</tool_call>"), "orphan close tag must be removed");
        assert!(out.contains("Some prose."), "surrounding prose must survive");
        assert!(out.contains("More prose."), "trailing prose must survive");
    }

    #[test]
    fn well_formed_span_is_stripped() {
        let content = r#"Before.<tool_call>{"name":"foo","arguments":{}}</tool_call>After."#;
        let out = strip_tool_call_tags(content);
        assert!(!out.contains("<tool_call>"), "open tag must be removed");
        assert!(!out.contains("</tool_call>"), "close tag must be removed");
        assert_eq!(out, "Before.After.");
    }

    #[test]
    fn surrounding_prose_preserved_by_stripper() {
        let content = "Hello world.

<tool_call>{}</tool_call>

Bye.";
        let out = strip_tool_call_tags(content);
        assert!(out.contains("Hello world."), "leading prose must survive");
        assert!(out.contains("Bye."), "trailing prose must survive");
        assert!(!out.contains("<tool_call>"), "span must be removed");
    }

    #[test]
    fn unmatched_open_truncates_to_end() {
        let content = r#"Prose here. <tool_call>{"name":"foo"#;
        let out = strip_tool_call_tags(content);
        assert_eq!(out.trim(), "Prose here.");
    }
}

#[cfg(test)]
mod sanitize_tool_arguments_tests {
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
}

#[cfg(test)]
mod strip_ansi_tests {
    use super::strip_ansi;

    #[test]
    fn strips_color_codes() {
        // ESC[96m...ESC[0m — the canonical colorized output case
        assert_eq!(strip_ansi("\x1b[96mhello\x1b[0m"), "hello");
    }

    #[test]
    fn plain_text_unchanged() {
        let plain = "just a normal string with no escapes";
        assert_eq!(strip_ansi(plain), plain);
    }

    #[test]
    fn strips_bold_and_multi_param() {
        assert_eq!(strip_ansi("\x1b[1;31merror\x1b[0m: bad"), "error: bad");
    }

    #[test]
    fn strips_mixed_content() {
        assert_eq!(strip_ansi("\x1b[32mok\x1b[0m plain \x1b[31mfail\x1b[0m"), "ok plain fail");
    }
}
