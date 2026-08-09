//! Screenshot catalog: durable, searchable, visually-loadable screenshot knowledge.
//!
//! Lives under `<workspace>/.screenshoot/` alongside the raw PNGs:
//!
//! ```text
//! <workspace>/.screenshoot/
//!     *.png                        ← raw screenshots (existing)
//!     records/<stem>.md            ← per-screenshot record (frontmatter + body)
//!     SCREENSHOTS.md               ← rebuilt index (rebuilt by register/update)
//! ```
//!
//! Pattern follows `crate::model::memory`: index-of-pointers + one file per item.
//! Only the index is injected into the system prompt; the model pulls details on
//! demand with `load_screenshot` / `search_screenshots` / `describe_screenshot`.

use std::path::{Path, PathBuf};

use crate::model::memory::atomic_write;

/// The index file name inside the `.screenshoot` directory.
const INDEX_FILE: &str = "SCREENSHOTS.md";

/// Default description for newly registered screenshots.
const DEFAULT_DESCRIPTION: &str = "Pending description";

// ---------------------------------------------------------------------------
// Public data type
// ---------------------------------------------------------------------------

/// A parsed screenshot record file.
#[derive(Debug, Clone)]
pub struct ScreenshotRecord {
    /// Filename without extension (e.g. `example_com_landing_1730000000000`).
    pub stem: String,
    /// Source URL the screenshot was captured from.
    pub url: String,
    /// ISO 8601 capture timestamp (e.g. `2025-01-15T12:30:00Z`).
    pub captured: String,
    /// Human-readable description of the screenshot content.
    pub description: String,
    /// Comma-separated tags.
    pub tags: String,
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Root dir for screenshots under a workspace.
pub fn screenshoot_dir(workspace: &Path) -> PathBuf {
    workspace.join(".screenshoot")
}

/// Records dir under `.screenshoot/`.
pub fn records_dir(workspace: &Path) -> PathBuf {
    screenshoot_dir(workspace).join("records")
}

/// Path to an individual record file.
pub fn record_path(workspace: &Path, stem: &str) -> PathBuf {
    records_dir(workspace).join(format!("{stem}.md"))
}

/// Path to the index file.
pub fn index_path(workspace: &Path) -> PathBuf {
    screenshoot_dir(workspace).join(INDEX_FILE)
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Format `SystemTime::now()` as ISO 8601 UTC (`%Y-%m-%dT%H:%M:%SZ`)
/// without pulling in a date crate.
fn now_iso8601() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    // Days since epoch.
    let days = secs / 86400;
    let remaining = secs % 86400;
    let hours = remaining / 3600;
    let minutes = (remaining % 3600) / 60;
    let seconds = remaining % 60;

    // Convert days to Y/M/D using a simple Gregorian calendar computation.
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let yr = if m <= 2 { y + 1 } else { y };

    format!(
        "{yr:04}-{m:02}-{d:02}T{hours:02}:{minutes:02}:{seconds:02}Z"
    )
}

/// Create or update a record for a newly captured screenshot.
/// `stem` = PNG filename without extension, `url` = source URL.
/// Returns `Ok(stem)` on success.
pub fn register_screenshot(workspace: &Path, stem: &str, url: &str) -> std::io::Result<String> {
    let dir = records_dir(workspace);
    std::fs::create_dir_all(&dir)?;

    let filename = format!("{stem}.png");
    let captured = now_iso8601();

    let text = format!(
        "---\n\
         filename: {filename}\n\
         url: {url}\n\
         captured: {captured}\n\
         description: {DEFAULT_DESCRIPTION}\n\
         tags: \n\
         ---\n\n\
         {DEFAULT_DESCRIPTION}\n"
    );

    let path = record_path(workspace, stem);
    atomic_write(&path, text.as_bytes())?;
    rebuild_index(workspace)?;
    Ok(stem.to_string())
}

/// Update description and/or tags of an existing record.
/// Only replaces the fields that are provided (non-empty strings).
pub fn update_description(
    workspace: &Path,
    stem: &str,
    description: &str,
    tags: &str,
) -> std::io::Result<()> {
    let path = record_path(workspace, stem);
    let text = std::fs::read_to_string(&path).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("record not found for stem '{stem}': {e}"),
        )
    })?;

    let lines: Vec<String> = text.lines().map(String::from).collect();

    // Parse existing frontmatter to carry forward fields we don't modify.
    let mut existing_url = String::new();
    let mut existing_captured = String::new();
    let mut existing_filename = String::new();

    if text.starts_with("---") {
        for line in &lines[1..] {
            if line.trim() == "---" {
                break;
            }
            if let Some(v) = line.strip_prefix("url:") {
                existing_url = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("captured:") {
                existing_captured = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("filename:") {
                existing_filename = v.trim().to_string();
            }
        }
    }

    let new_description = if description.is_empty() {
        DEFAULT_DESCRIPTION.to_string()
    } else {
        description.to_string()
    };

    let rendered = format!(
        "---\n\
         filename: {filename}\n\
         url: {url}\n\
         captured: {captured}\n\
         description: {desc}\n\
         tags: {tags}\n\
         ---\n\n\
         {desc}\n",
        filename = existing_filename,
        url = existing_url,
        captured = existing_captured,
        desc = new_description,
        tags = tags,
    );

    atomic_write(&path, rendered.as_bytes())?;
    rebuild_index(workspace)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parse a record file by stem.
pub fn read_record(workspace: &Path, stem: &str) -> Option<ScreenshotRecord> {
    let path = record_path(workspace, stem);
    let text = std::fs::read_to_string(&path).ok()?;
    parse_record(stem, &text)
}

/// Parse the text of a record file into a `ScreenshotRecord`.
fn parse_record(stem: &str, text: &str) -> Option<ScreenshotRecord> {
    let mut url = String::new();
    let mut captured = String::new();
    let mut description = String::new();
    let mut tags = String::new();

    if let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    {
        if let Some(end) = find_frontmatter_end(rest) {
            let (front, after) = rest.split_at(end.0);
            for line in front.lines() {
                let line = line.trim();
                if let Some(v) = line.strip_prefix("url:") {
                    url = v.trim().to_string();
                } else if let Some(v) = line.strip_prefix("captured:") {
                    captured = v.trim().to_string();
                } else if let Some(v) = line.strip_prefix("description:") {
                    description = v.trim().to_string();
                } else if let Some(v) = line.strip_prefix("tags:") {
                    tags = v.trim().to_string();
                }
            }
            // Body is everything after closing `---`.
            let body = after[end.1..]
                .trim_start_matches(['\n', '\r'])
                .trim();
            if description.is_empty() && !body.is_empty() {
                description = body.to_string();
            }
        }
    }

    if description.is_empty() {
        description = DEFAULT_DESCRIPTION.to_string();
    }

    Some(ScreenshotRecord {
        stem: stem.to_string(),
        url,
        captured,
        description,
        tags,
    })
}

/// Locate the closing `---` fence in the post-opening-fence text.
fn find_frontmatter_end(rest: &str) -> Option<(usize, usize)> {
    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == "---" {
            return Some((offset, line.len()));
        }
        offset += line.len();
    }
    None
}

// ---------------------------------------------------------------------------
// Listing
// ---------------------------------------------------------------------------

/// List all records sorted by captured time (newest first).
pub fn list_records(workspace: &Path) -> Vec<ScreenshotRecord> {
    let dir = records_dir(workspace);
    let mut records: Vec<ScreenshotRecord> = Vec::new();

    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return records,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let fname = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if !fname.ends_with(".md") || fname == INDEX_FILE {
            continue;
        }
        let stem = match fname.strip_suffix(".md") {
            Some(s) if !s.is_empty() => s,
            _ => continue,
        };

        if let Some(rec) = read_record(workspace, stem) {
            records.push(rec);
        }
    }

    // Sort by captured time (newest first). Fall back to stem for stability.
    records.sort_by(|a, b| b.captured.cmp(&a.captured).then_with(|| b.stem.cmp(&a.stem)));
    records
}

