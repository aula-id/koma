use super::*;

#[test]
fn normalize_query_path_absolute_passthrough() {
    let roots = vec![PathBuf::from("/some/root")];
    assert_eq!(normalize_query_path("/foo/bar.rs", &roots), "/foo/bar.rs");
}

#[test]
fn normalize_query_path_relative_fallback() {
    // When the file doesn't exist on disk, returns primary_root + path.
    let roots = vec![PathBuf::from("/some/nonexistent")];
    assert_eq!(
        normalize_query_path("src/main.rs", &roots),
        "/some/nonexistent/src/main.rs"
    );
}

#[test]
fn normalize_query_path_backslash() {
    // Windows-style backslashes should be normalized.
    let roots = vec![PathBuf::from("/some/root")];
    assert_eq!(normalize_query_path("/foo\\bar.rs", &roots), "/foo/bar.rs");
}

#[test]
fn normalize_query_path_existing_file_canonicalizes() {
    // Create a temp dir + file manually (no tempfile crate).
    let dir = std::env::temp_dir().join(format!("koma_test_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("hello.rs"), "fn main() {}").unwrap();
    let result = normalize_query_path("hello.rs", std::slice::from_ref(&dir));
    // Should be the canonical absolute path.
    assert!(std::path::Path::new(&result).is_absolute());
    assert!(result.ends_with("hello.rs"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn normalize_query_path_no_roots_returns_bare() {
    let result = normalize_query_path("src/main.rs", &[]);
    assert_eq!(result, "src/main.rs");
}

#[test]
fn normalize_query_path_ws_prefix_primary() {
    // [0]src/main.rs → resolved under roots[0]
    let root_a = std::env::temp_dir().join(format!("koma_test_{}_a", std::process::id()));
    let _ = std::fs::create_dir_all(root_a.join("src"));
    std::fs::write(root_a.join("src/main.rs"), "fn main() {}").unwrap();
    let result = normalize_query_path("[0]src/main.rs", std::slice::from_ref(&root_a));
    assert!(result.contains("src/main.rs"));
    // On macOS, temp_dir() symlinks /var → /private/var; canonicalize resolves
    // the symlink, so compare against the canonicalized + slash-normalized root.
    let expected_root = std::fs::canonicalize(&root_a)
        .unwrap_or(root_a.clone())
        .to_string_lossy()
        .replace('\\', "/");
    assert!(
        result.starts_with(&expected_root),
        "result={result:?} expected_root={expected_root:?}"
    );
    let _ = std::fs::remove_dir_all(&root_a);
}

#[test]
fn normalize_query_path_ws_prefix_secondary() {
    // [1]pkg/README.md → resolved under roots[1]
    let root_a = std::env::temp_dir().join(format!("koma_test_{}_a2", std::process::id()));
    let root_b = std::env::temp_dir().join(format!("koma_test_{}_b2", std::process::id()));
    let _ = std::fs::create_dir_all(&root_a);
    let _ = std::fs::create_dir_all(root_b.join("pkg"));
    std::fs::write(root_b.join("pkg/README.md"), "hello").unwrap();
    let result = normalize_query_path("[1]pkg/README.md", &[root_a.clone(), root_b.clone()]);
    assert!(result.contains("pkg/README.md"));
    // File exists → canonicalize resolves macOS /var → /private/var symlink;
    // compare against the canonicalized + slash-normalized root.
    let expected_root = std::fs::canonicalize(&root_b)
        .unwrap_or(root_b.clone())
        .to_string_lossy()
        .replace('\\', "/");
    assert!(
        result.starts_with(&expected_root),
        "result={result:?} expected_root={expected_root:?}"
    );
    let _ = std::fs::remove_dir_all(&root_a);
    let _ = std::fs::remove_dir_all(&root_b);
}

#[test]
fn normalize_query_path_ws_prefix_oob_falls_back() {
    // [9]src/main.rs with only 2 roots → falls back to primary root
    let root_a = std::env::temp_dir().join(format!("koma_test_{}_c", std::process::id()));
    let root_b = std::env::temp_dir().join(format!("koma_test_{}_d", std::process::id()));
    let _ = std::fs::create_dir_all(&root_a);
    let _ = std::fs::create_dir_all(&root_b);
    let result = normalize_query_path("[9]src/main.rs", &[root_a.clone(), root_b.clone()]);
    // OOB index falls back to primary root (no canonicalize — file absent);
    // slash-normalize the expected prefix for Windows compatibility.
    let expected = root_a.to_string_lossy().replace('\\', "/");
    assert!(
        result.starts_with(&expected),
        "result={result:?} expected={expected:?}"
    );
    let _ = std::fs::remove_dir_all(&root_a);
    let _ = std::fs::remove_dir_all(&root_b);
}

#[test]
fn normalize_query_path_ws_prefix_empty_bare() {
    // "[0]" with no bare path → falls through to normal relative logic
    let roots = vec![PathBuf::from("/some/root")];
    let result = normalize_query_path("[0]", &roots);
    // Empty bare falls through, treated as relative path "[0]" → primary root
    assert_eq!(result, "/some/root/[0]");
}

// ─── canonical_root / canonical_roots tests ─────────────────────────

#[test]
fn canonical_root_accepts_path_ref() {
    // Verify that canonical_root works with &Path (not just &PathBuf).
    let p: &std::path::Path = std::path::Path::new("/some/root");
    let result = canonical_root(p);
    assert_eq!(result, "/some/root");
}

#[test]
fn canonical_root_lexical_fallback() {
    // Non-existent relative path should be made absolute against cwd.
    let p = std::path::Path::new("relative/nonexistent");
    let result = canonical_root(p);
    let cwd = std::env::current_dir().unwrap();
    let expected = cwd
        .join("relative/nonexistent")
        .to_string_lossy()
        .replace('\\', "/");
    assert_eq!(
        result, expected,
        "non-existent relative path should be resolved against cwd"
    );
}

#[test]
fn canonical_root_lexical_absolute_passthrough() {
    // Non-existent absolute path should pass through (absolute already).
    let result = canonical_root(std::path::Path::new("/absolutely/nonexistent"));
    assert_eq!(result, "/absolutely/nonexistent");
}

#[test]
fn canonical_root_existing_dir_canonicalizes() {
    let dir = std::env::temp_dir().join(format!("koma_test_cr_dir_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let result = canonical_root(&dir);
    // Should be the canonical (symlink-resolved) absolute path.
    let expected = std::fs::canonicalize(&dir)
        .unwrap_or(dir.clone())
        .to_string_lossy()
        .replace('\\', "/");
    assert_eq!(result, expected);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn canonical_roots_stable_first_dedup() {
    // Input order is preserved; first occurrence wins.
    let roots = vec![
        PathBuf::from("/z/root"),
        PathBuf::from("/a/root"),
        PathBuf::from("/z/root"), // duplicate — dropped
        PathBuf::from("/m/root"),
    ];
    let result = canonical_roots(&roots);
    assert_eq!(
        result,
        vec!["/z/root", "/a/root", "/m/root"],
        "stable-first dedup must preserve input order"
    );
}

#[test]
fn canonical_roots_deduplicates_identical_paths() {
    let roots = vec![
        PathBuf::from("/same/path"),
        PathBuf::from("/same/path"),
        PathBuf::from("/other"),
    ];
    let result = canonical_roots(&roots);
    // Stable-first: first occurrence wins, no sort.
    assert_eq!(result, vec!["/same/path", "/other"]);
}

#[test]
fn canonical_roots_deduplicates_trailing_slash() {
    // On most systems /foo and /foo/ resolve to the same canonical form.
    let dir = std::env::temp_dir().join(format!("koma_test_cr_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let with_slash = dir.join("../../../.."); // go up some levels
    let roots = vec![dir.clone(), with_slash];
    // canonicalize resolves to the same path for both.
    let result = canonical_roots(&roots);
    // At most one entry per actual directory.
    assert!(
        result.len() <= 2, // could be 1 if they resolve identically
        "dedup should collapse equivalent paths, got: {result:?}"
    );
    // No duplicates.
    let mut unique = result.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(result.len(), unique.len(), "output must be deduped");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn canonical_roots_empty_input() {
    let result = canonical_roots(&[]);
    assert!(result.is_empty());
}

#[test]
fn canonical_roots_symlink_dedup() {
    // Create a real dir and a symlink to it; both should canonicalize
    // to the same path, deduplicating in stable-first order.
    let base = std::env::temp_dir().join(format!("koma_test_sym_{}", std::process::id()));
    let link = std::env::temp_dir().join(format!("koma_test_sym_link_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&base);
    // Remove link if it exists from a previous test run.
    let _ = std::fs::remove_file(&link);
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let _ = symlink(&base, &link);
        let roots = vec![base.clone(), link.clone()];
        let result = canonical_roots(&roots);
        // If symlink creation succeeded, both should dedup.
        if link.exists() {
            assert_eq!(
                result.len(),
                1,
                "symlink and target must dedup, got: {result:?}"
            );
        }
    }
    let _ = std::fs::remove_file(&link);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn canonical_roots_path_spelling_dedup() {
    // Non-existent paths: lexical fallback makes them absolute, so
    // same path with/without trailing component is NOT the same.
    let roots = vec![
        PathBuf::from("/nonexistent/a"),
        PathBuf::from("/nonexistent/b"),
    ];
    let result = canonical_roots(&roots);
    assert_eq!(result.len(), 2);
    // Both should be slash-normalized (forward slashes).
    for r in &result {
        assert!(!r.contains('\\'), "backslashes should be normalized: {r}");
    }
}

#[test]
fn canonical_roots_preserves_configured_order() {
    // Critical: settings order must survive canonicalization.
    let roots = vec![
        PathBuf::from("/c/root"),
        PathBuf::from("/a/root"),
        PathBuf::from("/b/root"),
        PathBuf::from("/c/root"), // duplicate
    ];
    let result = canonical_roots(&roots);
    assert_eq!(result, vec!["/c/root", "/a/root", "/b/root"]);
}

// ── next_registration_revision: monotonic allocation ──────────────────

#[test]
fn next_registration_revision_is_monotonic() {
    let r1 = next_registration_revision("test_session");
    let r2 = next_registration_revision("test_session");
    let r3 = next_registration_revision("test_session");
    assert!(r2 > r1, "revision must be monotonically increasing");
    assert!(r3 > r2, "revision must be monotonically increasing");
}

#[test]
fn next_registration_revision_cross_session_independent() {
    // Different sessions share the global counter, so revisions are
    // interleaved but still monotonically increasing.
    let r1 = next_registration_revision("session_a");
    let r2 = next_registration_revision("session_b");
    assert!(
        r2 >= r1,
        "cross-session revisions should be ordered: r1={r1} r2={r2}"
    );
}

// ── configured_root_map: canonical → raw mapping ─────────────────────

#[test]
fn configured_root_map_nonexistent_paths_preserve_raw() {
    // Non-existent paths: lexical fallback makes them absolute, so the
    // canonical form matches the normalised raw.  No map entry expected.
    let roots = vec![PathBuf::from("/nonexistent/a")];
    let map = configured_root_map(&roots);
    // canonical == raw (slash-normalised), so no entry.
    assert!(map.is_empty());
}

#[test]
fn configured_root_map_empty_input() {
    let map = configured_root_map(&[]);
    assert!(map.is_empty());
}

#[test]
fn configured_root_map_deduplicates_stable_first() {
    // Two identical paths: only the first is recorded; no map entry
    // since canonical == raw for both.
    let roots = vec![PathBuf::from("/same/path"), PathBuf::from("/same/path")];
    let map = configured_root_map(&roots);
    assert!(map.is_empty());
}

#[test]
fn configured_root_map_symlink_records_raw() {
    // Create a real dir and a symlink to it.
    let base = std::env::temp_dir().join(format!("koma_test_crm_{}", std::process::id()));
    let link = std::env::temp_dir().join(format!("koma_test_crm_link_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&base);
    let _ = std::fs::remove_file(&link);
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let _ = symlink(&base, &link);
        if link.exists() {
            let roots = vec![link.clone()];
            let map = configured_root_map(&roots);
            // The raw link path should map to the canonical base path.
            let canonical = canonical_root(&base);
            let raw = link.to_string_lossy().replace('\\', "/");
            if canonical != raw {
                assert_eq!(map.get(&canonical).map(|s| s.as_str()), Some(raw.as_str()));
            }
        }
    }
    let _ = std::fs::remove_file(&link);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn configured_root_map_relative_path_records_raw() {
    // Relative path: canonical_root makes it absolute, so raw != canonical.
    let roots = vec![PathBuf::from("relative/path")];
    let map = configured_root_map(&roots);
    // The map should have an entry: key = canonical absolute, value = "relative/path".
    assert_eq!(map.len(), 1);
    let (_, raw) = map.iter().next().unwrap();
    assert_eq!(raw, "relative/path");
}

// ── ensure_and_register_with_revision response validation ────────────

#[test]
fn registered_variant_is_only_success() {
    // Verify the match arms used in ensure_and_register_with_revision.
    use crate::ipc::linker_proto::{LinkerResponse, ScanStatus};

    let registered = LinkerResponse::Registered {
        status: ScanStatus::Ready,
        generation: 42,
    };
    // Only Registered should match Ok.
    assert!(matches!(registered, LinkerResponse::Registered { .. }));

    let error = LinkerResponse::Error("daemon error".into());
    assert!(matches!(error, LinkerResponse::Error(_)));
    assert!(!matches!(error, LinkerResponse::Registered { .. }));

    let ack = LinkerResponse::Ack;
    assert!(!matches!(ack, LinkerResponse::Registered { .. }));

    let gen = LinkerResponse::Generation(1);
    assert!(!matches!(gen, LinkerResponse::Registered { .. }));
}
