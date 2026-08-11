//! TypeScript/JavaScript configuration parser.
//!
//! Parses `tsconfig.json`, `tsconfig.*.json`, and `jsconfig.json` files
//! (JSONC format — JSON with comments and trailing commas) to extract
//! `baseUrl`, `paths`, and `moduleResolution` settings used for import resolution.

use crate::linker::path::normalize_lexical;
use std::collections::HashSet;
use std::path::Path;

/// Parsed TypeScript/JavaScript project configuration.
#[derive(Debug, Clone, Default)]
pub struct TsConfig {
    /// Base URL for non-relative module resolution (default: directory of tsconfig).
    pub base_url: Option<String>,
    /// Path mapping patterns → target patterns.
    /// e.g. `"@/*": ["src/*"]` → `("@/*", vec!["src/*"])`.
    pub paths: Vec<(String, Vec<String>)>,
    /// `moduleResolution` setting: "node", "node16", "nodenext", "bundler", "classic".
    pub module_resolution: Option<String>,
    /// `rootDirs` setting for multi-root compilation.
    pub root_dirs: Vec<String>,
    /// `baseUrl` resolved to an absolute path.
    pub base_url_resolved: Option<String>,
    /// The directory containing this tsconfig.
    pub config_dir: String,
    /// Parse/discovery error for a config that exists but is unusable.
    pub unsupported: Option<(String, String)>,
}

/// Strip JSONC comments and trailing commas, then parse as JSON.
///
/// This is a best-effort parser. It handles:
/// - `// line comments`
/// - `/* block comments */`
/// - Trailing commas in objects and arrays
///
/// Limitations:
/// - Does not handle comments inside string literals (unusual in tsconfig).
/// - Does not handle regex literals (not applicable in JSON).
pub fn parse_jsonc(content: &str) -> Result<serde_json::Value, String> {
    let stripped = strip_jsonc(content);
    serde_json::from_str(&stripped).map_err(|e| format!("JSON parse error: {e}"))
}

/// Strip JSONC comments and trailing commas from content.
fn strip_jsonc(content: &str) -> String {
    let stripped = strip_comments(content);
    strip_trailing_commas(&stripped)
}

/// Strip `//` line comments and `/* ... */` block comments.
fn strip_comments(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let chars: Vec<char> = content.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let mut in_string = false;
    let mut escape_next = false;

    while i < len {
        let c = chars[i];

        if escape_next {
            result.push(c);
            escape_next = false;
            i += 1;
            continue;
        }

        if in_string {
            if c == '\\' {
                escape_next = true;
            } else if c == '"' {
                in_string = false;
            }
            result.push(c);
            i += 1;
            continue;
        }

        match c {
            '"' => {
                in_string = true;
                result.push(c);
                i += 1;
            }
            '/' if i + 1 < len && chars[i + 1] == '/' => {
                while i < len && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if i + 1 < len && chars[i + 1] == '*' => {
                i += 2;
                while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
            }
            _ => {
                result.push(c);
                i += 1;
            }
        }
    }
    result
}

/// Remove trailing commas before `}` and `]`, respecting strings.
fn strip_trailing_commas(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    let mut out = String::with_capacity(s.len());
    let mut in_string = false;
    let mut escape_next = false;

    for (i, &c) in chars.iter().enumerate() {
        if escape_next {
            out.push(c);
            escape_next = false;
            continue;
        }
        if in_string {
            if c == '\\' {
                escape_next = true;
            } else if c == '"' {
                in_string = false;
            }
            out.push(c);
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            ',' => {
                let mut j = i + 1;
                while j < len && matches!(chars[j], ' ' | '\n' | '\r' | '\t') {
                    j += 1;
                }
                if j < len && matches!(chars[j], '}' | ']') {
                    continue;
                }
                out.push(c);
            }
            _ => {
                out.push(c);
            }
        }
    }
    out
}

/// Find and parse the nearest tsconfig/jsconfig for a source file.
///
/// Retained as public API.  Scan code now uses `ProjectIndex` config
/// caches instead of per-file filesystem walks.
#[allow(dead_code)] // Public API; scan uses ProjectIndex caches.
pub fn find_tsconfig_for_file(
    source_dir: &Path,
    known_files: &HashSet<String>,
) -> Option<TsConfig> {
    let mut dir = source_dir.to_path_buf();
    loop {
        for name in &["tsconfig.json", "jsconfig.json"] {
            let path = dir.join(name);
            let path_s = normalize_lexical(&path.to_string_lossy().replace('\\', "/"));
            if known_files.contains(&path_s) || path.exists() {
                return Some(
                    parse_tsconfig_file(&path, &dir).unwrap_or_else(|detail| TsConfig {
                        config_dir: normalize_lexical(&dir.to_string_lossy().replace('\\', "/")),
                        unsupported: Some((path_s, detail)),
                        ..Default::default()
                    }),
                );
            }
        }

        // Check for tsconfig.*.json variants (common in monorepos).
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_s = name.to_string_lossy();
                if name_s.starts_with("tsconfig")
                    && name_s.ends_with(".json")
                    && name_s != "tsconfig.json"
                {
                    let path = entry.path();
                    let path_s = normalize_lexical(&path.to_string_lossy().replace('\\', "/"));
                    return Some(parse_tsconfig_file(&path, &dir).unwrap_or_else(|detail| {
                        TsConfig {
                            config_dir: normalize_lexical(
                                &dir.to_string_lossy().replace('\\', "/"),
                            ),
                            unsupported: Some((path_s, detail)),
                            ..Default::default()
                        }
                    }));
                }
            }
        }

        if !dir.pop() {
            break;
        }
    }
    None
}

