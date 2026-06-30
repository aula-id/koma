//! Application state: the single source of truth the UI renders from.
//!
//! [`AppState`] = the current [`Mode`] (which screen + its form/picker data)
//! plus [`AppStateRest`], the mode-independent rest of the world: the active
//! session, input buffer, status line, scroll, and the streaming machinery.
//!
//! Data flow: a keystroke becomes an `Action` (controller), the runtime applies
//! that `Action` by mutating this state, and `view::draw` reads it. Async
//! request output arrives via [`AppStateRest::active_rx`] — the receiver for the
//! one in-flight request. The runtime drains it each tick and folds the events
//! in here; dropping it cancels delivery from a superseded task.
//!
//! # Module layout
//!
//! - `types`   – [`AgentMode`], [`ToastKind`], [`TranscriptCache`], [`CataloguePending`]
//! - `rest`    – [`AppStateRest`] struct + constructor
//! - `runtime` – [`SessionRuntime`]: the per-session execution state + stream methods
//! - `input`   – input-editing and history `impl` blocks
//! - `scroll`  – scroll `impl` block
//! - `misc`    – credentials, catalogue requests, toast `impl` block

mod types;
mod rest;
mod runtime;
mod input;
mod scroll;
mod misc;

use crate::app::mode::Mode;

// Re-export everything that was public in the original state.rs so all
// external paths remain identical.
pub use types::{AgentMode, ToastKind};
pub use rest::AppStateRest;
// Public surface for the upcoming multi-session stages; not yet referenced
// outside this module while there is a single foreground session.
#[allow(unused_imports)]
pub use runtime::SessionRuntime;

pub struct AppState {
    pub rest: AppStateRest,
}

impl AppState {
    /// Construct a fresh `AppState` whose SOLE (foreground) session starts in `mode`.
    ///
    /// `mode` is PER-SESSION now (C3): it lives on each [`SessionRuntime`], reached via
    /// the foreground in [`mode`](Self::mode) / [`mode_mut`](Self::mode_mut). The
    /// constructor builds the rest (which seeds one session) and writes `mode` onto that
    /// first session, so a freshly-built state renders `mode` exactly as before the move.
    pub fn new(mode: Mode) -> Self {
        let mut rest = AppStateRest::new();
        rest.fg_mut().mode = mode;
        Self { rest }
    }

    /// The CURRENT mode = the FOREGROUND session's mode (C3). Mode is per-session, so
    /// every read routes through the foreground; in the daemon the per-client foreground
    /// is swapped in before each per-client request/projection, making overlays per-client.
    pub fn mode(&self) -> &Mode {
        &self.rest.fg().mode
    }

    /// Mutable handle to the FOREGROUND session's mode (C3).
    ///
    /// NOTE: this borrows `self.rest` mutably for as long as the returned reference is
    /// alive, so a site that needs `state.rest.<field>` WHILE pattern-matching the mode
    /// must use [`take_mode`](Self::take_mode) + [`set_mode`](Self::set_mode) instead (the
    /// take/put-back pattern) to avoid an overlapping `&mut state.rest` borrow.
    pub fn mode_mut(&mut self) -> &mut Mode {
        &mut self.rest.fg_mut().mode
    }

    /// Take the foreground session's mode OUT, leaving a cheap [`Mode::Chat`] placeholder,
    /// and return the old value. Paired with [`set_mode`](Self::set_mode) for the
    /// mixed-borrow sites: own the mode in a local so `state.rest` is free to be borrowed
    /// alongside it, then write the (possibly mutated) mode back. `Mode::Chat` is a unit
    /// variant, so the placeholder is allocation-free.
    pub fn take_mode(&mut self) -> Mode {
        std::mem::replace(&mut self.rest.fg_mut().mode, Mode::Chat)
    }

    /// Write `mode` onto the foreground session (the put-back half of the take/put-back
    /// pattern; also the plain setter for a write that needs no concurrent `state.rest`).
    pub fn set_mode(&mut self, mode: Mode) {
        self.rest.fg_mut().mode = mode;
    }
}
