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
    let band = Style::default().bg(palette.panel);
    let rail = Style::default().fg(palette.accent).bg(palette.panel);
    let text = Style::default().fg(palette.accent).bg(palette.panel);
    let dim_quote = Style::default()
        .fg(palette.accent)
        .bg(palette.panel)
        .add_modifier(Modifier::DIM);
    let full_w = wrap_w + 2;
    // Text sits after the 1-col rail AND a 1-col gap, so it wraps to `full_w - 2`.
    let inner = full_w.saturating_sub(2).max(1);

    let mut out: Vec<Line<'static>> = Vec::new();
    out.push(band_row(&rail, &band, full_w, Vec::new()));

    // Walk content, expanding paste fences inline so quote body lines can keep a
    // `│` prefix on EVERY visual wrap row (plain string collapse loses the rail
    // when soft-wrap continues past the first segment).
    let mut cursor = 0usize;
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        crate::re_util::static_re(
            r#"(?s)<<<pasted_text n=(\d+) path="([^"]*)">>>(.*?)<<<end_pasted_text n=\d+>>>"#,
        )
    });
    for caps in re.captures_iter(content) {
        let Some(m) = caps.get(0) else {
            continue;
        };
        // Plain text before this fence.
        if m.start() > cursor {
            push_plain_band_lines(
                &mut out,
                &content[cursor..m.start()],
                &rail,
                &band,
                &text,
                full_w,
                inner,
            );
        }
        let n = caps.get(1).map(|c| c.as_str()).unwrap_or("?");
        let body = caps
            .get(3)
            .map(|c| c.as_str().trim_matches('\n'))
            .unwrap_or("");
        push_paste_quote_band_lines(
            &mut out,
            PasteQuoteDraw {
                n,
                body,
                rail: &rail,
                band: &band,
                text: &text,
                dim_quote: &dim_quote,
                full_w,
                inner,
            },
        );
        cursor = m.end();
    }
    if cursor < content.len() {
        push_plain_band_lines(
            &mut out,
            &content[cursor..],
            &rail,
            &band,
            &text,
            full_w,
            inner,
        );
    } else if cursor == 0 {
        // No fences at all — whole body is plain.
        push_plain_band_lines(&mut out, content, &rail, &band, &text, full_w, inner);
    }

    out.push(band_row(&rail, &band, full_w, Vec::new()));
    out
}

/// Max preview lines shown inside a collapsed paste quote in the transcript.
const PASTE_QUOTE_MAX_LINES: usize = 4;

fn push_plain_band_lines(
    out: &mut Vec<Line<'static>>,
    content: &str,
    rail: &Style,
    band: &Style,
    text: &Style,
    full_w: usize,
    inner: usize,
) {
    if content.is_empty() {
        return;
    }
    for logical in content.split('\n') {
        let wrapped = crate::view::markdown::wrap_spans_preserve(
            &[Span::styled(logical.to_string(), *text)],
            inner,
        );
        for visual in wrapped {
            let line_text: String = visual.iter().map(|s| s.content.as_ref()).collect();
            out.push(band_row(
                rail,
                band,
                full_w,
                vec![Span::styled(line_text, *text)],
            ));
        }
    }
}

/// Bundle of styles + geometry for paste-quote band rows (keeps the helper
/// under clippy's too-many-arguments limit).
struct PasteQuoteDraw<'a> {
    n: &'a str,
    body: &'a str,
    rail: &'a Style,
    band: &'a Style,
    text: &'a Style,
    dim_quote: &'a Style,
    full_w: usize,
    inner: usize,
}

/// Paste quote: chip label, then ≤4 body lines. Each body logical line is wrapped
/// with a dedicated width so **every** visual row is prefixed with `│ ` — no bleed
/// when the body soft-wraps past the first segment.
fn push_paste_quote_band_lines(out: &mut Vec<Line<'static>>, d: PasteQuoteDraw<'_>) {
    let PasteQuoteDraw {
        n,
        body,
        rail,
        band,
        text,
        dim_quote,
        full_w,
        inner,
    } = d;
    // Label row — preserve spaces in chip label path (usually no WS issue).
    let label = format!("[Pasted Text #{n}]");
    let wrapped =
        crate::view::markdown::wrap_spans_preserve(&[Span::styled(label, *text)], inner);
    for visual in wrapped {
        let line_text: String = visual.iter().map(|s| s.content.as_ref()).collect();
        out.push(band_row(
            rail,
            band,
            full_w,
            vec![Span::styled(line_text, *text)],
        ));
    }

    const PREFIX: &str = "│ ";
    let prefix_cols = 2usize; // │ + space
    let body_inner = inner.saturating_sub(prefix_cols).max(1);

    let mut lines: Vec<&str> = body.lines().collect();
    let truncated = lines.len() > PASTE_QUOTE_MAX_LINES;
    if truncated {
        lines.truncate(PASTE_QUOTE_MAX_LINES);
    }
    for logical in lines {
        // Preserve indent/spaces inside paste quote previews (composer parity).
        let wrapped = crate::view::markdown::wrap_spans_preserve(
            &[Span::styled(logical.to_string(), *dim_quote)],
            body_inner,
        );
        for visual in wrapped {
            let line_text: String = visual.iter().map(|s| s.content.as_ref()).collect();
            out.push(band_row(
                rail,
                band,
                full_w,
                vec![
                    Span::styled(PREFIX.to_string(), *dim_quote),
                    Span::styled(line_text, *dim_quote),
                ],
            ));
        }
    }
    if truncated {
        out.push(band_row(
            rail,
            band,
            full_w,
            vec![Span::styled(format!("{PREFIX}…"), *dim_quote)],
        ));
    }
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
