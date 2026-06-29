use super::types::{FunctionCall, ToolCall};

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
pub(super) fn parse_tool_call_json(inner: &str) -> Option<(String, String)> {
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

/// Collapse any run of 3+ consecutive newlines into exactly 2, leaving other
/// whitespace untouched. Used to tidy the gap left behind when a `<tool_call>`
/// span is removed from between blocks of prose.
pub(super) fn collapse_blank_runs(s: &str) -> String {
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