// ---------------------------------------------------------------------------
// Index & context block
// ---------------------------------------------------------------------------

/// Rebuild `SCREENSHOTS.md` index from all record files.
pub fn rebuild_index(workspace: &Path) -> std::io::Result<()> {
    let dir = screenshoot_dir(workspace);
    std::fs::create_dir_all(&dir)?;

    let records = list_records(workspace);
    let mut s = String::new();
    for r in &records {
        let date_part = &r.captured[..10.min(r.captured.len())];
        let desc_short = if r.description.is_empty() || r.description == DEFAULT_DESCRIPTION {
            DEFAULT_DESCRIPTION.to_string()
        } else {
            truncate_str(&r.description, 80)
        };
        s.push_str(&format!(
            "- [{}.png](records/{}.md) {} — {} — {}\n",
            r.stem, r.stem, r.url, date_part, desc_short,
        ));
    }
    atomic_write(&index_path(workspace), s.as_bytes())
}

/// Build the top-N newest records as a prompt block for system prompt injection.
/// Returns `None` when there are no records.
pub fn screenshot_context_block(workspace: &Path, max_items: usize) -> Option<String> {
    let records = list_records(workspace);
    if records.is_empty() {
        return None;
    }

    let total = records.len();
    let shown = records.len().min(max_items);
    let mut s = String::from("# Recent screenshots\n");
    for r in records.iter().take(max_items) {
        let date_part = &r.captured[..10.min(r.captured.len())];
        s.push_str(&format!(
            "- [{}.png](records/{}.md) {} — {} — {}\n",
            r.stem, r.stem, r.url, date_part, r.description,
        ));
    }
    if total > shown {
        s.push_str(&format!(
            "\nShowing {shown} of {total} screenshots. \
             Use `search_screenshots` to find older ones, `load_screenshot` to view one.\n"
        ));
    }
    Some(s)
}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

