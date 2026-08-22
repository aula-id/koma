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
