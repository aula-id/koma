//! Static, regex-driven filter specs for well-known noisy commands (npm/pip
//! install, docker build, make, wget/curl). Matched against the full shell
//! command (as typed, may contain `&&`/pipes) by [`super::filter_output`].

use std::sync::OnceLock;
use regex::Regex;

pub struct FilterSpec {
    pub name: &'static str,
    pub match_command: Regex,
    /// Drop lines matching any of these.
    pub strip_lines: Vec<Regex>,
    /// If non-empty, keep ONLY lines matching one of these (applied after `strip_lines`).
    pub keep_lines: Vec<Regex>,
    /// Keep the first N lines.
    pub head: Option<usize>,
    /// Keep the last N lines. With `head` also set: first N + "..." + last M.
    pub tail: Option<usize>,
    /// Absolute cap applied after every other stage; keeps the tail side.
    pub max_lines: Option<usize>,
    /// Replacement text if the result ends up empty/whitespace-only.
    pub on_empty: Option<&'static str>,
}

fn is_marker(line: &str) -> bool {
    line.starts_with("... [") && line.ends_with("lines omitted]")
}

fn elision(n: usize) -> String {
    format!("... [{n} lines omitted]")
}

/// Drop lines for which `drop` returns true, collapsing each contiguous run of
/// dropped lines into a single elision marker.
fn drop_matching(lines: Vec<String>, drop: impl Fn(&str) -> bool) -> Vec<String> {
    let mut out = Vec::with_capacity(lines.len());
    let mut run = 0usize;
    for line in lines {
        if drop(&line) {
            run += 1;
        } else {
            if run > 0 {
                out.push(elision(run));
                run = 0;
            }
            out.push(line);
        }
    }
    if run > 0 {
        out.push(elision(run));
    }
    out
}

fn apply_head_tail(lines: Vec<String>, head: Option<usize>, tail: Option<usize>) -> Vec<String> {
    match (head, tail) {
        (None, None) => lines,
        (Some(h), None) => {
            if lines.len() <= h {
                lines
            } else {
                let mut out: Vec<String> = lines[..h].to_vec();
                out.push(elision(lines.len() - h));
                out
            }
        }
        (None, Some(t)) => {
            if lines.len() <= t {
                lines
            } else {
                let mut out = vec![elision(lines.len() - t)];
                out.extend_from_slice(&lines[lines.len() - t..]);
                out
            }
        }
        (Some(h), Some(t)) => {
            if lines.len() <= h + t {
                lines
            } else {
                let mut out: Vec<String> = lines[..h].to_vec();
                out.push(elision(lines.len() - h - t));
                out.extend_from_slice(&lines[lines.len() - t..]);
                out
            }
        }
    }
}

fn apply_max(lines: Vec<String>, max: usize) -> Vec<String> {
    if lines.len() <= max {
        lines
    } else {
        let mut out = Vec::with_capacity(max + 1);
        out.push(elision(lines.len() - max));
        out.extend_from_slice(&lines[lines.len() - max..]);
        out
    }
}

/// Run `spec`'s pipeline (strip -> keep -> head/tail -> max_lines -> on_empty)
/// over `raw`. Returns `None` when the result doesn't meaningfully differ from
/// the input (so callers can report `changed = false`).
///
/// SAFETY RULE: when `exit_code != Some(0)` (command failed / unknown), `strip_lines`
/// and `keep_lines` still apply (they only ever drop noise, never signal), but the
/// `head`/`tail`/`max_lines` size limits are relaxed to 4x their configured value —
/// error output needs more context to be useful than happy-path output does.
pub fn apply(spec: &FilterSpec, raw: &str, exit_code: Option<i32>) -> Option<String> {
    let relax: usize = if exit_code != Some(0) { 4 } else { 1 };

    let original: Vec<String> = raw.lines().map(|s| s.to_string()).collect();

    let stage = drop_matching(original.clone(), |l| spec.strip_lines.iter().any(|re| re.is_match(l)));

    let stage = if spec.keep_lines.is_empty() {
        stage
    } else {
        drop_matching(stage, |l| !is_marker(l) && !spec.keep_lines.iter().any(|re| re.is_match(l)))
    };

    let stage = apply_head_tail(stage, spec.head.map(|h| h * relax), spec.tail.map(|t| t * relax));

    let stage = match spec.max_lines {
        Some(max) => apply_max(stage, max * relax),
        None => stage,
    };

    let mut changed = stage != original;

    // "Empty" means no real content survived — elision markers alone don't count,
    // otherwise a fully-stripped output would render as a bare marker line instead
    // of the configured `on_empty` replacement.
    let has_content = stage.iter().any(|l| !is_marker(l));
    let result = if !has_content {
        match spec.on_empty {
            Some(rep) => {
                changed = true;
                rep.to_string()
            }
            None => stage.join("\n"),
        }
    } else {
        stage.join("\n")
    };

    if changed {
        Some(result)
    } else {
        None
    }
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("filter spec regex must be valid")
}

pub fn table() -> &'static [FilterSpec] {
    static TABLE: OnceLock<Vec<FilterSpec>> = OnceLock::new();
    TABLE.get_or_init(build_table).as_slice()
}

fn build_table() -> Vec<FilterSpec> {
    vec![
        FilterSpec {
            name: "npm-install",
            match_command: re(r"(^|\s|&&\s*)(npm|pnpm|yarn)\s+(install|ci|i|add)\b"),
            strip_lines: vec![
                re(r"^[\s\S]*[⠀-⣿]"),           // progress-bar / spinner frames (braille block)
                re(r"^npm (timing|http|sill|verb)"),
                re(r"^npm warn deprecated"),
            ],
            keep_lines: vec![],
            head: None,
            tail: None,
            max_lines: Some(40),
            on_empty: None,
        },
        FilterSpec {
            name: "pip-install",
            match_command: re(r"(^|\s|&&\s*)(pip3?|uv)\s+(install|sync)\b"),
            strip_lines: vec![
                re(r"^\s*(Collecting|Downloading|Using cached|Preparing metadata|Installing collected|Requirement already satisfied)"),
            ],
            keep_lines: vec![],
            head: None,
            tail: None,
            max_lines: Some(40),
            on_empty: Some("install: ok (no notable output)"),
        },
        FilterSpec {
            name: "docker",
            match_command: re(r"(^|\s|&&\s*)docker\s+(build|pull|push)\b"),
            strip_lines: vec![
                re(r"^\s*([0-9a-f]{12}:|\s*(Pulling|Waiting|Verifying|Download complete|Pull complete|Extracting|Layer already exists))"),
                re(r"^#\d+ (sha256:|extracting|DONE \d)"),
            ],
            keep_lines: vec![],
            head: None,
            tail: None,
            max_lines: Some(60),
            on_empty: None,
        },
        FilterSpec {
            name: "make",
            match_command: re(r"(^|\s|&&\s*)make(\s|$)"),
            strip_lines: vec![
                re(r"^make\[\d+\]: (Entering|Leaving) directory"),
            ],
            keep_lines: vec![],
            head: None,
            tail: None,
            max_lines: Some(80),
            on_empty: None,
        },
        FilterSpec {
            name: "wget-curl",
            match_command: re(r"(^|\s|&&\s*)(wget|curl)\b.*(-O|--output|--remote-name|-o )"),
            strip_lines: vec![
                re(r"^\s*[\d.]+[KMG%]?\s|^\s*#+\s*$|\r"),
            ],
            keep_lines: vec![],
            head: None,
            tail: None,
            max_lines: Some(30),
            on_empty: None,
        },
    ]
}

#[cfg(test)]
mod tests {
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
}
