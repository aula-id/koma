//! Project index over normalized absolute roots and known source files.
//!
//! Registration is lexical and does not require paths to exist. A scanner may
//! canonicalize paths before registration, but this index does not do so itself.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::linker::config::go_mod::GoModuleConfig;
use crate::linker::config::package_json::PackageJsonInfo;
use crate::linker::config::python::PythonConfig;
use crate::linker::config::tsconfig::{self, TsConfig};
use crate::linker::config::{CompileDB, CompileFlags};
use crate::linker::graph::Lang;
use crate::linker::path::{is_absolute_lexical, normalize_lexical, owner_root};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownFileEntry {
    /// Normalized absolute path (canonical only if supplied canonical by a scanner).
    pub abs_path: String,
    pub lang: Lang,
    pub dir: String,
    pub rel_path: String,
    /// Longest owning normalized absolute root.
    pub workspace_root: String,
}

/// A config file that existed but failed to parse.  Stored as a structured
/// record so importer resolution can surface `UnsupportedConfig` instead of
/// silently acting unconfigured.
#[derive(Debug, Clone)]
pub struct ConfigParseFailure {
    #[allow(dead_code)] // Retained for diagnostic tooling.
    pub path: String,
    #[allow(dead_code)] // Retained for diagnostic tooling.
    pub detail: String,
}

/// Per-root configuration cache, built once per full scan generation and
/// rebuilt only for the owning root on config watcher events.
///
/// All config file reads and parses happen here during `rebuild_root_config`.
/// Per-import resolver code uses only index lookups — no filesystem reads.
#[derive(Debug, Default, Clone)]
pub struct RootConfig {
    /// compile_commands.json files found under root, keyed by parent directory.
    pub compile_dbs: HashMap<String, CompileDB>,
    /// compile_flags.txt files found under root, keyed by parent directory.
    pub compile_flags_map: HashMap<String, CompileFlags>,
    /// All tsconfig/jsconfig files found under root (nearest-lookup by prefix).
    pub tsconfigs: Vec<TsConfig>,
    /// All package.json files found under root (nearest-lookup by prefix).
    pub package_jsons: Vec<PackageJsonInfo>,
    /// Python project configuration (pyproject.toml / setup.cfg).
    pub python_config: Option<PythonConfig>,
    /// Go module configuration (go.mod / go.work).
    pub go_module_config: Option<GoModuleConfig>,
    /// Config files that existed but failed to parse.
    pub parse_failures: Vec<ConfigParseFailure>,
}

#[derive(Debug, Default)]
pub struct ProjectIndex {
    /// Sorted normalized absolute roots.
    roots: Vec<String>,
    files: HashMap<String, KnownFileEntry>,
    by_dir: HashMap<String, Vec<String>>,
    by_lang: HashMap<Lang, Vec<String>>,
    /// Per-root configuration caches indexed by normalized root path.
    root_configs: HashMap<String, RootConfig>,
    /// Monotonically increasing generation counter for cache identity.
    generation: u64,
    /// Cached set of all file names (keys of `files`). Rebuilt on add/remove.
    file_names: HashSet<String>,
}

