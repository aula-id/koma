//! Core [`Renderer`] struct, constructor, and the main event-dispatch methods.

use pulldown_cmark::{Alignment, Event, HeadingLevel, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

use crate::view::theme::Palette;

use super::super::helpers::{coalesce, fit_cell, shrink_widths, syntaxes, syntect_fg, theme, wrap_spans};
use super::super::parse::{Block, Inline, TableBuf};

// --- the renderer ------------------------------------------------------------

pub struct Renderer<'p> {
    pub(super) palette: &'p Palette,
    pub(super) width: usize,
    pub(super) out: Vec<Vec<Span<'static>>>,
    /// Inline spans accumulated for the current text block (paragraph/heading/
    /// quote/table cell).
    pub(super) cur: Vec<Span<'static>>,
    pub(super) stack: Vec<Inline>,
    pub(super) block: Block,
    /// List marker stack: `Some(n)` = ordered (next number), `None` = unordered.
    /// Depth = `lists.len()`.
    pub(super) lists: Vec<Option<u64>>,
    /// Block-quote nesting depth. While `> 0`, paragraphs inside the quote do not
    /// start/flush their own block — the quote owns the buffered inline content.
    pub(super) in_quote: u32,
    /// True once at least one block has been emitted (drives blank separators).
    pub(super) started: bool,
    /// True once the current top-level list has emitted its leading separator, so
    /// items within one list stay tight (no blank line between them).
    pub(super) list_sep_done: bool,
}

impl<'p> Renderer<'p> {
    pub fn new(palette: &'p Palette, width: usize) -> Self {
        Renderer {
            palette,
            width,
            out: Vec::new(),
            cur: Vec::new(),
            stack: Vec::new(),
            block: Block::None,
            lists: Vec::new(),
            in_quote: 0,
            started: false,
            list_sep_done: false,
        }
    }

    /// Emit a blank separator line before a new top-level block, except the very
    /// first. List items handle their own spacing, so the marker emits directly.
    pub(super) fn sep(&mut self) {
        if self.started {
            self.out.push(vec![Span::raw("")]);
        }
        self.started = true;
    }

    /// Fold the inline stack into one effective [`Style`]. Inline code wins the
    /// colour slot (accent); link adds underline; the rest are modifiers.
    fn cur_style(&self) -> Style {
        let mut st = Style::default().fg(self.palette.fg);
        for inl in &self.stack {
            match inl {
                Inline::Bold => st = st.add_modifier(Modifier::BOLD),
                Inline::Italic => st = st.add_modifier(Modifier::ITALIC),
                Inline::Strike => st = st.add_modifier(Modifier::CROSSED_OUT),
                Inline::Link(_) => st = st.add_modifier(Modifier::UNDERLINED),
            }
        }
        st
    }

    /// Append a styled run of text to the current inline buffer.
    fn push_text(&mut self, t: &str) {
        if t.is_empty() {
            return;
        }
        self.cur.push(Span::styled(t.to_string(), self.cur_style()));
    }

    pub fn event(&mut self, ev: Event<'_>) {
        match ev {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.text(&t),
            Event::Code(t) => {
                // Inline code span: force accent regardless of surrounding stack.
                self.cur.push(Span::styled(
                    t.to_string(),
                    Style::default().fg(self.palette.accent),
                ));
            }
            Event::SoftBreak | Event::HardBreak => {
                // Inside prose blocks a break becomes a new visual line; the
                // word-wrapper splits on the embedded newline.
                if let Block::Code { text, .. } = &mut self.block {
                    text.push('\n');
                } else {
                    self.cur.push(Span::raw("\n"));
                }
            }
            Event::Rule => {
                self.sep();
                self.out.push(vec![Span::styled(
                    "─".repeat(self.width),
                    Style::default().fg(self.palette.dim),
                )]);
            }
            // Lists/tasks beyond the handled set degrade to their text content,
            // which already arrives via Text events; nothing extra to do.
            _ => {}
        }
    }

