//! Transcript area: committed messages, live streaming buffer, sub-agent
//! inline indicator, and the follow-scroll logic.

use ratatui::{
    layout::{Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use crate::app::state::AppStateRest;
use crate::dto::chat::Role;
use crate::view::theme::Palette;
use super::helpers::{
    push_thinking_line, render_block, render_tool_box, split_thinking, truncate_chars, THINK_BAR,
};

/// Render the transcript area into `body_chunk`.
///
/// Padded, flat. Each message is a block: a coloured bullet (★ user / ● ai)
/// on the first line, text hanging-indented under it, blank line between
/// blocks. Pre-wrapped by hand for the hanging indent.
pub(super) fn render_transcript(
    frame: &mut Frame,
    body_chunk: Rect,
    rest: &AppStateRest,
    palette: &Palette,
) {
    let body = body_chunk.inner(Margin { horizontal: 2, vertical: 0 });
    let wrap_w = (body.width as usize).saturating_sub(2).max(1);

    // Render (or reuse) each committed message's lines. Cache is keyed by width
    // + palette; only NEW messages are rendered, so syntect doesn't re-run every
    // frame. A shrink (compaction / resend) or key change forces a full rebuild.
    {
        let mut cache = rest.transcript_cache.borrow_mut();
        if cache.width != wrap_w || cache.palette != Some(*palette) {
            cache.width = wrap_w;
            cache.palette = Some(*palette);
            cache.blocks.clear();
        }
        let committed: Vec<&crate::dto::chat::ChatMessage> = rest
            .fg()
            .session
            .as_ref()
            .map(|s| {
                s.conversation
                    .messages()
                    .iter()
                    .filter(|m| m.role != Role::System)
                    .collect()
            })
            .unwrap_or_default();

        // Which tool calls have COMPLETED: a `tool`-role result message exists
        // whose `tool_call_id` points back at the call. Built fresh every frame
        // from the live conversation so the gear→check flip is NOT baked into the
        // (one-shot) cached Assistant block — the result message is committed a
        // round LATER than the assistant call, so the cached block can't carry the
        // final glyph. The tool-call lines are therefore rendered fresh at frame
        // assembly (below), consulting this set, while the heavy markdown body
        // stays cached. `&str` borrows from `committed`, valid for this frame.
        let completed_tool_ids: std::collections::HashSet<&str> = committed
            .iter()
            .filter(|m| m.role == Role::Tool)
            .filter_map(|m| m.tool_call_id.as_deref())
            .collect();
        // tool_call_id → result content, harvested from the `tool`-role result
        // messages so each call can render its own result inline. Borrows from
        // `committed`, valid for this frame.
        let tool_results: std::collections::HashMap<&str, &str> = committed
            .iter()
            .filter(|m| m.role == Role::Tool)
            .filter_map(|m| m.tool_call_id.as_deref().map(|id| (id, m.content.as_str())))
            .collect();
        if cache.blocks.len() > committed.len() {
            cache.blocks.clear(); // shrank → stale prefix can't be trusted
        }
        let start = cache.blocks.len();
        for msg in committed.iter().skip(start) {
            // One block per message, index-aligned with `committed`. A hidden
            // harness tool result yields an EMPTY block (skipped at assembly), so
            // the cache never falls out of step with the message list. A `tool`-role
            // result now renders EMPTY here — its output is rendered inline under its
            // own call by `render_tool_lines`, so the cached block stays trivial.
            cache.blocks.push(render_message_block(msg, palette, wrap_w));
        }

        // Assemble the frame: cached blocks (with blank separators) + the live
        // streaming line (rendered fresh — it changes every token). `cache.blocks`
        // is index-aligned with `committed` (one block per non-system message), so
        // we zip them: the block carries the cached body, and for an Assistant turn
        // the tool-call lines are appended fresh here (glued to the same block, no
        // separator) with a live ⚙/✓ glyph from `completed_tool_ids`.
        let mut lines: Vec<Line<'static>> = Vec::new();
        let mut first = true;
        for (i, block) in cache.blocks.iter().enumerate() {
            // The fresh tool-call lines for this block, if it's an assistant turn
            // that requested calls. A finished call (its id is in the completed set)
            // gets an accent `✓ `; an in-flight one keeps the dim `⚙ `. Normally
            // indented 2 cols so they hang under the `●` bullet, BUT when the
            // assistant body is empty (a pure tool-call turn → empty cached block)
            // the FIRST tool line takes the `● ` bullet so the block isn't a
            // bullet-less orphan.
            let has_body = !block.is_empty();
            let tool_lines: Vec<Line<'static>> = committed
                .get(i)
                .map(|m| {
                    render_tool_lines(
                        m,
                        &completed_tool_ids,
                        has_body,
                        palette,
                        wrap_w,
                        &tool_results,
                    )
                })
                .unwrap_or_default();

            // Empty blocks (hidden harness messages) with no tool lines leave no
            // visual trace: skip both the block AND its blank separator so the
            // transcript is clean. (A hidden message never carries tool calls.)
            if block.is_empty() && tool_lines.is_empty() {
                continue;
            }
            if !first {
                lines.push(Line::from(""));
            }
            first = false;
            lines.extend(block.iter().cloned());
            lines.extend(tool_lines);
        }
        // Live partial turn: the in-progress reasoning (dim+italic, on top) and
        // content (fg). Reasoning typically streams first (the model thinks, then
        // answers), so the block shows whenever EITHER buffer has text — they
        // share one `●` bullet. Stream renders plain (not markdown) for perf +
        // partial-fence safety.
        let partial_content = rest.fg().streaming.as_deref().unwrap_or("");
        let partial_reasoning = rest.fg().stream_reasoning.as_str();
        if !partial_content.is_empty() || !partial_reasoning.is_empty() {
            if !first {
                lines.push(Line::from(""));
            }
            let thinking_style = Style::default()
                .fg(palette.dim)
                .add_modifier(Modifier::ITALIC);
            let bar_style = Style::default().fg(palette.dim);
            let mut logical: Vec<Vec<Span<'static>>> = Vec::new();
            // Partial reasoning first, dim+italic, each line prefixed with the
            // blockquote bar (mirrors the committed-message reasoning render).
            // These are emitted pre-wrapped, so render_block passes them through.
            if !partial_reasoning.is_empty() {
                for line in partial_reasoning.lines() {
                    push_thinking_line(&mut logical, line, thinking_style, bar_style, wrap_w);
                }
            }
            // Blank line between the barred thinking block and the answer so the
            // transition is clear, when both are present.
            if !logical.is_empty() && !partial_content.is_empty() {
                logical.push(vec![]);
            }
            // Then the partial answer in the theme fg (one logical line; wraps).
            // Strip residual tool-call markup so tags don't flash mid-stream; the
            // "unmatched open → cut to end" rule in the stripper naturally hides a
            // call that is still being emitted. Render nothing if the result is empty.
            if !partial_content.is_empty() {
                let stripped = crate::dto::chat::strip_tool_call_tags(partial_content);
                if !stripped.is_empty() {
                    logical.push(vec![Span::styled(
                        stripped,
                        Style::default().fg(palette.fg),
                    )]);
                }
            }
            lines.extend(render_block(logical, "● ", palette.fg, wrap_w, true));
        }

        // Sub-agent inline indicator: one animated line per RUNNING sub-agent,
        // appended at the bottom of the transcript so it sits just above the input
        // box and has full width. Uses the same time-driven braille spinner as the
        // compact animation (80ms/frame cadence). Only rendered while at least one
        // sub-agent is Running; disappears automatically when all finish.
        const SA_SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let running_agents: Vec<&crate::app::subagent::SubAgent> = rest
            .fg()
            .subagents
            .iter()
            .filter(|s| matches!(s.status, crate::app::subagent::SubAgentStatus::Running) && !s.detached)
            .collect();
        if !running_agents.is_empty() {
            let elapsed_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let frame_idx = (elapsed_ms / 80) as usize;
            let glyph = SA_SPINNER[frame_idx % SA_SPINNER.len()];
            if !first {
                lines.push(Line::from(""));
            }
            for sa in &running_agents {
                // Last meaningful transcript line as the "current action"; fall
                // back to "starting…" when the transcript is still empty.
                let action = sa
                    .transcript
                    .last()
                    .map(|s| s.as_str())
                    .unwrap_or("starting…");
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(glyph.to_string(), Style::default().fg(palette.accent)),
                    Span::styled(
                        format!(" {} · {}", sa.agent_name, action),
                        Style::default().fg(palette.dim),
                    ),
                ]));
            }
        }

        // Scroll model: follow pins to the bottom (auto-scrolls as content grows);
        // otherwise show the stored offset, clamped. Publish max_scroll so the key/
        // mouse handlers can clamp + detect bottom.
        let total = u16::try_from(lines.len()).unwrap_or(u16::MAX);
        let max_scroll = total.saturating_sub(body.height);
        rest.last_max_scroll.set(max_scroll);
        let top = if rest.fg().follow { max_scroll } else { rest.fg().scroll.min(max_scroll) };
        let messages = Paragraph::new(lines).scroll((top, 0));
        frame.render_widget(messages, body);
    } // cache borrow ends
}

