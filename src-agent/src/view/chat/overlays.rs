//! Floating overlay widgets: slash-command palette, file-reference palette,
//! toast notification, and tool-approval prompt.
//!
//! (The `/help` reference is no longer an overlay — it is a dedicated
//! full-screen mode; see [`crate::view::help`].)

use super::helpers::truncate_chars;
use crate::app::state::AppStateRest;
use crate::controller::command;
use crate::view::theme::Palette;
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Padding, Paragraph},
    Frame,
};

/// Render the slash-command palette if the current input starts with `/`.
///
/// Floats above the input while typing a `/` command name. Cleared background
/// so it overlays the transcript.
///
/// Returns `true` when the palette is visible (so callers can suppress other
/// overlays that share the same space).
pub(super) fn render_command_palette(
    frame: &mut Frame,
    input_chunk: Rect,
    transcript_chunk: Rect,
    rest: &AppStateRest,
    palette: &Palette,
) -> bool {
    let cmd_matches = command::palette_matches(&rest.fg().input);
    if cmd_matches.is_empty() {
        return false;
    }
    const MAX_VIS: usize = 7;
    let sel = rest.palette_sel.min(cmd_matches.len() - 1);
    // Scrolloff window: the selection walks within the visible rows and only
    // scrolls at the edges (persisted offset), instead of pinning to the bottom.
    let (start, end) =
        crate::view::scroll::scroll_window(&rest.palette_offset, sel, cmd_matches.len(), MAX_VIS);
    let rows: Vec<Line> = cmd_matches[start..end]
        .iter()
        .enumerate()
        .map(|(vi, (name, desc))| {
            let i = start + vi;
            if i == sel {
                let hl = Style::default().fg(palette.sel_fg).bg(palette.sel_bg);
                Line::from(vec![
                    Span::styled(format!(" {name}  "), hl),
                    Span::styled(format!("{desc} "), hl),
                ])
            } else {
                Line::from(vec![
                    Span::styled(format!(" {name}  "), Style::default().fg(palette.accent)),
                    Span::styled(*desc, Style::default().fg(palette.dim)),
                ])
            }
        })
        .collect();

    // Size the box to the visible window (+2 for borders), clamped to the space
    // between the header rule and the input, and anchor it just above input.
    let avail = input_chunk.y.saturating_sub(transcript_chunk.y);
    let h = ((rows.len() as u16) + 2).min(avail.max(3));
    let y = input_chunk.y.saturating_sub(h);
    let popup = Rect {
        x: input_chunk.x,
        y,
        width: input_chunk.width,
        height: h,
    };

    let block = Block::bordered()
        .border_style(Style::default().fg(palette.dim))
        .title(Span::styled(" commands ", Style::default().fg(palette.dim)))
        .padding(Padding::horizontal(1));
    let inner = block.inner(popup);
    crate::view::clear_and_fill(frame, popup, palette.bg);
    frame.render_widget(block, popup);
    frame.render_widget(Paragraph::new(rows), inner);
    true
}

