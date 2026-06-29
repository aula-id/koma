//! Code-block flusher and per-row helpers on [`Renderer`].

use ratatui::style::Style;
use ratatui::text::Span;
use syntect::easy::HighlightLines;
use syntect::util::LinesWithEndings;

use super::super::helpers::{coalesce, syntaxes, syntect_fg, theme};
use super::super::parse::Block;
use super::renderer::Renderer;

impl<'p> Renderer<'p> {
    /// Draw a fenced/indented code block as a full box: `┌─ {lang} ───┐`, one
    /// padded content row per (possibly hard-split) source line with syntect
    /// colours, and `└────┘`. Indentation is preserved verbatim; lines wider
    /// than the inner width are hard-split at a char boundary.
    pub(super) fn flush_code(&mut self) {
        let (lang, code) = match std::mem::replace(&mut self.block, Block::None) {
            Block::Code { lang, text } => (lang, text),
            _ => return,
        };
        self.sep();

        let dim = Style::default().fg(self.palette.dim);
        let w = self.width;
        let iw = w.saturating_sub(4); // "│ " + content + " │"

        // --- top border with optional language label ---
        let top = if lang.is_empty() {
            vec![Span::styled(
                format!("┌{}┐", "─".repeat(w.saturating_sub(2))),
                dim,
            )]
        } else {
            // Build: '┌' '─' ' lang ' then '─'*fill then '┐', totalling `w` cols.
            let label = format!(" {lang} ");
            let used = 1 /*┌*/ + 1 /*─*/ + label.chars().count();
            let fill = w.saturating_sub(used + 1 /*┐*/);
            vec![
                Span::styled("┌─".to_string(), dim),
                Span::styled(label, Style::default().fg(self.palette.accent)),
                Span::styled(format!("{}┐", "─".repeat(fill)), dim),
            ]
        };
        self.out.push(top);

        // --- content rows ---
        let syntax = syntaxes()
            .find_syntax_by_token(&lang)
            .unwrap_or_else(|| syntaxes().find_syntax_plain_text());
        let mut hl = HighlightLines::new(syntax, theme());

        // Drop a single trailing newline so we don't emit a spurious blank row.
        let body = code.strip_suffix('\n').unwrap_or(&code);
        if body.is_empty() {
            // Empty code block: one blank content row keeps the box non-degenerate.
            self.out.push(self.code_row(Vec::new(), iw, dim));
        } else {
            for line in LinesWithEndings::from(body) {
                let ranges = hl
                    .highlight_line(line, syntaxes())
                    .unwrap_or_else(|_| vec![(syntect::highlighting::Style::default(), line)]);
                // Convert to (style, char-run) spans, stripping the line ending.
                let styled: Vec<(Style, String)> = ranges
                    .into_iter()
                    .map(|(s, txt)| {
                        let t = txt.trim_end_matches(['\n', '\r']).to_string();
                        (syntect_fg(s), t)
                    })
                    .filter(|(_, t)| !t.is_empty())
                    .collect();
                self.emit_code_line(&styled, iw, dim);
            }
        }

        // --- bottom border ---
        self.out.push(vec![Span::styled(
            format!("└{}┘", "─".repeat(w.saturating_sub(2))),
            dim,
        )]);
    }

    /// Build one code content row from already-styled spans whose total width is
    /// `<= iw`, padding with spaces to `iw` and wrapping in the box borders.
    pub(super) fn code_row(&self, mut content: Vec<Span<'static>>, iw: usize, dim: Style) -> Vec<Span<'static>> {
        let used: usize = content.iter().map(|s| s.content.chars().count()).sum();
        let pad = iw.saturating_sub(used);
        if pad > 0 {
            content.push(Span::raw(" ".repeat(pad)));
        }
        let mut line = Vec::with_capacity(content.len() + 2);
        line.push(Span::styled("│ ".to_string(), dim));
        line.extend(content);
        line.push(Span::styled(" │".to_string(), dim));
        line
    }

    /// Emit a (possibly hard-split) source line as one or more boxed rows. The
    /// styled `(Style, String)` runs are walked char-by-char so a split lands on
    /// a char boundary at exactly `iw` columns, preserving all whitespace.
    pub(super) fn emit_code_line(&mut self, styled: &[(Style, String)], iw: usize, dim: Style) {
        if iw == 0 {
            // Degenerate width: emit a single empty-bordered row and bail.
            self.out.push(self.code_row(Vec::new(), 0, dim));
            return;
        }
        // Flatten to (char, style) for exact-width chunking.
        let mut chars: Vec<(char, Style)> = Vec::new();
        for (st, s) in styled {
            for ch in s.chars() {
                chars.push((ch, *st));
            }
        }
        if chars.is_empty() {
            self.out.push(self.code_row(Vec::new(), iw, dim));
            return;
        }
        let mut i = 0;
        while i < chars.len() {
            let end = (i + iw).min(chars.len());
            let chunk = &chars[i..end];
            self.out.push(self.code_row(coalesce(chunk), iw, dim));
            i = end;
        }
    }
}
