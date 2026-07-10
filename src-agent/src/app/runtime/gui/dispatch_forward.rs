//! Generic request-forwarding helpers for [`super::dispatch::handle_gui_req`], split out
//! of `dispatch.rs` for file size — PURE code motion, no behaviour change. These are not
//! git-specific: they route attachments, config setters, and catalogue fetches to either
//! the attached daemon (`ctx.req`) or the host-relay control channel (`ctx.ctl`).

use std::sync::mpsc::Sender;
use std::sync::Mutex;

use crate::app::runtime::client::HostCtl;
use crate::ipc::proto::ClientRequest;

/// Write `bytes` to a host-writable scratch file, returning its absolute path.
///
/// Used by the [`super::proto::GuiReq::AttachFile`] raw-bytes route: the host can't address
/// the daemon's per-session `images/` dir (it knows neither `pwd_hash` nor the session
/// uuid), so it drops the incoming bytes into `<tmp>/koma/gui-attach/<uuid>-<name>`
/// and hands the daemon that path via [`ClientRequest::Paste`] — the daemon then
/// re-copies it into the session's `images/` on ingest. The original basename +
/// extension are preserved (behind a uuid to avoid collisions) so the daemon's
/// extension-based image sniff still fires. Returns `None` on any fs error (the ipc
/// handler must never panic).
pub(super) fn write_attach_scratch(name: &str, bytes: &[u8]) -> Option<std::path::PathBuf> {
    let mut dir = std::env::temp_dir();
    dir.push("koma");
    dir.push("gui-attach");
    std::fs::create_dir_all(&dir).ok()?;
    let base = std::path::Path::new(name)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "file".to_string());
    let unique = format!("{}-{}", uuid::Uuid::new_v4(), base);
    let path = dir.join(unique);
    std::fs::write(&path, bytes).ok()?;
    Some(path)
}

/// Forward a `ClientRequest::Paste { text: path }` to the currently-attached daemon
/// through the shared live-request slot. Shared by the [`super::proto::GuiReq::AttachFile`]
/// and [`super::proto::GuiReq::AttachPath`] arms — both funnel a filesystem path into the
/// daemon's existing paste/attachment ingest. A missing live sender (no session attached
/// yet) is a silent no-op.
pub(super) fn forward_paste(live_req: &Mutex<Option<Sender<ClientRequest>>>, path: String) {
    if let Ok(g) = live_req.lock() {
        if let Some(tx) = g.as_ref() {
            let _ = tx.send(ClientRequest::Paste { text: path });
        }
    }
}

/// Route a CONFIG-mutating `ClientRequest` to the daemon when a session is ATTACHED, else
/// to the swapper thread for PRE-SESSION apply.
///
/// The Connector/theme setters live in BOTH host states: while attached they forward to
/// the daemon (which owns the authoritative `AppConfig` + re-pushes `Config`); during
/// onboarding/empty-state (the swapper, before any session exists) there is no `live_req`
/// sender, so the request is handed to the client-thread as a [`HostCtl::ConfigMutate`],
/// which applies the config-global subset straight to `~/.koma/config.json` and re-pushes.
/// This is what lets the onboarding theme + provider + model steps work with NO session.
pub(super) fn forward_config_req(
    live_req: &Mutex<Option<Sender<ClientRequest>>>,
    ctl: &Sender<HostCtl>,
    req: ClientRequest,
) {
    if let Ok(g) = live_req.lock() {
        if let Some(tx) = g.as_ref() {
            let _ = tx.send(req);
            return;
        }
    }
    let _ = ctl.send(HostCtl::ConfigMutate(req));
}

/// Route a live-catalogue fetch to the ATTACHED daemon when a session is live, else to the
/// host/swapper thread — the ListModels/ListRoutes twin of [`forward_config_req`].
///
/// Unlike a config setter (which the swapper applies to disk as a single
/// [`HostCtl::ConfigMutate`] wrapping the SAME `ClientRequest`), a catalogue fetch is
/// SERVICED differently on each side — the attached daemon runs it as a `ClientRequest` and
/// replies over the frame stream, while the un-attached swapper runs it as a distinct
/// [`HostCtl`] variant that does the network GET itself — so the two carry different payloads
/// and the caller supplies both. This is what makes the Connector model/route pickers work
/// during onboarding (no session attached), where the plain daemon-only path silently drops
/// the request and strands the picker's spinner.
pub(super) fn forward_or_host(
    live_req: &Mutex<Option<Sender<ClientRequest>>>,
    ctl: &Sender<HostCtl>,
    attached: ClientRequest,
    detached: HostCtl,
) {
    if let Ok(g) = live_req.lock() {
        if let Some(tx) = g.as_ref() {
            let _ = tx.send(attached);
            return;
        }
    }
    let _ = ctl.send(detached);
}
