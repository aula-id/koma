//! Agents-mode types: the sub-mode state machine and the [`AgentsState`] draft
//! holder for the in-app `/agents` management dashboard.
//!
//! The dashboard is modelled on `/settings`: a LIST + DETAIL two-pane layout
//! with a small state machine layered on top. The data layer
//! ([`crate::model::agent_def`]) owns load/save/delete; this module only holds
//! the working drafts and navigation state, applying them via the data-layer
//! API on confirm (see `app::runtime::actions`).
//!
//! Sub-mode state machine ([`AgentSubMode`]):
//!
//! ```text
//!   Browse ── →/Enter (file-backed) ──▶ Edit ── s ──▶ save ──▶ Browse
//!     │                                   │
//!     │── n ──▶ Create ── s ──▶ create ──▶ Browse
//!     │
//!     └── d (file-backed) ──▶ DeleteConfirm ── y ──▶ delete ──▶ Browse
//! ```
//!
//! Built-in agents (`AgentSource::Builtin`, `file_path == None`) can be edited;
//! saving creates a session-scoped override that shadows the built-in. They cannot
//! be deleted directly (only disabled via a disable flag).

mod picker;
mod state;
mod types;

pub use picker::{ModelPickerState, ToolPickerState};
pub use state::{source_label, AgentsState};
pub use types::{AgentEditField, AgentScope, AgentSubMode};
