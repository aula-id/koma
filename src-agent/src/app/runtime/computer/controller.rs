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
            "Ready; select a window".into()
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
        self.changed = true;
        // Retain owner until the disabled status has been delivered by the hub.
        self.release_lock();
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
    pub fn begin(&mut self, id: String, operation: Operation, plan: bool) -> Result<()> {
        if !self.status.enabled || self.owner.is_none() || self.status.paused {
            bail!("no active local GUI controller");
        }
        if plan && operation.mutates() {
            bail!("plan mode blocks focus and input");
        }
        if self.pending.is_some() || self.used.contains(&id) {
            bail!("duplicate or concurrent computer request");
        }
        let caps = &self.status.capabilities;
        match &operation {
            Operation::Select { generation, .. } if generation != &self.status.generation => {
                bail!("controller changed since window listing/approval")
            }
            Operation::Windows if !caps.windows => bail!("window listing unsupported"),
            Operation::Select { .. } if !caps.focus || !caps.capture => {
                bail!("window selection unsupported; use the OS source picker")
            }
            Operation::Observe { .. } | Operation::InspectWindow { .. } if !caps.capture => {
                bail!("capture unsupported")
            }
            Operation::Observe { crop: Some(_) } if !self.actionable => {
                bail!("cannot crop a stale observation; capture first")
            }
            Operation::Act {
                observation,
                actions,
                ..
            } => {
                let current = self
                    .status
                    .observation
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("observe before acting"))?;
                if !self.actionable
                    || current.id != *observation
                    || current.generation != self.status.generation
                {
                    bail!("stale observation");
                }
                super::executor::validate_actions(current, actions, caps)?;
            }
            _ => {}
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
        self.pending
            .as_ref()
            .is_some_and(|(request, t)| {
                // Only an explicit human source-picker operation gets time for
                // portal consent. Model input keeps its short, non-replayed deadline.
                let seconds = if matches!(&request.operation, Operation::InspectWindow { window } if window == "portal:choose") { 120 } else { 30 };
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
        assert!(a
            .begin("before-enable".into(), Operation::Windows, false)
            .is_err());
        a.enable(1, "s", "fixture", caps.clone(), &path).unwrap();
        let inherited = a.lock.as_ref().unwrap().try_clone().unwrap();
        assert!(b
            .enable(2, "other", "fixture", caps.clone(), &path)
            .is_err());
        a.begin("call".into(), Operation::Windows, false).unwrap();
        let r = a.outbound.clone().unwrap();
        let reply = Reply {
            id: r.id,
            session: r.session,
            generation: r.generation,
            ..Default::default()
        };
        assert!(a.accepts(1, &reply));
        assert!(!a.accepts(2, &reply));
        assert!(a.begin("call".into(), Operation::Windows, false).is_err());
        a.pause(true);
        assert!(!a.accepts(1, &reply));
        a.pause(false);
        assert!(a.begin("call".into(), Operation::Windows, false).is_err());
        a.stop("disconnect");
        assert!(!a.accepts(1, &reply));
        b.enable(2, "other", "fixture", caps, &path).unwrap();
        drop(inherited);
        drop(a);
        drop(b);
        let _ = std::fs::remove_file(path);
    }
}