/// Map a tool's function name to a short box LABEL, or `None` when the tool's
/// result should NOT be boxed (terse-status tools keep the compact one-liner).
/// MCP (`mcp__…`) and security (`sec_…`) tool families collapse to one label each.
fn tool_box_label(name: &str) -> Option<&'static str> {
    if name.starts_with("mcp__") {
        return Some("mcp");
    }
    if name.starts_with("sec_") {
        return Some("sec");
    }
    Some(match name {
        "bash" => "bash",
        "read" => "read",
        "grep" => "grep",
        "glob" => "glob",
        "dir_list" => "dir",
        "git_operator" | "git_cred" | "git_worktree" => "git",
        "web_fetch" | "web_search" => "web",
        "recall" => "memory",
        _ => return None, // None → not boxed, keep the single-line rendering
    })
}

/// Render a tool call as a clean, quote-less signature for the transcript header:
/// `bash(ls src-agent/)`, `git_operator(log --oneline -5)`, `grep(fn main)`,
/// `read(Cargo.toml)`. Display-only; the real JSON sent to the model is untouched.
/// Unmapped tools (mcp__*, sec_*, future) fall back to their object values, or the
/// raw args if parsing fails.
fn format_tool_signature(name: &str, args_json: &str) -> String {
    let v: serde_json::Value =
        serde_json::from_str(args_json).unwrap_or(serde_json::Value::Null);
    let inner = tool_signature_inner(name, &v).unwrap_or_else(|| generic_inner(&v, args_json));
    // Collapse newlines/runs of whitespace so the header stays one line, then cap.
    let flat = inner.split_whitespace().collect::<Vec<_>>().join(" ");
    let capped = truncate_chars(&flat, 60);
    format!("{name}({capped})")
}

