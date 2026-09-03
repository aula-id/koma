//! Transcript area: committed messages, live streaming buffer, sub-agent
//! inline indicator, and the follow-scroll logic.

use super::blocks::{
    render_attachment_card, render_bash_nudge_block, render_shell_block, render_user_message,
};
use super::helpers::{
    push_thinking_viewport, render_block, render_tool_box, split_thinking, truncate_chars, THINK_BAR,
};
use crate::app::state::AppStateRest;
use crate::dto::chat::Role;
use crate::view::theme::Palette;
use ratatui::{
    layout::{Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
// `tool_box_label` / `format_tool_signature` now live in the sibling `tool_format`
// module (file size); re-exported here so the existing
// `crate::view::chat::transcript::{tool_box_label, format_tool_signature}` call
// sites (the GUI push-projection in `app::runtime::client::render`) keep
// resolving unchanged.
pub(crate) use super::tool_format::{format_tool_signature, tool_box_label};

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
    let body = body_chunk.inner(Margin {
        horizontal: 2,
        vertical: 0,
    });
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
            cache
                .blocks
                .push(render_message_block(msg, palette, wrap_w));
        }

        // Assemble the frame: cached blocks (with blank separators) + the live
        // streaming line (rendered fresh — it changes every token). `cache.blocks`
        // is index-aligned with `committed` (one block per non-system message), so
        // we zip them: the block carries the cached body, and for an Assistant turn
        // the tool-call lines are appended fresh here (glued to the same block, no
        // separator) with a live ⚙/✓ glyph from `completed_tool_ids`.
        // Pre-extract the session path so we can resolve attachment image files.
        let session_path = rest.fg().session.as_ref().map(|s| s.path.clone());

        let mut lines: Vec<Line<'static>> = Vec::new();
        let mut first = true;
        for (i, block) in cache.blocks.iter().enumerate() {
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

            if block.is_empty() && tool_lines.is_empty() {
                continue;
            }
            if !first {
                lines.push(Line::from(""));
            }
            first = false;
            lines.extend(block.iter().cloned());
            lines.extend(tool_lines);

            // Inline image rendering: when a user message carries image
            // attachments, render each image as half-block art right in the
            // transcript instead of just the text card.
            if let Some(msg) = committed.get(i) {
                if msg.role == Role::User && !msg.attachments.is_empty() {
                    if let Some(ref sess_path) = session_path {
                        for att in &msg.attachments {
                            let img_path = sess_path.join(&att.rel_path);
                            if img_path.exists() {
                                let max_w = body.width.min(80);
                                if let Ok(img_lines) =
                                    crate::view::image_render::ImageRenderer::render_to_lines(
                                        &img_path, max_w,
                                    )
                                {
                                    for line in &img_lines {
                                        lines.push(line.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
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
            // Partial reasoning first, dim+italic, barred — fixed-height tail viewport
            // so long CoT doesn't grow the transcript forever (full buffer still
            // streams/stores). Pre-wrapped; render_block passes them through.
            if !partial_reasoning.is_empty() {
                push_thinking_viewport(
                    &mut logical,
                    partial_reasoning,
                    thinking_style,
                    bar_style,
                    wrap_w,
                );
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
                // Decode any escaped reasoning tag echoed mid-stream so it doesn't
                // flash as `&lt;think&gt;` before finalize. Display-only, on the
                // partial buffer; committed text already reads real from storage.
                let stripped = crate::dto::chat::unescape_reasoning_tags(&stripped).into_owned();
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
            .filter(|s| {
                matches!(s.status, crate::app::subagent::SubAgentStatus::Running) && !s.detached
            })
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
        let top = if rest.fg().follow {
            max_scroll
        } else {
            rest.fg().scroll.min(max_scroll)
        };
        let messages = Paragraph::new(lines).scroll((top, 0));
        frame.render_widget(messages, body);
    } // cache borrow ends
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
            // Extension-prompt injection (grant broker `chat.prompt`): same compact
            // dim render as the bg-bash nudge — line 1 only (the first buffered
            // `[ext:<id>] <text>`), NOT a `★` user turn. The full multi-prompt body +
            // trailer is model-only context (stripped on the wire).
            if let Some(body) = msg.content.strip_prefix(crate::dto::chat::EXT_PROMPT_MARK) {
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
            lines.extend(render_attachment_card(&msg.attachments, palette));
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
            // Native reasoning + legacy wanderer "thinking" peel: one combined body
            // through the fixed-height tail viewport. Display-only — never
            // re-enters the conversation or disk; stream/storage stay full.
            {
                let mut thinking_src = String::new();
                if let Some(reasoning) = msg.reasoning.as_deref() {
                    if !reasoning.is_empty() {
                        thinking_src.push_str(reasoning);
                    }
                }
                if let Some(thinking) = thinking_block {
                    if !thinking_src.is_empty() && !thinking_src.ends_with('\n') {
                        thinking_src.push('\n');
                    }
                    thinking_src.push_str(thinking);
                }
                if !thinking_src.is_empty() {
                    push_thinking_viewport(
                        &mut logical,
                        &thinking_src,
                        thinking_style,
                        bar_style,
                        wrap_w,
                    );
                }
            }
            // Blank line between the (barred) thinking block and the answer so the
            // quote→answer transition is clear. Only when there IS both.
            if !logical.is_empty() && !response_body.is_empty() {
                logical.push(vec![]);
            }
            if !response_body.is_empty() {
                logical.extend(crate::view::markdown::render(
                    response_body,
                    palette,
                    wrap_w,
                ));
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
        if call.function.name == "plan_ready" || call.function.name == "mission_ready" {
            if let Some(digest) =
                serde_json::from_str::<serde_json::Value>(&call.function.arguments)
                    .ok()
                    .and_then(|v| {
                        v.get("highlights")
                            .and_then(|s| s.as_str())
                            .map(str::to_string)
                    })
            {
                // Header: "⚙/✓ plan ready" — the glyph flips to ✓ once the user
                // decides and the plan_ready tool result lands.
                let ready_label = if call.function.name == "mission_ready" {
                    "mission ready"
                } else {
                    "plan ready"
                };
                lines.push(Line::from(vec![
                    prefix,
                    Span::styled(glyph, glyph_style),
                    Span::styled(ready_label, Style::default().fg(palette.dim)),
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
                // For show_image: render the actual image as half-block art inline
                // under the call, then the terse status text below it.
                if call.function.name == "show_image" {
                    // Result format: "image resolved: {path} ({label})\nRendered inline..."
                    if let Some(img_path) = result
                        .strip_prefix("image resolved: ")
                        .and_then(|r| r.split_once('\n').map(|(p, _)| p))
                        .and_then(|p| p.rsplit_once(" (").map(|(path, _)| path))
                    {
                        let p = std::path::Path::new(img_path);
                        if p.exists() {
                            let max_w = (wrap_w as u16).min(80);
                            if let Ok(img_lines) =
                                crate::view::image_render::ImageRenderer::render_to_lines(p, max_w)
                            {
                                lines.extend(img_lines);
                            }
                        }
                    }
                    // Still show the terse status line below the image.
                    let first = truncate_chars(result.lines().next().unwrap_or(""), 80);
                    lines.extend(render_block(
                        vec![vec![Span::styled(first, Style::default().fg(palette.dim))]],
                        "    ",
                        palette.dim,
                        wrap_w,
                        false,
                    ));
                } else {
                    lines.extend(render_tool_result(
                        result,
                        &call.function.name,
                        palette,
                        wrap_w,
                    ));
                }
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
        let tool_lines =
            render_tool_lines(msg, &completed, has_body, palette, wrap_w, &tool_results);
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