impl ProjectIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a normalized absolute workspace root. Returns whether it was new.
    pub fn register_root(&mut self, root: String) -> Result<bool, String> {
        let root = normalize_lexical(&root);
        if !is_absolute_lexical(&root) {
            return Err(format!("workspace root is not absolute: {root}"));
        }
        if self.roots.contains(&root) {
            return Ok(false);
        }
        self.roots.push(root);
        self.roots.sort();
        // A newly-added nested root may become the owner of existing files.
        for entry in self.files.values_mut() {
            if let Some(owner) = owner_root(&entry.abs_path, &self.roots) {
                entry.workspace_root = owner.to_string();
                entry.rel_path = relative_to(owner, &entry.abs_path);
            }
        }
        Ok(true)
    }

    pub fn roots(&self) -> &[String] {
        &self.roots
    }

    /// Add or replace an absolute file owned by a registered root.
    pub fn add_file(&mut self, abs_path: &str, lang: Lang) -> Result<(), String> {
        let path = normalize_lexical(abs_path);
        if !is_absolute_lexical(&path) {
            return Err(format!("file path is not absolute: {path}"));
        }
        let owner = owner_root(&path, &self.roots)
            .ok_or_else(|| format!("file is outside registered roots: {path}"))?
            .to_string();
        self.remove_file(&path);

        let dir = path.rsplit_once('/').map_or_else(
            || path.clone(),
            |(dir, _)| {
                if dir.is_empty() {
                    "/".into()
                } else {
                    dir.into()
                }
            },
        );
        let entry = KnownFileEntry {
            abs_path: path.clone(),
            lang,
            dir: dir.clone(),
            rel_path: relative_to(&owner, &path),
            workspace_root: owner,
        };
        self.by_dir.entry(dir).or_default().push(path.clone());
        self.by_lang.entry(lang).or_default().push(path.clone());
        self.file_names.insert(path.clone());
        self.files.insert(path, entry);
        Ok(())
    }

    /// Remove a file and all grouping entries. Returns whether it existed.
    pub fn remove_file(&mut self, path: &str) -> bool {
        let path = normalize_lexical(path);
        let Some(old) = self.files.remove(&path) else {
            return false;
        };
        self.file_names.remove(&path);
        remove_group_entry(&mut self.by_dir, &old.dir, &path);
        remove_group_entry(&mut self.by_lang, &old.lang, &path);
        true
    }

    pub fn get_file(&self, path: &str) -> Option<&KnownFileEntry> {
        self.files.get(&normalize_lexical(path))
    }

    pub fn file_owner(&self, path: &str) -> Option<&str> {
        let path = normalize_lexical(path);
        owner_root(&path, &self.roots)
    }

    #[allow(dead_code)] // Public API surface retained for future tooling.
    pub fn by_dir(&self) -> &HashMap<String, Vec<String>> {
        &self.by_dir
    }
    #[allow(dead_code)]
    pub fn by_lang(&self) -> &HashMap<Lang, Vec<String>> {
        &self.by_lang
    }
    #[allow(dead_code)]
    pub fn known_files(&self) -> &HashMap<String, KnownFileEntry> {
        &self.files
    }
    pub fn known_file_set(&self) -> &HashSet<String> {
        &self.file_names
    }
    #[allow(dead_code)]
    pub fn file_count(&self) -> usize {
        self.files.len()
    }
    #[allow(dead_code)]
    pub fn sorted_files(&self) -> Vec<&KnownFileEntry> {
        let mut entries: Vec<_> = self.files.values().collect();
        entries.sort_by(|a, b| a.abs_path.cmp(&b.abs_path));
        entries
    }

    // ─── Phase 3: per-generation configuration caching ───────────────────

    /// Current generation counter for cache identity testing.
    #[allow(dead_code)] // Used in tests and by external tooling.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Rebuild the config cache for a specific root.  Called once per root
    /// during `scan_roots` and on config/manifest watcher events.
    pub fn rebuild_root_config(&mut self, root: &str) {
        let root_s = normalize_lexical(root);
        let config = build_root_config(&root_s);
        self.root_configs.insert(root_s, config);
        self.generation += 1;
    }

    /// Get the RootConfig for a root, if it exists.
    pub fn root_config(&self, root: &str) -> Option<&RootConfig> {
        self.root_configs.get(root)
    }

    /// Nearest compile DB entry flags for an importer file.
    ///
    /// Selects the nearest applicable compile_commands.json (longest directory
    /// prefix within the owner root), then checks for an importer-specific
    /// entry.  Returns the extracted flags if an entry exists.
    pub fn compile_db_entry_for_file(&self, abs_path: &str) -> Option<CompileFlags> {
        let owner = self.file_owner(abs_path)?;
        let config = self.root_configs.get(owner)?;
        let importer_dir = normalize_lexical(
            &Path::new(abs_path)
                .parent()?
                .to_string_lossy()
                .replace('\\', "/"),
        );
        // Find nearest compile_commands.json by longest directory prefix.
        let mut best_db: Option<&CompileDB> = None;
        let mut best_len = 0usize;
        for (dir, db) in &config.compile_dbs {
            if importer_dir.starts_with(dir) && dir.len() > best_len {
                best_db = Some(db);
                best_len = dir.len();
            }
        }
        let db = best_db?;
        let entry = db.lookup(abs_path)?;
        Some(entry.extract_flags())
    }

    /// Nearest compile_flags.txt for an importer file.
    ///
    /// Falls back to compile_flags.txt when no compile_commands.json entry
    /// is found.  Selects the nearest applicable flags file (longest
    /// directory prefix within the owner root).
    pub fn compile_flags_for_file(&self, abs_path: &str) -> Option<&CompileFlags> {
        let owner = self.file_owner(abs_path)?;
        let config = self.root_configs.get(owner)?;
        let importer_dir = normalize_lexical(
            &Path::new(abs_path)
                .parent()?
                .to_string_lossy()
                .replace('\\', "/"),
        );
        let mut best: Option<&CompileFlags> = None;
        let mut best_len = 0usize;
        for (dir, flags) in &config.compile_flags_map {
            if importer_dir.starts_with(dir) && dir.len() > best_len {
                best = Some(flags);
                best_len = dir.len();
            }
        }
        best
    }

    /// Nearest tsconfig/jsconfig for an importer file, bounded to the
    /// importer's owner root.  Index lookup only — no filesystem walk.
    pub fn tsconfig_for_importer(&self, abs_path: &str) -> Option<&TsConfig> {
        let owner = self.file_owner(abs_path)?;
        let config = self.root_configs.get(owner)?;
        let importer_dir = normalize_lexical(
            &Path::new(abs_path)
                .parent()?
                .to_string_lossy()
                .replace('\\', "/"),
        );
        let mut best: Option<&TsConfig> = None;
        let mut best_len = 0usize;
        for tc in &config.tsconfigs {
            if importer_dir.starts_with(&tc.config_dir) && tc.config_dir.len() > best_len {
                best = Some(tc);
                best_len = tc.config_dir.len();
            }
        }
        best
    }

    /// Nearest package.json for an importer file, bounded to the
    /// importer's owner root.  Index lookup only — no filesystem walk.
    pub fn package_json_for_importer(&self, abs_path: &str) -> Option<&PackageJsonInfo> {
        let owner = self.file_owner(abs_path)?;
        let config = self.root_configs.get(owner)?;
        let importer_dir = normalize_lexical(
            &Path::new(abs_path)
                .parent()?
                .to_string_lossy()
                .replace('\\', "/"),
        );
        let mut best: Option<&PackageJsonInfo> = None;
        let mut best_len = 0usize;
        for pj in &config.package_jsons {
            if importer_dir.starts_with(&pj.dir) && pj.dir.len() > best_len {
                best = Some(pj);
                best_len = pj.dir.len();
            }
        }
        best
    }

    /// Python project config for the importer's owner root.
    pub fn python_config_for_importer(&self, abs_path: &str) -> Option<&PythonConfig> {
        let owner = self.file_owner(abs_path)?;
        let config = self.root_configs.get(owner)?;
        config.python_config.as_ref()
    }

    /// Go module config for the importer's owner root.
    pub fn go_module_config_for_importer(&self, abs_path: &str) -> Option<&GoModuleConfig> {
        let owner = self.file_owner(abs_path)?;
        let config = self.root_configs.get(owner)?;
        config.go_module_config.as_ref()
    }
}

