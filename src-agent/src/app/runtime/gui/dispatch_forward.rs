//! Generic request-forwarding helpers for [`super::dispatch::handle_gui_req`], split out
//! of `dispatch.rs` for file size — PURE code motion, no behaviour change. These are not
//! git-specific: they route attachments, config setters, and catalogue fetches to either
//! the attached daemon (`ctx.req`) or the host-relay control channel (`ctx.ctl`).

use std::sync::mpsc::Sender;
use std::sync::Mutex;

use crate::app::runtime::client::HostCtl;
use crate::ipc::proto::{ClientRequest, MAX_FRAME_BYTES};
use crate::model::skill::{
    append_chunk, sweep_stale, upload_dir, SkillItemOutcome, Staged, MAX_SKILL_ZIP_BYTES,
};

use super::dispatch::GuiReqCtx;
use super::proto::UserEvent;

/// Staged files older than this are swept when a new upload starts.
const STALE_UPLOAD_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Room left in a frame for the JSON envelope around an inline base64 package.
const INLINE_ENVELOPE_BYTES: usize = 4096;

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
            if tx.send(attached).is_ok() {
                return;
            }
            // During detach/swap the live slot may still hold a closed sender.
            // Fall back to the host route rather than dropping the request and
            // leaving its GUI operation unacknowledged.
        }
    }
    let _ = ctl.send(detached);
}

/// Tell the UI an install failed, through the same `SkillOp` push a daemon reply produces.
pub(super) fn push_skill_failure(
    proxy: &tao::event_loop::EventLoopProxy<UserEvent>,
    request_id: String,
    session_epoch: u64,
    tab_id: String,
    name: String,
    message: String,
) {
    let outcomes = vec![SkillItemOutcome {
        name,
        status: "failed".into(),
        error: Some(message),
    }];
    crate::app::runtime::client::push_proto::push_skill_op(
        &|json| {
            let _ = proxy.send_event(UserEvent::Push(json));
        },
        request_id,
        session_epoch,
        tab_id,
        "install-zip".into(),
        outcomes,
        Vec::new(),
    );
}

/// True when a package of `raw_len` bytes still fits one IPC frame once base64-encoded
/// (the only way to hand it to a remote daemon, which cannot read the host's files).
pub(super) fn fits_inline_frame(raw_len: u64) -> bool {
    let encoded = raw_len.div_ceil(3).saturating_mul(4);
    encoded.saturating_add(INLINE_ENVELOPE_BYTES as u64) <= MAX_FRAME_BYTES as u64
}

/// Whether an inline `InstallSkillZip` payload can be written as one frame.
pub(super) fn inline_payload_fits(data_b64_len: usize) -> bool {
    data_b64_len.saturating_add(INLINE_ENVELOPE_BYTES) <= MAX_FRAME_BYTES
}

fn log_upload_rejected(route: &str, detail: &str) {
    crate::model::store::append_global_error_log(
        "skill upload rejected",
        &format!("{route}: {detail}"),
    );
}

/// Metadata of one `SkillUploadChunk` request.
pub(super) struct UploadChunk {
    pub(super) target: String,
    pub(super) name: String,
    pub(super) upload_id: String,
    pub(super) offset: u64,
    pub(super) total: u64,
    pub(super) data_b64: String,
    pub(super) request_id: String,
    pub(super) session_epoch: u64,
    pub(super) tab_id: String,
}

/// Append a chunk to the staged file; once the package is complete hand it to the daemon
/// (local attach or no session: by path) or, for a remote session, inline as bytes.
pub(super) fn handle_upload_chunk(ctx: &GuiReqCtx, chunk: UploadChunk) {
    let UploadChunk {
        target,
        name,
        upload_id,
        offset,
        total,
        data_b64,
        request_id,
        session_epoch,
        tab_id,
    } = chunk;
    let staged = upload_dir().map_err(|e| e.to_string()).and_then(|dir| {
        if offset == 0 {
            sweep_stale(&dir, STALE_UPLOAD_AGE);
        }
        append_chunk(&dir, &upload_id, offset, total, &data_b64)
    });
    let path = match staged {
        Ok(Staged::More) => return,
        Ok(Staged::Complete(path)) => path,
        Err(message) => {
            log_upload_rejected("GUI upload staging", &message);
            push_skill_failure(
                &ctx.loop_proxy,
                request_id,
                session_epoch,
                tab_id,
                name,
                message,
            );
            return;
        }
    };
    if !crate::app::runtime::client::remote_attached() {
        let request = ClientRequest::InstallSkillZipPath {
            target,
            name,
            path: path.to_string_lossy().into_owned(),
            request_id,
            session_epoch,
            tab_id,
        };
        forward_or_host(
            &ctx.req,
            &ctx.ctl,
            request.clone(),
            HostCtl::SkillMutation(request),
        );
        return;
    }
    let req = std::sync::Arc::clone(&ctx.req);
    let ctl = ctx.ctl.clone();
    let proxy = ctx.loop_proxy.clone();
    std::thread::spawn(move || {
        let fail = |message: String| {
            log_upload_rejected("remote session upload", &message);
            push_skill_failure(
                &proxy,
                request_id.clone(),
                session_epoch,
                tab_id.clone(),
                name.clone(),
                message,
            );
        };
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if size > MAX_SKILL_ZIP_BYTES || !fits_inline_frame(size) {
            let _ = std::fs::remove_file(&path);
            fail("Skill package is too large to install into a remote session.".into());
            return;
        }
        let bytes = std::fs::read(&path);
        let _ = std::fs::remove_file(&path);
        let Ok(bytes) = bytes else {
            fail("Could not read the staged upload.".into());
            return;
        };
        use base64::Engine;
        let request = ClientRequest::InstallSkillZip {
            target,
            name: name.clone(),
            data_b64: base64::engine::general_purpose::STANDARD.encode(bytes),
            request_id: request_id.clone(),
            session_epoch,
            tab_id: tab_id.clone(),
        };
        forward_or_host(&req, &ctl, request.clone(), HostCtl::SkillMutation(request));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_frame_limit_matches_the_frame_cap() {
        assert!(fits_inline_frame(1024));
        assert!(fits_inline_frame(40 * 1024 * 1024));
        // 48 MiB raw is 64 MiB of base64, which no longer leaves room for the envelope.
        assert!(!fits_inline_frame(48 * 1024 * 1024));
        assert!(!fits_inline_frame(u64::MAX));
        assert!(inline_payload_fits(1024));
        assert!(!inline_payload_fits(MAX_FRAME_BYTES));
    }

    #[test]
    fn closed_attached_request_channel_uses_host_route_instead_of_losing_save() {
        let (dead_tx, dead_rx) = std::sync::mpsc::channel();
        drop(dead_rx);
        let (ctl_tx, ctl_rx) = std::sync::mpsc::channel();
        let request = ClientRequest::UpdateSkill {
            skill_id: "opaque".into(),
            generation: "old".into(),
            name: "demo".into(),
            description: "new".into(),
            triggers: String::new(),
            allowed_tools: vec![],
            instruction: "body".into(),
            reload_after_save: false,
            target_session_id: None,
            request_id: "save-1".into(),
            session_epoch: 1,
            tab_id: "skill:demo".into(),
        };
        forward_or_host(
            &Mutex::new(Some(dead_tx)),
            &ctl_tx,
            request.clone(),
            HostCtl::SkillMutation(request),
        );
        assert!(
            matches!(ctl_rx.try_recv(), Ok(HostCtl::SkillMutation(ClientRequest::UpdateSkill { request_id, .. })) if request_id == "save-1")
        );
    }
}
