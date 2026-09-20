#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::append_ext_context;
use std::collections::BTreeMap;

#[test]
fn inactive_extension_context_does_not_leak_between_sessions() {
    use crate::app::{
        mode::Mode,
        state::{AppState, SessionRuntime},
    };
    use crate::model::{
        app_config::InstalledExtension, conversation::Conversation, session::Session,
        settings::Settings,
    };
    let mut state = AppState::new(Mode::Chat);
    state.rest.config.installed_extensions = vec![InstalledExtension {
        id: "run.koma.context".into(),
        enabled: true,
        ..Default::default()
    }];
    state
        .rest
        .ext_context
        .insert("run.koma.context".into(), "extension-only-context".into());
    state.rest.sessions.push(SessionRuntime::new());
    state.rest.sessions[1].session = Some(Session::new(
        "selected".into(),
        "/unused".into(),
        "test".into(),
        Settings {
            active_extensions: vec!["run.koma.context".into()],
            ..Default::default()
        },
        Conversation::from_messages(vec![]),
    ));
    let mut inactive = String::from("system");
    super::append_session_ext_context(&mut inactive, &state.rest, 0);
    assert_eq!(inactive, "system");
    let mut active = String::from("system");
    super::append_session_ext_context(&mut active, &state.rest, 1);
    assert!(active.contains("extension-only-context"));
    state.rest.sessions[1]
        .session
        .as_mut()
        .unwrap()
        .settings
        .active_extensions
        .clear();
    let mut unloaded = String::from("system");
    super::append_session_ext_context(&mut unloaded, &state.rest, 1);
    assert_eq!(unloaded, "system");
}

/// Blobs are appended in deterministic BTreeMap key order (alpha before zebra),
/// each as `\n\n# Extension context: <id>\n<text>`, and a blank blob is skipped.
#[test]
fn append_is_ordered_and_skips_blank() {
    let mut ctx = BTreeMap::new();
    ctx.insert("zebra.ext".to_string(), "z-blob".to_string());
    ctx.insert("alpha.ext".to_string(), "a-blob".to_string());
    ctx.insert("blank.ext".to_string(), "   ".to_string());
    let mut dst = String::from("HEAD");
    append_ext_context(&mut dst, &ctx);
    assert_eq!(
        dst,
        "HEAD\n\n# Extension context: alpha.ext\na-blob\n\n# Extension context: zebra.ext\nz-blob"
    );
}

/// An empty map is a no-op — the volatile tail is byte-identical to before.
#[test]
fn empty_map_is_noop() {
    let ctx: BTreeMap<String, String> = BTreeMap::new();
    let mut dst = String::from("HEAD");
    append_ext_context(&mut dst, &ctx);
    assert_eq!(dst, "HEAD");
}