fn relative_to(root: &str, path: &str) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .trim_start_matches('/')
        .to_string()
}

/// Build a RootConfig for a workspace root by walking it once and parsing
/// every relevant config file.  Called during `scan_roots` (once per root)
/// and on config/manifest watcher events (only for the owning root).
///
/// The walk uses the same `ignore::WalkBuilder` settings as `collect_source_files`
/// for consistency.  Each config file is parsed exactly once; parse failures
/// are recorded as structured `ConfigParseFailure` entries.
fn build_root_config(root: &str) -> RootConfig {
    let root_path = Path::new(root);
    let mut config = RootConfig::default();

    let walker = ignore::WalkBuilder::new(root_path)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false)
        .hidden(false)
        .filter_entry(|dent| {
            if dent.depth() > 0 && dent.file_type().is_some_and(|t| t.is_dir()) {
                if let Some(name) = dent.file_name().to_str() {
                    return !crate::linker::scan::is_pruned_dir_name(name);
                }
            }
            true
        })
        .build();

    for dent in walker.flatten() {
        if !dent.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = dent.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        let dir = normalize_lexical(
            &path
                .parent()
                .unwrap_or(root_path)
                .to_string_lossy()
                .replace('\\', "/"),
        );
        let path_s = normalize_lexical(&path.to_string_lossy().replace('\\', "/"));

        if name == "compile_commands.json" {
            match CompileDB::from_path(path) {
                Ok(db) => {
                    config.compile_dbs.insert(dir, db);
                }
                Err(e) => {
                    config.parse_failures.push(ConfigParseFailure {
                        path: path_s,
                        detail: e,
                    });
                }
            }
        } else if name == "compile_flags.txt" {
            let flags = crate::linker::config::parse_compile_flags(path);
            config.compile_flags_map.insert(dir, flags);
        } else if (name.starts_with("tsconfig") && name.ends_with(".json"))
            || name == "jsconfig.json"
        {
            match tsconfig::parse_tsconfig_file(path, Path::new(&dir)) {
                Ok(tc) => {
                    config.tsconfigs.push(tc);
                }
                Err(e) => {
                    config.parse_failures.push(ConfigParseFailure {
                        path: path_s,
                        detail: e,
                    });
                }
            }
        } else if name == "package.json" {
            if let Some(info) =
                crate::linker::config::package_json::parse_package_json_file(path, Path::new(&dir))
            {
                config.package_jsons.push(info);
            } else {
                config.parse_failures.push(ConfigParseFailure {
                    path: path_s,
                    detail: "failed to parse package.json".into(),
                });
            }
        }
        // Python: no per-file parsing needed; pyproject.toml handled below.
        // Go: go.mod is handled below.
    }

    // Build Python config if pyproject.toml or setup.cfg exists at root.
    let pyproject = root_path.join("pyproject.toml");
    let setup_cfg = root_path.join("setup.cfg");
    if pyproject.exists() || setup_cfg.exists() {
        config.python_config = Some(crate::linker::config::python::build_python_config(root));
    }

    // Build Go module config if go.mod or go.work exists at root.
    let gomod = root_path.join("go.mod");
    let gowork = root_path.join("go.work");
    if gomod.exists() || gowork.exists() {
        config.go_module_config = Some(crate::linker::config::go_mod::build_go_module_config(root));
    }

    config
}

