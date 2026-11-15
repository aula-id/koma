//! Host work that runs in the adapter process: `config.json` reads and
//! writes, the SDK lock on it, and the projections that keep secrets out of
//! every result.
pub mod config;
pub mod lock;
pub mod redact;
