//! In-app `/extension` dashboard mode: list installed extensions → per-extension detail →
//! uninstall (with confirm), and — for extensions that declare `contributes.tui_screens` —
//! a jump into the extension-driven [`crate::app::mode::ExtScreenState`] full-screen view.
//!
//! A read-only sibling of `/mcp` (no editor/create): rows are a snapshot rebuilt from the
//! registry + manifests, so key handling is Browse → Detail → UninstallConfirm.

mod state;
mod types;

pub use state::{ExtRow, ExtTuiScreen, ExtensionsState};
pub use types::ExtSubMode;
