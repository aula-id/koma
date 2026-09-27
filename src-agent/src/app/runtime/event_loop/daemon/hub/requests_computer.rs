use super::core::DaemonHub;
use crate::{
    app::{
        runtime::computer::{self, Control},
        state::AppState,
    },
    ipc::proto::DaemonEvent,
};
impl DaemonHub {
    pub(super) fn computer(&mut self, idx: usize, state: &mut AppState, control: Control) {
        let owner = self.clients[idx].id;
        if !cfg!(feature = "gui") || !self.clients[idx].attached {
            self.send_to(
                idx,
                DaemonEvent::Error("Computer control requires an attached local GUI".into()),
            );
            return;
        }
        if let Control::Register { desktop } = control {
            if desktop.is_empty() || desktop.len() > 256 || desktop.chars().any(char::is_control) {
                self.send_to(
                    idx,
                    DaemonEvent::Error("Invalid GUI desktop identity".into()),
                );
                return;
            }
            if self.clients[idx]
                .computer_desktop
                .as_ref()
                .is_some_and(|old| old != &desktop)
            {
                for rt in &mut state.rest.sessions {
                    if rt.computer.owner == Some(owner) {
                        computer::bridge::stop(rt, "GUI desktop changed; reactivate explicitly");
                    }
                }
            }
            self.clients[idx].computer_desktop = Some(desktop);
            return;
        }
        let Some(desktop) = self.clients[idx].computer_desktop.clone() else {
            self.send_to(
                idx,
                DaemonEvent::Error("Computer control requires native GUI registration".into()),
            );
            return;
        };
        let rt = state.rest.fg_mut();
        match control {
            Control::Enable { capabilities } => {
                let result = crate::model::store::base_dir().and_then(|dir| {
                    std::fs::create_dir_all(&dir)?;
                    rt.computer.enable(
                        owner,
                        &rt.id,
                        &desktop,
                        capabilities,
                        &dir.join("computer.lock"),
                    )
                });
                if let Err(e) = result {
                    self.send_to(
                        idx,
                        DaemonEvent::ComputerStatus(computer::Status {
                            session: rt.id.clone(),
                            message: e.to_string(),
                            ..Default::default()
                        }),
                    );
                }
            }
            Control::Result(reply) => computer::bridge::receive(rt, owner, *reply),
            other if rt.computer.owner == Some(owner) => match other {
                Control::ListWindows | Control::InspectWindow { .. } => {
                    let operation = match other {
                        Control::InspectWindow { window } => {
                            computer::Operation::InspectWindow { window }
                        }
                        _ => computer::Operation::Windows,
                    };
                    if let Err(e) =
                        rt.computer
                            .begin(format!("gui:{}", uuid::Uuid::new_v4()), operation, false)
                    {
                        rt.computer.status.message = e.to_string();
                        rt.computer.changed = true;
                    }
                }
                Control::Pause | Control::Resume => {
                    computer::bridge::cancel_approval(rt, "computer paused or resumed");
                    if let Some(id) = rt.computer.pause(matches!(other, Control::Pause)) {
                        computer::bridge::settle(rt, id, "cancelled; partial outcome may be uncertain; observe before continuing".into());
                    }
                }
                Control::Stop => computer::bridge::stop(rt, "Stopped; reactivate explicitly"),
                _ => {}
            },
            _ => self.send_to(
                idx,
                DaemonEvent::Error("computer controller ownership mismatch".into()),
            ),
        }
    }
    pub(in crate::app::runtime::event_loop::daemon) fn drain_computer(
        &mut self,
        state: &mut AppState,
    ) {
        for rt in &mut state.rest.sessions {
            if rt.computer.expired() {
                computer::bridge::stop(rt, "Computer operation timed out");
            }
            if rt.closed && rt.computer.status.enabled {
                computer::bridge::stop(rt, "Session closed");
            }
            let Some(owner) = rt.computer.owner else {
                continue;
            };
            let Some(idx) = self.clients.iter().position(|c| c.id == owner) else {
                computer::bridge::stop(rt, "GUI disconnected");
                rt.computer.owner = None;
                continue;
            };
            if self.clients[idx].foreground.as_deref() != Some(rt.id.as_str()) {
                computer::bridge::stop(rt, "Session switched");
            }
            if self.clients[idx].computer_desktop.as_deref()
                != Some(rt.computer.status.desktop.as_str())
            {
                computer::bridge::stop(rt, "GUI desktop registration changed");
            }
            if rt.computer.changed {
                self.send_to(idx, DaemonEvent::ComputerStatus(rt.computer.status.clone()));
                rt.computer.changed = false;
            }
            if let Some(request) = rt.computer.outbound.take() {
                self.send_to(idx, DaemonEvent::ComputerOperation(request));
            }
            if !rt.computer.status.enabled {
                rt.computer.owner = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{mode::Mode, runtime::event_loop::daemon::hub::core::HubInbound};

    #[test]
    fn computer_registration_required_and_desktop_change_revokes_owner() {
        let mut state = AppState::new(Mode::Chat);
        let (mut hub, _sender) = DaemonHub::new();
        let (tx, rx) = std::sync::mpsc::channel();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        hub.handle_inbound(
            HubInbound::Register {
                client_id: 1,
                frame_tx: tx,
            },
            &mut state,
            &mut None,
            runtime.handle(),
        );
        hub.computer(
            0,
            &mut state,
            Control::Register {
                desktop: "fixture".into(),
            },
        );
        assert!(matches!(rx.recv().unwrap().event, DaemonEvent::Error(_)));
        assert!(hub.clients[0].computer_desktop.is_none());
        hub.clients[0].attached = true;
        hub.computer(
            0,
            &mut state,
            Control::Enable {
                capabilities: Default::default(),
            },
        );
        assert!(matches!(rx.recv().unwrap().event, DaemonEvent::Error(_)));
        assert!(!state.rest.fg().computer.status.enabled);
        hub.computer(
            0,
            &mut state,
            Control::Register {
                desktop: "fixture".into(),
            },
        );
        if !cfg!(feature = "gui") {
            assert!(matches!(rx.recv().unwrap().event, DaemonEvent::Error(_)));
            assert!(hub.clients[0].computer_desktop.is_none());
            return;
        }
        assert_eq!(hub.clients[0].computer_desktop.as_deref(), Some("fixture"));
        let lock = std::env::temp_dir().join(format!("koma-desktop-{}", uuid::Uuid::new_v4()));
        let rt = state.rest.fg_mut();
        rt.computer
            .enable(1, &rt.id, "fixture", Default::default(), &lock)
            .unwrap();
        hub.computer(
            0,
            &mut state,
            Control::Register {
                desktop: "replacement".into(),
            },
        );
        assert!(!state.rest.fg().computer.status.enabled);
        hub.drain_computer(&mut state);
        assert!(state.rest.fg().computer.owner.is_none());
        drop(state);
        std::fs::remove_file(lock).unwrap();
    }
}
