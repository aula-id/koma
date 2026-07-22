//! Session-side knowledge daemon client.
//!
//! Public API: [`proxy_push_fact`] (fire-and-forget) and [`proxy_expand`]
//! (blocking graph-expanded recall). Both degrade gracefully when the daemon
//! is unavailable — the session never fails because the knowledge daemon is down.

mod proxy;
#[allow(unused_imports)]
pub use proxy::{proxy_expand, proxy_push_fact, ExpandResult};
