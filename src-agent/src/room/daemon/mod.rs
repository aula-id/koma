//! Bridging the room to the live daemon: the real model and tool seams, and
//! the secure daemon kind (`koma --daemon --kind secure`).

pub mod assemble;
pub mod builtins;
pub mod model_client;
pub mod models;
mod run;
mod selftest;
mod serve;

#[cfg(test)]
#[path = "host_test.rs"]
mod host_test;
#[cfg(test)]
#[path = "wire_test.rs"]
mod wire_test;

#[allow(unused_imports)]
pub use assemble::{RoomSession, TICK_MS};
#[allow(unused_imports)]
pub use builtins::CtxBuiltins;
#[allow(unused_imports)]
pub use model_client::ProviderModelClient;
pub use run::run_secure_daemon;
