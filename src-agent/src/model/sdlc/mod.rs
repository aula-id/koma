//! SDLC mode model types: Mission contract (L1) and graph projection.
//!
//! The mission contract (`mission.json`) is the frozen source of truth for an
//! SDLC session's goals, acceptance criteria, and phase. The graph (`sdlc_nodes`
//! + `sdlc_events`) lives in `messages.sqlite` alongside the chat log and
//!   provides the live checklist/status authority (TODO.md is projection only).

pub mod decompose;
pub mod graph;
pub mod integrate;
pub mod keeper;
pub mod mission;

pub use mission::Mission;