/// The salient argument(s) for a known tool, positional and quote-less. `None`
/// means "not specially mapped" → caller uses the generic fallback.
fn tool_signature_inner(name: &str, v: &serde_json::Value) -> Option<String> {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
    let arr = |k: &str| {
        v.get(k).and_then(|x| x.as_array()).map(|a| {
            a.iter().filter_map(|e| e.as_str()).collect::<Vec<_>>().join(" ")
        })
    };
    match name {
        "bash" => s("command"),
        "git_operator" => arr("args"),
        "git_cred" => {
            let action = s("action")?;
            Some(match s("key") {
                Some(k) => format!("{action} {k}"),
                None => action,
            })
        }
        "git_worktree" => {
            let action = s("action")?;
            let extra = s("path").or_else(|| s("name")).or_else(|| s("branch"));
            Some(match extra {
                Some(e) => format!("{action} {e}"),
                None => action,
            })
        }
        "read" | "write" | "edit" | "delete" | "cd" => s("path"),
        "dir_list" => s("path").or_else(|| arr("paths")),
        "grep" | "glob" => s("pattern"),
        "web_fetch" | "web_download" => s("url"),
        "web_search" => s("query"),
        "remember" => s("slug").or_else(|| s("description")),
        "forget" | "recall" => s("slug"),
        "task" => {
            let agent = s("agent")?;
            Some(match s("prompt") {
                Some(p) => format!("{agent}: {p}"),
                None => agent,
            })
        }
        "todowrite" => v
            .get("todos")
            .and_then(|x| x.as_array())
            .map(|a| format!("{} todos", a.len())),
        "bash_output" | "bash_kill" => s("job_id"),
        "task_output" | "task_kill" => v.get("id").map(|x| match x {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        }),
        _ => None,
    }
}

