//! Per-kind message block renderers (user band, `!`-shell entries, bg-bash
//! nudges, image-attachment cards) used by [`super::transcript`]'s per-message
//! renderer. Split out of `transcript` for file size; `pub(super)` (bumped
//! from private) so the sibling `transcript` module can still call them — no
//! other behaviour change and no external call sites.

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::view::theme::Palette;

use super::helpers::render_block;

/// Render a user message body for the transcript. Paste machine fences are
/// collapsed to a short quote block (label + ≤4 lines of body) so huge dumps
/// don't flood the chat view; full body stays on disk / in the editor.
pub(super) fn render_user_message(
    content: &str,
    palette: &Palette,
    wrap_w: usize,
) -> Vec<Line<'static>> {
    let display = collapse_paste_fences_for_display(content);
    render_user_message_raw(&display, palette, wrap_w)
}

/// Max preview lines shown inside a collapsed paste quote in the transcript.
const PASTE_QUOTE_MAX_LINES: usize = 4;

/// Replace `<<<pasted_text…>>>…<<<end…>>>` fences with a compact quote-ish block.
fn collapse_paste_fences_for_display(text: &str) -> String {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        crate::re_util::static_re(
            r#"(?s)<<<pasted_text n=(\d+) path="([^"]*)">>>(.*?)<<<end_pasted_text n=\d+>>>"#,
        )
    });
    re.replace_all(text, |caps: &regex::Captures| {
        let n = &caps[1];
        let path = &caps[2];
        let body = caps[3].trim_matches('\n');
        let mut lines: Vec<&str> = body.lines().collect();
        let truncated = lines.len() > PASTE_QUOTE_MAX_LINES;
        if truncated {
            lines.truncate(PASTE_QUOTE_MAX_LINES);
        }
        let mut out = format!("[Pasted Text #{n}] ({path})");
        for line in lines {
            out.push('\n');
            out.push_str("  │ ");
            out.push_str(line);
        }
        if truncated {
            out.push_str("\n  │ …");
        }
        out
    })
    .into_owned()
}

fn render_user_message_raw(
    content: &str,
    palette: &Palette,
    wrap_w: usize,
) -> Vec<Line<'static>> {
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
            out.push(band_row(
                &rail,
                &band,
                full_w,
                vec![Span::styled(line_text, text)],
            ));
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
pub(super) fn band_row(
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
pub(super) fn render_shell_block(
    body: &str,
    palette: &Palette,
    wrap_w: usize,
) -> Vec<Line<'static>> {
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

/// Render a background-bash completion nudge as a single compact line: a
/// `palette.success` `✓` glyph followed by the dim per-job summary (line 1 of
/// `body`). The remaining lines of `body` are model-only context and are NOT
/// displayed. Styled like a tool-call sub-line (2-col indent + dim text), not
/// a `★` user turn.
pub(super) fn render_bash_nudge_block(body: &str, palette: &Palette) -> Vec<Line<'static>> {
    let summary = body.lines().next().unwrap_or("").to_string();
    vec![Line::from(vec![
        Span::raw("  "),
        Span::styled("\u{2713} ", Style::default().fg(palette.success)),
        Span::styled(summary, Style::default().fg(palette.dim)),
    ])]
}

/// Render the warn-coloured attachment folder-tree lines for a user message
/// that carries attachments (images and/or pasted text). Minimalist design: an
/// "attachments" root line, then one tree branch per attachment (├─ for non-last,
/// └─ for the last). Returns an empty `Vec` when there are no attachments.
///
/// Uses `palette.warn`, matching the approval card in overlays.rs, so it
/// always reads as a warn cue.
pub(super) fn render_attachment_card(
    attachments: &[crate::dto::chat::Attachment],
    palette: &Palette,
) -> Vec<Line<'static>> {
    if attachments.is_empty() {
        return Vec::new();
    }
    let style = Style::default().fg(palette.warn);
    let dim = Style::default()
        .fg(palette.warn)
        .add_modifier(Modifier::DIM);
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Root: "  attachments"
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("attachments", style),
    ]));

    // One line per attachment, using tree connectors.
    let last_idx = attachments.len().saturating_sub(1);
    for (i, att) in attachments.iter().enumerate() {
        let connector = if i == last_idx {
            Span::styled("\u{2514}\u{2500} ", dim) // └─
        } else {
            Span::styled("\u{251C}\u{2500} ", dim) // ├─
        };
        let label = if att.is_pasted_text() {
            format!("[Pasted Text #{}] {}", att.marker_n, att.file_name())
        } else {
            format!("[Image #{}] {}", att.marker_n, att.file_name())
        };
        lines.push(Line::from(vec![
            Span::raw("  "),
            connector,
            Span::styled(label, style),
        ]));
    }
    lines
}
