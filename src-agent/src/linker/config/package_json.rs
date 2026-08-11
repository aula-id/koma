//! Package.json parsing for import resolution.
//!
//! Extracts `name`, `exports`, `types`/`typings`, `main`, `module`, and
//! `imports` fields used by the JS/TS resolver for package self-imports,
//! subpath exports, and `#` imports.

use crate::linker::path::normalize_lexical;
use std::collections::HashSet;
use std::path::Path;

/// Parsed package.json metadata for import resolution.
#[derive(Debug, Clone, Default)]
pub struct PackageJsonInfo {
    /// The `name` field (e.g. `@myorg/pkg`).
    pub name: Option<String>,
    /// The `exports` field, if present.
    pub exports: Option<serde_json::Value>,
    /// The `types`/`typings` field.
    pub types: Option<String>,
    /// The `main` field.
    pub main: Option<String>,
    /// The `module` field (ESM entry point).
    pub module: Option<String>,
    /// The `imports` field for package-private `#` imports.
    pub imports: Option<serde_json::Value>,
    /// The directory containing this package.json.
    pub dir: String,
}

/// Parse a package.json file into PackageJsonInfo.
pub fn parse_package_json_file(path: &Path, dir: &Path) -> Option<PackageJsonInfo> {
    let content = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;

    let dir_s = normalize_lexical(&dir.to_string_lossy().replace('\\', "/"));

    Some(PackageJsonInfo {
        name: value.get("name").and_then(|v| v.as_str()).map(String::from),
        exports: value.get("exports").cloned(),
        types: value
            .get("types")
            .or_else(|| value.get("typings"))
            .and_then(|v| v.as_str())
            .map(String::from),
        main: value.get("main").and_then(|v| v.as_str()).map(String::from),
        module: value
            .get("module")
            .and_then(|v| v.as_str())
            .map(String::from),
        imports: value.get("imports").cloned(),
        dir: dir_s,
    })
}

/// Find and parse the nearest package.json upward from a directory.
///
/// Retained as public API.  Scan code now uses `ProjectIndex` config
/// caches instead of per-file filesystem walks.
#[allow(dead_code)] // Public API; scan uses ProjectIndex caches.
pub fn find_package_json(
    start_dir: &Path,
    known_files: &HashSet<String>,
) -> Option<PackageJsonInfo> {
    let mut dir = start_dir.to_path_buf();
    loop {
        let path = dir.join("package.json");
        let path_s = normalize_lexical(&path.to_string_lossy().replace('\\', "/"));
        if known_files.contains(&path_s) || path.exists() {
            return parse_package_json_file(&path, &dir);
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

/// Walk upward to find the node_modules/package.json for a given package name.
pub fn find_package_in_node_modules(
    start_dir: &Path,
    package_name: &str,
    known_files: &HashSet<String>,
) -> Option<PackageJsonInfo> {
    let mut dir = start_dir.to_path_buf();
    loop {
        let pkg_path = dir
            .join("node_modules")
            .join(package_name)
            .join("package.json");
        let pkg_path_s = normalize_lexical(&pkg_path.to_string_lossy().replace('\\', "/"));
        if known_files.contains(&pkg_path_s) || pkg_path.exists() {
            let pkg_dir = dir.join("node_modules").join(package_name);
            return parse_package_json_file(&pkg_path, &pkg_dir);
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

/// Resolve a package's `exports` field for a given subpath.
///
/// Returns the resolved file path within the package directory, or None
/// if the exports map doesn't match the subpath.
pub fn resolve_package_exports(
    pkg: &PackageJsonInfo,
    subpath: &str,
    known_files: &HashSet<String>,
) -> Option<String> {
    let exports = pkg.exports.as_ref()?;

    // Try direct export map: "exports": { ".": "./dist/index.js", "./foo": "./foo.js" }
    if let Some(obj) = exports.as_object() {
        // Exact subpath match.
        if let Some(target) = obj.get(subpath) {
            return resolve_export_target(target, &pkg.dir, known_files);
        }
        // Pattern match: "exports": { "./*": "./dist/*" }
        for (pattern, target) in obj {
            if let Some(star_pos) = pattern.find('*') {
                let prefix = &pattern[..star_pos];
                let suffix = &pattern[star_pos + 1..];
                if subpath.starts_with(prefix) && subpath.ends_with(suffix) {
                    let matched = &subpath[prefix.len()..subpath.len() - suffix.len()];
                    if let Some(t) = target.as_str() {
                        let resolved = t.replace('*', matched);
                        let candidate = format!("{}/{}", pkg.dir, resolved.trim_start_matches("."));
                        let normalized = normalize_lexical(&candidate);
                        if known_files.contains(&normalized) {
                            return Some(normalized);
                        }
                    }
                }
            }
        }
    }

    // Try conditional exports: "exports": { ".": { "import": "...", "require": "..." } }
    if let Some(obj) = exports.as_object() {
        if let Some(target) = obj.get(subpath) {
            if let Some(condition_obj) = target.as_object() {
                // Use a conservative deterministic condition order.
                for condition in &["import", "require", "default"] {
                    if let Some(cond_val) = condition_obj.get(*condition) {
                        if let Some(result) = resolve_export_target(cond_val, &pkg.dir, known_files)
                        {
                            return Some(result);
                        }
                    }
                }
            }
        }
    }

    None
}

/// Resolve an export target value to a file path.
fn resolve_export_target(
    target: &serde_json::Value,
    pkg_dir: &str,
    known_files: &HashSet<String>,
) -> Option<String> {
    match target {
        serde_json::Value::String(s) => {
            let candidate = format!("{}/{}", pkg_dir, s.trim_start_matches("."));
            let normalized = normalize_lexical(&candidate);
            if known_files.contains(&normalized) {
                Some(normalized)
            } else {
                None
            }
        }
        serde_json::Value::Object(obj) => {
            // Conditional: try "import", "require", "default".
            for condition in &["import", "require", "default"] {
                if let Some(val) = obj.get(*condition) {
                    if let Some(result) = resolve_export_target(val, pkg_dir, known_files) {
                        return Some(result);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn parse_package_json_basic() {
        let tmp = std::env::temp_dir().join(format!(
            "koma-linker-test-pkgparse-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("package.json"),
            r#"{
                "name": "my-pkg",
                "main": "dist/index.js",
                "types": "dist/index.d.ts",
                "module": "dist/index.mjs"
            }"#,
        )
        .unwrap();
        let info = parse_package_json_file(&tmp.join("package.json"), &tmp).unwrap();
        assert_eq!(info.name.as_deref(), Some("my-pkg"));
        assert_eq!(info.main.as_deref(), Some("dist/index.js"));
        assert_eq!(info.types.as_deref(), Some("dist/index.d.ts"));
        assert_eq!(info.module.as_deref(), Some("dist/index.mjs"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn resolve_package_exports_exact() {
        let tmp = std::env::temp_dir().join(format!(
            "koma-linker-test-exports-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let dist = tmp.join("dist");
        std::fs::create_dir_all(&dist).unwrap();
        std::fs::write(dist.join("index.js"), "").unwrap();
        std::fs::write(
            tmp.join("package.json"),
            r#"{"exports": {".": "./dist/index.js"}}"#,
        )
        .unwrap();
        let info = parse_package_json_file(&tmp.join("package.json"), &tmp).unwrap();
        let mut known = HashSet::new();
        known.insert(normalize_lexical(
            &dist.join("index.js").to_string_lossy().replace('\\', "/"),
        ));
        let result = resolve_package_exports(&info, ".", &known);
        assert!(result.is_some());
        assert!(result.unwrap().ends_with("dist/index.js"));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
