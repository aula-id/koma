//! Python project configuration caching for the linker daemon.
//!
//! Parses `pyproject.toml` (setuptools backend) and `setup.cfg` to extract
//! package discovery roots so that Python import resolution knows where to
//! search for modules.
//!
//! No new crate dependency: TOML/INI parsing is done with a minimal
//! line-based parser handling only the subset used by setuptools manifests.

use crate::linker::path::normalize_lexical;
use std::collections::HashMap;
use std::path::Path;

/// Parsed Python project configuration, cached per owner root.
#[derive(Debug, Default, Clone)]
pub struct PythonConfig {
    /// Ordered search roots for module resolution.
    ///
    /// Built from: project root, conventional `src/`, setuptools
    /// `package-dir` mappings, and package discovery patterns.
    pub search_roots: Vec<String>,
}

/// Parse `pyproject.toml` and `setup.cfg` under a root to build a
/// `PythonConfig`. Called once per generation during `rebuild_root_config`.
pub fn build_python_config(root: &str) -> PythonConfig {
    let root_path = Path::new(root);
    let mut search_roots = Vec::new();

    // 1. Project root itself is always a search root.
    search_roots.push(normalize_lexical(root));

    // 2. Conventional `src/` directory.
    let src_dir = root_path.join("src");
    if src_dir.is_dir() {
        search_roots.push(normalize_lexical(
            &src_dir.to_string_lossy().replace('\\', "/"),
        ));
    }

    // 3. Try pyproject.toml (setuptools backend).
    let pyproject_path = root_path.join("pyproject.toml");
    if let Some(cfg) = parse_pyproject_toml(&pyproject_path) {
        // package-dir entries add their values as search roots.
        for dir in cfg.package_dir.values() {
            let full = root_path.join(dir);
            if full.is_dir() {
                search_roots.push(normalize_lexical(
                    &full.to_string_lossy().replace('\\', "/"),
                ));
            }
        }
        // package discovery patterns: resolve include patterns to directories.
        for pattern in &cfg.package_include {
            let resolved = resolve_package_pattern(root_path, pattern);
            for dir in resolved {
                search_roots.push(normalize_lexical(&dir.replace('\\', "/")));
            }
        }
    }

    // 4. Try setup.cfg as fallback.
    if search_roots.len() <= 2 {
        // No pyproject.toml results or only default roots.
        let setup_cfg_path = root_path.join("setup.cfg");
        if let Some(cfg) = parse_setup_cfg(&setup_cfg_path) {
            for dir in cfg.package_dir.values() {
                let full = root_path.join(dir);
                if full.is_dir() {
                    search_roots.push(normalize_lexical(
                        &full.to_string_lossy().replace('\\', "/"),
                    ));
                }
            }
            for pattern in &cfg.packages {
                let resolved = resolve_package_pattern(root_path, pattern);
                for dir in resolved {
                    search_roots.push(normalize_lexical(&dir.replace('\\', "/")));
                }
            }
        }
    }

    // Deduplicate while preserving order.
    let mut seen = std::collections::HashSet::new();
    search_roots.retain(|r| seen.insert(r.clone()));

    PythonConfig { search_roots }
}

/// Minimal setuptools pyproject.toml parsing result.
#[derive(Debug, Default)]
struct PyprojectParsed {
    package_dir: HashMap<String, String>,
    package_include: Vec<String>,
}

/// Parse a limited subset of pyproject.toml sufficient for setuptools
/// package discovery. Handles:
/// - `[tool.setuptools.package-dir]` key = value mappings
/// - `[tool.setuptools.packages.find]` include = ["..."]
/// - `[tool.setuptools.packages]` include = ["..."]
fn parse_pyproject_toml(path: &Path) -> Option<PyprojectParsed> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut result = PyprojectParsed::default();
    let mut current_section = String::new();
    let mut in_array = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip comments and empty lines.
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }

        // Section header: [tool.setuptools.package-dir]
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            current_section = trimmed[1..trimmed.len() - 1].trim().to_string();
            in_array = false;
            continue;
        }

        // Array continuation.
        if in_array {
            let items = parse_toml_array_items(trimmed);
            if !items.is_empty() {
                let section = current_section.clone();
                if section == "tool.setuptools.packages.find"
                    || section == "tool.setuptools.packages"
                {
                    result.package_include.extend(items);
                }
            }
            if trimmed.contains(']') {
                in_array = false;
            }
            continue;
        }

        // Key = value or Key = [array]
        if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos].trim().to_string();
            let value = trimmed[eq_pos + 1..].trim();

            if value.starts_with('[') && value.ends_with(']') {
                // Inline array.
                let items = parse_toml_inline_array(value);
                if current_section == "tool.setuptools.package-dir" {
                    // package-dir maps are string values, not arrays.
                } else if current_section == "tool.setuptools.packages.find"
                    || current_section == "tool.setuptools.packages"
                {
                    result.package_include.extend(items);
                }
            } else if value.starts_with('[') {
                // Multi-line array start.
                in_array = true;
                let items = parse_toml_array_items(value);
                if current_section == "tool.setuptools.packages.find"
                    || current_section == "tool.setuptools.packages"
                {
                    result.package_include.extend(items);
                }
            } else if current_section == "tool.setuptools.package-dir" {
                // String value in package-dir.
                let val = value.trim_matches('"').trim_matches('\'');
                result.package_dir.insert(key, val.to_string());
            }
        }
    }

    Some(result)
}