/// Generic fallback for unmapped tools: the object's scalar/array values joined,
/// or the raw args string if it isn't a JSON object / failed to parse.
fn generic_inner(v: &serde_json::Value, raw: &str) -> String {
    match v {
        serde_json::Value::Object(map) => {
            let parts: Vec<String> = map
                .values()
                .filter_map(|val| match val {
                    serde_json::Value::String(s) => Some(s.clone()),
                    serde_json::Value::Bool(b) => Some(b.to_string()),
                    serde_json::Value::Number(n) => Some(n.to_string()),
                    serde_json::Value::Array(a) => Some(
                        a.iter().filter_map(|e| e.as_str()).collect::<Vec<_>>().join(" "),
                    ),
                    _ => None,
                })
                .collect();
            if parts.is_empty() { raw.to_string() } else { parts.join(", ") }
        }
        _ => raw.to_string(),
    }
}

/// Render a completed tool call's RESULT: a rounded-dotted box for output-producing
/// tools, else the compact dim one-liner. Empty/whitespace results and the harness
/// plan-nudge render nothing. Narrow panes (`wrap_w < 8`) skip the box.
fn render_tool_result(
    content: &str,
    name: &str,
    palette: &Palette,
    wrap_w: usize,
) -> Vec<Line<'static>> {
    if content.starts_with(crate::dto::chat::PLAN_NUDGE_MARK) {
        return Vec::new();
    }
    if content.trim().is_empty() {
        return Vec::new();
    }
    if let Some(lbl) = tool_box_label(name) {
        if wrap_w >= 8 {
            return render_tool_box(content, lbl, palette, wrap_w);
        }
    }
    // Terse fallback: first line only, truncated, dim, under a 4-col indent.
    let first = truncate_chars(content.lines().next().unwrap_or(""), 80);
    render_block(
        vec![vec![Span::styled(first, Style::default().fg(palette.dim))]],
        "    ",
        palette.dim,
        wrap_w,
        false,
    )
}

