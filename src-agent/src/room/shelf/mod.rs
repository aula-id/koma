//! The shelf: the room's shared, revisioned memory and the only channel
//! between agents. Ported from Orpheus `db/src/shelf.ts`.
//!
//! Left out on purpose: the CWE "speculation" follow-up filing on confirmed
//! verdicts. It needs the CWE relation catalog and is not part of the gate.

mod args;
mod links;
mod record;
mod reporter;
mod schema;
mod similar;
mod store;
mod trajectory;

#[cfg(test)]
mod tests;

// Public surface for the coordinator tools and the trajectory view (later steps).
#[allow(unused_imports)]
pub use schema::typed_tool;
#[allow(unused_imports)]
pub use similar::near as claims_are_near;
#[allow(unused_imports)]
pub use store::{Shelf, ShelfEntry, ShelfEvent};

/// A shelf rule or storage failure, as text the model can act on.
#[derive(Debug, Clone, PartialEq)]
pub struct ShelfError(pub String);

impl std::fmt::Display for ShelfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ShelfError {}

impl From<rusqlite::Error> for ShelfError {
    fn from(e: rusqlite::Error) -> Self {
        ShelfError(format!("shelf storage error: {e}"))
    }
}

impl From<serde_json::Error> for ShelfError {
    fn from(e: serde_json::Error) -> Self {
        ShelfError(format!("shelf encoding error: {e}"))
    }
}