/// Render the `@`-triggered file-reference palette if appropriate.
///
/// Only shown when the command palette is NOT active and the current token is
/// `@partial`. Uses `search` so deep files appear on every keystroke.
pub(super) fn render_file_palette(
    frame: &mut Frame,
    input_chunk: Rect,
    transcript_chunk: Rect,
    rest: &AppStateRest,
    palette: &Palette,
) {
    if let Some(partial) = crate::controller::input::file_ref_partial(&rest.fg().input) {
        const MAX_VIS: usize = 10; // visible window height (rows shown at once)
        const FILE_PAL_RESULTS: usize = 200; // result cap — larger than the window so the list scrolls
                                             // Prefer the daemon-projected palette when present (the thin attach client,
                                             // whose reconstructed `dir_cache` is empty — it would otherwise show no rows).
                                             // `None` on a local TUI: compute the matches live from the session index. Both
                                             // return up to FILE_PAL_RESULTS matches; the window (MAX_VIS) scrolls through them.
        let files: Vec<String> = match &rest.file_palette {
            Some(projected) => projected.clone(),
            None => rest
                .fg()
                .dir_cache
                .read()
                .map(|c| c.search(partial, FILE_PAL_RESULTS))
                .unwrap_or_default(),
        };
        if !files.is_empty() {
            let sel = rest.palette_sel.min(files.len() - 1);
            // Scrolloff window: selection walks within the visible rows; the
            // window only scrolls at the edges (persisted offset).
            let (start, end) =
                crate::view::scroll::scroll_window(&rest.palette_offset, sel, files.len(), MAX_VIS);
            let rows: Vec<Line> = files[start..end]
                .iter()
                .enumerate()
                .map(|(vi, f)| {
                    let i = start + vi;
                    if i == sel {
                        let hl = Style::default().fg(palette.sel_fg).bg(palette.sel_bg);
                        Line::from(Span::styled(format!(" {f} "), hl))
                    } else {
                        Line::from(Span::styled(
                            format!(" {f} "),
                            Style::default().fg(palette.fg),
                        ))
                    }
                })
                .collect();
            // title shows position when there are more entries than fit
            let title = if files.len() > MAX_VIS {
                format!(" files {}/{} ", sel + 1, files.len())
            } else {
                " files ".to_string()
            };
            let avail = input_chunk.y.saturating_sub(transcript_chunk.y);
            let h = ((rows.len() as u16) + 2).min(avail.max(3));
            let y = input_chunk.y.saturating_sub(h);
            let popup = Rect {
                x: input_chunk.x,
                y,
                width: input_chunk.width,
                height: h,
            };
            let block = Block::bordered()
                .border_style(Style::default().fg(palette.dim))
                .title(Span::styled(title, Style::default().fg(palette.dim)))
                .padding(Padding::horizontal(1));
            let inner = block.inner(popup);
            crate::view::clear_and_fill(frame, popup, palette.bg);
            frame.render_widget(block, popup);
            frame.render_widget(Paragraph::new(rows), inner);
        }
    }
}

/// Render the transient toast notification pinned to the top of the transcript.
///
/// `Error` is a red box ("error"); `Info` is a neutral accent box ("info") used
/// for notices like the post-compaction summary. Both wrap to width and span
/// multiple lines; the info box is capped taller so a short summary fits.
pub(super) fn render_toast(
    frame: &mut Frame,
    transcript_chunk: Rect,
    rest: &AppStateRest,
    palette: &Palette,
) {
    // Toast is per-session (C6): render the FOREGROUND session's toast.
    if let Some((msg, _, kind)) = rest.fg().toast.as_ref() {
        let (border_color, title, max_rows) = match kind {
            crate::app::state::ToastKind::Error => (palette.error, " error ", 6u16),
            crate::app::state::ToastKind::Info => (palette.accent, " info ", 10u16),
        };
        let tw = transcript_chunk.width;
        let inner_w = (tw as usize).saturating_sub(4).max(1);
        // Wrap each logical line (the summary toast embeds a leading newline so the
        // "compacted ✓" header sits on its own row above the body).
        let rows: Vec<Line> = msg
            .split('\n')
            .flat_map(|logical| {
                crate::view::markdown::wrap_spans(
                    &[Span::styled(
                        logical.to_string(),
                        Style::default().fg(palette.fg),
                    )],
                    inner_w,
                )
            })
            .map(Line::from)
            .collect();
        // Cap the box height: never exceed the transcript, and clamp Info/Error to
        // their own row budget so a long message stays contained.
        let body_rows = (rows.len() as u16).min(max_rows);
        let h = (body_rows + 2).min(transcript_chunk.height.max(3));
        let rect = Rect {
            x: transcript_chunk.x,
            y: transcript_chunk.y,
            width: tw,
            height: h,
        };
        let block = Block::bordered()
            .border_style(Style::default().fg(border_color))
            .title(Span::styled(title, Style::default().fg(border_color)))
            .padding(Padding::horizontal(1));
        let inner = block.inner(rect);
        crate::view::clear_and_fill(frame, rect, palette.bg);
        frame.render_widget(block, rect);
        frame.render_widget(Paragraph::new(rows), inner);
    }
}

