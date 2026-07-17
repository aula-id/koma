//! Sub-mode state machine for the `/store` marketplace browser.
//!
//! A network-backed sibling of `app::mode::extensions::ExtSubMode`: Browse (async
//! catalogue fetch) -> Detail (async detail fetch) -> InstallConfirm (y/n, gated on a
//! koma.run OAuth sign-in).
//!
//! ```text
//!   Browse ── Enter ──▶ Detail ── i ──▶ InstallConfirm ── y ──▶ install ──▶ Detail
//!     │                    │
//!     └── Esc ── chat      └── Esc ── Browse
//! ```

/// The active sub-mode of the `/store` marketplace browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreSubMode {
    /// Navigating the fetched catalogue list.
    Browse,
    /// Reading one extension's full detail (description, contributions, requires,
    /// versions).
    Detail,
    /// Confirming an install of the selected (not-yet-installed) extension (`y`/`n`).
    InstallConfirm,
}
