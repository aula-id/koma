//! Sync-loop <-> per-client bridge and render-state streaming engine.
//!
//! [`DaemonHub`] owns the inbound message receiver and the enrolled client
//! registry; submodules add `impl DaemonHub` blocks for requests, streaming,
//! and the session-lifecycle helpers (`repoint_foreground_off_closed`,
//! `close_all_sessions`) — now methods on [`DaemonHub`] (C1.5) so they can repoint
//! the per-client foreground pointers alongside the global one.

#![allow(unused_imports)]
#![allow(dead_code)]

mod core;
mod requests;
mod streaming;

pub(crate) use core::HubInbound;
pub(in crate::app::runtime) use core::DaemonHub;
