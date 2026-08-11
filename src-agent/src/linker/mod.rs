//! Linker daemon: import-graph engine.
//!
//! Provides the core data structures, language extractors, and workspace scanner
//! used by the linker daemon process to build and query in-memory import graphs.

pub mod client;
pub mod config;
pub mod graph;
pub mod lang;
pub mod path;
pub mod project;
pub mod reference;
pub mod resolve;
pub mod scan;
pub mod watch;
