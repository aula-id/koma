use super::types::{FunctionCall, ToolCall};

/// Parse out tool calls a model emitted as TEXT in its content rather than via
/// the native OpenAI-style `tool_calls` field. Budget / gpt-oss / GLM routes
/// (and Hermes/Qwen/ChatML-trained models generally) tend to write a call as a
/// `<tool_call>{"name": "...", "arguments": {...}}</tool_call>` span in plain
/// text; without this fallback such calls render as literal text and never run.
///
/// Also handles the "harmony"-style XML form emitted by gpt-oss / mimo models:
/// ```text
/// <tool_call>
/// <function=NAME>
/// <parameter=KEY>VALUE
/// </tool_call>
/// ```
/// and the standalone (unwrapped) variant without a `<tool_call>` outer tag.
///
/// Scans `content` for `<tool_call>` … `</tool_call>` spans. Each `</tool_call>`
/// close tag is paired with the NEAREST `<tool_call>` open that precedes it
/// (within the not-yet-consumed region), so a stray/unclosed open tag before a
/// valid block never swallows the valid block's call. Trims inner text and
/// parses it as JSON or as the harmony XML form. A span counts as a tool call
/// only when the inner text resolves to a non-empty name and arguments.
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
                if let Some((name, arguments)) = parse_tool_call_inner(inner) {
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

    // --- STEP 3: Standalone (unwrapped) harmony calls ---
    // After the <tool_call> scan, look for <function=NAME>...</function> spans
    // that are NOT already inside a removed range (to avoid double-counting).
    const FN_OPEN: &str = "<function=";
    const FN_CLOSE: &str = "</function>";

    let mut fn_search_from = 0usize;
    while let Some(rel_fn_open) = content[fn_search_from..].find(FN_OPEN) {
        let fn_start = fn_search_from + rel_fn_open;

        // Skip if this position is already inside a removed range.
        let already_removed = remove
            .iter()
            .any(|&(rs, re)| fn_start >= rs && fn_start < re);
        if already_removed {
            fn_search_from = fn_start + FN_OPEN.len();
            continue;
        }

        // A span runs from <function= to its matching </function>, or to the
        // next <function=, or to end-of-string — whichever comes first.
        let search_after_open = fn_start + FN_OPEN.len();
        let next_fn = content[search_after_open..]
            .find(FN_OPEN)
            .map(|r| search_after_open + r);
        let close_fn = content[search_after_open..]
            .find(FN_CLOSE)
            .map(|r| search_after_open + r);

        let (span_inner_end, span_end) = match (close_fn, next_fn) {
            (Some(cf), Some(nf)) if cf < nf => (cf, cf + FN_CLOSE.len()),
            (Some(cf), None) => (cf, cf + FN_CLOSE.len()),
            (_, Some(nf)) => (nf, nf), // next <function= is the terminator, not consumed
            (None, None) => (content.len(), content.len()),
        };

        let inner = &content[fn_start..span_inner_end];
        if let Some((name, arguments)) = parse_function_param_call(inner) {
            calls.push(ToolCall {
                id: format!("text_call_{}", calls.len()),
                kind: "function".to_string(),
                function: FunctionCall { name, arguments },
            });
            remove.push((fn_start, span_end));
        }

        fn_search_from = span_end.max(fn_start + FN_OPEN.len());
    }

    if calls.is_empty() {
        return (content.to_string(), Vec::new());
    }

    // Sort and deduplicate removal ranges (standalone scan adds after wrapped scan).
    remove.sort_by_key(|&(s, _)| s);

    // Rebuild the content with the parsed spans removed.
    let mut cleaned = String::with_capacity(content.len());
    let mut cursor = 0usize;
    for (start, end) in remove {
        if start > cursor {
            cleaned.push_str(&content[cursor..start]);
        }
        cursor = cursor.max(end);
    }
    cleaned.push_str(&content[cursor..]);

    let cleaned = collapse_blank_runs(cleaned.trim());
    (cleaned, calls)
}

