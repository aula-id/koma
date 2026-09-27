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
        let rt = state.rest.fg_mut();
        match control {
            Control::Enable { capabilities } => {
                if !cfg!(feature = "gui") {
                    self.send_to(
                        idx,
                        DaemonEvent::Error("Computer control requires a GUI build".into()),
                    );
                    return;
                }
                let result = crate::model::store::base_dir().and_then(|dir| {
                    std::fs::create_dir_all(&dir)?;
                    rt.computer
                        .enable(owner, &rt.id, capabilities, &dir.join("computer.lock"))
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