    fn text(&mut self, t: &str) {
        if let Block::Code { text, .. } = &mut self.block {
            text.push_str(t);
        } else {
            self.push_text(t);
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                // A paragraph inside a list item or block-quote continues that
                // container's buffered content; only a standalone paragraph is its
                // own block. Consecutive paragraphs in one quote get a blank line.
                if self.in_quote > 0 {
                    if !self.cur.is_empty() {
                        self.cur.push(Span::raw("\n\n"));
                    }
                } else if self.lists.is_empty() {
                    self.block = Block::Paragraph;
                }
            }
            Tag::Heading { level, .. } => self.block = Block::Heading(level),
            Tag::CodeBlock(kind) => {
                let lang = match kind {
                    pulldown_cmark::CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().unwrap_or("").to_string()
                    }
                    pulldown_cmark::CodeBlockKind::Indented => String::new(),
                };
                self.block = Block::Code {
                    lang,
                    text: String::new(),
                };
            }
            Tag::BlockQuote(_) => {
                self.in_quote += 1;
                self.block = Block::Quote;
            }
            Tag::List(start) => {
                // A new top-level list is one block: arm its single leading
                // separator (nested lists inherit the parent's tight spacing).
                if self.lists.is_empty() {
                    self.list_sep_done = false;
                }
                self.lists.push(start);
            }
            Tag::Item => {
                // Render the marker line for this item immediately; the item's
                // inline text accumulates into `cur` and flushes on `End(Item)`.
                self.flush_cur_as_paragraph_if_pending();
                let depth = self.lists.len().saturating_sub(1);
                let indent = "  ".repeat(depth);
                let marker = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        let m = format!("{n}. ");
                        *n += 1;
                        m
                    }
                    _ => "• ".to_string(),
                };
                // Stash the prefix on `cur` as a leading dim span; the wrapper
                // hanging-indents continuations under it on flush.
                let prefix = format!("{indent}{marker}");
                self.cur.push(Span::styled(
                    prefix,
                    Style::default().fg(self.palette.dim),
                ));
                self.block = Block::ListItem;
            }
            Tag::Table(aligns) => {
                self.block = Block::Table(TableBuf {
                    aligns,
                    head: Vec::new(),
                    rows: Vec::new(),
                    in_head: false,
                    cur_row: Vec::new(),
                });
            }
            Tag::TableHead => {
                if let Block::Table(tb) = &mut self.block {
                    tb.in_head = true;
                }
            }
            Tag::TableRow => {
                if let Block::Table(tb) = &mut self.block {
                    tb.cur_row = Vec::new();
                }
            }
            Tag::TableCell => {
                // Cell inline content accumulates into `cur`; committed on
                // End(TableCell).
                self.cur = Vec::new();
            }
            Tag::Emphasis => self.stack.push(Inline::Italic),
            Tag::Strong => self.stack.push(Inline::Bold),
            Tag::Strikethrough => self.stack.push(Inline::Strike),
            Tag::Link { dest_url, .. } => self.stack.push(Inline::Link(dest_url.to_string())),
            Tag::Image { .. } => {
                // Images have no terminal rendering; their alt text flows through
                // as Text. Push a no-op inline so the matching End pops cleanly.
                self.stack.push(Inline::Italic);
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                // Inside a list/quote the container flushes; standalone paragraphs
                // flush here.
                if self.in_quote == 0 && self.lists.is_empty() {
                    self.flush_paragraph();
                }
            }
            TagEnd::Heading(_) => self.flush_heading(),
            TagEnd::CodeBlock => self.flush_code(),
            TagEnd::BlockQuote(_) => {
                self.in_quote = self.in_quote.saturating_sub(1);
                // Only flush at the outermost level so nested quotes coalesce.
                if self.in_quote == 0 {
                    self.flush_quote();
                }
            }
            TagEnd::List(_) => {
                self.lists.pop();
            }
            TagEnd::Item => self.flush_list_item(),
            TagEnd::Table => self.flush_table(),
            TagEnd::TableHead => {
                if let Block::Table(tb) = &mut self.block {
                    tb.in_head = false;
                    tb.head = std::mem::take(&mut tb.cur_row);
                }
            }
            TagEnd::TableRow => {
                if let Block::Table(tb) = &mut self.block {
                    if !tb.in_head {
                        let row = std::mem::take(&mut tb.cur_row);
                        tb.rows.push(row);
                    }
                }
            }
            TagEnd::TableCell => {
                let cell = std::mem::take(&mut self.cur);
                if let Block::Table(tb) = &mut self.block {
                    tb.cur_row.push(cell);
                }
            }
            TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Strikethrough
            | TagEnd::Image => {
                self.stack.pop();
            }
            TagEnd::Link => {
                // Append ` (url)` in dim after the link text, if any.
                if let Some(Inline::Link(url)) = self.stack.pop() {
                    if !url.is_empty() {
                        self.cur.push(Span::styled(
                            format!(" ({url})"),
                            Style::default().fg(self.palette.dim),
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    /// Defensive flush: if a previous list item's inline buffer is still pending
    /// when a new item starts (e.g. loose lists), emit it first.
    pub(super) fn flush_cur_as_paragraph_if_pending(&mut self) {
        if matches!(self.block, Block::ListItem) && !self.cur.is_empty() {
            self.flush_list_item();
        }
    }

    /// Flush any block left open by a malformed / truncated document, dispatching
    /// to the matching block flusher so a half-finished heading/quote/code/table
    /// still renders correctly (each flusher reads `self.block` + `self.cur`).
    pub fn finish(mut self) -> Vec<Vec<Span<'static>>> {
        match &self.block {
            Block::Heading(_) => self.flush_heading(),
            Block::Code { .. } => self.flush_code(),
            Block::Quote => self.flush_quote(),
            Block::ListItem => self.flush_list_item(),
            Block::Table(_) => self.flush_table(),
            // Paragraph or None: any pending inline content is a final paragraph.
            _ => self.flush_paragraph(),
        }
        self.out
    }
}
