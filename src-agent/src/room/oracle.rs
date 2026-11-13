//! The proof check: a program, not a model, reads the files a claim cites.
//!
//! A claim may carry `proof`: a list of `{path, needle}` pairs, each saying "this exact text is in
//! this file". The check opens the file and looks for the text. A claim whose cited code is not
//! there is fabricated, and that is the one thing a program can establish for certain. The result
//! is one of three, and the third is never a rejection:
//!
//! * `grounded`: every needle was found. The code exists; whether it is a bug is still the Reviewer's call.
//! * `refuted`: a cited file or needle is not there. The claim goes back to the Red that made it.
//! * `unproven`: nothing could be checked (no proof given, no workspace, a path outside it). Held for review, never dropped.
//!
//! The check runs in the background, off every lane, and its result sits on the shelf under
//! `notes/oracle/` where the Reviewer and the user can read it.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::room::shelf::Shelf;

/// A needle shorter than this proves nothing: it would match almost any file.
pub const MIN_NEEDLE: usize = 8;
/// Larger files are not read whole; a claim about one is unproven.
const MAX_FILE: u64 = 4 * 1024 * 1024;
/// More proofs than this are ignored, so one claim cannot hold the check for long.
const MAX_PROOFS: usize = 16;

#[derive(Debug, Clone, PartialEq)]
pub struct Proof {
    pub path: String,
    pub needle: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Grounded,
    Refuted,
    Unproven,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Grounded => "grounded",
            Verdict::Refuted => "refuted",
            Verdict::Unproven => "unproven",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub verdict: Verdict,
    /// One line per proof, saying what was found or why not.
    pub lines: Vec<String>,
}

/// The proof entries of a claim record. Entries that are not `{path, needle}` text are skipped.
pub fn proofs(claim: &Value) -> Vec<Proof> {
    claim
        .get("proof")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let path = item.get("path")?.as_str()?.trim().to_string();
                    let needle = item.get("needle")?.as_str()?.to_string();
                    (!path.is_empty() && !needle.trim().is_empty()).then_some(Proof { path, needle })
                })
                .take(MAX_PROOFS)
                .collect()
        })
        .unwrap_or_default()
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The 1-based line a needle starts on, matching exactly first and then ignoring runs of whitespace.
fn find(haystack: &str, needle: &str) -> Option<usize> {
    if let Some(at) = haystack.find(needle) {
        return Some(haystack[..at].matches('\n').count() + 1);
    }
    let wanted = collapse(needle);
    if wanted.is_empty() {
        return None;
    }
    // Compare line windows so a needle that spans lines still lands on a line number.
    let lines: Vec<&str> = haystack.lines().collect();
    let span = needle.lines().count().max(1);
    (0..lines.len()).find(|&i| collapse(&lines[i..(i + span).min(lines.len())].join(" ")).contains(&wanted)).map(|i| i + 1)
}

enum Located {
    Found(PathBuf),
    /// The path names something outside every root, so it cannot be checked here.
    Outside,
    Missing,
}

fn locate(path: &str, roots: &[PathBuf]) -> Located {
    let canon: Vec<PathBuf> = roots.iter().filter_map(|r| r.canonicalize().ok()).collect();
    let inside = |p: &Path| p.canonicalize().ok().filter(|c| canon.iter().any(|r| c.starts_with(r)));
    let given = Path::new(path);
    if given.is_absolute() {
        return match given.canonicalize() {
            Ok(c) if canon.iter().any(|r| c.starts_with(r)) => Located::Found(c),
            Ok(_) => Located::Outside,
            Err(_) if canon.iter().any(|r| given.starts_with(r)) => Located::Missing,
            Err(_) => Located::Outside,
        };
    }
    for root in &canon {
        let joined = root.join(given);
        if joined.exists() {
            // A path that climbs out of the root by `..` or a link is outside, not found.
            return match inside(&joined) {
                Some(c) => Located::Found(c),
                None => Located::Outside,
            };
        }
    }
    Located::Missing
}

