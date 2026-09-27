//! On-demand desktop control. The daemon owns policy and correlation; only the
//! GUI worker owns native handles. No timer ever captures a desktop frame.
pub(crate) mod bridge;
pub(crate) mod contract;
pub(crate) mod controller;
pub(crate) mod executor;
pub(crate) use contract::*;
pub(crate) use controller::Controller;
#[cfg(feature = "gui")]
pub(crate) mod desktop;

#[cfg(all(feature = "gui", target_os = "linux"))]
pub(crate) mod enrichment;
