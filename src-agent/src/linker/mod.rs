//! Linker daemon: import-graph engine.
//!
//! Provides the core data structures, language extractors, and workspace scanner
//! used by the linker daemon process to build and query in-memory import graphs.

pub mod client;
pub mod graph;
pub mod lang;
pub mod scan;
pub mod watch;

// Re-exports for convenience.
pub use graph::{Edge, EdgeKind, EdgeTarget, ImportGraph, Lang};
pub use lang::{detect_lang, extract_imports, SOURCE_EXTENSIONS};
pub use scan::scan_roots;