pub fn parse_tsconfig_file(path: &Path, dir: &Path) -> Result<TsConfig, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let value = parse_jsonc(&content)?;

    let config_dir = normalize_lexical(&dir.to_string_lossy().replace('\\', "/"));

    let mut config = TsConfig {
        config_dir: config_dir.clone(),
        ..Default::default()
    };

    if let Some(opts) = value.get("compilerOptions") {
        if let Some(bu) = opts.get("baseUrl").and_then(|v| v.as_str()) {
            config.base_url = Some(bu.to_string());
            config.base_url_resolved = Some(normalize_lexical(&format!("{config_dir}/{bu}")));
        }
        if let Some(paths_obj) = opts.get("paths").and_then(|v| v.as_object()) {
            for (pattern, targets) in paths_obj {
                let target_list: Vec<String> = match targets {
                    serde_json::Value::String(s) => vec![s.clone()],
                    serde_json::Value::Array(arr) => arr
                        .iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect(),
                    _ => continue,
                };
                config.paths.push((pattern.clone(), target_list));
            }
        }
        if let Some(mr) = opts.get("moduleResolution").and_then(|v| v.as_str()) {
            config.module_resolution = Some(mr.to_ascii_lowercase());
        }
        if let Some(rd) = opts.get("rootDirs").and_then(|v| v.as_array()) {
            for dir_val in rd {
                if let Some(d) = dir_val.as_str() {
                    config
                        .root_dirs
                        .push(normalize_lexical(&format!("{config_dir}/{d}")));
                }
            }
        }
    }

    Ok(config)
}

/// Resolve a TS import specifier using tsconfig paths and baseUrl.
///
/// For a specifier like `@app/foo` with paths `{ "@app/*": ["src/*"] }` and
/// baseUrl `/proj`, tries `src/foo` relative to baseUrl.
///
/// Returns a list of candidate relative paths (relative to config_dir) to try.
pub fn resolve_paths(specifier: &str, config: &TsConfig) -> Vec<String> {
    let mut candidates = Vec::new();

    for (pattern, targets) in &config.paths {
        if let Some(star_pos) = pattern.find('*') {
            let prefix = &pattern[..star_pos];
            let suffix = &pattern[star_pos + 1..];
            if specifier.starts_with(prefix) && (suffix.is_empty() || specifier.ends_with(suffix)) {
                let matched = if suffix.is_empty() {
                    &specifier[prefix.len()..]
                } else {
                    &specifier[prefix.len()..specifier.len() - suffix.len()]
                };
                for target in targets {
                    let resolved = target.replace('*', matched);
                    candidates.push(resolved);
                }
            }
        } else if specifier == *pattern {
            for target in targets {
                candidates.push(target.clone());
            }
        }
    }

    candidates
}

