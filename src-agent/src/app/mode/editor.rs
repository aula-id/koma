//! A small, reusable full-screen multi-line text editor (nano-style).
//!
//! [`TextEditorState`] is a self-contained 2D text buffer with a char-indexed
//! cursor. It is mode-agnostic: it owns no rendering and no key mapping, only
//! the buffer + cursor mutations. The `/agents` prompt editor drives it (open on
//! the Prompt field, edit comfortably, Esc commits the text back into the
//! draft), but nothing here is agents-specific.
//!
//! # Invariants
//! - `lines` is never empty: an empty buffer is `vec![String::new()]`, so `row`
//!   always indexes a real line.
//! - `row` is in `0..lines.len()`; `col` is a CHAR index in
//!   `0..=lines[row].chars().count()` (one past the last char = end-of-line).
//! - All edits / moves end by calling [`clamp`](TextEditorState::clamp), so the
//!   two bounds above hold after every public mutation.
//!
//! # Unicode
//! Every operation works on `char`s (via `chars().count()` and char-index
//! splicing), never raw byte offsets, so multi-byte text never splits a
//! codepoint or panics on a byte boundary.

/// Soft word-wrap one logical line (`chars`) into visual segments of at most
/// `wrap_w` cells, returning each segment as a `(start, end)` CHAR-index range
/// into `chars` (`start` inclusive, `end` exclusive).
///
/// Greedy: a segment grows until the next char would overflow `wrap_w`, then it
/// breaks at the LAST space that still fits — that space is consumed (it ends a
/// line and is not re-rendered at the start of the next). A single word longer
/// than `wrap_w` (no usable space) HARD-breaks exactly at `wrap_w`. An empty
/// line yields one empty segment `(0, 0)` so it still occupies a visual row.
///
/// `wrap_w` must be `>= 1` (the caller clamps it); every segment width
/// (`end - start`) is then `<= wrap_w`. Works purely on char indices, so it
/// never splits a multi-byte codepoint.
///
/// This is the SINGLE source of wrap truth: both the renderer
/// (`view::agents::editor::draw_field_editor`) and the editor's own visual
/// Up/Down navigation call it, so what's on screen and what the cursor walks
/// agree exactly.
pub fn wrap_segments(chars: &[char], wrap_w: usize) -> Vec<(usize, usize)> {
    let n = chars.len();
    if n == 0 {
        return vec![(0, 0)];
    }
    let mut segs = Vec::new();
    let mut start = 0;
    while start < n {
        if n - start <= wrap_w {
            // The remainder fits on one line.
            segs.push((start, n));
            break;
        }
        // `limit` is the first char index that does NOT fit on this line; since
        // `n - start > wrap_w` here, `limit < n`, so `chars[limit]` is in range.
        // Scan downward for the rightmost space in `start+1 ..= limit` to break
        // on cleanly (that space ends the line and is consumed on the next row).
        let limit = start + wrap_w;
        let mut brk = None;
        let mut j = limit;
        while j > start {
            if chars[j] == ' ' {
                brk = Some(j);
                break;
            }
            j -= 1;
        }
        match brk {
            Some(j) => {
                segs.push((start, j));
                start = j + 1; // consume the breaking space
            }
            None => {
                // No usable space in the window: hard-break at `wrap_w`.
                segs.push((start, limit));
                start = limit;
            }
        }
    }
    segs
}

/// A 2D text buffer with a char-indexed cursor and vertical scroll offset.
///
/// See the module docs for the invariants the methods preserve.
#[derive(Debug, Clone)]
pub struct TextEditorState {
    /// The text, one entry per logical line (no trailing `\n`). Never empty.
    pub lines: Vec<String>,
    /// Cursor line index, in `0..lines.len()`.
    pub row: usize,
    /// Cursor column as a CHAR index, in `0..=lines[row].chars().count()`.
    pub col: usize,
    /// Index of the top visible line (the view scrolls vertically by whole
    /// lines). The renderer adjusts this to keep the cursor row on screen.
    pub scroll: usize,
    /// Last wrap width the renderer drew with, published each frame via
    /// interior mutability (the renderer borrows the state immutably). Seeded to
    /// `usize::MAX` so that before the first render each line is a single segment
    /// → vertical moves fall back to logical-line behaviour. `move_up`/
    /// `move_down` read this so they navigate by the SAME visual rows on screen.
    pub wrap_w: std::cell::Cell<usize>,
}

