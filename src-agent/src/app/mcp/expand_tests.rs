use super::expand::*;
use crate::model::app_config::McpServerEntry;

fn lookup(n: &str) -> Option<String> {
    match n {
        "A" => Some("alpha".into()),
        "E" => Some(String::new()),
        "B" => Some("beta".into()),
        _ => None,
    }
}

fn ex(s: &str) -> (String, Vec<String>) {
    let mut missing = Vec::new();
    let out = expand_str(s, &lookup, &mut missing);
    (out, missing)
}

#[test]
fn expands_set_and_defaults() {
    assert_eq!(ex("${A}").0, "alpha");
    assert_eq!(ex("${U:-d}").0, "d");
    assert_eq!(ex("${E:-d}").0, "d");
    assert_eq!(ex("${E}").0, "");
    assert_eq!(ex("${A:-d}").0, "alpha");
    assert_eq!(ex("${env:A}").0, "alpha");
    assert_eq!(ex("x${A}/${B}y").0, "xalpha/betay");
}

#[test]
fn unset_without_default_is_literal_and_reported_once() {
    let (out, missing) = ex("${MISSING}-${MISSING}");
    assert_eq!(out, "${MISSING}-${MISSING}");
    assert_eq!(missing, vec!["MISSING"]);
}

#[test]
fn malformed_forms_stay_literal() {
    assert_eq!(ex("${input:x}").0, "${input:x}");
    assert_eq!(ex("${1A}").0, "${1A}");
    assert_eq!(ex("${A").0, "${A");
    assert_eq!(ex("$A").0, "$A");
    assert_eq!(ex("${}").0, "${}");
    assert_eq!(ex("${${A}").0, "${alpha");
    assert_eq!(ex("é${A}é").0, "éalphaé");
}

#[test]
fn default_text_is_not_expanded() {
    assert_eq!(ex("${U:-$A}").0, "$A");
}

#[test]
fn entry_expansion_skips_names_and_leaves_input_untouched() {
    let server = McpServerEntry {
        name: "${A}".into(),
        command: "${A}".into(),
        args: vec!["${B}".into(), "plain".into()],
        env: vec![("${A}".into(), "${B}".into())],
        url: "https://h/${A}".into(),
        headers: vec![("${A}".into(), "Bearer ${B}".into())],
        cwd: Some("${A}/work".into()),
        ..Default::default()
    };
    let (e, missing) = expand_entry(&server, &lookup);
    assert!(missing.is_empty());
    assert_eq!(e.name, "${A}");
    assert_eq!(e.command, "alpha");
    assert_eq!(e.args, vec!["beta", "plain"]);
    assert_eq!(e.env, vec![("${A}".to_string(), "beta".to_string())]);
    assert_eq!(e.url, "https://h/alpha");
    assert_eq!(
        e.headers,
        vec![("${A}".to_string(), "Bearer beta".to_string())]
    );
    assert_eq!(e.cwd.as_deref(), Some("alpha/work"));
    assert_eq!(server.command, "${A}");
}
