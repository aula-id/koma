//! Syntect singletons and pure helper functions shared by the renderer.
//!
//! All free functions are `pub(super)` so `render.rs` can call them; `wrap_spans`
//! is additionally `pub(crate)` because the chat view imports it directly.

use std::sync::OnceLock;

use pulldown_cmark::Alignment;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;

// --- syntect singletons (loaded once, on first markdown render with code) ----

static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
static THEME: OnceLock<Theme> = OnceLock::new();

/// Bundled syntax definitions (newline-terminated variants so `LinesWithEndings`
/// feeds `highlight_line` correctly). Loaded once and shared for the process.
pub(super) fn syntaxes() -> &'static SyntaxSet {
    SYNTAXES.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// Bundled dark theme for code highlighting. `base16-ocean.dark` reads well on
/// the app's default dark background; only the foreground is used.
pub(super) fn theme() -> &'static Theme {
    THEME.get_or_init(|| ThemeSet::load_defaults().themes["base16-ocean.dark"].clone())
}

/// Map a `syntect` style to a ratatui [`Style`], keeping only the foreground RGB
/// so the terminal background shows through (code boxes are not filled).
pub(super) fn syntect_fg(s: syntect::highlighting::Style) -> Style {
    Style::default().fg(Color::Rgb(s.foreground.r, s.foreground.g, s.foreground.b))
}

// --- coalesce ----------------------------------------------------------------

/// Coalesce a run of `(char, Style)` into owned spans, merging adjacent chars
/// that share a style. Used to rebuild code-row spans after char-level chunking.
pub(super) fn coalesce(run: &[(char, Style)]) -> Vec<Span<'static>> {
    let mut line: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut cur: Option<Style> = None;
    for &(ch, style) in run {
        match cur {
            Some(s) if s == style => buf.push(ch),
            _ => {
                if let Some(s) = cur {
                    line.push(Span::styled(std::mem::take(&mut buf), s));
                }
                buf.push(ch);
                cur = Some(style);
            }
        }
    }
    if let Some(s) = cur {
        line.push(Span::styled(buf, s));
    }
    line
}

// --- fit_cell ----------------------------------------------------------------

/// Truncate a cell's inline spans to `w` chars (appending `…` when it overflows)
/// and pad to `w` honouring `align`. Header cells are forced bold.
pub(super) fn fit_cell(
    cell: &[Span<'static>],
    w: usize,
    align: Alignment,
    bold: bool,
    _palette: &crate::view::theme::Palette,
) -> Vec<Span<'static>> {
    // Flatten to (char, style), truncate with an ellipsis if needed.
    let mut chars: Vec<(char, Style)> = Vec::new();
    for s in cell {
        let st = if bold {
            s.style.add_modifier(Modifier::BOLD)
        } else {
            s.style
        };
        for ch in s.content.chars() {
            chars.push((ch, st));
        }
    }
    let len = chars.len();
    if len > w {
        if w == 0 {
            chars.clear();
        } else {
            chars.truncate(w.saturating_sub(1));
            let st = chars.last().map(|c| c.1).unwrap_or_default();
            chars.push(('…', st));
        }
    }
    let content_len = chars.len();
    let pad = w.saturating_sub(content_len);
    let (lpad, rpad) = match align {
        Alignment::Right => (pad, 0),
        Alignment::Center => (pad / 2, pad - pad / 2),
        _ => (0, pad),
    };
    let mut out: Vec<Span<'static>> = Vec::new();
    if lpad > 0 {
        out.push(Span::raw(" ".repeat(lpad)));
    }
    out.extend(coalesce(&chars));
    if rpad > 0 {
        out.push(Span::raw(" ".repeat(rpad)));
    }
    out
}

// --- shrink_widths -----------------------------------------------------------

