//! C/C++ import resolver.
//!
//! Resolves `#include` directives using flags already selected for the importer
//! from compile_commands.json or compile_flags.txt.

use crate::linker::config::CompileDB;
use crate::linker::config::CompileFlags;
use crate::linker::path::normalize_lexical;
use crate::linker::reference::{ImportKind, ImportRef, Resolution, UnresolvedReason};

use std::collections::HashSet;
use std::path::Path;

/// Context for resolving C/C++ references.
pub struct CFamilyResolveContext<'a> {
    /// The importer file path (normalized absolute).
    pub importer_path: &'a str,
    /// Flags already selected for this importer. A compile database entry takes
    /// precedence over the workspace compile_flags.txt fallback at the caller.
    pub compile_flags: Option<&'a CompileFlags>,
    /// Known files set for lookup.
    pub known_files: &'a HashSet<String>,
    /// Workspace root which owns the importer.
    pub owner_root: &'a str,
}

/// Resolve a C/C++ include or module reference.
pub fn resolve_c_include(import_ref: &ImportRef, ctx: &CFamilyResolveContext<'_>) -> Resolution {
    if import_ref.kind == ImportKind::ModuleDecl {
        return Resolution::Unresolved {
            reason: UnresolvedReason::ConfigRequired {
                detail: format!(
                    "C++ named module '{}' requires compiler module mapping metadata",
                    import_ref.specifier
                ),
            },
        };
    }

    let is_quoted = import_ref.kind == ImportKind::IncludeQuoted;
    let is_angle = import_ref.kind == ImportKind::IncludeAngle;
    if !is_quoted && !is_angle {
        return Resolution::Unresolved {
            reason: UnresolvedReason::UnsupportedSyntax {
                detail: "non-standard C/C++ reference form".into(),
            },
        };
    }

    let importer_dir = match Path::new(ctx.importer_path).parent() {
        Some(parent) => parent.to_string_lossy().replace('\\', "/"),
        None => "/".into(),
    };
    let search_paths = build_search_paths(is_quoted, &importer_dir, ctx.compile_flags);
    let owner_root = normalize_lexical(ctx.owner_root);
    let mut candidates = Vec::new();

    for search_dir in &search_paths {
        let candidate = normalize_lexical(&format!("{search_dir}/{}", import_ref.specifier));
        let search_is_owned = path_is_within(search_dir, &owner_root);
        let candidate_is_owned = path_is_within(&candidate, &owner_root);
        if (search_is_owned && !candidate_is_owned)
            || (ctx.known_files.contains(&candidate) && !candidate_is_owned)
        {
            return Resolution::Unresolved {
                reason: UnresolvedReason::OutsideWorkspace {
                    normalized_path: candidate,
                },
            };
        }
        if candidate_is_owned && ctx.known_files.contains(&candidate) {
            candidates.push(candidate);
        }
    }

    match candidates.len() {
        0 if is_angle && !looks_local(&import_ref.specifier) => Resolution::External {
            package: import_ref.specifier.clone(),
        },
        0 if is_angle && ctx.compile_flags.is_none() => Resolution::Unresolved {
            reason: UnresolvedReason::ConfigRequired {
                detail: format!(
                    "local-looking angle include '{}' needs compile_commands.json or compile_flags.txt",
                    import_ref.specifier
                ),
            },
        },
        0 => Resolution::Unresolved {
            reason: UnresolvedReason::NotFound,
        },
        1 => match candidates.into_iter().next() {
            Some(candidate) => Resolution::Resolved(vec![candidate]),
            None => Resolution::Unresolved {
                reason: UnresolvedReason::NotFound,
            },
        },
        _ => Resolution::Ambiguous { candidates },
    }
}

fn looks_local(specifier: &str) -> bool {
    specifier.starts_with('.') || specifier.contains('/') || specifier.contains('\\')
}

fn path_is_within(path: &str, root: &str) -> bool {
    Path::new(&normalize_lexical(path)).starts_with(Path::new(root))
}

/// Build search directories in compiler order.
/// Quoted: importer, -iquote, -I. Angle: -I, -isystem.
fn build_search_paths(
    is_quoted: bool,
    importer_dir: &str,
    compile_flags: Option<&CompileFlags>,
) -> Vec<String> {
    let mut paths = Vec::new();
    if is_quoted {
        paths.push(importer_dir.to_string());
    }
    if let Some(flags) = compile_flags {
        if is_quoted {
            paths.extend(flags.iquote.iter().cloned());
        }
        paths.extend(flags.include_paths.iter().cloned());
        if !is_quoted {
            paths.extend(flags.isystem.iter().cloned());
        }
    }
    let mut seen = HashSet::new();
    paths.retain(|path| seen.insert(path.clone()));
    paths
}

