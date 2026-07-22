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
    RE.get_or_init(|| crate::re_util::static_re(r"^\s*(Compiling|Checking|Downloading|Downloaded|Updating|Adding|Locking|Fresh|Building)\b"))
}

fn finished_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| crate::re_util::static_re(r"^\s*Finished\b"))
}

fn progress_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| crate::re_util::static_re(r"^\s*\[.*\]"))
}

fn running_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| crate::re_util::static_re(r"^\s*Running\b"))
}

fn doc_tests_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| crate::re_util::static_re(r"^\s*Doc-tests\b"))
}

fn error_start_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| crate::re_util::static_re(r"^error(\[\w+\])?:"))
}

fn warning_start_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| crate::re_util::static_re(r"^warning(\[\w+\])?:"))
}

fn passing_test_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| crate::re_util::static_re(r"^test .* \.\.\. ok$"))
}

/// Extract the `in <duration>` portion of a `Finished ...` line, e.g. `12.34s`.
fn extract_timing(finished_line: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| crate::re_util::static_re(r"in ([\d.]+s)"));
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
#[path = "cargo_test.rs"]
mod tests;
