//! Smart filter for `cargo build` / `check` / `clippy` / `test` / `nextest run`:
//! strips well-known progress noise while keeping every diagnostic block
//! (errors/warnings) verbatim, then appends a compact summary line.

use super::super::FilterOutcome;
use regex::Regex;
use std::sync::OnceLock;

/// Try to filter a (already `cd`-stripped) command starting with `cargo `.
/// Returns `None` for any subcommand this module doesn't special-case.
pub(crate) fn try_filter(command: &str, raw: &str, _exit_code: Option<i32>) -> Option<FilterOutcome> {
    let rest = command.strip_prefix("cargo ")?.trim_start();
    let mut tokens = rest.split_whitespace();
    let sub = tokens.next().unwrap_or("");

    match sub {
        "build" | "check" | "clippy" => filter_build(raw, sub),
        "test" => filter_test(raw),
        "nextest" if tokens.next() == Some("run") => filter_test(raw),
        _ => None,
    }
}

fn noise_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^\s*(Compiling|Checking|Downloading|Downloaded|Updating|Adding|Locking|Fresh|Building)\b")
            .expect("cargo noise regex must be valid")
    })
}

fn finished_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*Finished\b").expect("cargo finished regex must be valid"))
}

fn progress_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*\[.*\]").expect("cargo progress-bar regex must be valid"))
}

fn running_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*Running\b").expect("cargo running regex must be valid"))
}

fn doc_tests_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*Doc-tests\b").expect("cargo doc-tests regex must be valid"))
}

fn error_start_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^error(\[\w+\])?:").expect("cargo error-start regex must be valid"))
}

fn warning_start_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^warning(\[\w+\])?:").expect("cargo warning-start regex must be valid"))
}

fn passing_test_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^test .* \.\.\. ok$").expect("cargo passing-test regex must be valid"))
}

/// Extract the `in <duration>` portion of a `Finished ...` line, e.g. `12.34s`.
fn extract_timing(finished_line: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"in ([\d.]+s)").expect("cargo timing regex must be valid"));
    re.captures(finished_line).map(|c| c[1].to_string())
}

/// `cargo build` / `check` / `clippy`: drop progress noise, keep every
/// diagnostic block (error/warning + its `-->`/`|`/`=`/continuation lines)
/// verbatim, append a one-line summary.
fn filter_build(raw: &str, sub: &str) -> Option<FilterOutcome> {
    let mut kept: Vec<&str> = Vec::new();
    let mut finished_line: Option<&str> = None;
    let mut error_count = 0usize;
    let mut warning_count = 0usize;

    for line in raw.lines() {
        if noise_re().is_match(line) {
            continue;
        }
        if finished_re().is_match(line) {
            finished_line = Some(line.trim());
            continue;
        }
        if progress_re().is_match(line) {
            continue;
        }

        if error_start_re().is_match(line)
            && !line.contains("aborting due to")
            && !line.contains("could not compile")
        {
            error_count += 1;
        } else if warning_start_re().is_match(line) && !(line.contains("generated") && line.contains("warning")) {
            warning_count += 1;
        }

        kept.push(line);
    }

    let name: &'static str = match sub {
        "build" => "cargo-build",
        "check" => "cargo-check",
        "clippy" => "cargo-clippy",
        _ => "cargo-build",
    };

    let summary = if error_count == 0 {
        match finished_line.and_then(extract_timing) {
            Some(t) => format!("cargo {sub}: ok, {warning_count} warnings ({t})"),
            None => format!("cargo {sub}: ok, {warning_count} warnings"),
        }
    } else {
        format!("cargo {sub}: {error_count} errors, {warning_count} warnings")
    };

    let mut text = kept.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(&summary);

    super::finalize(raw, text, name)
}

