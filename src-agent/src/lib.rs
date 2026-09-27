//! Native linkage shared by the package's library and executable targets.
//!
//! Cargo attaches build-script `rustc-link-lib` directives to this library.
//! The executable must import the FFI here so the native archive, C++ runtime
//! and platform frameworks propagate to its final link.

#[cfg(all(feature = "gui", any(target_os = "macos", target_os = "windows")))]
pub mod computer_native;
