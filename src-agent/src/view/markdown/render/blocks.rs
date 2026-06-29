//! Block-flusher methods on [`Renderer`]: paragraph, heading, quote, list item, table.

use pulldown_cmark::Alignment;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

use crate::view::theme::Palette;

use super::super::helpers::{fit_cell, shrink_widths, wrap_spans};
use super::super::parse::{Block, TableBuf};
use super::renderer::Renderer;

impl<'p> Renderer<'p> {
    pub(super) fn flush_paragraph(&mut self) {
        if self.cur.is_empty() {
            self.block = Block::None;
            return;
        }
        self.sep();
        let spans = std::mem::take(&mut self.cur);
        for vl in wrap_spans(&spans, self.width) {
            self.out.push(vl);
        }
        self.block = Block::None;
    }

    pub(super) fn flush_heading(&mut self) {
        if self.cur.is_empty() {
            self.block = Block::None;
            return;
        }
        let level = match &self.block {
            Block::Heading(l) => *l,
            _ => pulldown_cmark::HeadingLevel::H1,
        };
        // Levels 1-2 use the accent colour; 3-6 stay fg. All bold.
        let color = match level {
            pulldown_cmark::HeadingLevel::H1 | pulldown_cmark::HeadingLevel::H2 => self.palette.accent,
            _ => self.palette.fg,
        };
        // Restyle the accumulated spans to the heading colour + bold (headings
        // ignore inner emphasis colour, just force bold heading styling).
        let spans = std::mem::take(&mut self.cur);
        let restyled: Vec<Span<'static>> = spans
            .into_iter()
            .map(|s| {
                let m = s.style.add_modifier(Modifier::BOLD).fg(color);
                Span::styled(s.content.into_owned(), m)
            })
            .collect();
        self.sep();
        for vl in wrap_spans(&restyled, self.width) {
            self.out.push(vl);
        }
        self.block = Block::None;
    }

    pub(super) fn flush_quote(&mut self) {
        if self.cur.is_empty() {
            self.block = Block::None;
            return;
        }
        let spans = std::mem::take(&mut self.cur);
        // Dim the quote body slightly and prefix each wrapped line with "│ ".
        let dimmed: Vec<Span<'static>> = spans
            .into_iter()
            .map(|s| {
                let st = s.style.fg(self.palette.dim);
                Span::styled(s.content.into_owned(), st)
            })
            .collect();
        let inner_w = self.width.saturating_sub(2).max(1);
        self.sep();
        for mut vl in wrap_spans(&dimmed, inner_w) {
            let mut line = vec![Span::styled(
                "│ ".to_string(),
                Style::default().fg(self.palette.dim),
            )];
            line.append(&mut vl);
            self.out.push(line);
        }
        self.block = Block::None;
    }

    pub(super) fn flush_list_item(&mut self) {
        if self.cur.is_empty() {
            self.block = Block::None;
            return;
        }
        // One blank line before the list as a whole, then items stay tight.
        if !self.list_sep_done {
            self.sep();
            self.list_sep_done = true;
        }
        let depth = self.lists.len().saturating_sub(1);
        let indent = depth * 2;
        // The first span is the dim marker (indent+marker) we pushed on
        // Start(Item). Measure it to derive marker width + the hanging indent.
        let spans = std::mem::take(&mut self.cur);
        let marker_len = spans
            .first()
            .map(|s| s.content.chars().count())
            .unwrap_or(indent);
        // Wrap only the text spans (everything after the marker) to the width
        // left after the marker; then re-attach the marker to the first line and
        // hanging-indent the rest.
        let text_spans = &spans[1..];
        let avail = self.width.saturating_sub(marker_len).max(1);
        let wrapped = wrap_spans(text_spans, avail);
        let marker_span = spans[0].clone();
        let pad = " ".repeat(marker_len);
        for (i, mut vl) in wrapped.into_iter().enumerate() {
            let mut line = Vec::with_capacity(vl.len() + 1);
            if i == 0 {
                line.push(marker_span.clone());
            } else {
                line.push(Span::raw(pad.clone()));
            }
            line.append(&mut vl);
            self.out.push(line);
        }
        self.block = Block::None;
    }

    /// Render a buffered GFM table: fit columns, then draw boxed rows honouring
    /// per-column alignment, header cells bold.
    pub(super) fn flush_table(&mut self) {
        let tb = match std::mem::replace(&mut self.block, Block::None) {
            Block::Table(tb) => tb,
            _ => return,
        };
        let ncols = tb
            .head
            .len()
            .max(tb.rows.iter().map(|r| r.len()).max().unwrap_or(0));
        if ncols == 0 {
            return;
        }
        self.sep();
        let dim = Style::default().fg(self.palette.dim);

        // Natural content width per column (max over header + body), in chars.
        let span_w = |cell: &[Span<'static>]| -> usize {
            cell.iter().map(|s| s.content.chars().count()).sum()
        };
        let mut widths = vec![0usize; ncols];
        for (c, cell) in tb.head.iter().enumerate() {
            widths[c] = widths[c].max(span_w(cell));
        }
        for row in &tb.rows {
            for (c, cell) in row.iter().enumerate() {
                widths[c] = widths[c].max(span_w(cell));
            }
        }
        // Each column is padded with one space on each side: "│ " + cell + " ".
        // Total box width = sum(widths) + 3*ncols + 1  (│ + ' '+w+' ' per col + │).
        let chrome = 3 * ncols + 1;
        let avail = self.width.saturating_sub(chrome);
        let total: usize = widths.iter().sum();
        if total > avail && avail >= ncols {
            // Shrink proportionally so the table fits, never below 1 col each.
            shrink_widths(&mut widths, avail);
        } else if total > avail {
            // Not even 1 char per column fits; clamp everything to 1.
            widths.fill(1);
        }

        let aligns = &tb.aligns;
        let align_of = |c: usize| aligns.get(c).copied().unwrap_or(Alignment::None);

        // Borders.
        let border = |left: &str, mid: &str, right: &str| -> Vec<Span<'static>> {
            let mut s = String::from(left);
            for (c, w) in widths.iter().enumerate() {
                if c > 0 {
                    s.push_str(mid);
                }
                // " " + w + " " worth of ─ = w + 2.
                s.push_str(&"─".repeat(w + 2));
            }
            s.push_str(right);
            vec![Span::styled(s, dim)]
        };

        // A data/header row: "│ " cell " │ " cell " │", cells truncated+aligned.
        let make_row =
            |cells: &[Vec<Span<'static>>], bold: bool, widths: &[usize], palette: &Palette| -> Vec<Span<'static>> {
                let mut line: Vec<Span<'static>> = Vec::new();
                for (c, w) in widths.iter().enumerate() {
                    line.push(Span::styled("│ ".to_string(), Style::default().fg(palette.dim)));
                    let empty: Vec<Span<'static>> = Vec::new();
                    let cell = cells.get(c).unwrap_or(&empty);
                    let mut padded = fit_cell(cell, *w, align_of(c), bold, palette);
                    line.append(&mut padded);
                    line.push(Span::raw(" "));
                }
                line.push(Span::styled("│".to_string(), Style::default().fg(palette.dim)));
                line
            };

        self.out.push(border("┌", "┬", "┐"));
        self.out.push(make_row(&tb.head, true, &widths, self.palette));
        self.out.push(border("├", "┼", "┤"));
        for row in &tb.rows {
            self.out.push(make_row(row, false, &widths, self.palette));
        }
        self.out.push(border("└", "┴", "┘"));
    }
}
