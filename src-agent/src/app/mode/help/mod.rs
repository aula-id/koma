//! Help-mode types: the [`HelpState`] reference + launcher state for the
//! full-screen, searchable `/help` screen.
//!
//! Mirrors the `--resume` picker's filter/select pattern
//! ([`crate::app::mode::PickerState`]) over a static, data-driven list built
//! from the [`COMMANDS`](crate::controller::command::COMMANDS) +
//! [`KEYBINDINGS`](crate::controller::command::KEYBINDINGS) registries. There is
//! no editing and nothing to persist — Help is a read-only reference that can
//! also LAUNCH the highlighted command.

mod state;

pub use state::{HelpEntry, HelpKind, HelpState};
