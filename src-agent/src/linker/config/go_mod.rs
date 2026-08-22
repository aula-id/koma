//! Go module configuration caching for the linker daemon.
//!
//! Parses `go.mod` (module path, require, replace) and `go.work` (use,
//! replace) to provide module-scoped import resolution.
//!
//! No `go list` invocation — all config is parsed statically from files.

use crate::linker::path::normalize_lexical;
use std::collections::HashMap;
use std::path::Path;

/// A single `replace` directive from go.mod or go.work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoReplace {
    /// Old path (before `=>`).
    pub old: String,
    /// New path (after `=>`). If the replacement is local, this is the
    /// local directory path.
    pub new: String,
    /// Whether the replacement is a local path (no version).
    pub local: bool,
}

/// Parsed go.mod for a single module root.
#[derive(Debug, Clone, Default)]
pub struct GoModConfig {
    /// The module declaration path (e.g., `"github.com/foo/bar"`).
    pub module_path: String,
    /// Local replace directives: old → GoReplace.
    pub replaces: HashMap<String, GoReplace>,
}

/// Parsed go.work configuration.
#[derive(Debug, Clone, Default)]
pub struct GoWorkConfig {
    /// Local `use` directives: list of workspace module directories.
    pub uses: Vec<String>,
    /// Workspace-level replace directives.
    pub replaces: HashMap<String, GoReplace>,
}

/// Combined Go configuration for a workspace root, cached per generation.
#[derive(Debug, Clone, Default)]
pub struct GoModuleConfig {
    /// Per-directory go.mod configs, keyed by the directory containing
    /// the go.mod file (normalized absolute path).
    pub mods: HashMap<String, GoModConfig>,
    /// go.work config if present at root.
    pub work: Option<GoWorkConfig>,
    /// Whether vendor mode is detected (go.mod contains `// indirect`
    /// comments or vendor directory exists).
    pub vendor_mode: bool,
}

/// Build GoModuleConfig for a workspace root by scanning for go.mod
/// and go.work files. Called once per generation.
pub fn build_go_module_config(root: &str) -> GoModuleConfig {
    let root_path = Path::new(root);
    let mut config = GoModuleConfig::default();

    // Check for go.work at root.
    let gowork_path = root_path.join("go.work");
    if gowork_path.exists() {
        if let Some(work) = parse_go_work(&gowork_path) {
            // Resolve use paths to absolute directories.
            let mut resolved_uses = Vec::new();
            for use_path in &work.uses {
                let full = root_path.join(use_path);
                let full_s = normalize_lexical(&full.to_string_lossy().replace('\\', "/"));
                resolved_uses.push(full_s);

                // Parse go.mod in each used module directory.
                let mod_path = full.join("go.mod");
                if let Some(mod_cfg) = parse_go_mod(&mod_path) {
                    config.mods.insert(
                        normalize_lexical(&full.to_string_lossy().replace('\\', "/")),
                        mod_cfg,
                    );
                }
            }
            config.work = Some(GoWorkConfig {
                uses: resolved_uses,
                replaces: work.replaces,
            });
        }
    }

    // Parse go.mod at the root itself.
    let root_mod_path = root_path.join("go.mod");
    if root_mod_path.exists() {
        if let Some(mod_cfg) = parse_go_mod(&root_mod_path) {
            config.mods.insert(normalize_lexical(root), mod_cfg);
        }
    }

    // Also walk for nested go.mod files (bounded depth to avoid vendor/).
    walk_nested_gomods(root_path, &mut config, 0);

    // Detect vendor mode: vendor/ directory under any go.mod root.
    for mod_dir in config.mods.keys() {
        if Path::new(mod_dir).join("vendor").is_dir() {
            config.vendor_mode = true;
            break;
        }
    }

    config
}

/// Walk for nested go.mod files up to a bounded depth.
fn walk_nested_gomods(root: &Path, config: &mut GoModuleConfig, depth: u32) {
    if depth > 4 {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let name = entry.file_name();
            let name_s = name.to_string_lossy();
            // Skip vendor, hidden dirs, standard prune dirs.
            if name_s.starts_with('.')
                || name_s == "vendor"
                || name_s == "node_modules"
                || name_s == "target"
                || name_s == "__pycache__"
            {
                continue;
            }
            let sub = entry.path();
            let sub_mod = sub.join("go.mod");
            if sub_mod.exists() {
                let sub_s = normalize_lexical(&sub.to_string_lossy().replace('\\', "/"));
                if !config.mods.contains_key(&sub_s) {
                    if let Some(mod_cfg) = parse_go_mod(&sub_mod) {
                        config.mods.insert(sub_s, mod_cfg);
                    }
                }
            }
            walk_nested_gomods(&sub, config, depth + 1);
        }
    }
}

