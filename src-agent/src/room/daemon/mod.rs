//! Bridging the room to the live daemon: the real model and tool seams, and
//! (next) the secure daemon kind and the `/security` handoff.

pub mod builtins;
pub mod model_client;

#[allow(unused_imports)]
pub use builtins::{cyber_lane_for, CtxBuiltins};
#[allow(unused_imports)]
pub use model_client::ProviderModelClient;
