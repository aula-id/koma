use super::*;
use crate::app::state::SessionRuntime;
use crate::model::{
    conversation::Conversation, ext_workspace::tests::ExtensionFixture, session::Session,
    settings::Settings,
};

#[test]
fn headless_extension_batches_are_validated_before_mutating_selection() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let fixture = ExtensionFixture::new();
    let mut state = AppState::new(Mode::Chat);
    state.rest.config.installed_extensions = vec![fixture.ext.clone()];
    state.rest.fg_mut().session = Some(Session::new(
        "batch-test".into(),
        "/unused".into(),
        "test".into(),
        Settings::default(),
        Conversation::from_messages(vec![]),
    ));
    let load = vec![fixture.ext.id.clone(), "run.koma.not-installed".into()];
    let result = crate::app::runtime::commands::extensions::set_session_extensions(
        &mut state,
        0,
        rt.handle(),
        &load,
        &[],
    );
    assert!(result.unwrap_err().to_string().contains("not installed"));
    assert!(state
        .rest
        .fg()
        .session
        .as_ref()
        .unwrap()
        .settings
        .active_extensions
        .is_empty());
    assert!(!fixture.workspace.exists());
    state.rest.config.installed_extensions[0].activation =
        crate::model::app_config::ExtensionActivation::Global;
    let result = crate::app::runtime::commands::extensions::set_session_extensions(
        &mut state,
        0,
        rt.handle(),
        &[],
        &[fixture.ext.id.clone()],
    );
    assert!(result.unwrap_err().to_string().contains("global"));
}

#[test]
fn activation_picker_roundtrip_load_and_unload_are_session_local() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let fixture = ExtensionFixture::new();
    let root = std::env::temp_dir().join(format!("extension-picker-{}", uuid::Uuid::new_v4()));
    let mut state = AppState::new(Mode::Chat);
    state.rest.ext_manager = Some(crate::app::ext::ExtHostManager::new(rt.handle()));
    state.rest.config.installed_extensions = vec![fixture.ext.clone()];
    state.rest.sessions.push(SessionRuntime::new());
    for (idx, runtime) in state.rest.sessions.iter_mut().enumerate() {
        let path = root.join(idx.to_string());
        std::fs::create_dir_all(&path).unwrap();
        runtime.session = Some(Session::new(
            format!("scope-{idx}"),
            path.clone(),
            "scope-test".into(),
            Settings {
                workdir: vec![path.display().to_string()],
                ..Default::default()
            },
            Conversation::from_messages(vec![]),
        ));
    }
    let picker = build_extensions_state(&state.rest, ExtSubMode::UsePicker, None);
    assert!(!picker.rows[0].active);
    *state.mode_mut() = Mode::Extensions(Box::new(picker));
    let snapshot = crate::ipc::snapshot::projection::mode_snapshot(&state);
    let snapshot: crate::ipc::proto::ModeSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
    let crate::ipc::proto::ModeSnapshot::Extensions(snapshot) = snapshot else {
        panic!("expected picker");
    };
    let shadow = crate::app::runtime::client_shadow::shadow_extensions(*snapshot);
    assert_eq!(shadow.sub_mode, ExtSubMode::UsePicker);
    assert_eq!(
        shadow.rows[0].activation,
        crate::model::app_config::ExtensionActivation::OnDemand
    );
    assert!(!shadow.rows[0].active);
    // The same keys work through the daemon's mode and a reconstructed thin client.
    *state.mode_mut() = Mode::Extensions(Box::new(shadow));
    let action = crate::controller::input::handle_key(
        &mut state,
        ratatui::crossterm::event::KeyEvent::from(ratatui::crossterm::event::KeyCode::Enter),
    );
    assert!(matches!(
        action,
        crate::controller::input::Action::UseExtension
    ));
    handle_use_extension(&mut state, rt.handle(), true).unwrap();
    let a = state.rest.sessions[0].session.as_ref().unwrap();
    assert_eq!(a.settings.active_extensions, vec![fixture.ext.id.clone()]);
    assert_eq!(a.workdirs().len(), 2);
    assert_eq!(
        state.rest.mcp_manager.as_ref().unwrap().tool_names().len(),
        1
    );
    assert!(a.conversation.messages()[0]
        .content
        .contains(&fixture.agent));
    let saved: Settings =
        serde_json::from_slice(&std::fs::read(a.path.join("settings.json")).unwrap()).unwrap();
    assert_eq!(saved.active_extensions, a.settings.active_extensions);
    let b = state.rest.sessions[1].session.as_ref().unwrap();
    assert!(b.settings.active_extensions.is_empty());
    assert_eq!(b.workdirs().len(), 1);
    state
        .rest
        .fg_mut()
        .pending_ext_prompts
        .push((fixture.ext.id.clone(), "queued while active".into()));
    let picker = build_extensions_state(&state.rest, ExtSubMode::UsePicker, None);
    assert!(picker.rows[0].active);
    let old_cache = state.rest.fg().dir_cache.clone();
    old_cache
        .write()
        .unwrap()
        .files
        .push("[1]extension-only.txt".into());
    *state.mode_mut() = Mode::Extensions(Box::new(picker));
    handle_use_extension(&mut state, rt.handle(), false).unwrap();
    let a = state.rest.sessions[0].session.as_ref().unwrap();
    assert!(a.settings.active_extensions.is_empty());
    assert_eq!(a.workdirs().len(), 1);
    assert!(!a.conversation.messages()[0]
        .content
        .contains(&fixture.agent));
    assert!(state.rest.fg().pending_ext_prompts.is_empty());
    assert!(!std::sync::Arc::ptr_eq(
        &old_cache,
        &state.rest.fg().dir_cache
    ));
    // A global config reload elsewhere must not conceal a scope change.
    state.rest.config.installed_extensions[0].activation =
        crate::model::app_config::ExtensionActivation::Global;
    assert!(
        crate::app::runtime::commands::extensions::refresh_session_if_needed(
            &mut state,
            0,
            rt.handle()
        )
        .unwrap()
    );
    assert!(state
        .rest
        .fg()
        .session
        .as_ref()
        .unwrap()
        .conversation
        .messages()[0]
        .content
        .contains(&fixture.agent));
    assert!(
        !crate::app::runtime::commands::extensions::refresh_session_if_needed(
            &mut state,
            0,
            rt.handle()
        )
        .unwrap()
    );
    std::fs::remove_dir_all(root).unwrap();
}