/// Height (in rows) of the `plan_ready` approval box: 2 content rows (the
/// question + the y/a/n footer) plus the top/bottom border. Shared with
/// `mod::draw`, which reserves exactly this many rows at the bottom of the
/// transcript while a plan approval is parked so the summary block (rendered as
/// the last transcript content) isn't hidden under this box.
pub(super) const PLAN_APPROVAL_HEIGHT: u16 = 4;

/// Render the tool-approval prompt shown while a risky tool call is paused for
/// the user's y/n (Normal mode).
///
/// A small warning-coloured box anchored just above the input, listing the
/// pending call. Modal: keys are routed to the approve/deny handlers in the
/// input controller, not into the transcript.
pub(super) fn render_tool_approval(
    frame: &mut Frame,
    input_chunk: Rect,
    transcript_chunk: Rect,
    rest: &AppStateRest,
    palette: &Palette,
) {
    let warn = palette.warn;

    // plan_ready is a DISTINCT approval — a finished plan presented for the user's
    // y/a/n decision. The plan SUMMARY is rendered as normal transcript content
    // (see `transcript::render_tool_lines`), so this overlay stays minimal: just
    // the question + the three-way footer.
    let is_plan_ready = rest
        .fg()
        .pending_tool_calls
        .get(rest.fg().tool_idx)
        .map(|c| c.function.name == "plan_ready" || c.function.name == "mission_ready")
        .unwrap_or(false);
    if is_plan_ready {
        let is_mission = rest
            .fg()
            .pending_tool_calls
            .get(rest.fg().tool_idx)
            .map(|c| c.function.name == "mission_ready")
            .unwrap_or(false);
        let (question, title) = if is_mission {
            (" proceed with the mission?", " mission ready ")
        } else {
            (" proceed with the plan?", " plan ready ")
        };
        let rows: Vec<Line> = vec![
            Line::from(Span::styled(
                question,
                Style::default().fg(palette.fg),
            )),
            Line::from(vec![
                Span::styled(" [y] approve   ", Style::default().fg(warn)),
                Span::styled("[a] approve & compact   ", Style::default().fg(warn)),
                Span::styled("[n] chat more", Style::default().fg(palette.dim)),
            ]),
        ];

        let avail = input_chunk.y.saturating_sub(transcript_chunk.y);
        // Fixed height (2 content rows + 2 border) — shared with the transcript's
        // plan-approval reserve in `mod::draw` so the summary block clears this box.
        let h = PLAN_APPROVAL_HEIGHT.min(avail.max(3));
        let y = input_chunk.y.saturating_sub(h);
        let rect = Rect {
            x: input_chunk.x,
            y,
            width: input_chunk.width,
            height: h,
        };
        let block = Block::bordered()
            .border_style(Style::default().fg(warn))
            .title(Span::styled(title, Style::default().fg(warn)))
            .padding(Padding::horizontal(1));
        let inner = block.inner(rect);
        crate::view::clear_and_fill(frame, rect, palette.bg);
        frame.render_widget(block, rect);
        frame.render_widget(Paragraph::new(rows), inner);
        return;
    }

    let mut rows: Vec<Line> = Vec::new();
    if let Some(call) = rest.fg().pending_tool_calls.get(rest.fg().tool_idx) {
        let name = call.function.name.as_str();
        // header: which tool
        rows.push(Line::from(Span::styled(
            format!(" ⚙ {name}"),
            Style::default().fg(warn).add_modifier(Modifier::BOLD),
        )));
        // When the harness (TAC) forced this approval, show its reason so the
        // user knows WHY the call was flagged rather than auto-run.
        if let Some(reason) = rest.fg().approval_reason.as_ref() {
            if !reason.is_empty() {
                rows.push(Line::from(Span::styled(
                    format!(" harness: {reason}"),
                    Style::default().fg(palette.dim),
                )));
            }
        }
        let v: serde_json::Value =
            serde_json::from_str(&call.function.arguments).unwrap_or_default();
        let inner_w = (input_chunk.width as usize)
            .saturating_sub(2 /*borders*/ + 2 /*padding*/ + 3 /*indent*/)
            .max(8);
        match name {
            "write" => {
                let path = v["path"].as_str().unwrap_or("?");
                let content = v["content"].as_str().unwrap_or("");
                let n_lines = content.lines().count().max(1);
                rows.push(Line::from(vec![
                    Span::styled(" target:  ", Style::default().fg(palette.dim)),
                    Span::styled(path.to_string(), Style::default().fg(palette.fg)),
                ]));
                rows.push(Line::from(vec![
                    Span::styled(" payload: ", Style::default().fg(palette.dim)),
                    Span::styled(
                        format!("{n_lines} lines, {} bytes", content.len()),
                        Style::default().fg(palette.fg),
                    ),
                ]));
                // preview up to 8 content lines, each truncated to the box width
                for line in content.lines().take(8) {
                    rows.push(Line::from(Span::styled(
                        format!("   {}", truncate_chars(line, inner_w)),
                        Style::default().fg(palette.dim),
                    )));
                }
                if n_lines > 8 {
                    rows.push(Line::from(Span::styled(
                        format!("   … (+{} more lines)", n_lines - 8),
                        Style::default().fg(palette.dim),
                    )));
                }
            }
            "delete" => {
                let path = v["path"].as_str().unwrap_or("?");
                rows.push(Line::from(vec![
                    Span::styled(" target:  ", Style::default().fg(palette.dim)),
                    Span::styled(path.to_string(), Style::default().fg(palette.fg)),
                ]));
            }
            "edit" => {
                let path = v["path"].as_str().unwrap_or("?");
                let old = v["old"].as_str().unwrap_or("");
                let new = v["new"].as_str().unwrap_or("");
                rows.push(Line::from(vec![
                    Span::styled(" target:  ", Style::default().fg(palette.dim)),
                    Span::styled(path.to_string(), Style::default().fg(palette.fg)),
                ]));
                rows.push(Line::from(vec![
                    Span::styled(" payload: ", Style::default().fg(palette.dim)),
                    Span::styled(
                        format!(
                            "{} → {}",
                            truncate_chars(old, inner_w / 2),
                            truncate_chars(new, inner_w / 2)
                        ),
                        Style::default().fg(palette.fg),
                    ),
                ]));
            }
            "bash" => {
                let cmd = v["command"].as_str().unwrap_or("?");
                rows.push(Line::from(vec![
                    Span::styled(" command: ", Style::default().fg(palette.dim)),
                    Span::styled(
                        truncate_chars(cmd, inner_w),
                        Style::default().fg(palette.fg),
                    ),
                ]));
                // Show additional lines of multi-line commands.
                let lines: Vec<&str> = cmd.lines().collect();
                for line in lines.iter().skip(1).take(6) {
                    rows.push(Line::from(Span::styled(
                        format!("   {}", truncate_chars(line, inner_w)),
                        Style::default().fg(palette.dim),
                    )));
                }
                if lines.len() > 7 {
                    rows.push(Line::from(Span::styled(
                        format!("   … (+{} more lines)", lines.len() - 7),
                        Style::default().fg(palette.dim),
                    )));
                }
            }
            _ => {
                rows.push(Line::from(Span::styled(
                    format!(" {}", truncate_chars(&call.function.arguments, 120)),
                    Style::default().fg(palette.dim),
                )));
            }
        }
    }
    rows.push(Line::from(vec![
        Span::styled(" [y] run   ", Style::default().fg(warn)),
        Span::styled("[n] deny", Style::default().fg(palette.dim)),
    ]));

    // Anchor just above the input, full input width, growing upward —
    // identical placement to the slash-command / help boxes.
    let avail = input_chunk.y.saturating_sub(transcript_chunk.y);
    let h = ((rows.len() as u16) + 2).min(avail.max(3));
    let y = input_chunk.y.saturating_sub(h);
    let rect = Rect {
        x: input_chunk.x,
        y,
        width: input_chunk.width,
        height: h,
    };

    let block = Block::bordered()
        .border_style(Style::default().fg(warn))
        .title(Span::styled(" approve ", Style::default().fg(warn)))
        .padding(Padding::horizontal(1));
    let inner = block.inner(rect);
    crate::view::clear_and_fill(frame, rect, palette.bg);
    frame.render_widget(block, rect);
    frame.render_widget(Paragraph::new(rows), inner);
}
