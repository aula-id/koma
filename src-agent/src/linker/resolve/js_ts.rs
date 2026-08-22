//! JavaScript/TypeScript import resolver.
//!
//! Resolves ES module imports, CommonJS requires, and re-exports using
//! tsconfig paths, package.json exports, node_modules lookup, and
//! TypeScript extension substitution rules.

use crate::linker::config::package_json::{self, PackageJsonInfo};
use crate::linker::config::tsconfig::{self, TsConfig};
use crate::linker::path::normalize_lexical;
use crate::linker::reference::{ImportKind, ImportRef, Resolution, UnresolvedReason};

use std::collections::HashSet;
use std::path::Path;

/// Check if a candidate path is lexically within the owner root.
fn path_is_within_owner(path: &str, owner_root: &str) -> bool {
    let normalized = normalize_lexical(path);
    let owner = normalize_lexical(owner_root);
    Path::new(&normalized).starts_with(&owner)
}

fn resolved_if_owned(path: String, owner: &str) -> Resolution {
    if path_is_within_owner(&path, owner) {
        Resolution::Resolved(vec![path])
    } else {
        Resolution::Unresolved {
            reason: UnresolvedReason::OutsideWorkspace {
                normalized_path: path,
            },
        }
    }
}

/// Determine whether the moduleResolution mode allows extensionless / index
/// directory resolution.  `bundler` (and `classic`/unset) allow it; `node16`
/// and `nodenext` require explicit extensions.
fn allows_extensionless_index(module_resolution: Option<&str>) -> bool {
    match module_resolution {
        Some("node16" | "nodenext") => false,
        _ => true, // bundler, classic, unset → allow extensionless + index
    }
}

/// Context for resolving a JS/TS import.
pub struct JsTsResolveContext<'a> {
    /// The importer file path (normalized absolute).
    pub importer_path: &'a str,
    /// The nearest tsconfig/jsconfig (if found).
    pub ts_config: Option<&'a TsConfig>,
    /// The nearest package.json (if found).
    pub package_json: Option<&'a PackageJsonInfo>,
    /// Known files set for lookup.
    pub known_files: &'a HashSet<String>,
    /// Owner root for the importer.
    pub owner_root: &'a str,
}

