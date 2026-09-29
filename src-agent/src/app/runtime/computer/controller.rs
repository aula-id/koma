use super::*;
use anyhow::{bail, Result};
use std::{
    collections::HashSet,
    fs::File,
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct Controller {
    pub owner: Option<u64>,
    pub status: Status,
    pub outbound: Option<Request>,
    pub pending: Option<(Request, Instant)>,
    pub changed: bool,
    pub actionable: bool,
    pub latest_message: Option<crate::dto::chat::ChatMessage>,
    used: HashSet<String>,
    lock: Option<File>,
    /// Set when the owning GUI client left during a live turn. Sharing, the
    /// desktop lock, and the current observation stay put so the returning
    /// window can adopt the seat without a new grant.
    pub detached_at: Option<Instant>,
    pub relaunch_sent: bool,
    pub notified_client: Option<u64>,
}
impl Controller {
    pub fn preserve_observation(&self, history: &mut Vec<crate::dto::chat::ChatMessage>) {
        if !self.status.enabled || !self.actionable {
            return;
        }
        if let Some(message) = &self.latest_message {
            let present = history.iter().any(|m| m.attachments == message.attachments);
            if !present {
                history.push(message.clone());
            }
        }
    }
    pub fn enable(
        &mut self,
        owner: u64,
        session: &str,
        desktop: &str,
        capabilities: Capabilities,
        lock_path: &std::path::Path,
    ) -> Result<()> {
        if self.owner.is_some() {
            bail!("computer already controlled; stop first");
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)?;
        file.try_lock()
            .map_err(|_| anyhow::anyhow!("another Koma session controls this desktop"))?;
        self.lock = Some(file);
        self.owner = Some(owner);
        self.used.clear();
        let message = if capabilities.capture {
            "Ready; select a screen or application".into()
        } else {
            format!(
                "Capture unavailable: {}",
                capabilities.limitations.join("; ")
            )
        };
        self.status = Status {
            session: session.into(),
            desktop: desktop.into(),
            generation: uuid::Uuid::new_v4().to_string(),
            enabled: true,
            capabilities,
            message,
            ..Status::default()
        };
        self.changed = true;
        Ok(())
    }
    pub fn stop(&mut self, reason: &str) -> Option<String> {
        let pending = self.pending.take().map(|(r, _)| r.id);
        self.outbound = None;
        self.status.enabled = false;
        self.status.paused = false;
        self.status.busy = false;
        self.actionable = false;
        self.status.generation = uuid::Uuid::new_v4().to_string();
        self.status.message = reason.into();
        self.detached_at = None;
        self.relaunch_sent = false;
        self.notified_client = None;
        self.changed = true;
        // Retain owner until the disabled status has been delivered by the hub.
        self.release_lock();
        pending
    }
    /// The GUI client vanished mid-turn. Keep sharing and the observation.
    pub fn detach_for_reconnect(&mut self) {
        self.owner = None;
        if self.detached_at.is_none() {
            self.detached_at = Some(Instant::now());
        }
        self.status.message = "GUI reconnecting; sharing stays on".into();
        self.changed = true;
    }
    /// A replacement GUI for this same desktop takes the parked seat.
    /// The generation is unchanged, so the frame the model already has
    /// stays actionable.
    pub fn adopt(&mut self, owner: u64) {
        self.owner = Some(owner);
        self.detached_at = None;
        self.relaunch_sent = false;
        self.notified_client = None;
        self.status.message = "GUI reconnected; sharing stays on".into();
        self.changed = true;
    }
    /// The turn was interrupted. Drop the in-flight desktop action and keep
    /// sharing enabled; only the user's stop control turns it off.
    pub fn halt_turn(&mut self) -> Option<String> {
        let pending = self.pending.take().map(|(r, _)| r.id);
        self.outbound = None;
        self.status.busy = false;
        self.actionable = false;
        self.status.generation = uuid::Uuid::new_v4().to_string();
        if self.status.enabled {
            self.status.message = "Turn interrupted; sharing stays on".into();
        }
        self.changed = true;
        pending
    }
    fn release_lock(&mut self) {
        if let Some(file) = self.lock.take() {
            // Explicitly unlock before closing: concurrent process creation can
            // briefly inherit the open file description until exec completes.
            let _ = file.unlock();
        }
    }
    pub fn pause(&mut self, pause: bool) -> Option<String> {
        let pending = self.pending.take().map(|(r, _)| r.id);
        self.outbound = None;
        self.status.paused = pause;
        self.status.busy = false;
        self.actionable = false;
        self.status.generation = uuid::Uuid::new_v4().to_string();
        self.status.message = if pause {
            "Paused"
        } else {
            "Resumed; observe before acting"
        }
        .into();
        self.changed = true;
        pending
    }
    /// GUI enablement authorizes native operations until pause/stop/disconnect,
    /// independently of workspace approval modes and the tool classifier.
    pub fn begin(&mut self, id: String, operation: Operation) -> Result<()> {
        // A parked seat (owner gone, detach in progress) may queue the next
        // operation. It is delivered once the replacement GUI adopts.
        if !self.status.enabled
            || self.status.paused
            || (self.owner.is_none() && self.detached_at.is_none())
        {
            bail!("no active local GUI controller");
        }
        if self.pending.is_some() || self.used.contains(&id) {
            bail!("duplicate or concurrent computer request");
        }
        let caps = &self.status.capabilities;
        match &operation {
            Operation::Select { generation, .. } if generation != &self.status.generation => {
                bail!("controller changed since source listing")
            }
            Operation::Windows if !caps.windows => bail!("source listing unsupported"),
            Operation::Select { .. } if !caps.focus || !caps.capture => {
                bail!("display selection unsupported; use the OS source picker")
            }
            Operation::Select { window, .. }
                if !(is_screen(window)
                    || window == "portal:choose"
                    || window == "portal:choose:screen") =>
            {
                bail!(
                    "Computer use shares a whole screen. Choose a display: source, not an application window."
                )
            }
            Operation::Observe { .. } | Operation::InspectWindow { .. } if !caps.capture => {
                bail!("capture unsupported")
            }
            Operation::Observe {
                crop: Some(_),
                region: Some(_),
            } => bail!("choose crop or region, not both"),
            Operation::Observe { crop: Some(_), .. } if !self.actionable => {
                bail!("cannot crop a stale observation; capture first")
            }
            Operation::Observe {
                region: Some(_), ..
            } if !self.actionable => {
                bail!("cannot target a region from a stale observation; observe the display first")
            }
            Operation::Act {
                observation,
                actions,
                ..
            } => {
                let current = self.status.observation.as_ref().ok_or(ObservationRequired(
                    "No observation available; select a screen and observe before acting",
                ))?;
                if current.id != *observation && uuid::Uuid::parse_str(observation).is_err() {
                    return Err(ObservationRequired("Invalid observation reference: copy the exact observation_id from a computer observation result, not a description, request ID, or image number").into());
                }
                if !self.actionable
                    || current.id != *observation
                    || current.generation != self.status.generation
                {
                    return Err(ObservationRequired("Stale observation: this frame is no longer actionable; observe again and use the new observation_id").into());
                }
                if view_only(&current.window.id, caps.pointer, caps.keyboard) {
                    return Err(ScreenRequired.into());
                }
                if !is_screen(&current.window.id) {
                    bail!("Computer use shares a whole screen. Select a display and observe it.");
                }
                super::executor::validate_actions(current, actions, caps)?;
            }
            _ => {}
        }
        if let Operation::Observe { crop, region } = &operation {
            if let Some(bounds) = crop.or(*region) {
                let current = self
                    .status
                    .observation
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("observe before inspecting a region"))?;
                current.transform.crop(bounds)?;
                if region.is_some() && !current.window.id.starts_with("display:") {
                    return Err(RegionCaptureUnavailable(bounds).into());
                }
            }
        }
        let request = Request {
            id: id.clone(),
            session: self.status.session.clone(),
            generation: self.status.generation.clone(),
            operation,
            observation: self.status.observation.clone(),
        };
        if !matches!(request.operation, Operation::Windows) {
            self.actionable = false;
        }
        self.used.insert(id);
        self.pending = Some((request.clone(), Instant::now()));
        self.outbound = Some(request);
        self.status.busy = true;
        self.changed = true;
        Ok(())
    }
    pub fn accepts(&self, owner: u64, reply: &Reply) -> bool {
        self.owner == Some(owner)
            && !self.expired()
            && self.status.enabled
            && !self.status.paused
            && self.pending.as_ref().is_some_and(|(r, _)| {
                r.id == reply.id && r.session == reply.session && r.generation == reply.generation
            })
    }
    pub fn expired(&self) -> bool {
        // The reconnect grace owns the deadline while the window is coming back.
        if self.detached_at.is_some() {
            return false;
        }
        self.pending
            .as_ref()
            .is_some_and(|(request, t)| {
                // Only an explicit human source-picker operation gets time for
                // portal consent. Model input keeps its short, non-replayed deadline.
                let seconds = if matches!(&request.operation, Operation::InspectWindow { window } if window == "portal:choose" || window.starts_with("portal:choose:")) { 120 } else { 30 };
                t.elapsed() > Duration::from_secs(seconds)
            })
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.release_lock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ownership_duplicates_and_late_results() {
        let path = std::env::temp_dir().join(format!("computer-{}.lock", uuid::Uuid::new_v4()));
        let mut a = Controller::default();
        let mut b = Controller::default();
        let caps = Capabilities {
            windows: true,
            ..Default::default()
        };
        assert!(a.begin("before-enable".into(), Operation::Windows).is_err());
        a.enable(1, "s", "fixture", caps.clone(), &path).unwrap();
        let inherited = a.lock.as_ref().unwrap().try_clone().unwrap();
        assert!(b
            .enable(2, "other", "fixture", caps.clone(), &path)
            .is_err());
        a.begin("call".into(), Operation::Windows).unwrap();
        let r = a.outbound.clone().unwrap();
        let reply = Reply {
            id: r.id,
            session: r.session,
            generation: r.generation,
            ..Default::default()
        };
        assert!(a.accepts(1, &reply));
        assert!(!a.accepts(2, &reply));
        assert!(a.begin("call".into(), Operation::Windows).is_err());
        a.pause(true);
        assert!(!a.accepts(1, &reply));
        a.pause(false);
        assert!(a.begin("call".into(), Operation::Windows).is_err());
        a.stop("disconnect");
        assert!(!a.accepts(1, &reply));
        b.enable(2, "other", "fixture", caps, &path).unwrap();
        drop(inherited);
        drop(a);
        drop(b);
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn interrupting_a_turn_leaves_sharing_enabled() {
        let path =
            std::env::temp_dir().join(format!("computer-halt-{}.lock", uuid::Uuid::new_v4()));
        let mut controller = Controller::default();
        controller
            .enable(1, "s", "fixture", Capabilities::default(), &path)
            .unwrap();
        let generation = controller.status.generation.clone();
        controller.actionable = true;
        controller.status.busy = true;
        controller.halt_turn();
        assert!(controller.status.enabled);
        assert!(!controller.status.paused);
        assert!(!controller.actionable);
        assert!(!controller.status.busy);
        assert_ne!(controller.status.generation, generation);
        assert!(controller.status.message.contains("sharing stays on"));
        controller.stop("user turned sharing off");
        assert!(!controller.status.enabled);
        drop(controller);
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn a_parked_seat_still_queues_the_next_operation() {
        let path =
            std::env::temp_dir().join(format!("computer-park-{}.lock", uuid::Uuid::new_v4()));
        let mut controller = Controller::default();
        let caps = Capabilities {
            windows: true,
            ..Default::default()
        };
        controller.enable(1, "s", "fixture", caps, &path).unwrap();
        let generation = controller.status.generation.clone();
        controller.actionable = true;
        controller.detach_for_reconnect();
        assert!(controller.status.enabled);
        assert!(controller.owner.is_none());
        assert_eq!(controller.status.generation, generation);
        controller.begin("next".into(), Operation::Windows).unwrap();
        assert!(controller.outbound.is_some());
        assert!(!controller.expired());
        controller.adopt(7);
        assert_eq!(controller.owner, Some(7));
        assert_eq!(controller.status.generation, generation);
        assert!(controller.status.enabled);
        assert!(controller.detached_at.is_none());
        drop(controller);
        let _ = std::fs::remove_file(path);
    }
}
