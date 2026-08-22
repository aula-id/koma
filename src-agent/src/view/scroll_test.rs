#![allow(clippy::unwrap_used, clippy::expect_used)]
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