/// Determine the C/C++ language for a source or header file.
///
/// For `.h` files, uses the compile database entry's `-x` flag to determine
/// whether it's C or C++.  Falls back to `None` for ambiguous `.h` files
/// without a compile DB entry.
///
/// Retained as public API.  Scan code performs equivalent inline header
/// reclassification using `ProjectIndex` config caches.
#[allow(dead_code)] // Public API; scan does inline reclassification.
pub fn detect_header_language(
    file_path: &str,
    compile_db: Option<&CompileDB>,
) -> Option<crate::linker::graph::Lang> {
    let path = file_path.to_lowercase();
    if path.ends_with(".c") {
        return Some(crate::linker::graph::Lang::C);
    }
    if path.ends_with(".cpp")
        || path.ends_with(".cc")
        || path.ends_with(".cxx")
        || path.ends_with(".hpp")
        || path.ends_with(".hxx")
        || path.ends_with(".hh")
    {
        return Some(crate::linker::graph::Lang::Cpp);
    }
    if !path.ends_with(".h") {
        return None;
    }
    let entry = compile_db.and_then(|db| db.lookup(file_path))?;
    let flags = entry.extract_flags();
    match flags.language_mode.as_deref() {
        Some("c" | "c-header") => Some(crate::linker::graph::Lang::C),
        Some("c++" | "c++-header") => Some(crate::linker::graph::Lang::Cpp),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linker::reference::ByteSpan;

    fn import(specifier: &str, kind: ImportKind) -> ImportRef {
        ImportRef {
            specifier: specifier.into(),
            kind,
            span: Some(ByteSpan { start: 0, end: 1 }),
            condition: None,
        }
    }

    fn context<'a>(
        flags: Option<&'a CompileFlags>,
        files: &'a HashSet<String>,
    ) -> CFamilyResolveContext<'a> {
        CFamilyResolveContext {
            importer_path: "/owner/src/main.c",
            compile_flags: flags,
            known_files: files,
            owner_root: "/owner",
        }
    }

    #[test]
    fn search_order_matches_compiler_rules() {
        let flags = CompileFlags {
            language_mode: None,
            iquote: vec!["/q".into()],
            include_paths: vec!["/i".into()],
            isystem: vec!["/s".into()],
        };
        assert_eq!(
            build_search_paths(true, "/src", Some(&flags)),
            vec!["/src", "/q", "/i"]
        );
        assert_eq!(
            build_search_paths(false, "/src", Some(&flags)),
            vec!["/i", "/s"]
        );
    }

    #[test]
    fn no_guessed_versioned_system_paths() {
        assert!(build_search_paths(false, "/src", None).is_empty());
    }

    #[test]
    fn escaping_owner_is_outside_even_when_other_root_knows_file() {
        let files = HashSet::from(["/other/secret.h".to_string()]);
        let result = resolve_c_include(
            &import("../../other/secret.h", ImportKind::IncludeQuoted),
            &context(None, &files),
        );
        assert!(matches!(
            result,
            Resolution::Unresolved {
                reason: UnresolvedReason::OutsideWorkspace { .. }
            }
        ));
    }

    #[test]
    fn missing_headers_are_classified_precisely() {
        let files = HashSet::new();
        assert!(matches!(
            resolve_c_include(
                &import("vector", ImportKind::IncludeAngle),
                &context(None, &files)
            ),
            Resolution::External { .. }
        ));
        assert!(matches!(
            resolve_c_include(
                &import("sdk/header.h", ImportKind::IncludeAngle),
                &context(None, &files)
            ),
            Resolution::Unresolved {
                reason: UnresolvedReason::ConfigRequired { .. }
            }
        ));
        assert!(matches!(
            resolve_c_include(
                &import("missing.h", ImportKind::IncludeQuoted),
                &context(None, &files)
            ),
            Resolution::Unresolved {
                reason: UnresolvedReason::NotFound
            }
        ));
    }

    #[test]
    fn configured_missing_local_angle_is_not_found() {
        let files = HashSet::new();
        let flags = CompileFlags {
            include_paths: vec!["/owner/include".into()],
            ..CompileFlags::default()
        };
        assert!(matches!(
            resolve_c_include(
                &import("local/header.h", ImportKind::IncludeAngle),
                &context(Some(&flags), &files)
            ),
            Resolution::Unresolved {
                reason: UnresolvedReason::NotFound
            }
        ));
    }

    #[test]
    fn named_module_requires_mapping() {
        let files = HashSet::new();
        assert!(matches!(
            resolve_c_include(
                &import("std.core", ImportKind::ModuleDecl),
                &context(None, &files)
            ),
            Resolution::Unresolved {
                reason: UnresolvedReason::ConfigRequired { .. }
            }
        ));
    }

    #[test]
    fn detects_hh_and_ambiguous_h() {
        assert_eq!(
            detect_header_language("foo.hh", None),
            Some(crate::linker::graph::Lang::Cpp)
        );
        assert_eq!(detect_header_language("foo.h", None), None);
    }
}
