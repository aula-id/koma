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