fn remove_group_entry<K: std::hash::Hash + Eq>(
    groups: &mut HashMap<K, Vec<String>>,
    key: &K,
    path: &str,
) {
    if let Some(paths) = groups.get_mut(key) {
        paths.retain(|existing| existing != path);
        if paths.is_empty() {
            groups.remove(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_relative_root_and_file_and_outside_file() {
        let mut index = ProjectIndex::new();
        assert!(index.register_root("project".into()).is_err());
        index.register_root("/project".into()).unwrap();
        assert!(index.add_file("src/a.rs", Lang::Rust).is_err());
        assert!(index.add_file("/outside/a.rs", Lang::Rust).is_err());
        assert_eq!(index.file_count(), 0);
    }

    #[test]
    fn nested_root_owns_file_by_longest_prefix() {
        let mut index = ProjectIndex::new();
        index.register_root("/project".into()).unwrap();
        index.register_root("/project/pkg".into()).unwrap();
        index.add_file("/project/pkg/src/a.rs", Lang::Rust).unwrap();
        let file = index.get_file("/project/pkg/src/a.rs").unwrap();
        assert_eq!(file.workspace_root, "/project/pkg");
        assert_eq!(file.rel_path, "src/a.rs");
    }

    #[test]
    fn duplicate_replace_and_remove_readd_keep_groups_clean() {
        let mut index = ProjectIndex::new();
        index.register_root("/project".into()).unwrap();
        index.add_file("/project/src/a.rs", Lang::Rust).unwrap();
        index
            .add_file("/project/src/a.rs", Lang::TypeScript)
            .unwrap();
        assert!(index.by_lang().get(&Lang::Rust).is_none());
        assert_eq!(index.by_lang()[&Lang::TypeScript].len(), 1);
        assert_eq!(index.by_dir()["/project/src"].len(), 1);
        assert!(index.remove_file("/project/src/a.rs"));
        assert!(index.by_dir().is_empty());
        index.add_file("/project/src/a.rs", Lang::Rust).unwrap();
        assert_eq!(index.file_count(), 1);
        assert_eq!(index.by_lang()[&Lang::Rust].len(), 1);
    }

    #[test]
    fn adding_nested_root_reassigns_existing_owner() {
        let mut index = ProjectIndex::new();
        index.register_root("/project".into()).unwrap();
        index.add_file("/project/pkg/a.rs", Lang::Rust).unwrap();
        index.register_root("/project/pkg".into()).unwrap();
        assert_eq!(
            index.get_file("/project/pkg/a.rs").unwrap().workspace_root,
            "/project/pkg"
        );
    }

    // ─── Phase 3: per-generation configuration caching tests ───────────

    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "koma-linker-p3-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Verify that rebuild_root_config parses configs and caches them in the index.
    #[test]
    fn rebuild_root_config_caches_compile_db() {
        let tmp = TempDir::new("cache-ccdb");
        let root = tmp.path();
        // Use "." as directory so paths resolve to the temp root.
        let json = r#"[{"directory":".","file":"src/main.c","arguments":["gcc","-I","./include","src/main.c"]}]"#;
        std::fs::write(root.join("compile_commands.json"), json).unwrap();
        let mut index = ProjectIndex::new();
        let root_s = normalize_lexical(&root.to_string_lossy());
        index.register_root(root_s.clone()).unwrap();
        index.rebuild_root_config(&root_s);

        let config = index.root_config(&root_s).unwrap();
        assert!(
            !config.compile_dbs.is_empty(),
            "compile DB should be cached after rebuild"
        );
        assert!(
            config.parse_failures.is_empty(),
            "no parse failures expected"
        );
    }

    /// Verify that compile_db_entry_for_file returns flags from cached DB.
    #[test]
    fn compile_db_entry_for_file_returns_cached_flags() {
        let tmp = TempDir::new("cache-flags");
        let root = tmp.path();
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        // Use "." as directory so paths resolve to the temp root.
        let json = r#"[{"directory":".","file":"src/main.c","arguments":["gcc","-I","/usr/include","-I","./include","-x","c","src/main.c"]}]"#;
        std::fs::write(root.join("compile_commands.json"), json).unwrap();

        let mut index = ProjectIndex::new();
        let root_s = normalize_lexical(&root.to_string_lossy());
        index.register_root(root_s.clone()).unwrap();
        index.rebuild_root_config(&root_s);

        let main_c = normalize_lexical(&src.join("main.c").to_string_lossy());
        let flags = index.compile_db_entry_for_file(&main_c);
        assert!(
            flags.is_some(),
            "should find compile DB entry for main.c, root={root_s}"
        );
        let flags = flags.unwrap();
        assert_eq!(flags.include_paths.len(), 2);
        assert_eq!(flags.language_mode.as_deref(), Some("c"));
    }

    /// Verify that changing config content only takes effect after explicit rebuild.
    #[test]
    fn config_cache_requires_explicit_rebuild() {
        let tmp = TempDir::new("cache-rebuild");
        let root = tmp.path();
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();

        // Initial config: gcc only.
        let json1 = r#"[{"directory":".","file":"src/main.c","arguments":["gcc","-I","/old","src/main.c"]}]"#;
        std::fs::write(root.join("compile_commands.json"), json1).unwrap();

        let mut index = ProjectIndex::new();
        let root_s = normalize_lexical(&root.to_string_lossy());
        index.register_root(root_s.clone()).unwrap();
        index.rebuild_root_config(&root_s);

        let main_c = normalize_lexical(&src.join("main.c").to_string_lossy());
        let flags1 = index.compile_db_entry_for_file(&main_c).unwrap();
        assert_eq!(flags1.include_paths, vec!["/old".to_string()]);

        // Change the config file on disk WITHOUT rebuilding cache.
        let json2 = r#"[{"directory":".","file":"src/main.c","arguments":["gcc","-I","/new","src/main.c"]}]"#;
        std::fs::write(root.join("compile_commands.json"), json2).unwrap();

        // Cache should still have old value.
        let flags2 = index.compile_db_entry_for_file(&main_c).unwrap();
        assert_eq!(
            flags2.include_paths,
            vec!["/old".to_string()],
            "cache should not reflect disk changes until rebuild"
        );

        // After explicit rebuild, new value should be visible.
        index.rebuild_root_config(&root_s);
        let flags3 = index.compile_db_entry_for_file(&main_c).unwrap();
        assert_eq!(
            flags3.include_paths,
            vec!["/new".to_string()],
            "cache should reflect new value after rebuild"
        );
    }

    /// Verify compile_flags.txt fallback works from cache.
    #[test]
    fn compile_flags_txt_fallback_from_cache() {
        let tmp = TempDir::new("cache-flags-txt");
        let root = tmp.path();
        std::fs::write(root.join("compile_flags.txt"), "-I/usr/include\n-x\nc++\n").unwrap();
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();

        let mut index = ProjectIndex::new();
        let root_s = normalize_lexical(&root.to_string_lossy());
        index.register_root(root_s.clone()).unwrap();
        index.rebuild_root_config(&root_s);

        // Use a path inside the registered root so file_owner resolves.
        let main_c = normalize_lexical(&src.join("main.c").to_string_lossy());
        let flags = index.compile_flags_for_file(&main_c);
        assert!(
            flags.is_some(),
            "should find fallback flags for path inside root"
        );
        let flags = flags.unwrap();
        assert_eq!(flags.include_paths.len(), 1);
        assert_eq!(flags.language_mode.as_deref(), Some("c++"));
    }

    /// Verify multi-root: each root has its own config cache.
    #[test]
    fn multi_root_independent_config_caches() {
        let tmp = TempDir::new("cache-multi-root");
        let root_a = tmp.path().join("a");
        let root_b = tmp.path().join("b");
        std::fs::create_dir_all(root_a.join("src")).unwrap();
        std::fs::create_dir_all(root_b.join("src")).unwrap();

        // root_a has -I./a-include
        let json_a = r#"[{"directory":".","file":"src/main.c","arguments":["gcc","-I","./a-include","src/main.c"]}]"#;
        std::fs::write(root_a.join("compile_commands.json"), json_a).unwrap();

        // root_b has -I./b-include
        let json_b = r#"[{"directory":".","file":"src/main.c","arguments":["gcc","-I","./b-include","src/main.c"]}]"#;
        std::fs::write(root_b.join("compile_commands.json"), json_b).unwrap();

        let mut index = ProjectIndex::new();
        let root_a_s = normalize_lexical(&root_a.to_string_lossy());
        let root_b_s = normalize_lexical(&root_b.to_string_lossy());
        index.register_root(root_a_s.clone()).unwrap();
        index.register_root(root_b_s.clone()).unwrap();
        index.rebuild_root_config(&root_a_s);
        index.rebuild_root_config(&root_b_s);

        let main_c_a = normalize_lexical(&root_a.join("src/main.c").to_string_lossy());
        let main_c_b = normalize_lexical(&root_b.join("src/main.c").to_string_lossy());

        let flags_a = index.compile_db_entry_for_file(&main_c_a).unwrap();
        let flags_b = index.compile_db_entry_for_file(&main_c_b).unwrap();
        assert_eq!(flags_a.include_paths, vec![format!("{root_a_s}/a-include")]);
        assert_eq!(flags_b.include_paths, vec![format!("{root_b_s}/b-include")]);
    }

    /// Verify parse failures are recorded as structured records.
    #[test]
    fn parse_failure_recorded() {
        let tmp = TempDir::new("cache-parse-fail");
        let root = tmp.path();
        std::fs::write(root.join("compile_commands.json"), "NOT VALID JSON!!!").unwrap();

        let mut index = ProjectIndex::new();
        let root_s = normalize_lexical(&root.to_string_lossy());
        index.register_root(root_s.clone()).unwrap();
        index.rebuild_root_config(&root_s);

        let config = index.root_config(&root_s).unwrap();
        assert!(
            !config.parse_failures.is_empty(),
            "parse failure should be recorded"
        );
        assert!(config.parse_failures[0]
            .path
            .contains("compile_commands.json"));
        assert!(!config.parse_failures[0].detail.is_empty());
    }

    /// Verify generation counter increments on rebuild.
    #[test]
    fn generation_increments_on_rebuild() {
        let tmp = TempDir::new("cache-gen");
        let root = tmp.path();
        let mut index = ProjectIndex::new();
        let root_s = normalize_lexical(&root.to_string_lossy());
        index.register_root(root_s.clone()).unwrap();

        let gen0 = index.generation();
        index.rebuild_root_config(&root_s);
        assert_eq!(index.generation(), gen0 + 1);
        index.rebuild_root_config(&root_s);
        assert_eq!(index.generation(), gen0 + 2);
    }
}
