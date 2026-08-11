//! Project configuration caching for the linker daemon.
//!
//! Parsed configs (compile_commands.json, tsconfig.json, package.json) are
//! cached per `ProjectIndex` generation so that per-import resolution never
//! does a full workspace walk.

pub mod go_mod;
pub mod package_json;
pub mod python;
pub mod tsconfig;

use crate::linker::path::normalize_lexical;
use std::collections::HashMap;
use std::path::Path;

// ─── C/C++ compile database ───────────────────────────────────────────

/// A single entry from compile_commands.json.
#[derive(Debug, Clone, Default)]
pub struct CompileCommandEntry {
    /// The source file this command applies to.
    pub file: String,
    /// The directory from which the command was executed (resolved to absolute).
    pub directory: String,
    /// The `arguments` array, if present.
    pub arguments: Option<Vec<String>>,
    /// The `command` string, if present (fallback).
    pub command: Option<String>,
}

impl CompileCommandEntry {
    /// Extract include search paths and the language mode from this entry.
    pub fn extract_flags(&self) -> CompileFlags {
        // The JSON compilation database specification makes `arguments` the
        // preferred representation when both forms are present.  In
        // particular, an explicitly empty array must not fall back to command.
        let args = if let Some(args) = &self.arguments {
            args.clone()
        } else if let Some(command) = &self.command {
            tokenize_command(command)
        } else {
            Vec::new()
        };
        parse_flag_args(&self.directory, &args)
    }
}

/// Parsed flags from a compile command entry.
#[derive(Debug, Clone, Default)]
pub struct CompileFlags {
    /// Explicit language mode from `-x` flag (e.g. "c", "c++").
    pub language_mode: Option<String>,
    /// Quoted-include search paths from `-iquote`.
    pub iquote: Vec<String>,
    /// System include paths from `-isystem`.
    pub isystem: Vec<String>,
    /// General include paths from `-I`.
    pub include_paths: Vec<String>,
}

/// Resolve a relative path against a base directory.
fn resolve_rel(base: &str, path: &str) -> String {
    if Path::new(path).is_absolute() {
        normalize_lexical(path)
    } else {
        normalize_lexical(&format!("{base}/{path}"))
    }
}

fn parse_flag_args(base: &str, args: &[String]) -> CompileFlags {
    let mut flags = CompileFlags::default();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        let next = args.get(i + 1);
        let (joined, target) = if arg == "-iquote" {
            ("", Some(&mut flags.iquote))
        } else if arg == "-isystem" {
            ("", Some(&mut flags.isystem))
        } else if arg == "-I" {
            ("", Some(&mut flags.include_paths))
        } else if let Some(value) = arg.strip_prefix("-iquote") {
            (value, Some(&mut flags.iquote))
        } else if let Some(value) = arg.strip_prefix("-isystem") {
            (value, Some(&mut flags.isystem))
        } else if let Some(value) = arg.strip_prefix("-I") {
            (value, Some(&mut flags.include_paths))
        } else {
            ("", None)
        };
        if arg == "-x" {
            if let Some(mode) = next {
                flags.language_mode = Some(mode.clone());
                i += 2;
            } else {
                i += 1;
            }
        } else if let Some(mode) = arg.strip_prefix("-x").filter(|value| !value.is_empty()) {
            flags.language_mode = Some(mode.to_string());
            i += 1;
        } else if let Some(paths) = target {
            if !joined.is_empty() {
                paths.push(resolve_rel(base, joined));
                i += 1;
            } else if let Some(path) = next {
                paths.push(resolve_rel(base, path));
                i += 2;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    flags
}

/// Safe shell command tokenization. Handles single/double quotes but NOT
/// shell variables, escapes, or other shell features.
fn tokenize_command(cmd: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;

    for ch in cmd.chars() {
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
            }
            '"' if !in_single => {
                in_double = !in_double;
            }
            c if c.is_whitespace() && !in_single && !in_double => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => {
                current.push(c);
            }
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Parsed compile database, cached per ProjectIndex generation.
#[derive(Debug, Default, Clone)]
pub struct CompileDB {
    /// Per-source-file entries indexed by normalized absolute path.
    entries: HashMap<String, CompileCommandEntry>,
    /// The directory the compile_commands.json was found in.
    base_dir: String,
}

impl CompileDB {
    /// Parse compile_commands.json from the given path.
    pub fn from_path(path: &Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        Self::parse_json(&content, path)
    }

    /// Parse compile_commands.json content.
    pub fn parse_json(content: &str, path: &Path) -> Result<Self, String> {
        let config_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(path),
                Err(_) => path.to_path_buf(),
            }
        };
        let base_dir = match config_path.parent() {
            Some(parent) => normalize_lexical(&parent.to_string_lossy().replace('\\', "/")),
            None => "/".into(),
        };

        let entries: Vec<serde_json::Value> = serde_json::from_str(content)
            .map_err(|e| format!("failed to parse {}: {e}", path.display()))?;

        let mut result = CompileDB {
            entries: HashMap::new(),
            base_dir,
        };

        for entry in &entries {
            let file = match entry.get("file").and_then(|v| v.as_str()) {
                Some(f) => f,
                None => continue,
            };
            let directory = entry
                .get("directory")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let resolved_dir = resolve_rel(&result.base_dir, directory);

            let parsed = CompileCommandEntry {
                file: normalize_lexical(&resolve_rel(&resolved_dir, file)),
                directory: resolved_dir,
                arguments: entry
                    .get("arguments")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    }),
                command: entry
                    .get("command")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            };

            result.entries.insert(parsed.file.clone(), parsed);
        }

        Ok(result)
    }

    /// Look up compile flags for a source file.
    pub fn lookup(&self, file_path: &str) -> Option<&CompileCommandEntry> {
        let normalized = normalize_lexical(file_path);
        self.entries.get(&normalized)
    }
}