/// Parse items from a TOML inline array string like `["a", "b", "c"]`.
fn parse_toml_inline_array(s: &str) -> Vec<String> {
    let inner = s.trim().strip_prefix('[').unwrap_or(s);
    let inner = inner.strip_suffix(']').unwrap_or(inner);
    parse_toml_array_items(inner)
}

/// Parse TOML array items from the content between brackets.
fn parse_toml_array_items(s: &str) -> Vec<String> {
    let mut items = Vec::new();
    for item in s.split(',') {
        let item = item.trim().trim_matches('"').trim_matches('\'').trim();
        if !item.is_empty() && item != "]" {
            items.push(item.to_string());
        }
    }
    items
}

/// Resolve a package discovery pattern (like `scrapion_agent*`) to directories.
fn resolve_package_pattern(root: &Path, pattern: &str) -> Vec<String> {
    let mut result = Vec::new();
    // Strip trailing * wildcard for directory matching.
    let base = pattern.trim_end_matches('*').trim_end_matches('/');
    if base.is_empty() {
        // Empty pattern or just * means root packages.
        return result;
    }
    let dir = root.join(base);
    if dir.is_dir() {
        result.push(dir.to_string_lossy().replace('\\', "/"));
    }
    // Also check for sub-packages matching `base*`.
    if let Some(parent) = dir.parent() {
        if let Ok(entries) = std::fs::read_dir(parent) {
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    if let Some(name) = entry.file_name().to_str() {
                        if name.starts_with(base) && !name.starts_with('.') {
                            result.push(normalize_lexical(
                                &entry.path().to_string_lossy().replace('\\', "/"),
                            ));
                        }
                    }
                }
            }
        }
    }
    result
}

/// Minimal setup.cfg parsing result.
#[derive(Debug, Default)]
struct SetupCfgParsed {
    package_dir: HashMap<String, String>,
    packages: Vec<String>,
}

/// Parse a limited subset of setup.cfg for setuptools configuration.
/// Handles `[options]` package_dir and packages.
fn parse_setup_cfg(path: &Path) -> Option<SetupCfgParsed> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut result = SetupCfgParsed::default();
    let mut current_section = String::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }

        // Section header: [options], [options.packages.find]
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            current_section = trimmed[1..trimmed.len() - 1].trim().to_string();
            continue;
        }

        // Key = value
        if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos].trim().to_string();
            let value = trimmed[eq_pos + 1..].trim();

            if current_section == "options" && key == "package_dir" && !value.is_empty() {
                // INI multi-line or single: root = src
                parse_ini_mapping(value, &mut result.package_dir);
            } else if current_section == "options" && key == "packages" && !value.is_empty() {
                for pkg in value.split_whitespace() {
                    let pkg = pkg.trim().trim_matches('"').trim_matches('\'');
                    if !pkg.is_empty() {
                        result.packages.push(pkg.to_string());
                    }
                }
            }
        }

        // INI multi-line continuation: key = value (indented or after newline).
        if current_section == "options"
            && !trimmed.starts_with('[')
            && !trimmed.contains('=')
            && !trimmed.starts_with('#')
        {
            // Continuation of previous key (e.g., multi-line package_dir).
            // Check if last value was empty.
        }
    }

    Some(result)
}

/// Parse an INI-style mapping value like `root = src`.
fn parse_ini_mapping(value: &str, map: &mut HashMap<String, String>) {
    // Single key = value pair on this line.
    if let Some(eq_pos) = value.find('=') {
        let k = value[..eq_pos].trim().trim_matches('"').trim_matches('\'');
        let v = value[eq_pos + 1..]
            .trim()
            .trim_matches('"')
            .trim_matches('\'');
        if !k.is_empty() {
            map.insert(k.to_string(), v.to_string());
        }
    }
}

#[cfg(test)]
#[path = "python_test.rs"]
mod tests;
