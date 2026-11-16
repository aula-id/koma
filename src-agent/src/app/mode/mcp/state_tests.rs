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