/// Resolve a JS/TS import specifier to a structured Resolution.
pub fn resolve_js_ts_import(import_ref: &ImportRef, ctx: &JsTsResolveContext<'_>) -> Resolution {
    let specifier = &import_ref.specifier;

    // Dynamic imports cannot be statically resolved.
    if import_ref.kind == ImportKind::Dynamic {
        return Resolution::Dynamic {
            expression: specifier.clone(),
        };
    }

    if let Some(config) = ctx.ts_config {
        if let Some((path, detail)) = &config.unsupported {
            return Resolution::Unresolved {
                reason: UnresolvedReason::UnsupportedConfig {
                    path: path.clone(),
                    detail: detail.clone(),
                },
            };
        }
    }

    // Side-effect imports and type-only imports still get resolved normally
    // (they can point to real files).

    // Check for package-private # imports.
    if specifier.starts_with('#') {
        return resolve_package_private_import(specifier, ctx);
    }

    // Relative imports.
    if specifier.starts_with("./") || specifier.starts_with("../") {
        return resolve_relative_import(specifier, ctx);
    }

    // Bare specifier → could be a package import or a tsconfig alias.
    // Phase 3: owner_root containment — all candidates must be within
    // the importer's owner root unless it's an explicit package relationship.
    let owner = normalize_lexical(ctx.owner_root);
    let allow_index = allows_extensionless_index(
        ctx.ts_config
            .as_ref()
            .and_then(|c| c.module_resolution.as_deref()),
    );

    // Try tsconfig paths first (aliases like `@app/foo`).
    if let Some(config) = ctx.ts_config {
        let path_candidates = tsconfig::resolve_paths(specifier, config);
        if !path_candidates.is_empty() {
            let result = resolve_with_path_candidates(
                specifier,
                &path_candidates,
                &config.config_dir,
                ctx.known_files,
            );
            // Enforce owner containment on resolved candidates.
            if let Resolution::Resolved(ref targets) = result {
                for target in targets {
                    if !path_is_within_owner(target, &owner) {
                        return Resolution::Unresolved {
                            reason: UnresolvedReason::OutsideWorkspace {
                                normalized_path: target.clone(),
                            },
                        };
                    }
                }
            }
            return result;
        }

        // baseUrl resolution for bare specifiers.
        if let Some(ref base_url) = config.base_url_resolved {
            let candidate = format!("{base_url}/{specifier}");
            let normalized = normalize_lexical(&candidate);
            if let Some(result) =
                try_resolve_with_extensions(&normalized, ctx.known_files, allow_index)
            {
                if path_is_within_owner(&result, &owner) {
                    return Resolution::Resolved(vec![result]);
                }
                return Resolution::Unresolved {
                    reason: UnresolvedReason::OutsideWorkspace {
                        normalized_path: result,
                    },
                };
            }
        }
    }

    // Package self-import: if the package name matches the specifier
    // and we have a package.json with exports.
    if let Some(pkg) = ctx.package_json {
        if let Some(ref name) = pkg.name {
            if specifier == name || specifier.starts_with(&format!("{name}/")) {
                let subpath = if specifier == name {
                    ".".to_string()
                } else {
                    format!("./{}", &specifier[name.len() + 1..])
                };

                // Try package exports.
                if let Some(resolved) =
                    package_json::resolve_package_exports(pkg, &subpath, ctx.known_files)
                {
                    return resolved_if_owned(resolved, &owner);
                }

                // Try types/main fields for root import.
                if subpath == "." {
                    if let Some(ref types) = pkg.types {
                        let candidate = format!("{}/{}", pkg.dir, types);
                        let normalized = normalize_lexical(&candidate);
                        if ctx.known_files.contains(&normalized) {
                            return resolved_if_owned(normalized, &owner);
                        }
                    }
                    if let Some(ref module) = pkg.module {
                        let candidate = format!("{}/{}", pkg.dir, module);
                        let normalized = normalize_lexical(&candidate);
                        if ctx.known_files.contains(&normalized) {
                            return resolved_if_owned(normalized, &owner);
                        }
                    }
                    if let Some(ref main) = pkg.main {
                        let candidate = format!("{}/{}", pkg.dir, main);
                        let normalized = normalize_lexical(&candidate);
                        if ctx.known_files.contains(&normalized) {
                            return resolved_if_owned(normalized, &owner);
                        }
                    }
                }

                // Package exists but export not found → precise diagnostic.
                return Resolution::Unresolved {
                    reason: UnresolvedReason::PackageNotExported {
                        package: name.clone(),
                        subpath: if subpath == "." { None } else { Some(subpath) },
                    },
                };
            }
        }
    }

    // Try to find as a package in node_modules.
    let pkg_name = extract_package_name(specifier);
    if let Some(pkg) = find_package_for_specifier(&pkg_name, ctx.importer_path, ctx.known_files) {
        let subpath = if *specifier == pkg_name {
            ".".to_string()
        } else {
            format!("./{}", &specifier[pkg_name.len() + 1..])
        };

        if let Some(resolved) =
            package_json::resolve_package_exports(&pkg, &subpath, ctx.known_files)
        {
            return resolved_if_owned(resolved, &owner);
        }

        // Try types/main.
        if subpath == "." {
            if let Some(ref types) = pkg.types {
                let candidate = format!("{}/{}", pkg.dir, types);
                let normalized = normalize_lexical(&candidate);
                if ctx.known_files.contains(&normalized) {
                    return resolved_if_owned(normalized, &owner);
                }
            }
            if let Some(ref module) = pkg.module {
                let candidate = format!("{}/{}", pkg.dir, module);
                let normalized = normalize_lexical(&candidate);
                if ctx.known_files.contains(&normalized) {
                    return resolved_if_owned(normalized, &owner);
                }
            }
            if let Some(ref main) = pkg.main {
                let candidate = format!("{}/{}", pkg.dir, main);
                let normalized = normalize_lexical(&candidate);
                if ctx.known_files.contains(&normalized) {
                    return resolved_if_owned(normalized, &owner);
                }
            }
            return Resolution::Unresolved {
                reason: UnresolvedReason::NotFound,
            };
        }

        return Resolution::Unresolved {
            reason: UnresolvedReason::PackageNotExported {
                package: pkg_name,
                subpath: Some(subpath),
            },
        };
    }

    // Bare specifier that's not a known package → External.
    Resolution::External {
        package: specifier.clone(),
    }
}

