//! Pure render-state projection: builds a frozen [`StateSnapshot`] from the
//! live [`AppState`]. Split into three focused submodules — token converters,
//! per-mode snapshot builders, and the top-level entry point.

mod core;
mod modes;
mod tokens;

pub use core::build_snapshot;
