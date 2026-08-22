//! Shared structured import reference types.
//!
//! An `ImportRef` captures the full lexical and semantic information for a
//! single import statement, while `Resolution` records the outcome of
//! resolving it against the project index.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Semantic import kind — covers all supported import mechanisms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ImportKind {
    /// Standard `use`, `import`, `require` etc.
    Static,
    /// `import type { ... }` / type-only import.
    TypeOnly,
    /// `pub use` / re-export.
    ReExport,
    /// Side-effect import: `import 'foo'`.
    SideEffect,
    /// `import('foo')` / dynamic import expression.
    Dynamic,
    /// `mod foo;` (Rust module declaration).
    ModuleDecl,
    /// `#include "path"` (C/C++ quoted include).
    IncludeQuoted,
    /// `#include <path>` (C/C++ system/angle include).
    IncludeAngle,
    /// `package:foo/bar.dart` / npm package reference.
    PackageImport,
    /// `part` statement (Dart).
    Part,
    /// `part of` statement (Dart).
    PartOf,
    /// `require()` in CommonJS-style module dependencies.
    ModuleRequires,
}

impl fmt::Display for ImportKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Static => write!(f, "Static"),
            Self::TypeOnly => write!(f, "TypeOnly"),
            Self::ReExport => write!(f, "ReExport"),
            Self::SideEffect => write!(f, "SideEffect"),
            Self::Dynamic => write!(f, "Dynamic"),
            Self::ModuleDecl => write!(f, "ModuleDecl"),
            Self::IncludeQuoted => write!(f, "IncludeQuoted"),
            Self::IncludeAngle => write!(f, "IncludeAngle"),
            Self::PackageImport => write!(f, "PackageImport"),
            Self::Part => write!(f, "Part"),
            Self::PartOf => write!(f, "PartOf"),
            Self::ModuleRequires => write!(f, "ModuleRequires"),
        }
    }
}

/// Byte span within a source file (0-based offsets).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ByteSpan {
    /// Start byte offset (inclusive).
    pub start: usize,
    /// End byte offset (exclusive).
    pub end: usize,
}

/// Language-specific import metadata, preserving structured information
/// that would be lost if encoded back into fragile strings.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ImportMeta {
    /// Python import-specific metadata.
    Python(PythonMeta),
    /// Go import-specific metadata.
    Go(GoMeta),
}

/// Python-specific import metadata.
///
/// Preserves relative import level, module path, and imported names
/// separately without encoding them back into the specifier string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PythonMeta {
    /// Relative import depth: 0 = absolute, 1 = `.`, 2 = `..`, etc.
    pub level: u32,
    /// Module component after dots (e.g., for `from ..pkg import foo`,
    /// module = Some("pkg")). `None` for bare `from . import foo`.
    pub module: Option<String>,
    /// Names imported (e.g., `from x import a, b` → `["a", "b"]`).
    /// Empty for `import x` (which imports the module itself).
    #[serde(default)]
    pub names: Vec<String>,
}

/// Go-specific import metadata.
///
/// Preserves import alias, dot/blank import status, and build constraints.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GoMeta {
    /// Local alias if present: `import alias "pkg"` → `Some("alias")`.
    /// Blank import `import _ "pkg"` → `Some("_")`.
    /// Dot import `import . "pkg"` → `Some(".")`.
    #[serde(default)]
    pub alias: Option<String>,
    /// `//go:build` and legacy `// +build` constraints.
    #[serde(default)]
    pub conditions: Vec<String>,
}

/// A structured import reference extracted from source code.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ImportRef {
    /// The raw import specifier string.
    pub specifier: String,
    /// Semantic import kind.
    pub kind: ImportKind,
    /// Optional byte span of the import statement in the source.
    #[serde(default)]
    pub span: Option<ByteSpan>,
    /// Optional compilation condition (e.g. `cfg(test)`, `cfg(feature = "x")`).
    #[serde(default)]
    pub condition: Option<String>,
}

/// Outcome of resolving an import specifier against the project.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Resolution {
    /// Successfully resolved to one or more file paths.
    Resolved(Vec<String>),
    /// External package — not a local file.
    External {
        /// Package name (e.g. "serde", "lodash").
        package: String,
    },
    /// Multiple candidate files matched (ambiguous).
    Ambiguous {
        /// Candidate file paths.
        candidates: Vec<String>,
    },
    /// Dynamic import — expression cannot be statically resolved.
    Dynamic {
        /// The dynamic expression string.
        expression: String,
    },
    /// Failed to resolve for a specific reason.
    Unresolved {
        /// Structured reason (not a string, for precise diagnostics).
        reason: UnresolvedReason,
    },
}

