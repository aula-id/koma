use super::*;

#[test]
fn slug_takes_first_two_words_and_joins_with_dash() {
    assert_eq!(command_slug("cargo build --release"), "cargo-build");
    assert_eq!(command_slug("git log --oneline"), "git-log");
}

#[test]
fn slug_lowercases_and_sanitizes_weird_chars() {
    assert_eq!(command_slug("RM -rf"), "rm-rf");
    // Leading `.`/`/` collapse into one leading dash, which trim_matches
    // then strips; internal runs of non-alnum chars (`/`, `-`, `.`, ` `)
    // each collapse to a single dash.
    assert_eq!(command_slug("./scripts/Run-Thing.sh --now"), "scripts-run-thing-sh-now");
    // Only the first two whitespace-separated words are used.
    assert_eq!(command_slug("echo $(git log)"), "echo-git");
}

#[test]
fn slug_caps_at_40_chars() {
    let long_command = "a".repeat(100);
    let slug = command_slug(&long_command);
    assert!(slug.chars().count() <= 40);
}

/// A unique path under the OS temp root for a single test, removed
/// recursively on drop (if it ever got created). No `tempfile` dep in this
/// crate's Cargo.toml, so roll our own. Deliberately does NOT create the
/// directory itself — several tests assert it's created lazily only when
/// something is actually written into it.
struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "koma-shell-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        TempDir(dir)
    }
    fn path(&self) -> &Path { &self.0 }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn gc_keeps_only_the_50_newest_logs() {
    let dir = TempDir::new("gc");
    std::fs::create_dir_all(dir.path()).unwrap();
    // 55 tiny fake logs, epoch-prefixed so filename sort is chronological.
    for i in 0..55u64 {
        let name = format!("{:020}_fake.log", i);
        std::fs::write(dir.path().join(name), b"x").unwrap();
    }
    gc_log_dir(dir.path());

    let mut remaining: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    remaining.sort();

    assert_eq!(remaining.len(), 50);
    // The 50 newest are indices 5..=54 (i.e. the oldest 5 got deleted).
    let expected_first = format!("{:020}_fake.log", 5u64);
    assert_eq!(remaining.first().unwrap(), &expected_first);
}

#[test]
fn tee_not_written_when_output_clean_unchanged_and_small() {
    let dir = TempDir::new("no-write");
    let opts = OutputOpts { saving: true, log_dir: Some(dir.path().to_path_buf()) };
    let raw = "hello world\n".to_string();
    let out = finalize_output("echo hello world", raw, ShellExit::Code(Some(0)), &opts);

    // No filter matched, no truncation, clean exit -> no tee write, no
    // full-output line, and the log dir was never even created.
    assert!(!out.contains("full-output:"));
    assert!(out.contains("exit code: 0"));
    assert!(!dir.path().exists());
}

#[test]
fn tee_written_when_output_would_truncate() {
    let dir = TempDir::new("truncate-write");
    let opts = OutputOpts { saving: true, log_dir: Some(dir.path().to_path_buf()) };
    const MAX_CHARS: usize = crate::config::MAX_TOOL_OUTPUT_CHARS;
    let raw = "a".repeat(MAX_CHARS + 10);
    let out = finalize_output("cat bigfile", raw, ShellExit::Code(Some(0)), &opts);

    assert!(out.contains("full-output:"));
    let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().filter_map(|e| e.ok()).collect();
    assert_eq!(entries.len(), 1);
}

#[test]
fn tee_written_on_nonzero_exit_even_when_small_and_unfiltered() {
    let dir = TempDir::new("nonzero-write");
    let opts = OutputOpts { saving: true, log_dir: Some(dir.path().to_path_buf()) };
    let out = finalize_output("false", "".to_string(), ShellExit::Code(Some(1)), &opts);

    assert!(out.contains("full-output:"));
}

#[test]
fn early_exit_never_tees() {
    let dir = TempDir::new("early-no-write");
    let opts = OutputOpts { saving: true, log_dir: Some(dir.path().to_path_buf()) };
    let out = finalize_output("whatever", "command timed out after 1ms".to_string(), ShellExit::Early, &opts);

    assert_eq!(out, "command timed out after 1ms");
    assert!(!dir.path().exists());
}