/// Build ONE message's visual block (the body, sans the fresh tool-call lines).
///
/// This is the per-message renderer the main transcript caches AND the
/// full-screen sub-agent viewer reuses, so both paths render identical markdown,
/// reasoning/thinking blocks, and compact tool-result rows.
///
/// - `User`     → `★` accent bullet, plain text.
/// - `Assistant`→ `●` bullet; native reasoning + wanderer "thinking" prefix
///   rendered dim+italic with the blockquote bar, then the body as markdown. The
///   per-tool-call lines are NOT included here — they carry a live ⚙→✓ glyph and
///   are appended fresh by [`render_tool_lines`] at assembly time.
/// - `Tool`     → EMPTY block: a tool result is now rendered inline directly under
///   its own call in [`render_tool_lines`], so the standalone tool block carries no
///   output here (and is skipped at assembly).
/// - `System`   → EMPTY block (never shown).
///
/// An empty `Vec` means "no visual block"; callers skip it (and its separator).
pub(super) fn render_message_block(
    msg: &crate::dto::chat::ChatMessage,
    palette: &Palette,
    wrap_w: usize,
) -> Vec<Line<'static>> {
    match msg.role {
        Role::User => {
            // bg-bash completion nudge: render as ONE compact dim line with a green
            // `✓` (just the `[bash-N] status` summary, line 1 of the body). The model-
            // only context on the remaining lines is NOT shown. NOT a `★` user turn.
            if let Some(body) = msg.content.strip_prefix(crate::dto::chat::BASH_NUDGE_MARK) {
                return render_bash_nudge_block(body, palette);
            }
            // `!` user-shell shortcut entry: a SHELL_MARK-prefixed user message
            // carrying `$ <cmd>\n<output>`. Render it DISTINCTLY (not a `★` user
            // turn): a `$ <cmd>` header in the accent, then the captured output dim
            // and wrapped under it — visually a command + its result, not a message.
            if let Some(body) = msg.content.strip_prefix(crate::dto::chat::SHELL_MARK) {
                return render_shell_block(body, palette, wrap_w);
            }
            // The typed message (with any `[Image #N]` markers) in the accent
            // colour, then -- when the message carries image attachments -- a
            // permanent yellow/orange warn-style card listing them. The card is
            // ALWAYS yellow (a warn cue): koma can't guarantee the model read the
            // image, and the model-visible strip warning is injected separately at
            // send. Styled like a tool-call card (icon + dim text) but in warn.
            let mut lines = render_user_message(&msg.content, palette, wrap_w);
            lines.extend(render_attachment_card(&msg.attachments));
            lines
        }
        Role::Assistant => {
            // If the message contains wanderer lead-in lines (`Word: ...`), the
            // entire block up to and including the LAST such line is rendered
            // dim+italic (the "thinking" block); the remainder is markdown.
            let (thinking_block, response_body) = split_thinking(&msg.content);
            let thinking_style = Style::default()
                .fg(palette.dim)
                .add_modifier(Modifier::ITALIC);
            let bar_style = Style::default().fg(palette.dim);
            let mut logical: Vec<Vec<Span<'static>>> = Vec::new();
            // Native reasoning channel (the model's streamed `reasoning`, captured
            // separately from `content`). Rendered first, dim + italic, each line
            // prefixed with the blockquote bar so the whole thinking block reads as
            // quoted text. Display-only — it never re-enters the conversation or disk.
            if let Some(reasoning) = msg.reasoning.as_deref() {
                if !reasoning.is_empty() {
                    for line in reasoning.lines() {
                        push_thinking_line(&mut logical, line, thinking_style, bar_style, wrap_w);
                    }
                }
            }
            if let Some(thinking) = thinking_block {
                for line in thinking.lines() {
                    push_thinking_line(&mut logical, line, thinking_style, bar_style, wrap_w);
                }
            }
            // Blank line between the (barred) thinking block and the answer so the
            // quote→answer transition is clear. Only when there IS both.
            if !logical.is_empty() && !response_body.is_empty() {
                logical.push(vec![]);
            }
            if !response_body.is_empty() {
                logical.extend(crate::view::markdown::render(response_body, palette, wrap_w));
            }
            render_block(logical, "● ", palette.fg, wrap_w, false)
        }
        // Tool results are now rendered inline directly under their own call in
        // `render_tool_lines`, so the standalone tool block is empty (and skipped at
        // assembly).
        Role::Tool => Vec::new(),
        Role::System => Vec::new(),
    }
}

/// Render a `★`-less user message as a full-width band: a solid accent rail in
/// column 0, a 1-column band-colored gap in column 1, then the message text
/// (accent on the gray band) starting in column 2, each visual line padded with
/// band-colored spaces out to the full body width so the band runs edge to edge.
/// One blank band row is emitted above and below the text (vertical padding).
fn render_user_message(content: &str, palette: &Palette, wrap_w: usize) -> Vec<Line<'static>> {
    let band = Style::default().bg(palette.panel);
    let rail = Style::default().fg(palette.accent).bg(palette.panel);
    let text = Style::default().fg(palette.accent).bg(palette.panel);
    let full_w = wrap_w + 2;
    // Text sits after the 1-col rail AND a 1-col gap, so it wraps to `full_w - 2`.
    let inner = full_w.saturating_sub(2).max(1);

    let mut out: Vec<Line<'static>> = Vec::new();
    // Top padding: a blank band row (rail + gap + band fill).
    out.push(band_row(&rail, &band, full_w, Vec::new()));
    for logical in content.split('\n') {
        let wrapped =
            crate::view::markdown::wrap_spans(&[Span::styled(logical.to_string(), text)], inner);
        for visual in wrapped {
            // wrap_spans inserts word-separator spaces with the DEFAULT style (no bg),
            // which would punch dark holes through the band — flatten each visual line
            // into ONE span carrying the band `text` style so spaces inherit the bg.
            let line_text: String = visual.iter().map(|s| s.content.as_ref()).collect();
            out.push(band_row(&rail, &band, full_w, vec![Span::styled(line_text, text)]));
        }
    }
    // Bottom padding.
    out.push(band_row(&rail, &band, full_w, Vec::new()));
    out
}

