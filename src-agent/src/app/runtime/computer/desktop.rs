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

pub fn capabilities() -> Capabilities {
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            return Capabilities {limitations:vec!["Wayland portal capture/input adapter is not yet available; XWayland is not whole-desktop control".into()],..Default::default()};
        }
        return x11::capabilities();
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
}
impl Worker {
    pub fn status(&mut self, status: &Status) {
        if self.generation != status.generation || !status.enabled || status.paused {
            self.cancelled.store(true, Ordering::SeqCst);
            self.cancelled = Arc::new(AtomicBool::new(false));
        }
        self.generation = status.generation.clone();
        self.session = status.session.clone();
        self.active = status.enabled && !status.paused;
    }
    pub fn cancel(&mut self) {
        self.active = false;
        self.cancelled.store(true, Ordering::SeqCst);
    }
    pub fn request(&mut self, request: Request, tx: Sender<ClientRequest>) {
        if !self.active
            || request.generation != self.generation
            || request.session != self.session
            || self.busy.swap(true, Ordering::SeqCst)
        {
            let _ = tx.send(ClientRequest::Computer(Control::Result(Reply {
                id: request.id,
                session: request.session,
                generation: request.generation,
                error: Some("native controller unavailable or busy".into()),
                ..Default::default()
            })));
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
                let _ = tx.send(ClientRequest::Computer(Control::Result(reply)));
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
