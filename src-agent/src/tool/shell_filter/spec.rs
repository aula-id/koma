//! Static, regex-driven filter specs for well-known noisy commands (npm/pip
//! install, docker build, make, wget/curl). Matched against the full shell
//! command (as typed, may contain `&&`/pipes) by [`super::filter_output`].

use regex::Regex;
use std::sync::OnceLock;

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

    let stage = drop_matching(original.clone(), |l| {
        spec.strip_lines.iter().any(|re| re.is_match(l))
    });

    let stage = if spec.keep_lines.is_empty() {
        stage
    } else {
        drop_matching(stage, |l| {
            !is_marker(l) && !spec.keep_lines.iter().any(|re| re.is_match(l))
        })
    };

    let stage = apply_head_tail(
        stage,
        spec.head.map(|h| h * relax),
        spec.tail.map(|t| t * relax),
    );

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

fn re(pattern: &'static str) -> Regex {
    crate::re_util::static_re(pattern)
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
                re(r"^[\s\S]*[⠀-⣿]"), // progress-bar / spinner frames (braille block)
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
            strip_lines: vec![re(
                r"^\s*(Collecting|Downloading|Using cached|Preparing metadata|Installing collected|Requirement already satisfied)",
            )],
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
                re(
                    r"^\s*([0-9a-f]{12}:|\s*(Pulling|Waiting|Verifying|Download complete|Pull complete|Extracting|Layer already exists))",
                ),
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
            strip_lines: vec![re(r"^make\[\d+\]: (Entering|Leaving) directory")],
            keep_lines: vec![],
            head: None,
            tail: None,
            max_lines: Some(80),
            on_empty: None,
        },
        FilterSpec {
            name: "wget-curl",
            match_command: re(r"(^|\s|&&\s*)(wget|curl)\b.*(-O|--output|--remote-name|-o )"),
            strip_lines: vec![re(r"^\s*[\d.]+[KMG%]?\s|^\s*#+\s*$|\r")],
            keep_lines: vec![],
            head: None,
            tail: None,
            max_lines: Some(30),
            on_empty: None,
        },
    ]
}

#[cfg(test)]
#[path = "spec_test.rs"]
mod tests;
