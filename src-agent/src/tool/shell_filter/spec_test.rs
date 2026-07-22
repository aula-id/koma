#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

fn spec_strip_only() -> FilterSpec {
    FilterSpec {
        name: "test-strip",
        match_command: re(r"^teststrip$"),
        strip_lines: vec![re(r"^noise")],
        keep_lines: vec![],
        head: None,
        tail: None,
        max_lines: None,
        on_empty: None,
    }
}

#[test]
fn strip_lines_drops_matching_and_marks() {
    let spec = spec_strip_only();
    let raw = "noise 1\nnoise 2\nkeep me\nnoise 3\n";
    let out = apply(&spec, raw, Some(0)).unwrap();
    assert!(out.contains("[2 lines omitted]"));
    assert!(out.contains("keep me"));
    assert!(out.contains("[1 lines omitted]"));
    assert!(!out.contains("noise"));
}

#[test]
fn keep_lines_keeps_only_matches() {
    let spec = FilterSpec {
        name: "test-keep",
        match_command: re(r"^testkeep$"),
        strip_lines: vec![],
        keep_lines: vec![re(r"^KEEP")],
        head: None,
        tail: None,
        max_lines: None,
        on_empty: None,
    };
    let raw = "drop me\nKEEP this\nalso drop\nKEEP that\n";
    let out = apply(&spec, raw, Some(0)).unwrap();
    assert!(out.contains("KEEP this"));
    assert!(out.contains("KEEP that"));
    assert!(!out.contains("drop me"));
    assert!(!out.contains("also drop"));
}

#[test]
fn head_only_keeps_first_n() {
    let spec = FilterSpec {
        name: "test-head",
        match_command: re(r"^testhead$"),
        strip_lines: vec![],
        keep_lines: vec![],
        head: Some(2),
        tail: None,
        max_lines: None,
        on_empty: None,
    };
    let raw = "a\nb\nc\nd\n";
    let out = apply(&spec, raw, Some(0)).unwrap();
    assert!(out.starts_with("a\nb\n"));
    assert!(out.contains("[2 lines omitted]"));
    assert!(!out.contains("c"));
}

#[test]
fn tail_only_keeps_last_n() {
    let spec = FilterSpec {
        name: "test-tail",
        match_command: re(r"^testtail$"),
        strip_lines: vec![],
        keep_lines: vec![],
        head: None,
        tail: Some(2),
        max_lines: None,
        on_empty: None,
    };
    let raw = "a\nb\nc\nd\n";
    let out = apply(&spec, raw, Some(0)).unwrap();
    assert!(out.contains("[2 lines omitted]"));
    assert!(out.ends_with("c\nd"));
    assert!(!out.contains("\na\n"));
}

#[test]
fn head_and_tail_keeps_both_ends() {
    let spec = FilterSpec {
        name: "test-headtail",
        match_command: re(r"^testheadtail$"),
        strip_lines: vec![],
        keep_lines: vec![],
        head: Some(1),
        tail: Some(1),
        max_lines: None,
        on_empty: None,
    };
    let raw = "a\nb\nc\nd\ne\n";
    let out = apply(&spec, raw, Some(0)).unwrap();
    assert!(out.starts_with("a\n"));
    assert!(out.ends_with("e"));
    assert!(out.contains("[3 lines omitted]"));
}

#[test]
fn max_lines_caps_and_keeps_tail_side() {
    let spec = FilterSpec {
        name: "test-max",
        match_command: re(r"^testmax$"),
        strip_lines: vec![],
        keep_lines: vec![],
        head: None,
        tail: None,
        max_lines: Some(2),
        on_empty: None,
    };
    let raw = "a\nb\nc\nd\n";
    let out = apply(&spec, raw, Some(0)).unwrap();
    assert!(out.contains("[2 lines omitted]"));
    assert!(out.ends_with("c\nd"));
}

#[test]
fn on_empty_replaces_blank_result() {
    let spec = FilterSpec {
        name: "test-empty",
        match_command: re(r"^testempty$"),
        strip_lines: vec![re(r".*")],
        keep_lines: vec![],
        head: None,
        tail: None,
        max_lines: None,
        on_empty: Some("nothing to see here"),
    };
    let raw = "line one\nline two\n";
    let out = apply(&spec, raw, Some(0)).unwrap();
    assert_eq!(out, "nothing to see here");
}

#[test]
fn no_change_returns_none() {
    let spec = spec_strip_only();
    let raw = "keep 1\nkeep 2\n";
    assert!(apply(&spec, raw, Some(0)).is_none());
}

#[test]
fn non_zero_exit_relaxes_head_tail_max_4x() {
    let spec = FilterSpec {
        name: "test-relax",
        match_command: re(r"^testrelax$"),
        strip_lines: vec![],
        keep_lines: vec![],
        head: None,
        tail: None,
        max_lines: Some(5),
        on_empty: None,
    };
    let mut raw = String::new();
    for i in 0..50 {
        raw.push_str(&format!("line {i}\n"));
    }
    let ok = apply(&spec, &raw, Some(0)).unwrap();
    let err = apply(&spec, &raw, Some(1)).unwrap();
    assert_eq!(ok.lines().filter(|l| !is_marker(l)).count(), 5);
    assert_eq!(err.lines().filter(|l| !is_marker(l)).count(), 20);
}
