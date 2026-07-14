//! `koma-extension` — the public, v0/unstable Rust SDK for building koma
//! extensions. See the crate README for an overview and `docs/EXTENSIONS.md`
//! in the koma repo for the direction this is heading.
//!
//! This crate has two parts:
//! - [`protocol`]: the wire types (manifest, handshake, envelopes).
//! - [`sdk`]: a thin helper layer, including a standalone demo mode used by
//!   the samples in `example/` since there is no host to connect to yet.

pub mod protocol;
pub mod sdk;

pub use protocol::{
    Contributes, ExtMsg, ExtensionKind, ExtensionManifest, Grant, KomaMsg, ModelDef, PanelDef,
    RunInfo, Runtime, SubAgentDef, Tier, ToolDef, MANIFEST_SCHEMA, PROTOCOL_VERSION,
};
pub use sdk::{run_daemon, run_oneshot, DaemonDemo, Extension, Koma, OneshotDemo};