/// Assemble one band row: a solid-accent rail cell in column 0, a band-colored
/// gap cell in column 1, then the (already `text`-styled) wrapped span run, then
/// a band-colored right-pad out to `full_w` columns so the band reaches the full
/// body width.
fn band_row(
    rail: &Style,
    band: &Style,
    full_w: usize,
    text_spans: Vec<Span<'static>>,
) -> Line<'static> {
    let text_cols: usize = text_spans.iter().map(|s| s.content.chars().count()).sum();
    let mut spans = vec![
        Span::styled("▌", *rail), // col 0: half-width accent rail (left half block)
        Span::styled(" ", *band), // col 1: band-colored gap
    ];
    spans.extend(text_spans); // col 2+: text on band
    let used = 2 + text_cols;
    if used < full_w {
        spans.push(Span::styled(" ".repeat(full_w - used), *band));
    }
    Line::from(spans)
}

/// Render a `!` user-shell entry's block: a `$ <cmd>` header (accent bullet +
/// command) over the captured output (dim, wrapped, hanging-indented).
///
/// `body` is the message content with the [`crate::dto::chat::SHELL_MARK`] prefix
/// already stripped, shaped `"$ <cmd>\n<output…>"`. The first line is the command
/// header; the remainder is the captured stdout+stderr (already ANSI-stripped and
/// output-capped at run time). The `$ ` bullet is split off the header so the
/// command renders right after an accent `$` glyph (no double `$`); an unexpectedly
/// header-less body degrades gracefully (the whole first line becomes the header).
fn render_shell_block(body: &str, palette: &Palette, wrap_w: usize) -> Vec<Line<'static>> {
    let mut lines = body.lines();
    let header = lines.next().unwrap_or("$");
    // Strip the leading "$ " so it can be re-emitted as the accent bullet.
    let cmd = header.strip_prefix("$ ").unwrap_or(header);

    let mut logical: Vec<Vec<Span<'static>>> = Vec::new();
    // Header line: the command in the accent colour (the `$ ` bullet is supplied by
    // render_block below).
    logical.push(vec![Span::styled(
        cmd.to_string(),
        Style::default().fg(palette.accent),
    )]);
    // Output lines: dim, one logical line each (wrapped by render_block). A blank
    // line is preserved as an empty logical line so output spacing is kept.
    for line in lines {
        logical.push(vec![Span::styled(
            line.to_string(),
            Style::default().fg(palette.dim),
        )]);
    }
    render_block(logical, "$ ", palette.accent, wrap_w, true)
}

/// Render a background-bash completion nudge as a single compact line: a GREEN
/// `✓` glyph followed by the dim per-job summary (line 1 of `body`). The remaining
/// lines of `body` are model-only context and are NOT displayed. Styled like a
/// tool-call sub-line (2-col indent + dim text), not a `★` user turn. The green is
/// hardcoded (theme-independent, like the orange attachment card) so the check
/// always reads as "success".
fn render_bash_nudge_block(body: &str, palette: &Palette) -> Vec<Line<'static>> {
    let summary = body.lines().next().unwrap_or("").to_string();
    let green = Color::Rgb(0, 200, 83);
    vec![Line::from(vec![
        Span::raw("  "),
        Span::styled("\u{2713} ", Style::default().fg(green)),
        Span::styled(summary, Style::default().fg(palette.dim)),
    ])]
}

/// Render the orange attachment folder-tree lines for a user message that
/// carries image attachments. Minimalist design: an "images" root line, then
/// one tree branch per attachment (├─ for non-last, └─ for the last).
/// Returns an empty `Vec` when there are no attachments.
///
/// ALWAYS orange-coloured (fixed Color::Rgb(255, 180, 60)), matching the approval
/// card in overlays.rs — independent of the theme palette so it always reads as
/// a warn cue.
fn render_attachment_card(
    attachments: &[crate::dto::chat::Attachment],
) -> Vec<Line<'static>> {
    if attachments.is_empty() {
        return Vec::new();
    }
    // Fixed orange colour matching the tool-approval card in overlays.rs.
    let orange = Color::Rgb(255, 180, 60);
    let style = Style::default().fg(orange);
    let dim = Style::default().fg(orange).add_modifier(Modifier::DIM);
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Root: "  images"
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("images", style),
    ]));

    // One line per attachment, using tree connectors.
    let last_idx = attachments.len().saturating_sub(1);
    for (i, att) in attachments.iter().enumerate() {
        let connector = if i == last_idx {
            Span::styled("\u{2514}\u{2500} ", dim)  // └─
        } else {
            Span::styled("\u{251C}\u{2500} ", dim)  // ├─
        };
        lines.push(Line::from(vec![
            Span::raw("  "),
            connector,
            Span::styled(
                format!("[Image #{}] {}", att.marker_n, att.file_name()),
                style,
            ),
        ]));
    }
    lines
}

