//! Pure utility functions shared across the chat view submodules.

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use ratatui::layout::Rect;
use crate::view::theme::Palette;

/// The blockquote bar drawn to the LEFT of every thinking/reasoning line, so the
/// gray-italic "thinking" reads as quoted text distinct from the answer. A single
/// dim vertical bar (U+258F) + a space — honours the minimalist top-down border
/// style (one bar, never a box). The answer + tool lines get no bar.
pub(super) const THINK_BAR: &str = "▏ ";

/// Truncate `s` to at most `max` characters (not bytes), appending `…` when it
/// was cut. Used to keep tool-call / tool-result preview lines on one row.
pub(super) fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}

/// Render the `/compact` waiting animation into `area` (the input box interior):
/// a cycling braille spinner + "Compacting conversation… ({elapsed}s)" on the
/// first row, and an indeterminate progress bar (a block sweeping across a hatch
/// track) on the second row when there's height for it. Driven purely by
/// `start.elapsed()` so it advances every redraw without any stored counter.
pub(super) fn render_compact_anim(frame: &mut Frame, area: Rect, start: std::time::Instant, palette: &Palette) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let elapsed = start.elapsed();
    let secs = elapsed.as_secs();
    // ~12.5 fps spinner cadence (80ms/frame) — smooth but not frantic.
    let frame_idx = (elapsed.as_millis() / 80) as usize;
    let glyph = SPINNER[frame_idx % SPINNER.len()];

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(format!("{glyph} "), Style::default().fg(palette.accent)),
        Span::styled(
            format!("Compacting conversation… ({secs}s)"),
            Style::default().fg(palette.dim),
        ),
    ]));

    // Indeterminate bar: a short solid block bounces across a hatched track. The
    // position ping-pongs over the free span so it never just wraps/jumps.
    if area.height >= 2 {
        let track = (area.width as usize).max(1);
        let block_w = 6usize.min(track);
        let span = track.saturating_sub(block_w);
        let pos = if span == 0 {
            0
        } else {
            // Advance one cell per ~60ms, ping-ponging over [0, span].
            let step = (elapsed.as_millis() / 60) as usize % (span * 2);
            if step <= span { step } else { span * 2 - step }
        };
        let mut spans: Vec<Span> = Vec::with_capacity(3);
        if pos > 0 {
            spans.push(Span::styled("░".repeat(pos), Style::default().fg(palette.dim)));
        }
        spans.push(Span::styled(
            "▓".repeat(block_w),
            Style::default().fg(palette.accent),
        ));
        let trailing = track - pos - block_w;
        if trailing > 0 {
            spans.push(Span::styled(
                "░".repeat(trailing),
                Style::default().fg(palette.dim),
            ));
        }
        lines.push(Line::from(spans));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

/// Build the animated "comet" spans for the status label while the app is
/// WORKING (waiting on the model / a tool / the fold).
///
/// A single bright accent **head** glides left→right across `text`, dragging a
/// short two-char tail behind it (the char just behind the head in `palette.fg`,
/// the one before that in `palette.dim`), over an otherwise-`dim` word. After the
/// head reaches the end it spends a `GAP`-cell pause off the right edge, during
/// which the whole word renders dim — the "breath" before the next sweep.
///
/// Time-driven only (no stored counter): the head position is derived from
/// `elapsed_ms`, so it advances on every redraw and the caller just needs to keep
/// repainting (the event loop forces ~12fps ticks while working). One span per
/// char keeps the colour mapping trivial and multibyte-safe (operates on
/// `chars()`); an empty `text` yields no spans (guarded `n == 0`).
pub(super) fn comet_spans(text: &str, elapsed_ms: u128, palette: &Palette) -> Vec<Span<'static>> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    if n == 0 {
        return Vec::new();
    }
    const FRAME_MS: u128 = 80; // ~12.5 fps advance cadence (matches the compact spinner)
    const GAP: usize = 4; // dark pause length after the comet exits the right edge
    // The head sweeps 0..n+GAP; once head >= n it sits in the gap (whole word dim).
    let period = n + GAP;
    let head = ((elapsed_ms / FRAME_MS) as usize) % period.max(1);
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(n);
    for (i, ch) in chars.iter().enumerate() {
        // head = bright bold accent; head-1 = fg tail (one char behind); everything
        // else (incl. head-2, which would be `dim` and so collapses with the rest)
        // is dim. Checked against `head` via `i + 1 == head` so the comparison never
        // underflows when head is small.
        let style = if i == head {
            Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)
        } else if i + 1 == head {
            Style::default().fg(palette.fg)
        } else {
            Style::default().fg(palette.dim)
        };
        spans.push(Span::styled(ch.to_string(), style));
    }
    spans
}

