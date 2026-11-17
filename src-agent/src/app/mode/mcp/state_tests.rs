#![allow(clippy::unwrap_used)]

use crate::model::app_config::McpServerEntry;

use super::McpState;

fn stored() -> McpServerEntry {
    McpServerEntry {
        uuid: "u1".into(),
        name: "gh".into(),
        enabled: true,
        command: "npx".into(),
        args: vec!["a b".into(), "-y".into()],
        env: vec![("K".into(), "x,y".into())],
        headers: vec![("X".into(), "y".into())],
        cwd: Some("/w".into()),
        startup_timeout_ms: Some(3000),
        tool_timeout_ms: Some(4000),
        allowed_tools: vec!["r*".into()],
        disabled_tools: vec!["w".into()],
        ..Default::default()
    }
}

fn editing(s: &McpServerEntry) -> McpState {
    let mut st = McpState::from(std::slice::from_ref(s));
    st.enter_edit();
    st
}

#[test]
fn renaming_keeps_hidden_fields_and_spaced_args() {
    let s = stored();
    let mut st = editing(&s);
    st.draft_name = "github".into();
    let e = st.to_entry();
    assert_eq!(e.name, "github");
    assert_eq!(e.args, s.args);
    assert_eq!(e.env, s.env);
    assert_eq!(e.headers, s.headers);
    assert_eq!(e.cwd, s.cwd);
    assert_eq!(e.startup_timeout_ms, s.startup_timeout_ms);
    assert_eq!(e.tool_timeout_ms, s.tool_timeout_ms);
    assert_eq!(e.allowed_tools, s.allowed_tools);
    assert_eq!(e.disabled_tools, s.disabled_tools);
}

#[test]
fn changed_args_text_is_reparsed() {
    let s = stored();
    let mut st = editing(&s);
    st.draft_args = "-y srv".into();
    assert_eq!(st.to_entry().args, vec!["-y", "srv"]);
}

use super::{entry_from_form, mask_headers, state::HEADER_KEEP};
use crate::ipc::proto::McpServerExtras;

#[test]
fn form_without_extras_keeps_stored_fields() {
    let s = stored();
    let e = entry_from_form(
        Some(&s),
        Some("u1".into()),
        " gh ",
        true,
        "stdio",
        "npx",
        "-y srv",
        "K=v",
        "",
        &McpServerExtras::default(),
    );
    assert_eq!(e.name, "gh");
    assert_eq!(e.args, vec!["-y", "srv"]);
    assert_eq!(e.env, vec![("K".to_string(), "v".to_string())]);
    assert_eq!(e.headers, s.headers);
    assert_eq!(e.cwd, s.cwd);
    assert_eq!(e.startup_timeout_ms, s.startup_timeout_ms);
    assert_eq!(e.allowed_tools, s.allowed_tools);
}

#[test]
fn form_extras_override_and_clear() {
    let s = stored();
    let x = McpServerExtras {
        argv: Some(vec!["a b".into()]),
        env_pairs: Some(vec![("E".into(), "1,2".into())]),
        cwd: Some("".into()),
        startup_timeout_ms: Some(0),
        tool_timeout_ms: Some(9000),
        allowed_tools: Some(vec![" x ".into(), "".into()]),
        disabled_tools: Some(vec![]),
        ..Default::default()
    };
    let e = entry_from_form(
        Some(&s),
        Some("u1".into()),
        "gh",
        true,
        "http",
        "",
        "ignored",
        "ignored=1",
        "https://u",
        &x,
    );
    assert_eq!(e.args, vec!["a b"]);
    assert_eq!(e.env, vec![("E".to_string(), "1,2".to_string())]);
    assert_eq!(e.cwd, None);
    assert_eq!(e.startup_timeout_ms, None);
    assert_eq!(e.tool_timeout_ms, Some(9000));
    assert_eq!(e.allowed_tools, vec!["x"]);
    assert!(e.disabled_tools.is_empty());
    assert_eq!(e.url, "https://u");
}

#[test]
fn masked_or_blank_header_values_keep_the_stored_value() {
    let mut s = stored();
    s.headers = vec![("Authorization".into(), "Bearer real".into())];
    assert_eq!(
        mask_headers(&s.headers),
        vec![("Authorization".to_string(), HEADER_KEEP.to_string())]
    );
    let send = |h: Vec<(&str, &str)>| {
        let x = McpServerExtras {
            headers: Some(h.into_iter().map(|(a, b)| (a.into(), b.into())).collect()),
            ..Default::default()
        };
        entry_from_form(
            Some(&s),
            Some("u1".into()),
            "gh",
            true,
            "http",
            "",
            "",
            "",
            "https://u",
            &x,
        )
        .headers
    };
    let pair = |a: &str, b: &str| (a.to_string(), b.to_string());
    assert_eq!(
        send(vec![("Authorization", HEADER_KEEP)]),
        vec![pair("Authorization", "Bearer real")]
    );
    assert_eq!(
        send(vec![("authorization", "")]),
        vec![pair("authorization", "Bearer real")]
    );
    assert_eq!(
        send(vec![("Authorization", "Bearer new"), ("X-A", "1")]),
        vec![pair("Authorization", "Bearer new"), pair("X-A", "1")]
    );
    assert_eq!(send(vec![]), Vec::<(String, String)>::new());
}