/// Dispatch: try JSON form first, then harmony XML form.
pub(super) fn parse_tool_call_inner(inner: &str) -> Option<(String, String)> {
    parse_tool_call_json(inner).or_else(|| parse_function_param_call(inner))
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

/// Parse the `<function=NAME>` + `<parameter=KEY>VALUE` XML tool-call form
/// (gpt-oss / mimo / "harmony"-style) into (name, arguments_json_string).
///
/// Recognises:
/// ```text
/// <function=NAME>
/// <parameter=KEY>VALUE
/// <parameter=KEY2>VALUE2
/// </function>
/// ```
/// NAME = text between `<function=` and the next `>`, trimmed.
/// KEY  = text between `<parameter=` and the next `>`, trimmed.
/// VALUE = text after that `>` up to the first of:
///   `</parameter>`, `<parameter=`, `</function>`, `</tool_call>`, end-of-string.
/// Values are JSON-coerced: if `serde_json::from_str` succeeds they keep their
/// type (number, bool, null, array, object); otherwise they become strings.
pub(super) fn parse_function_param_call(inner: &str) -> Option<(String, String)> {
    const FN_OPEN: &str = "<function=";
    const PARAM_OPEN: &str = "<parameter=";

    // --- Extract NAME ---
    let fn_pos = inner.find(FN_OPEN)?;
    let after_fn = fn_pos + FN_OPEN.len();
    let gt_pos = inner[after_fn..].find('>')?;
    let name = inner[after_fn..after_fn + gt_pos].trim().to_string();
    if name.is_empty() {
        return None;
    }

    // --- Extract parameters ---
    let mut map = serde_json::Map::new();
    let params_region = &inner[after_fn + gt_pos + 1..]; // everything after first `>`

    let mut search = 0usize;
    while let Some(rel_p) = params_region[search..].find(PARAM_OPEN) {
        let p_start = search + rel_p;
        let after_p = p_start + PARAM_OPEN.len();

        // KEY: up to next `>`
        let Some(rel_gt) = params_region[after_p..].find('>') else {
            break;
        };
        let key = params_region[after_p..after_p + rel_gt].trim().to_string();
        let value_start = after_p + rel_gt + 1;

        // VALUE: up to first terminator
        let rest = &params_region[value_start..];
        let terminators: &[&str] = &["</parameter>", "<parameter=", "</function>", "</tool_call>"];
        let value_len = terminators
            .iter()
            .filter_map(|t| rest.find(t))
            .min()
            .unwrap_or(rest.len());
        let value_raw = rest[..value_len].trim();

        // Coerce: try JSON parse, fall back to string.
        let coerced: serde_json::Value = serde_json::from_str(value_raw)
            .unwrap_or_else(|_| serde_json::Value::String(value_raw.to_string()));

        if !key.is_empty() {
            map.insert(key, coerced);
        }

        // Advance past this parameter's terminator (if it's </parameter> or <parameter=)
        // to avoid re-scanning the same position.
        let consumed_to = value_start + value_len;
        let skip = params_region[consumed_to..]
            .find(PARAM_OPEN)
            .map(|r| consumed_to + r)
            .unwrap_or(params_region.len());
        search = skip;
    }

    let arguments = serde_json::to_string(&map).unwrap_or_else(|_| "{}".to_string());
    Some((name, arguments))
}

/// Strip any residual inline tool-call markup from assistant CONTENT that the
/// structured-call path + `extract_text_tool_calls` didn't already remove:
/// leftover well-formed `<tool_call> ... </tool_call>` spans, and ORPHAN/stray
/// `<tool_call>` or `</tool_call>` tags a (often weak) model emitted without a
/// valid JSON body. Also removes orphan harmony tags (`<function=...>`,
/// `<parameter=...>`, `</function>`, `</parameter>`). Keeps surrounding prose;
/// collapses the blank lines a removed block leaves behind. This is
/// display/commit hygiene — actual call execution is handled by
/// `extract_text_tool_calls`.
///
/// Rules (applied in order):
/// 1. Remove every complete `<tool_call>...</tool_call>` span (non-greedy, all
///    occurrences) regardless of whether the inner text is valid JSON.
/// 2. If an unmatched `<tool_call>` remains (open tag with no following close —
///    e.g. a call still mid-stream or truncated), remove from that `<tool_call>`
///    to the END of the string.
/// 3. Remove any remaining bare/orphan `</tool_call>` or `<tool_call>` tags.
/// 4. Remove orphan harmony tags: `<function=...>` opening tags (up to `>`),
///    `<parameter=...>` opening tags (up to `>`), and `</function>` /
///    `</parameter>` close tags.
/// 5. Collapse 3+ consecutive newlines to 2, and trim trailing whitespace.
pub fn strip_tool_call_tags(content: &str) -> String {
    // Some summarizer models emit the bare plural <tool_calls>/</tool_calls> tags
    // (no inner payload) — distinct from the singular <tool_call> wrapper
    // that carries a JSON/harmony body. Handle both.
    const OPEN: &str = "<tool_call>";
    const CLOSE: &str = "</tool_call>";
    const PLURAL_OPEN: &str = "<tool_calls>";
    const PLURAL_CLOSE: &str = "</tool_calls>";

    // Work on a String copy so we can use replace_range without byte-boundary
    // panics — find() on &str always returns valid char boundaries.
    let mut s: String = content.to_string();

    // Step 1: Remove all well-formed <tool_call>...</tool_call> spans.
    // Repeat until no complete pair remains (handles adjacent occurrences).
    while let Some(open_pos) = s.find(OPEN) {
        // Search for the matching close AFTER the open tag.
        let after_open = open_pos + OPEN.len();
        let Some(rel_close) = s[after_open..].find(CLOSE) else {
            break;
        };
        let close_end = after_open + rel_close + CLOSE.len();
        s.replace_range(open_pos..close_end, "");
    }

    // Step 1b: Remove all <tool_calls>...</tool_calls> spans (plural form).
    while let Some(open_pos) = s.find(PLURAL_OPEN) {
        let after_open = open_pos + PLURAL_OPEN.len();
        let Some(rel_close) = s[after_open..].find(PLURAL_CLOSE) else {
            break;
        };
        let close_end = after_open + rel_close + PLURAL_CLOSE.len();
        s.replace_range(open_pos..close_end, "");
    }

    // Step 2: If a lone <tool_call> or <tool_calls> remains (open with no
    // matching close), truncate from that point to the end of the string.
    if let Some(open_pos) = s.find(OPEN).or_else(|| s.find(PLURAL_OPEN)) {
        s.truncate(open_pos);
    }

    // Step 3: Remove any remaining orphan close/open tags.
    for tag in [CLOSE, PLURAL_CLOSE, OPEN, PLURAL_OPEN] {
        while let Some(pos) = s.find(tag) {
            s.replace_range(pos..pos + tag.len(), "");
        }
    }

    // Step 4: Remove orphan harmony tags.
    // Remove <function=...> opening tags (tag up to and including its `>`).
    while let Some(pos) = s.find("<function=") {
        // Find the closing `>` of this opening tag.
        let after = pos + "<function=".len();
        if let Some(rel_gt) = s[after..].find('>') {
            s.replace_range(pos..after + rel_gt + 1, "");
        } else {
            // No closing `>` — remove to end of string to avoid infinite loop.
            s.truncate(pos);
            break;
        }
    }
    // Remove <parameter=...> opening tags.
    while let Some(pos) = s.find("<parameter=") {
        let after = pos + "<parameter=".len();
        if let Some(rel_gt) = s[after..].find('>') {
            s.replace_range(pos..after + rel_gt + 1, "");
        } else {
            s.truncate(pos);
            break;
        }
    }
    // Remove </function> and </parameter> close tags.
    while let Some(pos) = s.find("</function>") {
        s.replace_range(pos..pos + "</function>".len(), "");
    }
    while let Some(pos) = s.find("</parameter>") {
        s.replace_range(pos..pos + "</parameter>".len(), "");
    }

    // Step 5: Collapse blank-line runs and trim trailing whitespace.
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

#[cfg(test)]
#[path = "extract_test.rs"]
mod tests;
