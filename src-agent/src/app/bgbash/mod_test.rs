use super::*;

// `render_finished_output` is the pure decision+rendering seam for the
// `bash_output` "saving" path: given the raw buffer, the job's command, its
// exit code, and the `bash_saving` setting, it decides whether to run the
// shared shell-output filter and builds the model-visible text. The
// pattern/tail_lines short-circuit lives in
// `app::runtime::stream::tools::approval`'s `bash_output` arm (the runtime
// only calls this helper once it has already confirmed neither was passed),
// so that branch isn't reachable as a unit test here — it's covered by
// exercising this helper directly instead, matching what the spec asks for
// when the seam only exists at the call site.

#[test]
fn finished_cargo_build_with_spam_gets_filtered_and_marked() {
    let raw = "\
   Compiling foo v0.1.0
   Compiling bar v0.2.0
    Finished dev [unoptimized + debuginfo] target(s) in 1.00s
";
    let (text, should_tee) = render_finished_output("cargo build", raw, 0, true);
    assert!(text.contains("[filter: cargo-build,"), "expected a filter marker, got: {text}");
    assert!(!text.contains("Compiling"), "noise should have been stripped, got: {text}");
    assert!(should_tee, "a filter that changed the output should request a tee");
}

#[test]
fn finished_job_with_saving_off_is_raw_and_untee_d() {
    let raw = "\
   Compiling foo v0.1.0
    Finished dev [unoptimized + debuginfo] target(s) in 1.00s
";
    let (text, should_tee) = render_finished_output("cargo build", raw, 0, false);
    assert_eq!(text, raw, "saving off must return the raw buffer untouched");
    assert!(!should_tee, "saving off must never request a tee");
}

#[test]
fn nonzero_exit_git_output_passes_through_unchanged() {
    // The git smart filter bails on any non-zero exit status (see
    // `tool::shell_filter::smart::git::try_filter`), and no static spec-table
    // entry matches a bare `git status`, so this must come back byte-identical
    // to the raw buffer even though `saving` is on.
    let raw = "fatal: not a git repository (or any of the parent directories): .git\n";
    let (text, should_tee) = render_finished_output("git status", raw, 128, true);
    assert_eq!(text, raw, "a failed git command must pass through unfiltered");
    // Non-zero exit still requests a tee even though the filter left the text
    // unchanged — mirrors `tool::shell::finalize_output`'s "might have lost
    // information" heuristic (a failing command is worth keeping the full log
    // for, filter or no filter).
    assert!(should_tee, "a non-zero exit should still request a tee");
}

#[test]
fn unchanged_output_on_clean_exit_does_not_request_a_tee() {
    let raw = "hello world\n";
    let (text, should_tee) = render_finished_output("echo hi", raw, 0, true);
    assert_eq!(text, raw);
    assert!(!should_tee, "an unchanged, clean-exit output has nothing worth teeing");
}

#[test]
fn ensure_tee_log_is_idempotent_and_reuses_the_same_path() {
    let dir = TempDir::new("bgbash-tee");
    let shared = Arc::new(BashJobShared {
        output: Mutex::new(String::new()),
        status: Mutex::new(BashJobStatus::Done(1)),
        pid: Mutex::new(None),
        ended_at: Mutex::new(None),
        tee_path: Mutex::new(None),
    });
    let job = BashJob {
        id: 1,
        command: "cargo build".to_string(),
        started_at: Instant::now(),
        shared,
    };

    let first = job.ensure_tee_log(dir.path(), "some output\n");
    assert!(first.is_some(), "first tee should succeed and return a path");

    // Write again with DIFFERENT content — the path must not change, and the
    // file on disk must not be rewritten (still holds the FIRST content).
    let second = job.ensure_tee_log(dir.path(), "totally different output\n");
    assert_eq!(first, second, "a second poll must reuse the same tee path");

    let on_disk = std::fs::read_to_string(first.unwrap()).unwrap();
    assert_eq!(on_disk, "some output\n", "the tee file must not be rewritten on later polls");
}

/// A unique path under the OS temp root for a single test, removed
/// recursively on drop. Mirrors `tool::shell_test`'s `TempDir` helper — no
/// `tempfile` dep in this crate's Cargo.toml.
struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "koma-bgbash-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        TempDir(dir)
    }
    fn path(&self) -> &std::path::Path { &self.0 }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
