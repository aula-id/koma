//! SDLC mode model types: Mission contract (L1) and graph projection.
//!
//! The mission contract (`mission.json`) is the frozen source of truth for an
//! SDLC session's goals, acceptance criteria, and phase. The graph (`sdlc_nodes`
//! + `sdlc_events`) lives in `messages.sqlite` alongside the chat log and is
//! the sole SDLC checklist authority. `memory/TODO.md` is for ordinary project
//! todos only — never an SDLC projection.

pub mod branch_name;
pub mod decompose;
pub mod graph;
pub mod handoff;
pub mod history;
pub mod integrate;
pub mod keeper;
pub mod lane;
pub mod mission;

pub use mission::Mission;