/// Resolve a relative import specifier (starts with `./` or `../`).
fn resolve_relative_import(specifier: &str, ctx: &JsTsResolveContext<'_>) -> Resolution {
    let importer_dir = Path::new(ctx.importer_path)
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| "/".into());

    let base = normalize_lexical(&format!("{importer_dir}/{specifier}"));

    // Phase 3: owner_root containment — candidate must be within the
    // importer's owner root, otherwise OutsideWorkspace.
    let owner = normalize_lexical(ctx.owner_root);

    // Determine moduleResolution mode for extension/index behavior.
    let module_resolution = ctx.ts_config.and_then(|c| c.module_resolution.as_deref());
    let allow_index = allows_extensionless_index(module_resolution);

    // For node16/nodenext, extensionless directory index resolution is not
    // allowed.  The `allow_index` flag controls this.

    // Try exact path first.
    if ctx.known_files.contains(&base) {
        if path_is_within_owner(&base, &owner) {
            return Resolution::Resolved(vec![base]);
        }
        return Resolution::Unresolved {
            reason: UnresolvedReason::OutsideWorkspace {
                normalized_path: base,
            },
        };
    }

    // Try with TS extension substitutions.
    if let Some(result) = try_resolve_with_extensions(&base, ctx.known_files, allow_index) {
        if path_is_within_owner(&result, &owner) {
            return Resolution::Resolved(vec![result]);
        }
        return Resolution::Unresolved {
            reason: UnresolvedReason::OutsideWorkspace {
                normalized_path: result,
            },
        };
    }

    // Try as directory with index files.
    // Phase 3: moduleResolution gate — node16/nodenext disallow extensionless
    // directory index resolution.  bundler/classic/unset allow it.
    if allow_index {
        for index_ext in &[
            "/index.ts",
            "/index.tsx",
            "/index.js",
            "/index.jsx",
            "/index.mjs",
            "/index.cjs",
            "/index.mts",
            "/index.cts",
        ] {
            let candidate = format!("{base}{index_ext}");
            let normalized = normalize_lexical(&candidate);
            if ctx.known_files.contains(&normalized) {
                if path_is_within_owner(&normalized, &owner) {
                    return resolved_if_owned(normalized, &owner);
                }
                return Resolution::Unresolved {
                    reason: UnresolvedReason::OutsideWorkspace {
                        normalized_path: normalized,
                    },
                };
            }
        }
    }

    Resolution::Unresolved {
        reason: UnresolvedReason::NotFound,
    }
}

/// Resolve a package-private `#` import.
fn resolve_package_private_import(specifier: &str, ctx: &JsTsResolveContext<'_>) -> Resolution {
    let pkg = match ctx.package_json {
        Some(p) => p,
        None => {
            return Resolution::Unresolved {
                reason: UnresolvedReason::ConfigRequired {
                    detail: "package-private '#' import requires nearest package.json".into(),
                },
            }
        }
    };

    if let Some(ref imports) = pkg.imports {
        if let Some(obj) = imports.as_object() {
            // Try exact match.
            if let Some(target) = obj.get(specifier) {
                if let Some(resolved) =
                    resolve_imports_map_target(target, &pkg.dir, ctx.known_files)
                {
                    return Resolution::Resolved(vec![resolved]);
                }
            }
            // Try pattern match.
            for (pattern, target) in obj {
                if let Some(star_pos) = pattern.find('*') {
                    let prefix = &pattern[..star_pos];
                    let suffix = &pattern[star_pos + 1..];
                    if specifier.starts_with(prefix)
                        && (suffix.is_empty() || specifier.ends_with(suffix))
                    {
                        let matched = if suffix.is_empty() {
                            &specifier[prefix.len()..]
                        } else {
                            &specifier[prefix.len()..specifier.len() - suffix.len()]
                        };
                        if let Some(resolved) = resolve_imports_map_target_pattern(
                            target,
                            matched,
                            &pkg.dir,
                            ctx.known_files,
                        ) {
                            return Resolution::Resolved(vec![resolved]);
                        }
                    }
                }
            }
        }
    }

    Resolution::Unresolved {
        reason: UnresolvedReason::NotFound,
    }
}

/// Try to resolve a file path with TypeScript extension substitutions.
///
/// `allow_index` controls whether `/index.*` candidates are tried for
/// extensionless bases.  `node16`/`nodenext` callers pass `false`.
fn try_resolve_with_extensions(
    base: &str,
    known_files: &HashSet<String>,
    allow_index: bool,
) -> Option<String> {
    // Determine what extension the base has.
    let ext = Path::new(base)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let ext_with_dot = if ext.is_empty() {
        String::new()
    } else {
        format!(".{ext}")
    };

    let candidates = tsconfig::ts_extension_candidates(&ext_with_dot);

    for candidate_ext in candidates {
        // Phase 3: moduleResolution gate — skip /index.* candidates when
        // node16/nodenext disallows extensionless directory resolution.
        if !allow_index && candidate_ext.starts_with('/') {
            continue;
        }
        let path = format!("{base}{candidate_ext}");
        let normalized = normalize_lexical(&path);
        if known_files.contains(&normalized) {
            return Some(normalized);
        }
    }

    None
}

