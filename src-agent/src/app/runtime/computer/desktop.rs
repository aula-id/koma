//! GUI-owned worker. Dropping the connection guard cancels queued/native work.
use super::*;
use crate::ipc::proto::ClientRequest;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::Sender,
    Arc,
};
#[cfg(target_os = "linux")]
mod x11;

/// Connection registration identifies the GUI's desktop, never the daemon's
/// inherited display environment (a daemon may outlive several GUI logins).
pub fn identity() -> String {
    if let Ok(display) = std::env::var("WAYLAND_DISPLAY") {
        format!("wayland:{display}")
    } else if let Ok(display) = std::env::var("DISPLAY") {
        format!("x11:{display}")
    } else {
        format!("{}:local", std::env::consts::OS)
    }
}

pub fn capabilities() -> Capabilities {
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            return Capabilities {limitations:vec!["Wayland portal capture/input adapter is not yet available; XWayland is not whole-desktop control".into()],..Default::default()};
        }
        x11::capabilities()
    }
    #[cfg(not(target_os = "linux"))]
    Capabilities {
        limitations: vec![format!(
            "{} native capture/input adapter is not yet available in this build",
            std::env::consts::OS
        )],
        ..Default::default()
    }
}
#[derive(Default)]
pub struct Worker {
    generation: String,
    session: String,
    active: bool,
    cancelled: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
    seen: std::collections::HashSet<String>,
}
impl Worker {
    pub fn status(&mut self, status: &Status) {
        if self.generation != status.generation
            || !status.enabled
            || status.paused
            || status.desktop != identity()
        {
            self.cancelled.store(true, Ordering::SeqCst);
            self.cancelled = Arc::new(AtomicBool::new(false));
            self.seen.clear();
        }
        self.generation = status.generation.clone();
        self.session = status.session.clone();
        self.active = status.enabled && !status.paused && status.desktop == identity();
    }
    pub fn cancel(&mut self) {
        self.active = false;
        self.cancelled.store(true, Ordering::SeqCst);
    }
    pub fn request(&mut self, request: Request, tx: Sender<ClientRequest>) {
        if !self.active
            || request.generation != self.generation
            || request.session != self.session
            || !self.seen.insert(request.id.clone())
            || self.busy.swap(true, Ordering::SeqCst)
        {
            let _ = tx.send(ClientRequest::Computer(Control::Result(Box::new(Reply {
                id: request.id,
                session: request.session,
                generation: request.generation,
                error: Some("native controller unavailable or busy".into()),
                ..Default::default()
            }))));
            return;
        }
        let cancelled = self.cancelled.clone();
        let busy = self.busy.clone();
        std::thread::spawn(move || {
            let _guard = scopeguard::guard((), |_| {
                busy.store(false, Ordering::SeqCst);
            });
            let reply = run(&request, &cancelled);
            if !cancelled.load(Ordering::SeqCst) {
                let _ = tx.send(ClientRequest::Computer(Control::Result(Box::new(reply))));
            }
        });
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
    }
}
fn run(request: &Request, cancelled: &Arc<AtomicBool>) -> Reply {
    // A separate native lock survives daemon revocation until the worker has
    // released every injected key/button. A replacement GUI cannot overlap it.
    let lock = (|| -> anyhow::Result<std::fs::File> {
        let dir = crate::model::store::base_dir()?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(dir.join("computer-input.lock"))?;
        file.try_lock()
            .map_err(|_| anyhow::anyhow!("previous native controller is still stopping"))?;
        Ok(file)
    })();
    let lock = match lock {
        Ok(lock) => lock,
        Err(e) => {
            return Reply {
                id: request.id.clone(),
                session: request.session.clone(),
                generation: request.generation.clone(),
                error: Some(e.to_string()),
                ..Default::default()
            }
        }
    };
    let _lock = scopeguard::guard(lock, |file| {
        let _ = file.unlock();
    });
    #[cfg(target_os = "linux")]
    if let Operation::Observe { crop: Some(bounds) } = &request.operation {
        return super::enrichment::crop(request, *bounds).unwrap_or_else(|e| Reply {
            id: request.id.clone(),
            session: request.session.clone(),
            generation: request.generation.clone(),
            error: Some(e.to_string()),
            ..Default::default()
        });
    }
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        match x11::X11::open(cancelled.clone()) {
            Ok(mut desktop) => {
                let mut reply = super::executor::execute(&mut desktop, request, cancelled);
                super::enrichment::enrich(&mut reply, cancelled);
                return reply;
            }
            Err(e) => {
                return Reply {
                    id: request.id.clone(),
                    session: request.session.clone(),
                    generation: request.generation.clone(),
                    error: Some(e.to_string()),
                    ..Default::default()
                }
            }
        }
    }
    let _ = cancelled;
    Reply {
        id: request.id.clone(),
        session: request.session.clone(),
        generation: request.generation.clone(),
        error: Some("native desktop adapter unavailable".into()),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_superseded_and_duplicate_worker_requests_do_not_reach_desktop() {
        let mut worker = Worker::default();
        let (tx, rx) = std::sync::mpsc::channel();
        let request = Request {
            id: "duplicate".into(),
            session: "s".into(),
            generation: "g".into(),
            operation: Operation::Windows,
            observation: None,
        };
        worker.request(request.clone(), tx.clone());
        assert!(
            matches!(rx.recv().unwrap(),ClientRequest::Computer(Control::Result(r)) if r.error.is_some())
        );
        worker.status(&Status {
            enabled: true,
            desktop: identity(),
            session: "s".into(),
            generation: "new".into(),
            ..Default::default()
        });
        worker.request(request.clone(), tx.clone());
        assert!(
            matches!(rx.recv().unwrap(),ClientRequest::Computer(Control::Result(r)) if r.error.is_some())
        );
        worker.status(&Status {
            enabled: true,
            desktop: identity(),
            session: "s".into(),
            generation: "g".into(),
            ..Default::default()
        });
        worker.seen.insert(request.id.clone());
        worker.request(request, tx);
        assert!(
            matches!(rx.recv().unwrap(),ClientRequest::Computer(Control::Result(r)) if r.error.is_some())
        );
        assert!(!worker.busy.load(Ordering::SeqCst));
    }
}