/// The fresh per-tool-call lines for an Assistant turn that requested calls.
///
/// Rendered fresh (never cached) so the leading glyph flips `⚙`→`✓` the moment
/// the matching tool result lands (a later round): a finished call (its id in
/// `completed`) gets an accent `✓ `; an in-flight one keeps the dim `⚙ `. Lines
/// hang under the `●` bullet with a 2-col indent, EXCEPT when the assistant body
/// is empty (`has_body == false`) — then the first tool line takes the `● ` bullet
/// so a pure tool-call turn isn't a bullet-less orphan. A non-Assistant message
/// or one with no tool calls yields no lines.
///
/// Once a call's result has landed (its id is in `completed`), that result is
/// appended inline directly under the call's header — tight, no separator — via
/// [`render_tool_result`], looked up in `tool_results` by call id.
pub(super) fn render_tool_lines(
    msg: &crate::dto::chat::ChatMessage,
    completed: &std::collections::HashSet<&str>,
    has_body: bool,
    palette: &Palette,
    wrap_w: usize,
    tool_results: &std::collections::HashMap<&str, &str>,
) -> Vec<Line<'static>> {
    if msg.role != Role::Assistant {
        return Vec::new();
    }
    let Some(calls) = msg.tool_calls.as_ref() else {
        return Vec::new();
    };
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(calls.len());
    for (ci, call) in calls.iter().enumerate() {
        let done = completed.contains(call.id.as_str());
        let (glyph, glyph_style) = if done {
            ("✓ ", Style::default().fg(palette.accent))
        } else {
            ("⚙ ", Style::default().fg(palette.dim))
        };
        let prefix = if !has_body && ci == 0 {
            Span::styled("● ", Style::default().fg(palette.fg))
        } else {
            Span::raw("  ")
        };

        // plan_ready: render the composed user-facing plan (checklist + full plan,
        // or checklist + highlights when the plan is long) as a readable quote block
        // instead of the raw args blob. The interception rewrites the `highlights`
        // arg to this composed digest (see approval::process_tools +
        // conversation::set_tool_call_args), so we just parse it out; on any parse
        // failure fall through to the generic tool-call line. Display-only.
        if call.function.name == "plan_ready" {
            if let Some(digest) =
                serde_json::from_str::<serde_json::Value>(&call.function.arguments)
                    .ok()
                    .and_then(|v| v.get("highlights").and_then(|s| s.as_str()).map(str::to_string))
            {
                // Header: "⚙/✓ plan ready" — the glyph flips to ✓ once the user
                // decides and the plan_ready tool result lands.
                lines.push(Line::from(vec![
                    prefix,
                    Span::styled(glyph, glyph_style),
                    Span::styled("plan ready", Style::default().fg(palette.dim)),
                ]));
                // Digest body: the same dim left rule the reasoning block uses
                // (THINK_BAR — one rail, never a box), hung under a 2-col indent and
                // laid out to the pane width. The digest carries real Markdown
                // (**bold**, `code`, headings, lists), so it's rendered through the
                // block-aware markdown renderer instead of raw-styled per line; the
                // renderer already wraps/boxes each block, so its visual lines are
                // pushed as-is (NOT re-wrapped via `wrap_spans`) with the bar prefixed.
                let bar_style = Style::default().fg(palette.dim);
                let inner_w = wrap_w.saturating_sub(2 + THINK_BAR.chars().count()).max(1);
                for visual in crate::view::markdown::render(&digest, palette, inner_w) {
                    let mut line = vec![Span::raw("  "), Span::styled(THINK_BAR, bar_style)];
                    line.extend(visual);
                    lines.push(Line::from(line));
                }
                // Trailing clearance: 5 bar-only blank rows so the bottom approval
                // pane doesn't cover the last lines of the plan.
                for _ in 0..5 {
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(THINK_BAR, bar_style),
                    ]));
                }
                continue;
            }
        }

        lines.push(Line::from(vec![
            prefix,
            Span::styled(glyph, glyph_style),
            Span::styled(
                format_tool_signature(&call.function.name, &call.function.arguments),
                Style::default().fg(palette.dim),
            ),
        ]));

        // For background bash calls, append a dim+italic annotation sub-line.
        if call.function.name == "bash" {
            let parsed = serde_json::from_str::<serde_json::Value>(&call.function.arguments)
                .unwrap_or_else(|_| serde_json::json!({}));
            let is_background = parsed
                .get("run_in_background")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if is_background {
                let annotation_style = Style::default()
                    .fg(palette.dim)
                    .add_modifier(Modifier::ITALIC);
                lines.push(Line::from(vec![Span::styled(
                    "  ↳ running in background · /bash to manage",
                    annotation_style,
                )]));
            }
        }
        // For background task (sub-agent) calls, mirror the bash annotation.
        if call.function.name == "task" {
            let parsed = serde_json::from_str::<serde_json::Value>(&call.function.arguments)
                .unwrap_or_else(|_| serde_json::json!({}));
            let is_background = parsed
                .get("run_in_background")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if is_background {
                let annotation_style = Style::default()
                    .fg(palette.dim)
                    .add_modifier(Modifier::ITALIC);
                lines.push(Line::from(vec![Span::styled(
                    "  ↳ running in background · /task to manage",
                    annotation_style,
                )]));
            }
        }

        // The result glues directly under THIS call — tight, no separator. Only once
        // the result has landed (`done`). Output tools get a box; others a terse line.
        if done {
            if let Some(result) = tool_results.get(call.id.as_str()) {
                lines.extend(render_tool_result(result, &call.function.name, palette, wrap_w));
            }
        }
    }
    lines
}

