//! Shared interface for language-specific import resolvers.

pub mod c_family;
pub mod go;
pub mod js_ts;
pub mod python;

use crate::linker::project::ProjectIndex;
use crate::linker::reference::{ImportRef, Resolution};

pub struct ResolveContext<'a> {
    pub importer: &'a str,
    pub project: &'a ProjectIndex,
}

/// A language resolver maps one extracted import to a structured outcome.
#[allow(dead_code)] // Planned interface; per-language resolvers will implement this.
pub trait ImportResolver {
    fn resolve(&self, import: &ImportRef, context: &ResolveContext<'_>) -> Resolution;
}
