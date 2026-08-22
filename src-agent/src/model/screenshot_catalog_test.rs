#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

/// Create a temp workspace with `.screenshoot/records/` and return it.
fn temp_workspace() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ss_catalog_test_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(dir.join(".screenshoot/records")).unwrap();
    dir
}

#[test]
fn register_and_read_record() {
    let ws = temp_workspace();
    let stem = register_screenshot(
        &ws,
        "example_com_landing_123",
        "https://example.com/landing",
    )
    .unwrap();
    assert_eq!(stem, "example_com_landing_123");

    let rec = read_record(&ws, "example_com_landing_123").unwrap();
    assert_eq!(rec.stem, "example_com_landing_123");
    assert_eq!(rec.url, "https://example.com/landing");
    assert!(rec.captured.contains("T"));
    assert_eq!(rec.description, DEFAULT_DESCRIPTION);
    assert!(rec.tags.is_empty());

    std::fs::remove_dir_all(&ws).ok();
}

#[test]
fn update_description_preserves_other_fields() {
    let ws = temp_workspace();
    register_screenshot(&ws, "test_stem", "https://test.com").unwrap();

    update_description(
        &ws,
        "test_stem",
        "A dark dashboard with charts",
        "dashboard, dark, analytics",
    )
    .unwrap();

    let rec = read_record(&ws, "test_stem").unwrap();
    assert_eq!(rec.description, "A dark dashboard with charts");
    assert_eq!(rec.tags, "dashboard, dark, analytics");
    // Other fields preserved.
    assert_eq!(rec.url, "https://test.com");

    std::fs::remove_dir_all(&ws).ok();
}

#[test]
fn rebuild_index_and_context_block() {
    let ws = temp_workspace();
    register_screenshot(&ws, "aaa_111", "https://aaa.com").unwrap();
    register_screenshot(&ws, "bbb_222", "https://bbb.com").unwrap();
    register_screenshot(&ws, "ccc_333", "https://ccc.com").unwrap();

    rebuild_index(&ws).unwrap();

    let index_text = std::fs::read_to_string(index_path(&ws)).unwrap();
    assert!(index_text.contains("aaa_111.png"), "{index_text}");
    assert!(index_text.contains("bbb_222.png"), "{index_text}");
    assert!(index_text.contains("ccc_333.png"), "{index_text}");

    let block = screenshot_context_block(&ws, 10).unwrap();
    assert!(block.starts_with("# Recent screenshots"), "{block}");
    assert!(block.contains("aaa_111.png"), "{block}");

    std::fs::remove_dir_all(&ws).ok();
}

#[test]
fn search_records_finds_url_and_desc() {
    let ws = temp_workspace();
    register_screenshot(&ws, "dash_dark", "https://dashboard.example.com").unwrap();
    update_description(&ws, "dash_dark", "Dark mode dashboard view", "").unwrap();

    register_screenshot(&ws, "login_page", "https://auth.example.com/login").unwrap();
    update_description(&ws, "login_page", "Login form with Google SSO", "").unwrap();

    register_screenshot(&ws, "settings_page", "https://settings.example.com").unwrap();
    update_description(&ws, "settings_page", "User settings panel", "").unwrap();

    let results = search_records(&ws, "dashboard", 10);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].stem, "dash_dark");

    let results = search_records(&ws, "example.com", 10);
    assert_eq!(results.len(), 3);

    let results = search_records(&ws, "login SSO", 10);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].stem, "login_page");

    std::fs::remove_dir_all(&ws).ok();
}

#[test]
fn resolve_path_containment() {
    let ws = temp_workspace();
    let ss_dir = screenshoot_dir(&ws);

    // Create a test PNG file (minimal valid PNG header).
    let png_header: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
        0x44, 0x52,
    ];
    std::fs::write(ss_dir.join("good.png"), png_header).unwrap();
    // Create a directory named "is_dir.png".
    std::fs::create_dir(ss_dir.join("is_dir.png")).unwrap();

    // Valid file.
    assert!(resolve_screenshot_path(&ws, "good").is_some());
    assert!(resolve_screenshot_path(&ws, "good.png").is_some());

    // Traversal.
    assert!(resolve_screenshot_path(&ws, "../etc/passwd").is_none());
    assert!(resolve_screenshot_path(&ws, "foo/../../bar").is_none());

    // Absolute path.
    assert!(resolve_screenshot_path(&ws, "/etc/passwd").is_none());

    // Non-PNG.
    assert!(resolve_screenshot_path(&ws, "good.txt").is_none());

    // Missing file.
    assert!(resolve_screenshot_path(&ws, "nonexistent").is_none());

    // Directory.
    assert!(resolve_screenshot_path(&ws, "is_dir").is_none());

    std::fs::remove_dir_all(&ws).ok();
}

#[test]
fn search_no_records() {
    let ws = temp_workspace();
    let results = search_records(&ws, "anything", 10);
    assert!(results.is_empty());
}

#[test]
fn list_records_ordering() {
    let ws = temp_workspace();
    // Register multiple screenshots (they'll get different captured timestamps
    // because of the sleep-like ordering, but stems are different so sort is stable).
    register_screenshot(&ws, "zzz_last", "https://zzz.com").unwrap();
    register_screenshot(&ws, "aaa_first", "https://aaa.com").unwrap();
    register_screenshot(&ws, "mmm_middle", "https://mmm.com").unwrap();

    let records = list_records(&ws);
    assert_eq!(records.len(), 3);
    // All should be present (order depends on timestamps, but newest first).
    let stems: Vec<&str> = records.iter().map(|r| r.stem.as_str()).collect();
    assert!(stems.contains(&"zzz_last"));
    assert!(stems.contains(&"aaa_first"));
    assert!(stems.contains(&"mmm_middle"));

    std::fs::remove_dir_all(&ws).ok();
}

#[test]
fn context_block_empty_when_no_records() {
    let ws = temp_workspace();
    assert!(screenshot_context_block(&ws, 10).is_none());
}

#[test]
fn context_block_limits_items() {
    let ws = temp_workspace();
    for i in 0..5 {
        register_screenshot(&ws, &format!("s_{i}"), &format!("https://{i}.com")).unwrap();
    }
    let block = screenshot_context_block(&ws, 2).unwrap();
    assert!(block.contains("2 of 5 screenshots"), "{block}");

    std::fs::remove_dir_all(&ws).ok();
}
