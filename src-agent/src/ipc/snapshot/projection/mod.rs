//! Pure render-state projection: builds a frozen [`StateSnapshot`] from the
//! live [`AppState`]. Split into three focused submodules — token converters,
//! per-mode snapshot builders, and the top-level entry point.

mod core;
mod modes;
mod tokens;

pub use core::{build_snapshot, build_snapshot_with_mode};
// `bash_job_views` is re-exported (not just `mode_snapshot`) so the `/bash` command
// + its input handler can read the LIVE background-job registry to seed/refresh the
// panel, the same way the projection itself does.
pub use modes::{bash_job_views, mode_snapshot};