/// Determine the TypeScript/JS extension substitution order for a given file extension.
///
/// When resolving a specifier like `./foo.js`, the resolver should try:
/// .ts, .tsx, .d.ts (for .js)
/// .mts, .d.mts (for .mjs)
/// .cts, .d.cts (for .cjs)
/// (for extensionless or .ts) .ts, .tsx, .d.ts, .js, .jsx
pub fn ts_extension_candidates(spec_ext: &str) -> Vec<&'static str> {
    match spec_ext {
        ".js" => vec![".ts", ".tsx", ".d.ts"],
        ".mjs" => vec![".mts", ".d.mts"],
        ".cjs" => vec![".cts", ".d.cts"],
        "" => vec![
            ".ts",
            ".tsx",
            ".d.ts",
            ".js",
            ".jsx",
            ".mjs",
            ".cjs",
            ".mts",
            ".cts",
            "/index.ts",
            "/index.tsx",
            "/index.js",
            "/index.jsx",
        ],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_jsonc_comments() {
        let input = r#"{
            // This is a comment
            "baseUrl": ".",
            /* block
               comment */
            "paths": {
                "@/*": ["src/*"],
            }
        }"#;
        let result = strip_jsonc(input);
        assert!(!result.contains("comment"));
        assert!(result.contains("\"baseUrl\""));
        assert!(result.contains("\"@/*\""));
    }

    #[test]
    fn parse_jsonc_valid() {
        let input = r#"{
            // comment
            "compilerOptions": {
                "baseUrl": "./src",
                "paths": {
                    "@app/*": ["app/*"]
                },
                "moduleResolution": "bundler",
            },
        }"#;
        let value = parse_jsonc(input).unwrap();
        let opts = value.get("compilerOptions").unwrap();
        assert_eq!(opts.get("baseUrl").unwrap().as_str(), Some("./src"));
        assert_eq!(
            opts.get("moduleResolution").unwrap().as_str(),
            Some("bundler")
        );
    }

    #[test]
    fn parse_tsconfig_file_test() {
        let tmp = std::env::temp_dir().join(format!(
            "koma-linker-test-tsconfig-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("tsconfig.json"),
            r#"{
                "compilerOptions": {
                    "baseUrl": "./src",
                    "paths": { "@/*": ["*"] },
                    "moduleResolution": "node16"
                }
            }"#,
        )
        .unwrap();
        let config = parse_tsconfig_file(&tmp.join("tsconfig.json"), &tmp).unwrap();
        assert_eq!(config.base_url.as_deref(), Some("./src"));
        assert_eq!(config.paths.len(), 1);
        assert_eq!(config.paths[0].0, "@/*");
        assert_eq!(config.module_resolution.as_deref(), Some("node16"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn resolve_paths_test() {
        let config = TsConfig {
            paths: vec![
                ("@/*".into(), vec!["src/*".into()]),
                ("@lib/*".into(), vec!["lib/*".into(), "shared/*".into()]),
            ],
            ..Default::default()
        };
        // "@/*" pattern: prefix = "@/", suffix = "". Matches "@/<rest>".
        let candidates = resolve_paths("@/app/utils", &config);
        assert_eq!(candidates, vec!["src/app/utils"]);

        let candidates = resolve_paths("@lib/helper", &config);
        assert_eq!(candidates, vec!["lib/helper", "shared/helper"]);
    }

    #[test]
    fn ts_extension_candidates_test() {
        let c = ts_extension_candidates(".js");
        assert!(c.contains(&".ts"));
        assert!(c.contains(&".tsx"));
        assert!(c.contains(&".d.ts"));

        let c = ts_extension_candidates("");
        assert!(c.contains(&".ts"));
        assert!(c.contains(&"/index.ts"));

        let c = ts_extension_candidates(".mjs");
        assert!(c.contains(&".mts"));
        assert!(c.contains(&".d.mts"));
    }
}
