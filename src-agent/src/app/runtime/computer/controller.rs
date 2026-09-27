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
    used: HashSet<String>,
    lock: Option<File>,
}
impl Controller {
    pub fn enable(
        &mut self,
        owner: u64,
        session: &str,
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
        self.status = Status {
            session: session.into(),
            generation: uuid::Uuid::new_v4().to_string(),
            enabled: true,
            capabilities,
            message: "Ready; select a window".into(),
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
        self.lock = None;
        pending
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
            Operation::Windows if !caps.windows => bail!("window listing unsupported"),
            Operation::Select { .. } if !caps.focus || !caps.capture => {
                bail!("window selection unsupported; use the OS source picker")
            }
            Operation::Observe { .. } if !caps.capture => bail!("capture unsupported"),
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
            && self.status.enabled
            && !self.status.paused
            && self.pending.as_ref().is_some_and(|(r, _)| {
                r.id == reply.id && r.session == reply.session && r.generation == reply.generation
            })
    }
    pub fn expired(&self) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|(_, t)| t.elapsed() > Duration::from_secs(30))
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
        a.enable(1, "s", caps.clone(), &path).unwrap();
        assert!(b.enable(2, "other", caps.clone(), &path).is_err());
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
        b.enable(2, "other", caps, &path).unwrap();
        drop(a);
        drop(b);
        let _ = std::fs::remove_file(path);
    }
}
