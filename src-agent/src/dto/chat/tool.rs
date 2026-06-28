//! Tool-call types and the two parsing / sanitisation utilities.
//!
//! [`FunctionCall`] and [`ToolCall`] are the wire-format structs for OpenAI-style
//! tool calls. [`extract_text_tool_calls`] handles the text-embedded fallback used
//! by budget/ChatML-trained models, and [`sanitize_tool_arguments`] repairs the
//! duplicate-delta streaming bug found on some providers.

use serde::{Deserialize, Serialize};

/// The function-call payload inside a [`ToolCall`]: the tool name plus its
/// arguments as a JSON-encoded string (OpenAI/OpenRouter send `arguments` as a
/// stringified JSON object, not a nested object).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

/// One tool call requested by the assistant. `id` correlates the eventual
/// `tool` result message back to this call; `kind` is always `"function"`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type", default)]
    pub kind: String, // "function"
    pub function: FunctionCall,
}

/// Parse out tool calls a model emitted as TEXT in its content rather than via
/// the native OpenAI-style `tool_calls` field. Budget / gpt-oss / GLM routes
/// (and Hermes/Qwen/ChatML-trained models generally) tend to write a call as a
/// `<tool_call>{"name": "...", "arguments": {...}}</tool_call>` span in plain
/// text; without this fallback such calls render as literal text and never run.
///
/// Scans `content` for `<tool_call>` … `</tool_call>` spans. Each `</tool_call>`
/// close tag is paired with the NEAREST `<tool_call>` open that precedes it
/// (within the not-yet-consumed region), so a stray/unclosed open tag before a
/// valid block never swallows the valid block's call. Trims inner text and
/// parses it as JSON. A span counts as a tool call only when the JSON is an
/// object with a non-empty string `name` and an `arguments` (or `parameters`)
/// field. The arguments are normalised to a JSON-encoded string (mirroring how
/// the wire format carries `FunctionCall.arguments`): a JSON string is used
/// verbatim, a JSON object is stringified, and any other value (number, bool,
/// null, array) or a missing key degrades to `"{}"`.
///
/// Returns `(cleaned_content, calls)`. Successfully-parsed spans are removed
/// from the content (leading/trailing whitespace trimmed, 3+ newline runs
/// collapsed to 2); malformed or non-tool-call spans are SKIPPED and left in
/// the content so the user still sees the model's raw attempt. When nothing
/// parses, the content is returned unchanged with an empty vec.
pub fn extract_text_tool_calls(content: &str) -> (String, Vec<ToolCall>) {
    const OPEN: &str = "<tool_call>";
    const CLOSE: &str = "</tool_call>";

    let mut calls: Vec<ToolCall> = Vec::new();
    // Spans (byte ranges over `content`, including the tags) that parsed
    // successfully and must be stripped from the cleaned output.
    let mut remove: Vec<(usize, usize)> = Vec::new();

    let mut search_from = 0usize;
    while let Some(rel_close) = content[search_from..].find(CLOSE) {
        let close_start = search_from + rel_close;
        let close_end = close_start + CLOSE.len();

        // Within the not-yet-consumed region up to this close tag, find the
        // NEAREST (rightmost) preceding open tag.
        let region = &content[search_from..close_start];
        match region.rfind(OPEN) {
            Some(rel_open) => {
                let open_start = search_from + rel_open;
                let inner_start = open_start + OPEN.len();
                let inner = content[inner_start..close_start].trim();
                if let Some((name, arguments)) = parse_tool_call_json(inner) {
                    calls.push(ToolCall {
                        id: format!("text_call_{}", calls.len()),
                        kind: "function".to_string(),
                        function: FunctionCall { name, arguments },
                    });
                    remove.push((open_start, close_end));
                }
                // Whether it parsed or not, advance past this close tag.
                search_from = close_end;
            }
            None => {
                // No open tag precedes this close — orphan close; skip it.
                search_from = close_end;
            }
        }
    }

    if calls.is_empty() {
        return (content.to_string(), Vec::new());
    }

    // Rebuild the content with the parsed spans removed (ranges are in order
    // and non-overlapping since we scan left to right).
    let mut cleaned = String::with_capacity(content.len());
    let mut cursor = 0usize;
    for (start, end) in remove {
        cleaned.push_str(&content[cursor..start]);
        cursor = end;
    }
    cleaned.push_str(&content[cursor..]);

    let cleaned = collapse_blank_runs(cleaned.trim());
    (cleaned, calls)
}