/// Shrink `widths` so their sum equals `target`, taking from the widest columns
/// first and never reducing a column below 1.
pub(super) fn shrink_widths(widths: &mut [usize], target: usize) {
    let ncols = widths.len();
    if ncols == 0 {
        return;
    }
    // Start from a floor of 1 per column; distribute the remaining budget
    // proportionally to each column's natural width.
    let total: usize = widths.iter().sum();
    if total == 0 || target <= ncols {
        widths.fill(1);
        return;
    }
    let spare = target - ncols; // budget above the 1-per-col floor
    let mut scaled: Vec<usize> = widths
        .iter()
        .map(|&w| 1 + (w.saturating_sub(1) * spare) / (total.saturating_sub(ncols)).max(1))
        .collect();
    // Fix rounding drift so the sum lands exactly on `target`.
    let mut sum: usize = scaled.iter().sum();
    while sum > target {
        // Trim the currently-widest column.
        if let Some((idx, _)) = scaled
            .iter()
            .enumerate()
            .filter(|(_, &w)| w > 1)
            .max_by_key(|(_, &w)| w)
        {
            scaled[idx] -= 1;
            sum -= 1;
        } else {
            break;
        }
    }
    while sum < target {
        // Pad the currently-narrowest column up toward its natural width.
        if let Some((idx, _)) = scaled.iter().enumerate().min_by_key(|(_, &w)| w) {
            scaled[idx] += 1;
            sum += 1;
        } else {
            break;
        }
    }
    widths.copy_from_slice(&scaled);
}

// --- wrap_spans --------------------------------------------------------------

/// Word-wrap one logical line of styled `spans` to `width` columns, preserving
/// each span's [`Style`]. Returns visual lines (each a `Vec<Span>`). Counting is
/// in `char`s. Runs of non-whitespace are kept whole when they fit; words longer
/// than `width` are hard-split; `width` is clamped to `>= 1`. Embedded `\n`
/// chars force a hard break (soft/hard markdown breaks arrive as `\n`).
/// Consecutive same-style chars are coalesced back into one owned `Span` per run.
///
/// Shared by the chat view (`render_block`) and this module; `pub(crate)` so
/// there is a single implementation.
pub(crate) fn wrap_spans(spans: &[Span], width: usize) -> Vec<Vec<Span<'static>>> {
    let width = width.max(1);

    // Flatten the styled spans into a single (char, style) sequence so wrapping
    // can break anywhere while remembering each char's style.
    let mut chars: Vec<(char, Style)> = Vec::new();
    for span in spans {
        for ch in span.content.chars() {
            chars.push((ch, span.style));
        }
    }

    let mut out: Vec<Vec<Span<'static>>> = Vec::new();
    let mut line: Vec<(char, Style)> = Vec::new(); // chars committed to current visual line
    let mut word: Vec<(char, Style)> = Vec::new(); // current run of non-whitespace
    let mut line_len = 0usize;

    // Place the buffered `word` onto the current line, wrapping/splitting as
    // needed. A too-long word is hard-split across lines; otherwise it goes on
    // this line if it fits or starts a fresh one.
    let place_word = |out: &mut Vec<Vec<Span<'static>>>,
                      line: &mut Vec<(char, Style)>,
                      line_len: &mut usize,
                      word: &mut Vec<(char, Style)>| {
        if word.is_empty() {
            return;
        }
        let wlen = word.len();
        if wlen > width {
            if *line_len > 0 {
                out.push(coalesce(line));
                line.clear();
                *line_len = 0;
            }
            let mut start = 0usize;
            while word.len() - start > width {
                out.push(coalesce(&word[start..start + width]));
                start += width;
            }
            line.extend_from_slice(&word[start..]);
            *line_len = word.len() - start;
        } else if *line_len == 0 {
            line.extend_from_slice(word);
            *line_len = wlen;
        } else if *line_len + 1 + wlen <= width {
            line.push((' ', Style::default()));
            line.extend_from_slice(word);
            *line_len += 1 + wlen;
        } else {
            out.push(coalesce(line));
            line.clear();
            line.extend_from_slice(word);
            *line_len = wlen;
        }
        word.clear();
    };

    for &(ch, style) in &chars {
        if ch == '\n' {
            place_word(&mut out, &mut line, &mut line_len, &mut word);
            out.push(coalesce(&line));
            line.clear();
            line_len = 0;
        } else if ch.is_whitespace() {
            place_word(&mut out, &mut line, &mut line_len, &mut word);
        } else {
            word.push((ch, style));
        }
    }
    place_word(&mut out, &mut line, &mut line_len, &mut word);
    out.push(coalesce(&line));

    out
}
