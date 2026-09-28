//! On-demand desktop control. The daemon owns policy and correlation; only the
//! GUI worker owns native handles. Model observations are on demand; open GUI
//! previews may request separate ephemeral frames without mutating model state.
pub(crate) mod bridge;
pub(crate) mod contract;
pub(crate) mod controller;
// Headless CI builds this bin with `--no-default-features`, which drops the
// desktop adapters. Keep the glide out of that build, and in `cfg(test)` so
// the unit tests still run there.
#[cfg(any(test, feature = "gui"))]
mod cursor_glide;
pub(crate) mod executor;
mod keys;
mod ruler;
pub(crate) use contract::*;
pub(crate) use controller::Controller;
#[cfg(feature = "gui")]
pub(crate) mod desktop;

#[cfg(feature = "gui")]
pub(crate) mod enrichment;
