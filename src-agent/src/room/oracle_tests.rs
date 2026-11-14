use std::path::PathBuf;

use serde_json::json;

use super::*;

fn tree() -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("koma-oracle-{}-{}", std::process::id(), std::thread::current().name().unwrap_or("t").replace("::", "-")));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/pressure.ts"), "export function pressureTick(state) {\n  if (rss > limits.high) {\n    return reduce(state)\n  }\n}\n").unwrap();
    let outside = dir.parent().unwrap().join(format!("{}-outside.txt", dir.file_name().unwrap().to_string_lossy()));
    std::fs::write(&outside, "secret outside the workspace").unwrap();
    (dir, outside)
}

fn claim(proof: serde_json::Value) -> serde_json::Value {
    json!({ "kind": "claim", "proof": proof })
}

#[test]
fn a_needle_that_is_in_the_file_is_grounded_and_names_the_line() {
    let (dir, _) = tree();
    let r = check(&claim(json!([{ "path": "src/pressure.ts", "needle": "if (rss > limits.high)" }])), &[dir]);
    assert_eq!(r.verdict, Verdict::Grounded);
    assert!(r.lines[0].contains("at line 2"), "{:?}", r.lines);
}

#[test]
fn whitespace_does_not_matter_and_a_needle_may_span_lines() {
    let (dir, _) = tree();
    let r = check(&claim(json!([{ "path": "src/pressure.ts", "needle": "if (rss  >\n limits.high) {\n return reduce(state)" }])), &[dir]);
    assert_eq!(r.verdict, Verdict::Grounded, "{:?}", r.lines);
}

#[test]
fn a_missing_needle_or_file_refutes() {
    let (dir, _) = tree();
    let r = check(&claim(json!([{ "path": "src/pressure.ts", "needle": "Bun.gc(true) never called" }])), std::slice::from_ref(&dir));
    assert_eq!(r.verdict, Verdict::Refuted);
    assert!(feedback(&r.lines).contains("is not in the file"));
    let r = check(&claim(json!([{ "path": "src/nothing.ts", "needle": "does not matter at all" }])), &[dir]);
    assert_eq!(r.verdict, Verdict::Refuted);
    assert!(r.lines[0].contains("does not exist"));
}

#[test]
fn one_refuted_proof_refutes_the_claim_even_when_another_is_found() {
    let (dir, _) = tree();
    let r = check(&claim(json!([{ "path": "src/pressure.ts", "needle": "return reduce(state)" }, { "path": "src/pressure.ts", "needle": "an invented line of code" }])), &[dir]);
    assert_eq!(r.verdict, Verdict::Refuted);
}

#[test]
fn nothing_to_check_is_unproven_never_refuted() {
    let (dir, outside) = tree();
    assert_eq!(check(&claim(json!([])), std::slice::from_ref(&dir)).verdict, Verdict::Unproven);
    assert_eq!(check(&json!({ "kind": "claim" }), std::slice::from_ref(&dir)).verdict, Verdict::Unproven);
    assert_eq!(check(&claim(json!([{ "path": "src/pressure.ts", "needle": "short" }])), std::slice::from_ref(&dir)).verdict, Verdict::Unproven);
    assert_eq!(check(&claim(json!([{ "path": "src/pressure.ts", "needle": "return reduce(state)" }])), &[]).verdict, Verdict::Unproven);
    // A file outside the workspace, by absolute path or by climbing out, cannot be judged here.
    let abs = check(&claim(json!([{ "path": outside.to_string_lossy(), "needle": "secret outside the workspace" }])), std::slice::from_ref(&dir));
    assert_eq!(abs.verdict, Verdict::Unproven);
    let name = outside.file_name().unwrap().to_string_lossy().to_string();
    let up = check(&claim(json!([{ "path": format!("../{name}"), "needle": "secret outside the workspace" }])), &[dir]);
    assert_eq!(up.verdict, Verdict::Unproven, "{:?}", up.lines);
}

#[test]
fn a_result_is_kept_on_the_shelf_for_its_claim_revision_only() {
    let shelf = Shelf::open_in_memory().unwrap();
    assert!(stored(&shelf, "claims/web/ssrf", 2).is_none());
    mark_pending(&shelf, "claims/web/ssrf", 2);
    let waiting = stored(&shelf, "claims/web/ssrf", 2).unwrap();
    assert!(waiting.pending && waiting.verdict.is_none());
    save(&shelf, "claims/web/ssrf", 2, &Report { verdict: Verdict::Refuted, lines: vec!["refuted: x".into()] });
    let done = stored(&shelf, "claims/web/ssrf", 2).unwrap();
    assert!(!done.pending);
    assert_eq!(done.verdict, Some(Verdict::Refuted));
    assert!(stored(&shelf, "claims/web/ssrf", 3).is_none(), "a newer claim revision has not been checked");
    assert_eq!(key_of("claims/web/ssrf"), "notes/oracle/web/ssrf");
}
