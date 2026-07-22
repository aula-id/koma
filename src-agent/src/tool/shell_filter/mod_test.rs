#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[test]
fn pipe_command_passes_through_untouched() {
    let raw = "npm warn deprecated foo\nadded 3 packages in 2s\n";
    let outcome = filter_output("npm install | tee log.txt", raw, Some(0));
    assert!(!outcome.changed);
    assert_eq!(outcome.filter_name, None);
    assert_eq!(outcome.text, raw);
}

#[test]
fn unmatched_command_passes_through() {
    let raw = "hello\nworld\n";
    let outcome = filter_output("echo hello world", raw, Some(0));
    assert!(!outcome.changed);
    assert_eq!(outcome.filter_name, None);
    assert_eq!(outcome.text, raw);
}

#[test]
fn npm_install_realistic_fixture_strips_noise_keeps_summary() {
    let raw = "\
npm timing npm:load:whichnode Completed in 1ms
npm http fetch GET 200 https://registry.npmjs.org/foo 120ms
npm warn deprecated foo@1.0.0: use bar instead
added 42 packages in 3.2s
2 packages are looking for funding
";
    let outcome = filter_output("cd project && npm install", raw, Some(0));
    assert!(outcome.changed);
    assert_eq!(outcome.filter_name, Some("npm-install"));
    assert!(!outcome.text.contains("npm timing"));
    assert!(!outcome.text.contains("npm http"));
    assert!(!outcome.text.contains("npm warn deprecated"));
    assert!(outcome.text.contains("added 42 packages in 3.2s"));
    assert!(outcome.text.contains("2 packages are looking for funding"));
}

#[test]
fn pip_install_realistic_fixture_strips_noise_keeps_summary() {
    let raw = "\
Collecting requests
Downloading requests-2.31.0-py3-none-any.whl (62 kB)
Using cached idna-3.4-py3-none-any.whl
Installing collected packages: idna, requests
Successfully installed idna-3.4 requests-2.31.0
";
    let outcome = filter_output("pip install requests", raw, Some(0));
    assert!(outcome.changed);
    assert_eq!(outcome.filter_name, Some("pip-install"));
    assert!(!outcome.text.contains("Collecting"));
    assert!(!outcome.text.contains("Downloading"));
    assert!(!outcome.text.contains("Using cached"));
    assert!(!outcome.text.contains("Installing collected"));
    assert!(outcome
        .text
        .contains("Successfully installed idna-3.4 requests-2.31.0"));
}

#[test]
fn non_zero_exit_relaxes_caps_4x() {
    // max_lines for npm-install is 40; with 200 lines of plain (non-strippable)
    // content and a non-zero exit, the cap relaxes to 160, so more survives
    // than the zero-exit case would allow.
    let mut raw = String::new();
    for i in 0..200 {
        raw.push_str(&format!("line {i}\n"));
    }
    let ok = filter_output("npm install", &raw, Some(0));
    let err = filter_output("npm install", &raw, Some(1));
    let ok_lines = ok.text.lines().count();
    let err_lines = err.text.lines().count();
    assert!(err_lines > ok_lines);
}