/// Assemble a full transcript from a flat `&[ChatMessage]` slice into styled
/// visual lines, EXACTLY like the main chat (markdown bodies, reasoning/thinking
/// blocks, blank separators, and live ⚙/✓ tool-call lines).
///
/// Used by the full-screen sub-agent viewer, which renders a sub-agent's
/// structured `messages` view-only. Unlike the main transcript this does NOT
/// cache (the viewer is opened occasionally, not every frame), but it reuses the
/// very same per-message renderer + tool-line builder, so the output is identical
/// to the main chat. System messages are skipped; hidden harness tool nudges
/// leave no trace.
pub(super) fn assemble_messages(
    messages: &[crate::dto::chat::ChatMessage],
    palette: &Palette,
    wrap_w: usize,
) -> Vec<Line<'static>> {
    // Which tool calls have COMPLETED: a `tool`-role result message whose
    // `tool_call_id` points back at the call. Built from the same slice so the
    // glyph state matches what the sub-agent actually did.
    let completed: std::collections::HashSet<&str> = messages
        .iter()
        .filter(|m| m.role == Role::Tool)
        .filter_map(|m| m.tool_call_id.as_deref())
        .collect();
    // tool_call_id → result content, so each call can render its own result inline
    // (mirrors the main transcript renderer).
    let tool_results: std::collections::HashMap<&str, &str> = messages
        .iter()
        .filter(|m| m.role == Role::Tool)
        .filter_map(|m| m.tool_call_id.as_deref().map(|id| (id, m.content.as_str())))
        .collect();

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut first = true;
    for msg in messages {
        let block = render_message_block(msg, palette, wrap_w);
        let has_body = !block.is_empty();
        let tool_lines = render_tool_lines(msg, &completed, has_body, palette, wrap_w, &tool_results);
        // Empty block with no tool lines (system / hidden harness) → no trace.
        if block.is_empty() && tool_lines.is_empty() {
            continue;
        }
        if !first {
            lines.push(Line::from(""));
        }
        first = false;
        lines.extend(block);
        lines.extend(tool_lines);
    }
    lines
}
