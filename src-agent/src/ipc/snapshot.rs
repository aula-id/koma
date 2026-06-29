//! Pure render-state PROJECTION + DIFF for the daemon stage-4 streaming layer.
//!
//! Two pure functions, no runtime handles, no terminal, no channels:
//!
//! - [`build_snapshot`] reads the live [`AppState`] and copies out a frozen
//!   [`StateSnapshot`] (the [`super::proto`] projection): one [`SessionSnapshot`]
//!   per session + the foreground id + the [`GlobalSnapshot`]. It is the SINGLE
//!   source of truth for "what the client should render", so a client can never
//!   diverge from the daemon — it only ever renders this projection.
//! - [`diff`] compares a freshly-built snapshot against the previously-sent one and
//!   yields the minimal set of [`StateDelta`]s for the high-frequency per-tick
//!   changes, OR signals (`needs_full`) that a STRUCTURAL change happened that is
//!   not worth diffing incrementally (session added/removed, history changed,
//!   tokens/approval/subagents shifted) — in which case the caller resends a full
//!   [`StateSnapshot`] instead. Correctness-first (daemon stage 4): when in doubt,
//!   ask for a full snapshot; a full snapshot is ALWAYS a valid update.
//!
//! Keeping this PURE (a function of `&AppState`, not a method that also drives the
//! socket) is deliberate: the daemon loop owns the channels + the monotonic seq and
//! merely calls these, and a future local-TUI consumer could call the exact same
//! builder, so the wire projection can never drift from a second hand-rolled copy.

mod diff;
mod projection;

#[allow(unused_imports)]
pub use diff::{diff, DiffResult};
pub use projection::{build_snapshot, build_snapshot_with_mode, mode_snapshot};