/// `cargo test` / `cargo nextest run`: drop passing-test lines and build
/// noise, keep failing tests / failures section / panics / tally lines
/// verbatim, note how many passing lines were hidden.
fn filter_test(raw: &str) -> Option<FilterOutcome> {
    let mut kept: Vec<&str> = Vec::new();
    let mut hidden_ok = 0usize;

    for line in raw.lines() {
        if noise_re().is_match(line)
            || finished_re().is_match(line)
            || progress_re().is_match(line)
            || running_re().is_match(line)
            || doc_tests_re().is_match(line)
        {
            continue;
        }
        if passing_test_re().is_match(line) {
            hidden_ok += 1;
            continue;
        }
        kept.push(line);
    }

    let mut text = kept.join("\n");
    if hidden_ok > 0 {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&format!("({hidden_ok} passing tests hidden)"));
    }

    super::finalize(raw, text, "cargo-test")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_success_collapses_to_summary() {
        let raw = "\
   Compiling libc v0.2.153
   Compiling cfg-if v1.0.0
   Compiling koma v0.2.17
    Finished dev [unoptimized + debuginfo] target(s) in 15.23s
";
        let outcome = try_filter("cargo build", raw, Some(0)).expect("should filter");
        assert_eq!(outcome.filter_name, Some("cargo-build"));
        assert!(outcome.changed);
        assert!(!outcome.text.contains("Compiling"));
        assert!(outcome.text.contains("cargo build: ok, 0 warnings (15.23s)"));
    }

    #[test]
    fn build_failure_keeps_error_block_verbatim_and_counts() {
        let raw = "\
   Compiling libc v0.2.153
   Compiling cfg-if v1.0.0
   Compiling serde v1.0.188
   Compiling serde_derive v1.0.188
   Compiling anyhow v1.0.75
   Compiling koma v0.2.17
error[E0308]: mismatched types
 --> src/main.rs:10:5
  |
10 |     let x: i32 = \"hello\";
  |                  ^^^^^^^ expected `i32`, found `&str`

error: aborting due to 1 previous error
";
        let outcome = try_filter("cargo build", raw, Some(101)).expect("should filter");
        assert!(outcome.text.contains("error[E0308]: mismatched types"));
        assert!(outcome.text.contains("--> src/main.rs:10:5"));
        assert!(outcome.text.contains("expected `i32`, found `&str`"));
        assert!(outcome.text.contains("cargo build: 1 errors, 0 warnings"));
        assert!(!outcome.text.contains("Compiling"));
    }

    #[test]
    fn build_failure_survives_non_noise_line() {
        let raw = "\
   Compiling libc v0.2.153
   Compiling cfg-if v1.0.0
   Compiling serde v1.0.188
   Compiling serde_derive v1.0.188
   Compiling anyhow v1.0.75
   Compiling koma v0.2.17
cargo:warning=custom build script output
error: linking with `cc` failed: exit status: 1
  = note: some linker note

error: aborting due to previous error
";
        let outcome = try_filter("cargo build", raw, Some(101)).expect("should filter");
        assert!(outcome.text.contains("cargo:warning=custom build script output"));
        assert!(!outcome.text.contains("Compiling"));
        assert!(outcome.text.contains("cargo build: 1 errors, 0 warnings"));
    }

    #[test]
    fn test_ok_lines_dropped_failures_and_tally_kept() {
        let raw = "\
   Compiling koma v0.2.17
    Finished test [unoptimized + debuginfo] target(s) in 2.00s
     Running unittests src/lib.rs

running 4 tests
test foo::a ... ok
test foo::b ... ok
test foo::c ... FAILED
test foo::d ... ok

failures:

---- foo::c stdout ----
thread 'foo::c' panicked at 'assertion failed'

failures:
    foo::c

test result: FAILED. 3 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
";
        let outcome = try_filter("cargo test", raw, Some(101)).expect("should filter");
        assert!(!outcome.text.contains("test foo::a ... ok"));
        assert!(!outcome.text.contains("test foo::b ... ok"));
        assert!(!outcome.text.contains("test foo::d ... ok"));
        assert!(outcome.text.contains("test foo::c ... FAILED"));
        assert!(outcome.text.contains("failures:"));
        assert!(outcome.text.contains("panicked at 'assertion failed'"));
        assert!(outcome.text.contains(
            "test result: FAILED. 3 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s"
        ));
    }
}
