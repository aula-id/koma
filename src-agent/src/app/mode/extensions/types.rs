//! Sub-mode state machine for the `/extension` dashboard.
//!
//! A simpler sibling of the `/mcp` types module: there is no editor/create flow (an
//! extension is installed via the `/store` wave, not authored here), so the only moving
//! parts are Browse → Detail → UninstallConfirm.

/// The active sub-mode of the `/extension` dashboard.
///
/// ```text
///   Browse ── →/Enter ──▶ Detail ── u ──▶ UninstallConfirm ── y ──▶ uninstall ──▶ Browse
///     │                     │
///     └── Esc ── chat       └── Enter (on a tui-screen row) ──▶ Mode::ExtScreen
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtSubMode {
    /// Navigating the installed-extension list (read-only).
    Browse,
    /// Reading one extension's full detail (contributions, grants, tui-screens).
    Detail,
    /// Confirming an uninstall of the selected extension (`y`/`n`).
    UninstallConfirm,
}
