//! Dual-rail short-send: deterministic archive index plus a bounded live tail.
//! Only the outgoing request is shaped. Original messages and the visible
//! conversation are preserved; SQLite stores derived search/boundary metadata.

pub(crate) mod budget;
mod goal;
mod window;

#[cfg(test)]
use goal::GoalWire;
pub use goal::{load_mission_snap, refresh_goal_state};
pub use window::shape;

/// Shared conservative fallback for callers without the exact tool schema set.
pub(crate) fn estimate_prompt_tokens_for_max_clamp(
    history: &[crate::dto::chat::ChatMessage],
) -> u64 {
    budget::prompt_tokens(history, 12_000)
}

#[cfg(test)]
mod tests;
