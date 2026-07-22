//! Data types used during Markdown parse/event walking.
//!
//! [`Inline`] is the inline-style stack element; [`TableBuf`] accumulates a GFM
//! table until its `End` so column widths can be fitted; [`Block`] tracks which
//! top-level block the renderer is currently inside.

use pulldown_cmark::{Alignment, HeadingLevel};
use ratatui::text::Span;

// --- inline style stack ------------------------------------------------------

/// One pushed inline context. `Link` carries the destination so we can append
/// ` (url)` when the link closes. (Inline `code` arrives as a standalone
/// `Event::Code`, not a Start/End pair, so it isn't represented here.)
#[derive(Clone)]
pub(super) enum Inline {
    Bold,
    Italic,
    Strike,
    Link(String),
}

// --- table buffering ---------------------------------------------------------

/// A table accumulated until its `End` so column widths can be fitted before any
/// row is drawn. Cells are styled inline spans; `head` is rendered bold.
pub(super) struct TableBuf {
    pub(super) aligns: Vec<Alignment>,
    pub(super) head: Vec<Vec<Span<'static>>>,
    pub(super) rows: Vec<Vec<Vec<Span<'static>>>>,
    pub(super) in_head: bool,
    pub(super) cur_row: Vec<Vec<Span<'static>>>,
}

// --- block kind --------------------------------------------------------------

/// What kind of block we're currently inside. The renderer accumulates inline
/// content into `cur` and flushes it on the matching `End`.
pub(super) enum Block {
    None,
    Paragraph,
    Heading(HeadingLevel),
    /// Buffered raw code text + detected language token.
    Code {
        lang: String,
        text: String,
    },
    /// Block-quote: buffered like a paragraph but prefixed per visual line.
    Quote,
    /// A list item: inline text accumulates in `cur` after a leading marker span.
    ListItem,
    Table(TableBuf),
}
