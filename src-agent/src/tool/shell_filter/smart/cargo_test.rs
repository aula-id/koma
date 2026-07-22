#![allow(clippy::unwrap_used, clippy::expect_used)]
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
    assert!(outcome
        .text
        .contains("cargo build: ok, 0 warnings (15.23s)"));
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
    assert!(outcome
        .text
        .contains("cargo:warning=custom build script output"));
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