/// Fallback: parse compile_flags.txt (one flag per line, `-I` paths relative to the file).
pub fn parse_compile_flags(path: &Path) -> CompileFlags {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return CompileFlags::default(),
    };
    let base = path
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();

    let mut args: Vec<String> = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if !line.is_empty() && !line.starts_with('#') {
            args.push(line.to_string());
        }
    }

    parse_flag_args(&base, &args)
}

/// Find compile_commands.json or compile_flags.txt for a workspace root.
/// Returns the CompileDB if compile_commands.json is found, or None.
///
/// Retained as public API.  Scan code now uses `ProjectIndex` config
/// caches instead of per-file filesystem reads.
#[allow(dead_code)] // Public API; scan uses ProjectIndex caches.
pub fn load_compile_db(root: &Path) -> Option<CompileDB> {
    // compile_commands.json takes precedence.
    let cc_path = root.join("compile_commands.json");
    if cc_path.exists() {
        return CompileDB::from_path(&cc_path).ok();
    }
    None
}

/// Find compile_flags.txt for a workspace root. Used as fallback when
/// compile_commands.json is not present.
///
/// Retained as public API.  Scan code now uses `ProjectIndex` config
/// caches instead of per-file filesystem reads.
#[allow(dead_code)] // Public API; scan uses ProjectIndex caches.
pub fn load_compile_flags(root: &Path) -> Option<CompileFlags> {
    let cf_path = root.join("compile_flags.txt");
    if cf_path.exists() {
        Some(parse_compile_flags(&cf_path))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_command_basic() {
        let tokens = tokenize_command("gcc -I/usr/include -o out file.c");
        assert_eq!(tokens, vec!["gcc", "-I/usr/include", "-o", "out", "file.c"]);
    }

    #[test]
    fn tokenize_command_quoted() {
        let tokens = tokenize_command("gcc '-I/usr/my include' file.c");
        assert_eq!(tokens, vec!["gcc", "-I/usr/my include", "file.c"]);
    }

    #[test]
    fn compile_flags_parse_i() {
        let content = "-I/usr/include\n-I./src\n";
        let base = "/proj";
        // Simulate what parse_compile_flags does
        let mut flags = CompileFlags::default();
        for line in content.lines() {
            let line = line.trim();
            if line.starts_with("-I") {
                let rest = &line[2..];
                flags.include_paths.push(resolve_rel(base, rest));
            }
        }
        assert_eq!(flags.include_paths.len(), 2);
        assert_eq!(flags.include_paths[0], "/usr/include");
        assert_eq!(flags.include_paths[1], "/proj/src");
    }

    #[test]
    fn compile_db_from_json() {
        let json = r#"[
            {
                "directory": "/proj/build",
                "file": "../src/main.c",
                "arguments": ["gcc", "-I", "/usr/include", "-I", "./include", "-x", "c", "../src/main.c"]
            }
        ]"#;
        let path = Path::new("/proj/build/compile_commands.json");
        let db = CompileDB::parse_json(json, path).unwrap();
        assert_eq!(db.entries.len(), 1);
        let entry = db.lookup("/proj/src/main.c").unwrap();
        assert_eq!(entry.directory, "/proj/build");
        let flags = entry.extract_flags();
        assert_eq!(flags.include_paths.len(), 2);
        assert_eq!(flags.language_mode.as_deref(), Some("c"));
    }

    #[test]
    fn compile_db_command_fallback() {
        let json = r#"[
            {
                "directory": "/proj",
                "file": "src/main.c",
                "command": "gcc -I/usr/include -I./src src/main.c"
            }
        ]"#;
        let path = Path::new("/proj/compile_commands.json");
        let db = CompileDB::parse_json(json, path).unwrap();
        let entry = db.lookup("/proj/src/main.c").unwrap();
        let flags = entry.extract_flags();
        assert_eq!(flags.include_paths.len(), 2);
    }

    #[test]
    fn compile_flags_txt_parse() {
        let tmp = std::env::temp_dir().join(format!(
            "koma-linker-test-flags-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("compile_flags.txt"),
            "-I/usr/include\n-I./include\n-x\nc++\n",
        )
        .unwrap();
        let flags = parse_compile_flags(&tmp.join("compile_flags.txt"));
        assert_eq!(flags.include_paths.len(), 2);
        assert_eq!(flags.language_mode.as_deref(), Some("c++"));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