/// Why an import could not be resolved. Serializable and equatable for
/// diagnostic storage without string conflation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UnresolvedReason {
    /// No matching file found in any workspace root.
    NotFound,
    /// Resolved path escapes all registered workspace roots.
    OutsideWorkspace {
        /// The normalized path that escapes.
        normalized_path: String,
    },
    /// Multiple candidates with equal specificity.
    MultipleCandidates {
        /// Candidate file paths.
        paths: Vec<String>,
    },
    /// Specifier could not be parsed.
    ParseError {
        /// Description of the parse error.
        detail: String,
    },
    /// Language syntax not yet supported for resolution.
    UnsupportedSyntax {
        /// Description of the unsupported syntax.
        detail: String,
    },
    /// Requires compiler/project configuration that is missing or unparsable.
    ///
    /// For C/C++: include is local-looking but no compile_commands.json or
    /// compile_flags.txt provides the search path. For C++ named modules:
    /// compiler metadata needed to map the module name to a file.
    /// For JS/TS: a path-mapping alias requires a tsconfig that couldn't be found.
    ConfigRequired {
        /// What configuration is needed.
        detail: String,
    },
    /// A package import resolved to a package but the export path was not found.
    ///
    /// The package exists (has a package.json) but the requested subpath is
    /// not listed in `exports`, `types`, `typings`, or `main`.
    PackageNotExported {
        /// The package name.
        package: String,
        /// The requested subpath, if any.
        subpath: Option<String>,
    },
    /// A configuration file was found but could not be parsed or is unsupported.
    UnsupportedConfig {
        /// Path to the config file.
        path: String,
        /// Why it is unsupported.
        detail: String,
    },
}

impl fmt::Display for UnresolvedReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => write!(f, "not found"),
            Self::OutsideWorkspace { normalized_path } => {
                write!(f, "escapes workspace: {normalized_path}")
            }
            Self::MultipleCandidates { paths } => {
                write!(f, "ambiguous: {} candidates", paths.len())
            }
            Self::ParseError { detail } => write!(f, "parse error: {detail}"),
            Self::UnsupportedSyntax { detail } => write!(f, "unsupported: {detail}"),
            Self::ConfigRequired { detail } => write!(f, "config required: {detail}"),
            Self::PackageNotExported { package, subpath } => match subpath {
                Some(sp) => write!(f, "package '{package}' does not export '{sp}'"),
                None => write!(f, "package '{package}' has no exports"),
            },
            Self::UnsupportedConfig { path, detail } => {
                write!(f, "unsupported config '{path}': {detail}")
            }
        }
    }
}

/// An import and its resolution, kept as one record so the two values cannot
/// become misaligned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedImport {
    pub import_ref: ImportRef,
    pub resolution: Resolution,
    /// Language-specific structured metadata (Python level/names, Go alias/conditions).
    /// Stored here instead of on ImportRef to avoid breaking existing ImportRef
    /// constructors across all language extractors.
    #[serde(default)]
    pub meta: Option<ImportMeta>,
}

/// Structured references belonging to one source file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRefs {
    #[serde(default)]
    pub entries: Vec<ResolvedImport>,
}

impl SourceRefs {
    pub fn push(&mut self, import_ref: ImportRef, resolution: Resolution) {
        self.entries.push(ResolvedImport {
            import_ref,
            resolution,
            meta: None,
        });
    }

    /// Push an import with language-specific metadata.
    pub fn push_with_meta(
        &mut self,
        import_ref: ImportRef,
        resolution: Resolution,
        meta: Option<ImportMeta>,
    ) {
        self.entries.push(ResolvedImport {
            import_ref,
            resolution,
            meta,
        });
    }

    #[allow(dead_code)]
    fn count(&self, predicate: impl Fn(&Resolution) -> bool) -> usize {
        self.entries
            .iter()
            .filter(|entry| predicate(&entry.resolution))
            .count()
    }

    #[allow(dead_code)]
    pub fn resolved_count(&self) -> usize {
        self.count(|r| matches!(r, Resolution::Resolved(_)))
    }

    #[allow(dead_code)]
    pub fn external_count(&self) -> usize {
        self.count(|r| matches!(r, Resolution::External { .. }))
    }

    #[allow(dead_code)]
    pub fn ambiguous_count(&self) -> usize {
        self.count(|r| matches!(r, Resolution::Ambiguous { .. }))
    }

    #[allow(dead_code)]
    pub fn dynamic_count(&self) -> usize {
        self.count(|r| matches!(r, Resolution::Dynamic { .. }))
    }

    #[allow(dead_code)]
    pub fn unresolved_count(&self) -> usize {
        self.count(|r| matches!(r, Resolution::Unresolved { .. }))
    }
}

#[cfg(test)]
#[path = "reference_test.rs"]
mod tests;
