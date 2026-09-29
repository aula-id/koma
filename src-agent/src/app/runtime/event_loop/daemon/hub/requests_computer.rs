use std::time::Duration;

use super::core::DaemonHub;
use crate::{
    app::{
        runtime::computer::{self, Control},
        state::AppState,
    },
    ipc::proto::DaemonEvent,
};

const RECONNECT_RELAUNCH_AFTER: Duration = Duration::from_secs(4);
const RECONNECT_GIVE_UP_AFTER: Duration = Duration::from_secs(25);
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
                    if let Err(e) = rt
                        .computer
                        .begin(format!("gui:{}", uuid::Uuid::new_v4()), operation)
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
            if rt
                .computer
                .owner
                .is_some_and(|owner| self.clients.iter().all(|c| c.id != owner))
            {
                computer::bridge::gui_client_lost(rt);
            }
            if rt.computer.detached_at.is_some() {
                self.reconnect_computer(rt);
            }
            let Some(owner) = rt.computer.owner else {
                continue;
            };
            let Some(idx) = self.clients.iter().position(|c| c.id == owner) else {
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

    /// Parked computer seat: adopt a GUI that registered the same desktop, or
    /// tell the attached window sharing is still on so it registers. If the
    /// window is gone, open it again. Give up only after the grace.
    fn reconnect_computer(&mut self, rt: &mut crate::app::state::SessionRuntime) {
        let Some(since) = rt.computer.detached_at else {
            return;
        };
        if !rt.computer.status.enabled || !rt.agent_iterating() {
            computer::bridge::stop(rt, "GUI disconnected");
            rt.computer.owner = None;
            return;
        }
        let desktop = rt.computer.status.desktop.clone();
        if let Some(idx) = self.clients.iter().position(|c| {
            c.attached
                && c.foreground.as_deref() == Some(rt.id.as_str())
                && c.computer_desktop.as_deref() == Some(desktop.as_str())
        }) {
            let id = self.clients[idx].id;
            rt.computer.adopt(id);
            return;
        }
        if let Some(idx) = self
            .clients
            .iter()
            .position(|c| c.attached && c.foreground.as_deref() == Some(rt.id.as_str()))
        {
            let id = self.clients[idx].id;
            if rt.computer.notified_client != Some(id) {
                self.send_to(idx, DaemonEvent::ComputerStatus(rt.computer.status.clone()));
                rt.computer.notified_client = Some(id);
            }
        }
        if since.elapsed() >= RECONNECT_RELAUNCH_AFTER && !rt.computer.relaunch_sent {
            rt.computer.relaunch_sent = true;
            relaunch_gui(&rt.id);
        }
        if since.elapsed() >= RECONNECT_GIVE_UP_AFTER {
            computer::bridge::stop(rt, "GUI disconnected");
            rt.computer.owner = None;
        }
    }
}

fn relaunch_gui(session_id: &str) {
    #[cfg(test)]
    {
        let _ = session_id;
    }
    #[cfg(not(test))]
    {
        let Ok(exe) = std::env::current_exe() else {
            crate::model::store::append_global_error_log(
                "computer.reconnect",
                &format!(
                    "session={session_id} request=none relaunch failed: no current executable"
                ),
            );
            return;
        };
        match std::process::Command::new(exe)
            .args(["gui", "--session", session_id])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(child) => {
                crate::model::store::append_global_error_log(
                    "computer.reconnect",
                    &format!(
                        "session={session_id} request=none relaunched GUI pid={}",
                        child.id()
                    ),
                );
            }
            Err(e) => {
                crate::model::store::append_global_error_log(
                    "computer.reconnect",
                    &format!("session={session_id} request=none relaunch failed: {e}"),
                );
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

    #[test]
    fn gui_loss_during_a_turn_keeps_sharing_and_the_next_window_adopts() {
        let mut state = AppState::new(Mode::Chat);
        let (mut hub, _sender) = DaemonHub::new();
        let (tx, _rx) = std::sync::mpsc::channel();
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
        let lock = std::env::temp_dir().join(format!("koma-reconnect-{}", uuid::Uuid::new_v4()));
        let generation = {
            let rt = state.rest.fg_mut();
            let id = rt.id.clone();
            rt.computer
                .enable(1, &id, "fixture", Default::default(), &lock)
                .unwrap();
            rt.waiting = true;
            rt.agent_steps = 1;
            rt.computer.status.generation.clone()
        };
        hub.handle_inbound(
            HubInbound::Disconnect { client_id: 1 },
            &mut state,
            &mut None,
            runtime.handle(),
        );
        {
            let rt = state.rest.fg();
            assert!(rt.computer.status.enabled);
            assert!(rt.computer.owner.is_none());
            assert!(rt.computer.detached_at.is_some());
            assert!(rt.computer.status.message.contains("reconnecting"));
            assert_eq!(rt.computer.status.generation, generation);
        }
        let (tx2, rx2) = std::sync::mpsc::channel();
        hub.handle_inbound(
            HubInbound::Register {
                client_id: 2,
                frame_tx: tx2,
            },
            &mut state,
            &mut None,
            runtime.handle(),
        );
        let sid = state.rest.fg().id.clone();
        hub.clients[0].attached = true;
        hub.clients[0].foreground = Some(sid);
        hub.clients[0].computer_desktop = Some("fixture".into());
        hub.drain_computer(&mut state);
        let rt = state.rest.fg();
        assert!(rt.computer.status.enabled);
        assert_eq!(rt.computer.owner, Some(2));
        assert!(rt.computer.detached_at.is_none());
        assert_eq!(rt.computer.status.generation, generation);
        assert!(rt.computer.status.message.contains("reconnected"));
        assert!(matches!(
            rx2.try_recv().unwrap().event,
            DaemonEvent::ComputerStatus(_)
        ));
        drop(state);
        std::fs::remove_file(lock).unwrap();
    }

    #[test]
    fn gui_loss_while_idle_stops_sharing() {
        let mut state = AppState::new(Mode::Chat);
        let (mut hub, _sender) = DaemonHub::new();
        let (tx, _rx) = std::sync::mpsc::channel();
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
        let lock = std::env::temp_dir().join(format!("koma-idle-stop-{}", uuid::Uuid::new_v4()));
        {
            let rt = state.rest.fg_mut();
            let id = rt.id.clone();
            rt.computer
                .enable(1, &id, "fixture", Default::default(), &lock)
                .unwrap();
        }
        hub.handle_inbound(
            HubInbound::Disconnect { client_id: 1 },
            &mut state,
            &mut None,
            runtime.handle(),
        );
        let rt = state.rest.fg();
        assert!(!rt.computer.status.enabled);
        assert!(rt.computer.owner.is_none());
        assert!(rt.computer.detached_at.is_none());
        drop(state);
        std::fs::remove_file(lock).unwrap();
    }
}