/// Parse a go.mod file for module path, require, and replace directives.
fn parse_go_mod(path: &Path) -> Option<GoModConfig> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut config = GoModConfig::default();
    let mut in_require_block = false;
    let mut in_replace_block = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip comments and empty lines.
        if trimmed.starts_with("//") || trimmed.is_empty() {
            continue;
        }

        // Module declaration.
        if let Some(rest) = trimmed.strip_prefix("module ") {
            let module_path = rest.trim();
            if !module_path.is_empty() {
                config.module_path = module_path.to_string();
            }
            continue;
        }

        // Require block.
        if trimmed == "require (" {
            in_require_block = true;
            continue;
        }
        if in_require_block {
            if trimmed == ")" {
                in_require_block = false;
                continue;
            }
            // require path version
            // We don't track versions for resolution, just acknowledge the line.
            continue;
        }

        // Single require.
        if trimmed.starts_with("require ") {
            continue;
        }

        // Replace block.
        if trimmed == "replace (" {
            in_replace_block = true;
            continue;
        }
        if in_replace_block {
            if trimmed == ")" {
                in_replace_block = false;
                continue;
            }
            // Replace old => new [version]
            if let Some(rest) = trimmed.strip_prefix("replace ") {
                if let Some(rep) = parse_replace_directive(rest) {
                    config.replaces.insert(rep.old.clone(), rep);
                }
            }
            continue;
        }

        // Single replace.
        if let Some(rest) = trimmed.strip_prefix("replace ") {
            if let Some(rep) = parse_replace_directive(rest) {
                config.replaces.insert(rep.old.clone(), rep);
            }
        }
    }

    Some(config)
}

/// Parse a replace directive: `old => new [version]`.
fn parse_replace_directive(s: &str) -> Option<GoReplace> {
    let parts: Vec<&str> = s.splitn(2, "=>").collect();
    if parts.len() != 2 {
        return None;
    }
    let old = parts[0].trim().to_string();
    let new_full = parts[1].trim();
    // Split new into path and optional version.
    let mut tokens = new_full.split_whitespace();
    let new_path = tokens.next()?.to_string();
    let has_version = tokens.next().is_some();
    Some(GoReplace {
        old,
        new: new_path,
        local: !has_version,
    })
}

/// Parse a go.work file for use and replace directives.
fn parse_go_work(path: &Path) -> Option<GoWorkConfig> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut config = GoWorkConfig::default();
    let mut in_use_block = false;
    let mut in_replace_block = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.is_empty() {
            continue;
        }

        // Use block.
        if trimmed == "use (" {
            in_use_block = true;
            continue;
        }
        if in_use_block {
            if trimmed == ")" {
                in_use_block = false;
                continue;
            }
            let path = trimmed.trim().trim_matches('"').trim_matches('\'');
            if !path.is_empty() {
                config.uses.push(path.to_string());
            }
            continue;
        }

        // Single use.
        if let Some(rest) = trimmed.strip_prefix("use ") {
            let path = rest.trim().trim_matches('"').trim_matches('\'');
            if !path.is_empty() {
                config.uses.push(path.to_string());
            }
            continue;
        }

        // Replace block.
        if trimmed == "replace (" {
            in_replace_block = true;
            continue;
        }
        if in_replace_block {
            if trimmed == ")" {
                in_replace_block = false;
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("replace ") {
                if let Some(rep) = parse_replace_directive(rest) {
                    config.replaces.insert(rep.old.clone(), rep);
                }
            }
            continue;
        }

        // Single replace.
        if let Some(rest) = trimmed.strip_prefix("replace ") {
            if let Some(rep) = parse_replace_directive(rest) {
                config.replaces.insert(rep.old.clone(), rep);
            }
        }
    }

    Some(config)
}

#[cfg(test)]
#[path = "go_mod_test.rs"]
mod tests;