/// Compact token count: raw below 10k, else "10,1k" / "1,1m" (one decimal,
/// comma as the decimal mark, k=thousand m=million).
pub(super) fn fmt_count(n: u64) -> String {
    if n < 10_000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{:.1}k", n as f64 / 1_000.0).replace('.', ",")
    } else {
        format!("{:.1}m", n as f64 / 1_000_000.0).replace('.', ",")
    }
}

/// Split an assistant message into (thinking, response).
///
/// `thinking` = the prefix up to and INCLUDING the last line that starts with a
/// wanderer-word lead-in (`Word:` where `Word` is in the wanderer corpus,
/// case-insensitive). `response` = the remainder (leading blank lines trimmed).
/// Returns `(None, full)` when no wanderer-led line exists (normal message).
///
/// Only lines whose FIRST colon-delimited token is a wanderer word count; a
/// wanderer word appearing mid-sentence (no leading `Word:` pattern) is ignored.
pub(crate) fn split_thinking(content: &str) -> (Option<&str>, &str) {
    let corpus = crate::resources::wanderer_words();
    // Walk lines recording byte offsets. For each line, check whether the token
    // before the first ':' (trimmed, lowercased) is in the wanderer corpus.
    // Track the byte offset just past the last matching line's trailing '\n' (or
    // end of string when the matching line is the final line).
    let mut last_end: Option<usize> = None;
    let mut offset: usize = 0;
    for line in content.split('\n') {
        // `line` does not include the '\n'; `line_end` is the byte offset of the
        // character after the '\n' (or end of string for the final segment).
        let line_end = offset + line.len();
        // Check whether this line has a wanderer lead-in.
        let trimmed = line.trim();
        if let Some(colon_pos) = trimmed.find(':') {
            let token = trimmed[..colon_pos].trim().to_lowercase();
            if corpus.iter().any(|w| w == &token) {
                // Include the '\n' if present; clamp to content length.
                last_end = Some((line_end + 1).min(content.len()));
            }
        }
        // Advance past the '\n' separator (the split consumes it but we account
        // for it in the offset so our byte positions stay aligned with `content`).
        offset = line_end + 1;
    }
    match last_end {
        Some(e) => {
            let thinking = &content[..e];
            // Trim only leading newlines from the response so internal structure
            // of the response body is preserved.
            let response = content[e..].trim_start_matches('\n');
            (Some(thinking), response)
        }
        None => (None, content),
    }
}

/// Render one logical line of THINKING text into barred visual lines.
///
/// Wraps `text` to `wrap_w` MINUS the bar width, then prefixes each wrapped
/// visual line with a dim `THINK_BAR`, so the quote bar survives wrapping (it
/// appears on every wrapped line, not just the first). `text` styling (dim +
/// italic) is applied to the content spans; the bar is dim. A blank input line
/// yields a single bar-only row so paragraph breaks inside the block keep the
/// quote rail unbroken. The result is pushed as logical lines into `out` (the
/// caller's accumulator), where they are later passed through `render_block`
/// unwrapped (already exact-width).
pub(super) fn push_thinking_line(
    out: &mut Vec<Vec<Span<'static>>>,
    text: &str,
    style: Style,
    bar_style: Style,
    wrap_w: usize,
) {
    let bar = Span::styled(THINK_BAR, bar_style);
    // The bar eats 2 columns; wrap the text to the remainder so bar+text stays
    // within wrap_w. Floor at 1 so a pathologically narrow pane can't wrap to 0.
    let inner_w = wrap_w.saturating_sub(THINK_BAR.chars().count()).max(1);
    if text.trim().is_empty() {
        // Blank line inside the thinking block: keep the rail with a bar-only row.
        out.push(vec![bar]);
        return;
    }
    let spans = vec![Span::styled(text.to_string(), style)];
    for visual in crate::view::markdown::wrap_spans(&spans, inner_w) {
        let mut line = Vec::with_capacity(visual.len() + 1);
        line.push(bar.clone());
        line.extend(visual);
        out.push(line);
    }
}

/// One message's visual lines: bullet on the first line, 2-col indent on the
/// rest. `wrap` = wrap each logical line with `markdown::wrap_spans` (plain text
/// / user / streaming); pre-wrapped markdown passes its lines through unwrapped.
///
/// Returns the block in isolation — NO blank separator and NO `first` handling;
/// the caller stitches blocks together with blank lines. `bullet_color` styles
/// only the bullet; the wrapped/pre-wrapped spans keep their own styles. The
/// emitted line count equals the exact on-screen line count the follow-scroll
/// math relies on.
pub(super) fn render_block(
    logical: Vec<Vec<Span<'static>>>,
    bullet: &str,
    bullet_color: ratatui::style::Color,
    wrap_w: usize,
    wrap: bool,
) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut first_visual = true;
    for logical_line in logical {
        let visuals: Vec<Vec<Span<'static>>> = if wrap {
            crate::view::markdown::wrap_spans(&logical_line, wrap_w)
        } else {
            vec![logical_line]
        };
        for visual in visuals {
            // First visual line of the whole block gets the bullet; the rest get
            // a 2-col indent so wrapped/continuation/boxed content hangs under it.
            let prefix = if first_visual {
                Span::styled(bullet.to_string(), Style::default().fg(bullet_color))
            } else {
                Span::raw("  ")
            };
            first_visual = false;
            let mut spans = Vec::with_capacity(visual.len() + 1);
            spans.push(prefix);
            spans.extend(visual);
            out.push(Line::from(spans));
        }
    }
    out
}