/// Parse one `<tool_call>` inner JSON blob into `(name, arguments_string)`.
/// Returns `None` unless it is an object with a non-empty string `name` and an
/// `arguments` or `parameters` field (or `name` alone, which yields `"{}"`).
fn parse_tool_call_json(inner: &str) -> Option<(String, String)> {
    let value: serde_json::Value = serde_json::from_str(inner).ok()?;
    let obj = value.as_object()?;
    let name = obj.get("name")?.as_str()?;
    if name.is_empty() {
        return None;
    }
    let args_value = obj.get("arguments").or_else(|| obj.get("parameters"));
    let arguments = match args_value {
        // A JSON string is used verbatim (already stringified JSON / raw text).
        Some(serde_json::Value::String(s)) => s.clone(),
        // A JSON object is stringified into our String-encoded form.
        Some(v @ serde_json::Value::Object(_)) => serde_json::to_string(v).ok()?,
        // Scalars (number, bool, null) and arrays are not valid argument bags;
        // degrade to an empty object rather than emitting e.g. "5" or "null".
        Some(_) => "{}".to_string(),
        // name present but no arguments/parameters → empty object.
        None => "{}".to_string(),
    };
    Some((name.to_string(), arguments))
}

/// Strip any residual inline tool-call markup from assistant CONTENT that the
/// structured-call path + `extract_text_tool_calls` didn't already remove:
/// leftover well-formed `<tool_call> ... </tool_call>` spans, and ORPHAN/stray
/// `<tool_call>` or `</tool_call>` tags a (often weak) model emitted without a
/// valid JSON body. Keeps surrounding prose; collapses the blank lines a removed
/// block leaves behind. This is display/commit hygiene — actual call execution is
/// handled by `extract_text_tool_calls`.
///
/// Rules (applied in order):
/// 1. Remove every complete `<tool_call>...</tool_call>` span (non-greedy, all
///    occurrences) regardless of whether the inner text is valid JSON.
/// 2. If an unmatched `<tool_call>` remains (open tag with no following close —
///    e.g. a call still mid-stream or truncated), remove from that `<tool_call>`
///    to the END of the string.
/// 3. Remove any remaining bare/orphan `</tool_call>` or `<tool_call>` tags.
/// 4. Collapse 3+ consecutive newlines to 2, and trim trailing whitespace.
pub fn strip_tool_call_tags(content: &str) -> String {
    const OPEN: &str = "<tool_call>";
    const CLOSE: &str = "</tool_call>";

    // Work on a String copy so we can use replace_range without byte-boundary
    // panics — find() on &str always returns valid char boundaries.
    let mut s: String = content.to_string();

    // Step 1: Remove all well-formed <tool_call>...</tool_call> spans.
    // Repeat until no complete pair remains (handles adjacent occurrences).
    while let Some(open_pos) = s.find(OPEN) {
        // Search for the matching close AFTER the open tag.
        let after_open = open_pos + OPEN.len();
        let Some(rel_close) = s[after_open..].find(CLOSE) else { break };
        let close_end = after_open + rel_close + CLOSE.len();
        s.replace_range(open_pos..close_end, "");
    }

    // Step 2: If a lone <tool_call> remains (open with no matching close),
    // truncate from that point to the end of the string.
    if let Some(open_pos) = s.find(OPEN) {
        s.truncate(open_pos);
    }

    // Step 3: Remove any remaining orphan </tool_call> (and, defensively, any
    // stray <tool_call> that somehow survived).
    while let Some(pos) = s.find(CLOSE) {
        s.replace_range(pos..pos + CLOSE.len(), "");
    }
    while let Some(pos) = s.find(OPEN) {
        s.replace_range(pos..pos + OPEN.len(), "");
    }

    // Step 4: Collapse blank-line runs and trim trailing whitespace.
    collapse_blank_runs(s.trim_end())
}