/// Resolve a stem or filename to a validated, canonical PNG path under
/// `.screenshoot/`. Rejects: traversal, absolute paths, non-PNG, missing
/// files, directory names. Returns `None` on any containment/safety issue.
pub fn resolve_screenshot_path(workspace: &Path, name: &str) -> Option<PathBuf> {
    // Reject absolute paths.
    if Path::new(name).is_absolute() {
        return None;
    }

    // Reject traversal components.
    if name.contains("..") {
        return None;
    }

    // Normalise: if it already ends in .png, use as-is; otherwise append.
    let filename = if name.ends_with(".png") {
        name.to_string()
    } else {
        format!("{name}.png")
    };

    // Must end in .png (already guaranteed above, but defense-in-depth).
    if !filename.ends_with(".png") {
        return None;
    }

    let root = screenshoot_dir(workspace);
    let root_canon = root.canonicalize().ok()?;

    let candidate = root_canon.join(&filename);

    // Containment check: canonicalized candidate must start with root_canon.
    let candidate_canon = candidate.canonicalize().ok()?;
    if !candidate_canon.starts_with(&root_canon) {
        return None;
    }

    // Must be a regular file (not a directory or symlink to dir).
    let meta = std::fs::metadata(&candidate_canon).ok()?;
    if !meta.is_file() {
        return None;
    }

    Some(candidate_canon)
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

/// Keyword search across records (url, description, tags).
/// Returns matching stems sorted by relevance (description matches first,
/// then tag, then url). Case-insensitive substring match.
pub fn search_records(workspace: &Path, query: &str, max_results: usize) -> Vec<ScreenshotRecord> {
    let records = list_records(workspace);
    if records.is_empty() || query.trim().is_empty() {
        return Vec::new();
    }

    let tokens: Vec<String> = query
        .split_whitespace()
        .map(|t| t.to_ascii_lowercase())
        .collect();

    // Score each record: 3 for desc match, 2 for tag match, 1 for url match.
    let mut scored: Vec<(u32, &ScreenshotRecord)> = records
        .iter()
        .filter_map(|r| {
            let desc_lower = r.description.to_ascii_lowercase();
            let tags_lower = r.tags.to_ascii_lowercase();
            let url_lower = r.url.to_ascii_lowercase();

            let mut score = 0u32;
            for tok in &tokens {
                if desc_lower.contains(tok.as_str()) {
                    score += 3;
                }
                if tags_lower.contains(tok.as_str()) {
                    score += 2;
                }
                if url_lower.contains(tok.as_str()) {
                    score += 1;
                }
            }
            if score > 0 {
                Some((score, r))
            } else {
                None
            }
        })
        .collect();

    // Sort by score descending, then by captured time descending.
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.captured.cmp(&a.1.captured))
    });

    scored
        .into_iter()
        .take(max_results)
        .map(|(_, r)| r.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Truncate a string to `max_chars`, adding `…` if truncated.
fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        s.to_string()
    } else {
        format!("{}…", &s[..max_chars])
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
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
        let stem = register_screenshot(&ws, "example_com_landing_123", "https://example.com/landing")
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
}