/// Resolve a path candidate list (from tsconfig paths) against known files.
fn resolve_with_path_candidates(
    _specifier: &str,
    candidates: &[String],
    config_dir: &str,
    known_files: &HashSet<String>,
) -> Resolution {
    let mut resolved = Vec::new();
    for candidate in candidates {
        let base = normalize_lexical(&format!("{config_dir}/{candidate}"));

        // Try exact.
        if known_files.contains(&base) {
            resolved.push(base);
            continue;
        }

        // Try with TS extensions.
        if let Some(path) = try_resolve_with_extensions(&base, known_files, true) {
            resolved.push(path);
            continue;
        }

        // Try directory index.
        for index_ext in &["/index.ts", "/index.tsx", "/index.js", "/index.jsx"] {
            let path = format!("{base}{index_ext}");
            let normalized = normalize_lexical(&path);
            if known_files.contains(&normalized) {
                resolved.push(normalized);
                break;
            }
        }
    }

    match resolved.len() {
        0 => Resolution::Unresolved {
            reason: UnresolvedReason::NotFound,
        },
        1 => Resolution::Resolved(resolved),
        _ => {
            resolved.dedup();
            if resolved.len() == 1 {
                Resolution::Resolved(resolved)
            } else {
                Resolution::Ambiguous {
                    candidates: resolved,
                }
            }
        }
    }
}

/// Extract the package name from a bare specifier.
/// `@scope/pkg/sub` → `@scope/pkg`
/// `lodash/fp` → `lodash`
/// `lodash` → `lodash`
/// `@scope/pkg` → `@scope/pkg`
fn extract_package_name(specifier: &str) -> String {
    if specifier.starts_with('@') {
        // Scoped package: @scope/pkg[/subpath...]
        let rest = &specifier[1..];
        if let Some(first_slash) = rest.find('/') {
            // Check for a second slash (subpackage path).
            let after_pkg = &rest[first_slash + 1..];
            if let Some(second_slash) = after_pkg.find('/') {
                // @scope/pkg/sub/deep → @scope/pkg
                specifier[..first_slash + 1 + second_slash + 1].to_string()
            } else {
                // @scope/pkg or @scope/pkg/sub → @scope/pkg
                specifier.to_string()
            }
        } else {
            // @scope (no slash after @) → treat as package name
            specifier.to_string()
        }
    } else if let Some(slash) = specifier.find('/') {
        specifier[..slash].to_string()
    } else {
        specifier.to_string()
    }
}

/// Find a package for a bare specifier by walking up node_modules.
fn find_package_for_specifier(
    specifier: &str,
    importer_path: &str,
    known_files: &HashSet<String>,
) -> Option<PackageJsonInfo> {
    let pkg_name = extract_package_name(specifier);
    let importer_dir = Path::new(importer_path).parent()?;
    package_json::find_package_in_node_modules(importer_dir, &pkg_name, known_files)
}

/// Resolve an `imports` map target value.
fn resolve_imports_map_target(
    target: &serde_json::Value,
    pkg_dir: &str,
    known_files: &HashSet<String>,
) -> Option<String> {
    match target {
        serde_json::Value::String(s) => {
            if s.starts_with('.') {
                let candidate = format!("{pkg_dir}/{}", s.trim_start_matches('.'));
                let normalized = normalize_lexical(&candidate);
                if known_files.contains(&normalized) {
                    return Some(normalized);
                }
                try_resolve_with_extensions(&normalized, known_files, true)
            } else {
                None
            }
        }
        serde_json::Value::Object(obj) => {
            for condition in &["import", "require", "default"] {
                if let Some(val) = obj.get(*condition) {
                    if let Some(r) = resolve_imports_map_target(val, pkg_dir, known_files) {
                        return Some(r);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

/// Resolve an `imports` map target with a pattern match.
fn resolve_imports_map_target_pattern(
    target: &serde_json::Value,
    matched: &str,
    pkg_dir: &str,
    known_files: &HashSet<String>,
) -> Option<String> {
    match target {
        serde_json::Value::String(s) => {
            let resolved = s.replace('*', matched);
            if resolved.starts_with('.') {
                let candidate = format!("{pkg_dir}/{}", resolved.trim_start_matches("."));
                let normalized = normalize_lexical(&candidate);
                if known_files.contains(&normalized) {
                    return Some(normalized);
                }
                try_resolve_with_extensions(&normalized, known_files, true)
            } else {
                None
            }
        }
        serde_json::Value::Object(obj) => {
            for condition in &["import", "require", "default"] {
                if let Some(val) = obj.get(*condition) {
                    if let Some(r) =
                        resolve_imports_map_target_pattern(val, matched, pkg_dir, known_files)
                    {
                        return Some(r);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "js_ts_test.rs"]
mod tests;
