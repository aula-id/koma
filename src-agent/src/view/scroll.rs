//! Shared viewport math for scrollable, selectable list overlays.
//!
//! Every list overlay (command/file palette, pickers, hub, help, …) shows a
//! window of at most `h` rows over a longer list. The window must let the
//! selection walk WITHIN the visible rows and only scroll when it crosses the
//! top/bottom edge ("scrolloff" navigation) — NOT pin the selection to the last
//! row (the old copy-pasted `sel + 1 - h` math did the latter).
//!
//! Correct scrolloff needs a remembered offset, so callers pass a render-owned
//! [`Cell`] (persisted across frames, e.g. on `AppStateRest`, never serialized).
//! The window SIZE is a parameter (`h`), so each overlay keeps its own budget
//! (7, 10, dynamic height, …): the algorithm is shared, the dimensions are not.

use std::cell::Cell;

/// Return the visible `[start, end)` slice for a selectable list, updating the
/// persisted `offset` so the selection stays put within the window until it
/// reaches an edge.
///
/// - `offset`: render-owned scroll cursor (read + written here).
/// - `sel`: selected index (clamped into range).
/// - `n`: total item count.
/// - `h`: visible row budget.
///
/// Guarantees `start <= sel < end <= n` when `n > 0`; returns `(0, 0)` when the
/// list or the window is empty. When everything fits (`n <= h`) the offset is
/// reset to 0 and the whole list is shown.
pub fn scroll_window(offset: &Cell<usize>, sel: usize, n: usize, h: usize) -> (usize, usize) {
    if n == 0 || h == 0 {
        offset.set(0);
        return (0, 0);
    }
    let sel = sel.min(n - 1);
    if n <= h {
        offset.set(0);
        return (0, n);
    }
    let max_off = n - h;
    let mut off = offset.get().min(max_off);
    if sel < off {
        off = sel; // selection above the window → scroll up to it
    } else if sel >= off + h {
        off = sel + 1 - h; // selection below the window → scroll down to it
    }
    off = off.min(max_off);
    offset.set(off);
    (off, off + h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn everything_fits_shows_all() {
        let off = Cell::new(3);
        assert_eq!(scroll_window(&off, 2, 4, 7), (0, 4));
        assert_eq!(off.get(), 0);
    }

    #[test]
    fn empty_list_or_zero_height() {
        let off = Cell::new(5);
        assert_eq!(scroll_window(&off, 0, 0, 7), (0, 0));
        assert_eq!(scroll_window(&off, 0, 10, 0), (0, 0));
    }

    #[test]
    fn selection_walks_within_window_then_scrolls_at_edge() {
        let off = Cell::new(0);
        assert_eq!(scroll_window(&off, 6, 10, 7), (0, 7));
        assert_eq!(scroll_window(&off, 7, 10, 7), (1, 8));
        assert_eq!(scroll_window(&off, 6, 10, 7), (1, 8));
        assert_eq!(scroll_window(&off, 1, 10, 7), (1, 8));
        assert_eq!(scroll_window(&off, 0, 10, 7), (0, 7));
    }

    #[test]
    fn clamps_stale_offset_to_end() {
        let off = Cell::new(100);
        assert_eq!(scroll_window(&off, 9, 10, 7), (3, 10));
        assert_eq!(off.get(), 3);
    }
}
