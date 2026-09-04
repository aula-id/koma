//! Block-aware Markdown renderer for the chat transcript.
//!
//! Replaces `tui-markdown`, which flattens everything into a borderless `Text`
//! and therefore cannot preserve code indentation, draw boxes, or align tables.
//! Here we parse with `pulldown-cmark` and walk the event stream while keeping a
//! small inline-style stack and a notion of the *current block*. Each block is
//! laid out to a fixed column width and emitted as fully-formed **visual lines**
//! (`Vec<Span<'static>>`) — already wrapped, boxed, padded, and aligned — so the
//! caller only prepends a bullet/indent and never re-wraps. That contract is
//! what keeps the chat view's follow-scroll math exact: emitted line count ==
//! on-screen line count.
//!
//! Code blocks are the priority feature: every fenced/indented block is drawn as
//! a full box with a language label and syntax highlighting (via `syntect` using
//! the pure-Rust fancy-regex engine, no Oniguruma), preserving whitespace and
//! hard-splitting over-long lines rather than word-wrapping. GFM tables are
//! collected in full, column widths fitted to the available width, and rendered
//! with box-drawing borders honouring per-column alignment. Prose, headings,
//! lists, block quotes, and thematic breaks word-wrap with inline styles intact.
//!
//! All colours are tuned for a dark background (the app default); `syntect`'s
//! own background is dropped so highlighted code blends with the terminal.
//!
//! ## Module layout
//!
//! - [`parse`]   — data types: `Inline`, `TableBuf`, `Block`
//! - [`helpers`] — syntect singletons + free functions: `coalesce`, `fit_cell`,
//!   `shrink_widths`, `wrap_spans`
//! - [`render`]  — `Renderer` struct and all its event-handling / flush methods

mod helpers;
mod parse;
mod render;

use pulldown_cmark::{Options, Parser};

use crate::view::theme::Palette;
use render::Renderer;

// Re-export the one pub(crate) item that lives inside helpers so external
// callers keep the same path: `crate::view::markdown::wrap_spans`.
pub(crate) use helpers::{wrap_spans, wrap_spans_preserve};

// --- public API --------------------------------------------------------------

/// Render `md` into final visual lines laid out to exactly `width` columns.
///
/// Each inner `Vec<Span>` is one on-screen line (already wrapped/boxed/aligned).
/// The caller prepends a bullet/indent and must NOT re-wrap. All spans own their
/// text (`'static`). Top-level blocks are separated by a single blank line.
pub fn render(md: &str, palette: &Palette, width: usize) -> Vec<Vec<ratatui::text::Span<'static>>> {
    let width = width.max(1);

    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    let parser = Parser::new_ext(md, opts);

    let mut r = Renderer::new(palette, width);
    for ev in parser {
        r.event(ev);
    }
    r.finish()
}
