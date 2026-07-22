//! Shadow mode reconstruction for the thin attach client.
//!
//! Each `shadow_*` function rebuilds a REAL mode-state / runtime value from its
//! wire projection so the unmodified `view::draw` renders it.

#![allow(unused_imports)]
#![allow(dead_code)]

mod modes;
mod session;
mod settings;

pub use modes::*;
pub use session::*;
pub use settings::*;
