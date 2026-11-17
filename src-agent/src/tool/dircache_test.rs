use super::*;

#[test]
fn normalize_slashes_in_stored_paths() {
    let input = "src\\app\\main.rs";
    let normalized = input.replace('\\', "/");
    assert_eq!(normalized, "src/app/main.rs");
}

#[test]
fn compute_dirs_works_on_normalized_slashes() {
    let files = vec![
        "src/app/main.rs".into(),
        "src/lib/mod.rs".into(),
        "README.md".into(),
    ];
    let dirs = compute_dirs(&files);
    assert!(dirs.contains(&"src/".to_string()));
    assert!(dirs.contains(&"src/app/".to_string()));
    assert!(dirs.contains(&"src/lib/".to_string()));
    assert!(!dirs.contains(&"/".to_string()));
}

#[test]
fn compute_dirs_multi_root_prefixes() {
    let files = vec!["[0]src/main.rs".into(), "[1]pkg/README.md".into()];
    let dirs = compute_dirs(&files);
    assert!(dirs.contains(&"[0]src/".to_string()));
    assert!(dirs.contains(&"[1]pkg/".to_string()));
}

#[test]
fn search_finds_after_normalize() {
    let cache = DirCache {
        files: vec![
            "src/app/main.rs".into(),
            "src/lib/mod.rs".into(),
            "README.md".into(),
        ],
        dirs: compute_dirs(&[
            "src/app/main.rs".into(),
            "src/lib/mod.rs".into(),
            "README.md".into(),
        ]),
        indexing: false,
        missing_roots: Vec::new(),
        version: 1,
        memo: Mutex::new(SearchMemo::default()),
    };
    let results = cache.search("main", 10);
    assert!(results.iter().any(|r| r.contains("main.rs")));
}

#[test]
fn search_multi_root_basename_strips_prefix() {
    // "[0]README.md" should rank as basename "README.md" when searching "readme".
    let cache = DirCache {
        files: vec![
            "[0]README.md".into(),
            "[0]src/main.rs".into(),
            "[1]pkg/README.md".into(),
        ],
        dirs: compute_dirs(&[
            "[0]README.md".into(),
            "[0]src/main.rs".into(),
            "[1]pkg/README.md".into(),
        ]),
        indexing: false,
        missing_roots: Vec::new(),
        version: 1,
        memo: Mutex::new(SearchMemo::default()),
    };
    let results = cache.search("readme", 10);
    // Both README.md entries should appear in starts-with results.
    assert!(results.iter().any(|r| r.contains("[0]README.md")));
    assert!(results.iter().any(|r| r.contains("[1]pkg/README.md")));
}

#[test]
fn to_slash_keeps_backslash_off_windows() {
    assert_eq!(super::super::winpath::to_slash("a\\b", false), "a\\b");
    assert_eq!(super::super::winpath::to_slash("a\\b", true), "a/b");
}

#[cfg(unix)]
#[test]
fn index_keeps_backslash_file_name_on_posix() {
    let tmp = std::env::temp_dir().join(format!("koma-dc-bs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("a\\b.txt"), "x").unwrap();
    let cache = Arc::new(RwLock::new(DirCache::default()));
    reindex(vec![tmp.clone()], cache.clone());
    for _ in 0..200 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        if cache.read().is_ok_and(|c| !c.indexing && c.version > 0) {
            break;
        }
    }
    let files = cache.read().unwrap().files.clone();
    assert_eq!(files, vec!["a\\b.txt".to_string()]);
    let _ = std::fs::remove_dir_all(&tmp);
}