impl TextEditorState {
    /// Build an editor seeded with `s`, splitting on `'\n'` into lines.
    ///
    /// An empty `s` yields a single empty line (the buffer is never empty). The
    /// cursor starts at the very top-left and the view is scrolled to the top.
    pub fn from_text(s: &str) -> Self {
        let lines: Vec<String> = if s.is_empty() {
            vec![String::new()]
        } else {
            s.split('\n').map(|l| l.to_string()).collect()
        };
        Self {
            lines,
            row: 0,
            col: 0,
            scroll: 0,
            // No render has happened yet: a max width makes every line a single
            // segment, so Up/Down behave like logical-line moves until the first
            // frame publishes the real wrap width (the safe fallback).
            wrap_w: std::cell::Cell::new(usize::MAX),
        }
    }

    /// The full buffer as a single string, lines re-joined with `'\n'`.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// Number of chars on the current cursor line.
    fn line_len(&self) -> usize {
        self.lines[self.row].chars().count()
    }

    /// Insert `c` at the cursor by CHAR index, then advance the cursor past it.
    pub fn insert_char(&mut self, c: char) {
        let mut chars: Vec<char> = self.lines[self.row].chars().collect();
        let at = self.col.min(chars.len());
        chars.insert(at, c);
        self.lines[self.row] = chars.into_iter().collect();
        self.col = at + 1;
        self.clamp();
    }

    /// Insert a (possibly multi-line) string at the cursor (paste path).
    ///
    /// The current line is split at the cursor; `s` is spliced in between the
    /// head and the tail, honouring its own `'\n'`s. The cursor ends at the end
    /// of the inserted text:
    /// - single-line `s` → same row, `col` advanced by `s.chars().count()`;
    /// - multi-line `s` → the last inserted line, `col` = (len of `s`'s last
    ///   segment), with the original tail appended after it.
    pub fn insert_str(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        // Split the current line at the cursor into head | tail (char-indexed).
        let cur: Vec<char> = self.lines[self.row].chars().collect();
        let at = self.col.min(cur.len());
        let head: String = cur[..at].iter().collect();
        let tail: String = cur[at..].iter().collect();

        let segments: Vec<&str> = s.split('\n').collect();
        if segments.len() == 1 {
            // No newline in the paste: stitch head + s + tail back onto one line.
            let inserted_len = segments[0].chars().count();
            self.lines[self.row] = format!("{head}{}{tail}", segments[0]);
            self.col = at + inserted_len;
        } else {
            // Multi-line: first segment joins the head; middle segments become
            // whole new lines; the last segment carries the original tail.
            let last_idx = segments.len() - 1;
            let last_len = segments[last_idx].chars().count();

            let mut new_block: Vec<String> = Vec::with_capacity(segments.len());
            new_block.push(format!("{head}{}", segments[0]));
            for seg in &segments[1..last_idx] {
                new_block.push((*seg).to_string());
            }
            new_block.push(format!("{}{tail}", segments[last_idx]));

            let new_row = self.row + last_idx;
            // Replace the current line with the whole expanded block.
            self.lines.splice(self.row..=self.row, new_block);
            self.row = new_row;
            self.col = last_len;
        }
        self.clamp();
    }

    /// Split the current line at the cursor; the tail drops to a fresh line
    /// below and the cursor moves to its start (column 0).
    pub fn newline(&mut self) {
        let chars: Vec<char> = self.lines[self.row].chars().collect();
        let at = self.col.min(chars.len());
        let head: String = chars[..at].iter().collect();
        let tail: String = chars[at..].iter().collect();
        self.lines[self.row] = head;
        self.lines.insert(self.row + 1, tail);
        self.row += 1;
        self.col = 0;
        self.clamp();
    }

    /// Delete the char before the cursor.
    ///
    /// Mid-line (`col > 0`) removes one char and steps left. At column 0 of a
    /// non-first line it joins this line onto the end of the previous one, with
    /// the cursor landing at the join point (the previous line's old length).
    pub fn backspace(&mut self) {
        if self.col > 0 {
            let mut chars: Vec<char> = self.lines[self.row].chars().collect();
            self.col -= 1;
            chars.remove(self.col);
            self.lines[self.row] = chars.into_iter().collect();
        } else if self.row > 0 {
            let cur = self.lines.remove(self.row);
            self.row -= 1;
            let prev_len = self.lines[self.row].chars().count();
            self.lines[self.row].push_str(&cur);
            self.col = prev_len;
        }
        self.clamp();
    }

    /// Delete the char at the cursor (forward delete).
    ///
    /// Mid-line removes the char under the cursor (cursor stays put). At the end
    /// of a non-last line it joins the next line onto this one (cursor stays at
    /// the join point).
    pub fn delete(&mut self) {
        let len = self.line_len();
        if self.col < len {
            let mut chars: Vec<char> = self.lines[self.row].chars().collect();
            chars.remove(self.col);
            self.lines[self.row] = chars.into_iter().collect();
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
        }
        self.clamp();
    }