/// Check every proof of `claim` against files under `roots`.
pub fn check(claim: &Value, roots: &[PathBuf]) -> Report {
    let list = proofs(claim);
    if list.is_empty() {
        return Report { verdict: Verdict::Unproven, lines: vec!["no proof was given, so nothing could be checked against the code".into()] };
    }
    if roots.is_empty() {
        return Report { verdict: Verdict::Unproven, lines: vec!["no workspace to check against".into()] };
    }
    let (mut lines, mut refuted) = (Vec::new(), false);
    for proof in &list {
        let shown = format!("{} · \"{}\"", proof.path, clip(&proof.needle));
        if proof.needle.trim().chars().count() < MIN_NEEDLE {
            lines.push(format!("unproven: {shown} is too short to prove anything"));
            continue;
        }
        match locate(&proof.path, roots) {
            Located::Outside => lines.push(format!("unproven: {} is outside the workspace", proof.path)),
            Located::Missing => {
                refuted = true;
                lines.push(format!("refuted: {} does not exist", proof.path));
            }
            Located::Found(file) => match std::fs::metadata(&file) {
                Ok(meta) if meta.is_file() && meta.len() <= MAX_FILE => match std::fs::read(&file) {
                    Ok(bytes) => match find(&String::from_utf8_lossy(&bytes), &proof.needle) {
                        Some(line) => {
                            lines.push(format!("found: {shown} at line {line}"));
                        }
                        None => {
                            refuted = true;
                            lines.push(format!("refuted: {shown} is not in the file"));
                        }
                    },
                    Err(e) => lines.push(format!("unproven: {} could not be read ({e})", proof.path)),
                },
                _ => lines.push(format!("unproven: {} is not a file of a size that can be checked", proof.path)),
            },
        }
    }
    let verdict = if refuted {
        Verdict::Refuted
    } else if lines.iter().all(|l| l.starts_with("found")) {
        Verdict::Grounded
    } else {
        Verdict::Unproven
    };
    Report { verdict, lines }
}

fn clip(text: &str) -> String {
    let one = collapse(text);
    if one.chars().count() > 60 { format!("{}…", one.chars().take(60).collect::<String>()) } else { one }
}

/// Where a claim's check result is kept.
pub fn key_of(claim_key: &str) -> String {
    format!("notes/oracle/{}", claim_key.trim_start_matches("claims/").trim_start_matches("claim/"))
}

/// What the shelf holds for a claim's check.
#[derive(Debug, Clone, PartialEq)]
pub struct Stored {
    pub revision: u64,
    /// The check was started and has not answered yet.
    pub pending: bool,
    pub verdict: Option<Verdict>,
    pub lines: Vec<String>,
}

/// The stored result for this exact claim revision, if any.
pub fn stored(shelf: &Shelf, claim_key: &str, revision: u64) -> Option<Stored> {
    let row = shelf.peek(&key_of(claim_key)).ok().flatten()?;
    let body: Value = serde_json::from_str(row.content.as_deref()?).ok()?;
    if body.get("claim_revision").and_then(Value::as_u64) != Some(revision) {
        return None;
    }
    let verdict = match body.get("verdict").and_then(Value::as_str) {
        Some("grounded") => Some(Verdict::Grounded),
        Some("refuted") => Some(Verdict::Refuted),
        Some("unproven") => Some(Verdict::Unproven),
        _ => None,
    };
    Some(Stored {
        revision,
        pending: body.get("pending").and_then(Value::as_bool).unwrap_or(false),
        verdict,
        lines: body.get("lines").and_then(Value::as_array).map(|l| l.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default(),
    })
}

fn put(shelf: &Shelf, claim_key: &str, body: Value) {
    let key = key_of(claim_key);
    let expected = shelf.peek(&key).ok().flatten().map_or(0, |row| row.revision.max(0));
    let _ = shelf.call("put", &json!({ "key": key, "content": body.to_string(), "expected_revision": expected }));
}

/// Mark a check as started, so the review waits for it instead of racing past.
pub fn mark_pending(shelf: &Shelf, claim_key: &str, revision: u64) {
    put(shelf, claim_key, json!({ "kind": "oracle", "claim": claim_key, "claim_revision": revision, "pending": true, "verdict": null, "lines": [] }));
}

/// Keep a finished check.
pub fn save(shelf: &Shelf, claim_key: &str, revision: u64, report: &Report) {
    put(
        shelf,
        claim_key,
        json!({ "kind": "oracle", "claim": claim_key, "claim_revision": revision, "pending": false, "verdict": report.verdict.as_str(), "lines": report.lines }),
    );
}

/// The note sent back to Red when its claim is refuted.
pub fn feedback(report: &[String]) -> String {
    let bad: Vec<&String> = report.iter().filter(|l| l.starts_with("refuted")).collect();
    format!(
        "A program checked the code your claim cites and it is not there: {}. Quote the real text from the file, or fix the path, then publish the claim again.",
        bad.iter().map(|l| l.trim_start_matches("refuted: ")).collect::<Vec<_>>().join("; ")
    )
}

#[cfg(test)]
#[path = "oracle_tests.rs"]
mod tests;