/// Strip ANSI/VT escape sequences from `s`, returning clean plain text.
///
/// Handles the three common sequence families that colorized CLI output
/// (git, cargo, tsc, etc.) emits:
///
/// - **CSI** (`ESC [` followed by params ending in a final-byte in `@`..`~`):
///   covers `\x1b[96m`, `\x1b[0m`, `\x1b[1;31m`, cursor-move, erase, etc.
/// - **OSC** (`ESC ]` terminated by BEL `\x07` or ESC-backslash `ESC \`):
///   window-title and hyperlink sequences.
/// - **All others** (`ESC` followed by any single byte): two-byte escapes such
///   as `\x1b(B` (G0 charset select).
///
/// All other characters are copied verbatim; the scanner operates on Rust
/// `char`s so UTF-8 boundaries are always respected.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }
        // ESC: peek at the next char to determine sequence type.
        match chars.peek().copied() {
            Some('[') => {
                // CSI: consume '[' then skip until the final byte (0x40..=0x7E).
                chars.next(); // consume '['
                for c in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&c) {
                        break; // final byte consumed; sequence done
                    }
                }
            }
            Some(']') => {
                // OSC: consume ']' then skip until BEL or ESC-backslash ST.
                chars.next(); // consume ']'
                loop {
                    match chars.next() {
                        None => break,
                        Some('\u{07}') => break, // BEL terminator
                        Some('\u{1b}') => {
                            // ESC-backslash (ST) terminator: consume '\\'.
                            if chars.peek() == Some(&'\\') {
                                chars.next();
                            }
                            break;
                        }
                        Some(_) => {} // interior OSC byte, skip
                    }
                }
            }
            Some(_) => {
                // Any other two-byte escape (e.g. ESC ( B): skip the next byte.
                chars.next();
            }
            None => {} // lone ESC at end of string: nothing to consume
        }
    }
    out
}

/// Collapse any run of 3+ consecutive newlines into exactly 2, leaving other
/// whitespace untouched. Used to tidy the gap left behind when a `<tool_call>`
/// span is removed from between blocks of prose.
fn collapse_blank_runs(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut newline_run = 0usize;
    for ch in s.chars() {
        if ch == '\n' {
            newline_run += 1;
            if newline_run <= 2 {
                out.push(ch);
            }
        } else {
            newline_run = 0;
            out.push(ch);
        }
    }
    out
}

/// Repair a tool call's JSON-encoded `arguments` string against providers that
/// violate the streaming-delta contract.
///
/// OpenAI/OpenRouter `tool_calls[*].function.arguments` is meant to arrive as a
/// stream of pure DELTAS that, concatenated, form ONE JSON object. Some budget
/// routes instead re-send the COMPLETE arguments in every chunk (or repeat the
/// full arguments in the final frame). Blind concatenation then produces a valid
/// JSON document followed by trailing content — e.g. `{"command":"x"}{"command":"x"}`
/// or `{}{"command":"x"}` — which:
///   1. makes `serde_json::from_str` fail (trailing content), so the tool sees
///      `{}` and reports a missing required argument, and
///   2. is PERSISTED and re-sent, where the provider's prefill/validation rejects
///      the whole request ("unexpected content after document"), wedging the session.
///
/// This function parses the input as a STREAM of JSON values (stopping at the
/// first trailing garbage) and picks ONE clean value to keep:
/// - prefer the LAST parsed value that is a non-empty object (handles the
///   empty-then-full `{}` → `{...}` case by keeping the full one, and the
///   duplicate `{...}{...}` case by keeping a single copy);
/// - else the LAST parsed value (any kind), if anything parsed at all;
/// - else `"{}"` (empty input, or leading garbage that parses to nothing).
///
/// The chosen value is re-serialised compactly. For the NORMAL case — the input
/// is already a single valid JSON object — the result is a semantic no-op (only
/// whitespace/key-order may change, and arguments are read by key), so a working
/// tool call is never broken. On any serialisation error it returns `"{}"`.
pub fn sanitize_tool_arguments(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "{}".to_string();
    }

    // Parse the input as a SEQUENCE of JSON values. `into_iter::<Value>()` yields
    // one item per top-level value; the first `Err` marks trailing non-JSON
    // garbage (or a partial/incomplete value), at which point we stop and keep
    // whatever clean values we already collected.
    let mut last_any: Option<serde_json::Value> = None;
    let mut last_nonempty_obj: Option<serde_json::Value> = None;
    let stream = serde_json::Deserializer::from_str(trimmed).into_iter::<serde_json::Value>();
    for item in stream {
        match item {
            Ok(value) => {
                if value
                    .as_object()
                    .is_some_and(|obj| !obj.is_empty())
                {
                    last_nonempty_obj = Some(value.clone());
                }
                last_any = Some(value);
            }
            // Trailing garbage / incomplete value: stop; keep what parsed so far.
            Err(_) => break,
        }
    }

    // Prefer the last non-empty object; else the last value of any kind; else
    // nothing parsed → empty object.
    let chosen = match last_nonempty_obj.or(last_any) {
        Some(v) => v,
        None => return "{}".to_string(),
    };
    serde_json::to_string(&chosen).unwrap_or_else(|_| "{}".to_string())
}

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

