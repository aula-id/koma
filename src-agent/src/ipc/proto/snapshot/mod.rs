// ─── full-state snapshot and mode payload projections (pure data) ────────────
//
// Split into themed submodules (session/global/connector/settings/panels); every
// item is re-exported here so `crate::ipc::proto::snapshot::*` — and the parent
// `ipc::proto` module's `pub use snapshot::*` — keep resolving exactly as before
// the split. No behavior change, pure code motion.

pub mod connector;
pub mod ext;
pub mod global;
pub mod panels;
pub mod remote;
pub mod session;
pub mod settings;

pub use connector::*;
pub use ext::*;
pub use global::*;
pub use panels::*;
pub use remote::*;
pub use session::*;
pub use settings::*;