    /// Move the cursor one char left; past column 0 it wraps to the end of the
    /// previous line.
    pub fn move_left(&mut self) {
        if self.col > 0 {
            self.col -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.col = self.line_len();
        }
        self.clamp();
    }

    /// Move the cursor one char right; past end-of-line it wraps to the start of
    /// the next line.
    pub fn move_right(&mut self) {
        if self.col < self.line_len() {
            self.col += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = 0;
        }
        self.clamp();
    }

    /// Wrap the cursor's current logical line at the renderer's published width,
    /// and locate which visual segment holds the cursor.
    ///
    /// Returns `(segs, seg_i, visual_col)` where `segs` are the wrapped segments
    /// of `lines[row]`, `seg_i` is the index of the segment the cursor sits in
    /// (the LAST segment whose `start <= col`, matching the renderer's cursor
    /// mapping), and `visual_col = col - segs[seg_i].start` is the cursor's
    /// column offset within that visual row. Mirrors how `draw_field_editor`
    /// maps `col` onto a wrapped cell, so vertical moves track the on-screen rows.
    fn visual_position(&self) -> (Vec<(usize, usize)>, usize, usize) {
        let wrap_w = self.wrap_w.get().max(1);
        let chars: Vec<char> = self.lines[self.row].chars().collect();
        let segs = wrap_segments(&chars, wrap_w);
        // `segs` is never empty (an empty line yields one `(0, 0)` segment).
        let mut seg_i = 0;
        for (i, &(s, _e)) in segs.iter().enumerate() {
            if s <= self.col {
                seg_i = i;
            } else {
                break;
            }
        }
        let visual_col = self.col - segs[seg_i].0;
        (segs, seg_i, visual_col)
    }

    /// Move the cursor up one VISUAL (word-wrapped) row, preserving the column
    /// offset within the row.
    ///
    /// Within a wrapped logical line this steps to the previous segment; from a
    /// line's first visual row it crosses to the LAST visual row of the line
    /// above. The target column is the same offset, clamped to the destination
    /// segment's length so the cursor never lands past it. A no-op at the very
    /// first visual row of the buffer.
    pub fn move_up(&mut self) {
        let (segs, seg_i, visual_col) = self.visual_position();
        if seg_i > 0 {
            // Stay on this logical line; rise into the previous wrapped segment.
            let (s, e) = segs[seg_i - 1];
            self.col = s + visual_col.min(e - s);
        } else if self.row > 0 {
            // First visual row of this line → last visual row of the line above.
            self.row -= 1;
            let chars: Vec<char> = self.lines[self.row].chars().collect();
            let prev = wrap_segments(&chars, self.wrap_w.get().max(1));
            let &(s, e) = match prev.last() {
                Some(p) => p,
                None => {
                    self.clamp();
                    return;
                }
            };
            self.col = s + visual_col.min(e - s);
        }
        self.clamp();
    }

    /// Move the cursor down one VISUAL (word-wrapped) row, preserving the column
    /// offset within the row.
    ///
    /// Within a wrapped logical line this steps to the next segment; from a
    /// line's last visual row it crosses to the FIRST visual row of the line
    /// below. The target column is the same offset, clamped to the destination
    /// segment's length. A no-op at the very last visual row of the buffer.
    pub fn move_down(&mut self) {
        let (segs, seg_i, visual_col) = self.visual_position();
        if seg_i + 1 < segs.len() {
            // Stay on this logical line; descend into the next wrapped segment.
            let (s, e) = segs[seg_i + 1];
            self.col = s + visual_col.min(e - s);
        } else if self.row + 1 < self.lines.len() {
            // Last visual row of this line → first visual row of the line below.
            self.row += 1;
            let chars: Vec<char> = self.lines[self.row].chars().collect();
            let next = wrap_segments(&chars, self.wrap_w.get().max(1));
            let (s, e) = next[0];
            self.col = s + visual_col.min(e - s);
        }
        self.clamp();
    }

    /// Move the cursor to the start of the current line.
    pub fn home(&mut self) {
        self.col = 0;
    }

    /// Move the cursor to the end of the current line.
    pub fn end(&mut self) {
        self.col = self.line_len();
    }

    /// Re-establish the invariants: keep `row` in range and `col` no further than
    /// the end of the current line. Called at the end of every mutation/move.
    pub fn clamp(&mut self) {
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        if self.row >= self.lines.len() {
            self.row = self.lines.len() - 1;
        }
        let len = self.line_len();
        if self.col > len {
            self.col = len;
        }
    }
}
