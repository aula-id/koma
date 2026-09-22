//! Headless session inspection and activation use the live daemon state.
use super::core::DaemonHub;
use crate::app::{mode::ExtSubMode, state::AppState};
use crate::ipc::proto::{DaemonEvent, RunExtension, RunState};

pub(crate) fn run_state(state: &AppState) -> anyhow::Result<RunState> {
    let rt = state.rest.fg();
    let sess = rt
        .session
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No active session."))?;
    let settings = &sess.settings;
    let resolved = crate::app::resolve::resolve_role(
        &state.rest.config,
        settings,
        crate::model::app_config::ModelRole::Main,
    );
    let rows = crate::app::runtime::commands::extensions::build_extensions_state(
        &state.rest,
        ExtSubMode::Browse,
        None,
    )
    .rows;
    Ok(RunState {
        session_id: sess.id.clone(),
        name: sess.name.clone(),
        workdir: sess
            .workdirs()
            .iter()
            .map(|p| p.display().to_string())
            .collect(),
        model: resolved.map(|r| r.model_id).unwrap_or_default(),
        effort: settings.effort.clone(),
        mode: rt.agent_mode.label().into(),
        working: rt.is_working(),
        awaiting_approval: rt.awaiting_approval,
        security_enabled: state.rest.security_enabled,
        security_running: state
            .rest
            .sec_manager
            .as_ref()
            .is_some_and(|m| m.is_running()),
        yolo_armed: state.rest.yolo_armed,
        short_send: settings.short_send_enabled,
        drss_active: settings.short_send_enabled && rt.context_usage.is_some_and(|c| c.drss_active),
        max_output_tokens: settings.max_output_tokens,
        context_window_limit: settings.context_window_limit,
        context_model_alias: settings.context_model_alias.clone(),
        system_extra: !settings.session_system_extra.trim().is_empty(),
        extensions: rows
            .into_iter()
            .map(|r| RunExtension {
                id: r.id,
                kind: r.kind,
                name: r.name,
                activation: r.activation.label().into(),
                enabled: r.enabled,
                active: r.active,
                running: r.running,
            })
            .collect(),
    })
}

impl DaemonHub {
    pub(super) fn get_run_state(&mut self, idx: usize, state: &AppState, req_seq: u64) {
        match run_state(state) {
            Ok(state) => self.send_to(idx, DaemonEvent::RunState { req_seq, state }),
            Err(error) => self.send_to(idx, DaemonEvent::Error(error.to_string())),
        }
    }

    pub(super) fn set_session_extensions(
        &mut self,
        idx: usize,
        state: &mut AppState,
        handle: &tokio::runtime::Handle,
        load: Vec<String>,
        unload: Vec<String>,
    ) {
        state.rest.config.installed_extensions =
            crate::model::app_config::AppConfig::load().installed_extensions;
        let session_idx = state.rest.foreground;
        let result = crate::app::runtime::commands::extensions::set_session_extensions(
            state,
            session_idx,
            handle,
            &load,
            &unload,
        );
        self.ack_or_error(idx, result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{mode::Mode, state::SessionRuntime};
    use crate::model::{
        app_config::{ExtensionActivation, InstalledExtension},
        conversation::Conversation,
        session::Session,
        settings::Settings,
    };

    #[test]
    fn state_reports_current_session_selection_and_global_extensions_without_credentials() {
        let mut state = AppState::new(Mode::Chat);
        state.rest.config.installed_extensions = vec![
            InstalledExtension {
                id: "run.koma.global".into(),
                enabled: true,
                activation: ExtensionActivation::Global,
                ..Default::default()
            },
            InstalledExtension {
                id: "run.koma.selected".into(),
                enabled: true,
                ..Default::default()
            },
            InstalledExtension {
                id: "run.koma.disabled".into(),
                enabled: false,
                ..Default::default()
            },
        ];
        state.rest.sessions.push(SessionRuntime::new());
        for (idx, rt) in state.rest.sessions.iter_mut().enumerate() {
            rt.id = format!("session-{idx}");
            rt.session = Some(Session::new(
                rt.id.clone(),
                "/unused".into(),
                "test".into(),
                Settings {
                    api_key: "must-not-be-reported".into(),
                    active_extensions: if idx == 0 {
                        vec!["run.koma.selected".into(), "run.koma.disabled".into()]
                    } else {
                        vec![]
                    },
                    short_send_enabled: idx == 0,
                    ..Default::default()
                },
                Conversation::from_messages(vec![]),
            ));
        }
        let a = run_state(&state).unwrap();
        assert_eq!(a.extensions.iter().filter(|e| e.active).count(), 2);
        assert!(a.short_send);
        state.rest.foreground = 1;
        let b = run_state(&state).unwrap();
        assert_eq!(b.session_id, "session-1");
        assert_eq!(
            b.extensions
                .iter()
                .filter(|e| e.active)
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            vec!["run.koma.global"]
        );
        assert!(!b.short_send);
        let event = DaemonEvent::RunState {
            req_seq: 42,
            state: b,
        };
        let bytes = serde_json::to_vec(&event).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("must-not-be-reported"));
        assert_eq!(
            serde_json::from_slice::<DaemonEvent>(&bytes).unwrap(),
            event
        );

        // The public request path must report each client's session even when
        // another client most recently moved the daemon's foreground cursor.
        use super::super::core::HubInbound;
        use crate::ipc::proto::ClientRequest;
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (mut hub, _inbound) = DaemonHub::new();
        let mut client = None;
        let mut replies = Vec::new();
        for client_id in 0..2 {
            state.rest.foreground = client_id as usize;
            let (frame_tx, frame_rx) = std::sync::mpsc::channel();
            hub.handle_inbound(
                HubInbound::Register {
                    client_id,
                    frame_tx,
                },
                &mut state,
                &mut client,
                runtime.handle(),
            );
            replies.push(frame_rx);
        }
        for client_id in [0, 1, 0] {
            hub.handle_inbound(
                HubInbound::Request {
                    client_id,
                    req: ClientRequest::GetRunState { req_seq: 99 },
                },
                &mut state,
                &mut client,
                runtime.handle(),
            );
            let frame = replies[client_id as usize].try_recv().unwrap();
            let DaemonEvent::RunState {
                req_seq,
                state: reported,
            } = frame.event
            else {
                panic!("expected run state")
            };
            assert_eq!(req_seq, 99);
            assert_eq!(reported.session_id, format!("session-{client_id}"));
            assert_eq!(
                reported.extensions.iter().filter(|e| e.active).count(),
                if client_id == 0 { 2 } else { 1 }
            );
        }
    }
}
