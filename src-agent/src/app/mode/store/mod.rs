//! In-app `/store` marketplace browser mode: browse the koma.run extension catalogue ->
//! read one's detail -> confirm + install (requires a koma.run OAuth sign-in).
//!
//! Cloned from the `/extension` triad (see `crate::app::mode::extensions`), swapping the
//! read-only LOCAL registry snapshot for a NETWORK-fetched catalogue: Browse -> Detail ->
//! InstallConfirm. Installing hands off to the shared `install_extension_core` the daemon
//! store hub also drives (see `app::runtime::actions::ext_install`), so a TUI-installed
//! extension is byte-identical to a GUI-installed one.

mod state;
mod types;

pub use state::{ExtStoreState, StoreDetailData, StoreRow};
pub use types::StoreSubMode;