/// Render a completed tool RESULT as a rounded, light-dotted box.
///
/// Layout: a `╭┄ {label} ┄…┄╮` top border (label in the accent colour, dotted
/// edges dim), up to five ITALIC+DIM content rows, a literal `...` overflow row
/// when the result has MORE than five lines, and a `╰┄…┄╯` bottom border. Every
/// row is indented four columns so the box hangs under its tool-call line; the box
/// body is `wrap_w - 2` columns wide, making every emitted line exactly the
/// transcript body width (`wrap_w + 2`) and never more.
///
/// Each source line is TRUNCATED (never wrapped) to a single visual row via
/// [`truncate_chars`], so the emitted [`Line`] count is deterministic — one Line
/// per box row — which the transcript's follow-scroll math depends on. Callers box
/// only output-producing tools with non-empty content; terse/empty results keep
/// the compact single-line rendering instead.
pub(super) fn render_tool_box(
    content: &str,
    label: &str,
    palette: &Palette,
    wrap_w: usize,
) -> Vec<Line<'static>> {
    // Box body width: a fixed 2-col indent + `bw` == body width, so the box never
    // overruns the pane. Inner text width drops the 4 cols of chrome (`┆ ` on the left
    // + ` ┆` on the right). All arithmetic saturates so a pathologically narrow pane
    // can't underflow/panic.
    let bw = wrap_w;
    let iw = bw.saturating_sub(4);

    let dim = Style::default().fg(palette.dim);
    let dim_italic = Style::default().fg(palette.dim).add_modifier(Modifier::ITALIC);
    let accent = Style::default().fg(palette.accent);
    // Shared 2-col indent on EVERY row so the box aligns under the ✓ glyph.
    // `render_block` can't be reused here: it indents the first row 4 cols and the
    // rest 2, which would stagger the box edges.
    let indent = "  ";

    // One box content row: `┆ ` + `text` truncated+padded to `iw` (italic+dim) +
    // ` ┆`. `truncate_chars` appends `…` when it cuts, so the source is capped at
    // `iw - 1` to leave a column for it, then the string is padded back out to
    // EXACTLY `iw` for a flush right edge (mirrors `code_row`). The extra `.take(iw)`
    // is a belt-and-suspenders clamp (also handles `iw == 0`).
    let content_row = move |text: &str| -> Line<'static> {
        let mut s: String = if text.chars().count() > iw {
            truncate_chars(text, iw.saturating_sub(1)).chars().take(iw).collect()
        } else {
            text.to_string()
        };
        let pad = iw.saturating_sub(s.chars().count());
        if pad > 0 {
            s.push_str(&" ".repeat(pad));
        }
        Line::from(vec![
            Span::raw(indent),
            Span::styled("┆ ", dim),
            Span::styled(s, dim_italic),
            Span::styled(" ┆", dim),
        ])
    };

    let mut out: Vec<Line<'static>> = Vec::new();

    // --- top border: ╭┄ {label} ┄…┄╮ (label accent, dotted edges dim) ---
    let label_txt = format!(" {label} ");
    // Clamp the label so `╭┄` + label + `╮` can never exceed the box width `bw`
    // on a narrow pane (otherwise the top border overruns the transcript body).
    let label_txt: String = label_txt.chars().take(bw.saturating_sub(3)).collect();
    // Columns before the fill run: `╭┄` (2) + label + `╮` (1); saturating so a tiny
    // pane just drops the fill run rather than underflowing.
    let used = 2 + label_txt.chars().count() + 1;
    let fill = bw.saturating_sub(used);
    out.push(Line::from(vec![
        Span::raw(indent),
        Span::styled("╭┄", dim),
        Span::styled(label_txt, accent),
        Span::styled(format!("{}╮", "┄".repeat(fill)), dim),
    ]));

    // --- content rows: at most 5 real lines; a `...` row ONLY when there are more ---
    let lines: Vec<&str> = content.lines().collect();
    for &line in lines.iter().take(5) {
        out.push(content_row(line));
    }
    if lines.len() > 5 {
        out.push(content_row("..."));
    }

    // --- bottom border: ╰┄…┄╯ ---
    out.push(Line::from(vec![
        Span::raw(indent),
        Span::styled(format!("╰{}╯", "┄".repeat(bw.saturating_sub(2))), dim),
    ]));

    out
}
