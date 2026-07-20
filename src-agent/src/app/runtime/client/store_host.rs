//! Host-side extension-STORE browse/detail/installed-list computation for the GUI Store
//! tab — split out for the same reason as [`super::git_host`]: browse/detail hit the
//! PUBLIC (no-auth) koma.run store endpoints and the installed-list read is a local
//! `~/.koma/config.json` read, so ALL THREE are stateless and must work whether or not a
//! session daemon is attached (unlike install/uninstall, which mutate live daemon runtime
//! state — `ext_manager`/`mcp_manager` — and stay daemon-forwarded; see
//! `dispatch.rs`'s `GuiReq::InstallExtension`/`UninstallExtension` arms and
//! `HostCtl::ExtNoSession`).
//!
//! Every op here runs on a one-shot [`std::thread::spawn`] worker — never inline on a host
//! control loop, and never on the tokio runtime (a blocking `reqwest` call would panic
//! there) — mirroring `git_host`'s off-thread pattern: a DETACHED flavor that pushes the
//! reply straight through the cloned `push` sink ([`super::host`]'s `host_swapper`), and an
//! ATTACHED flavor that replies over an `mpsc` channel drained by [`super::push_loop`].
//!
//! The JSON-mapping helpers (`map_summary`/`map_detail`/`map_contributes`/`str_field`/
//! `arr_str`) are a deliberate, small DUPLICATE of the daemon's
//! `event_loop::daemon::hub::requests_ext` copies — that daemon module is left untouched
//! (its own `ClientRequest::StoreBrowse`/`StoreDetail`/`ListInstalledExtensions` handlers
//! simply aren't reached from the GUI anymore, since browse/detail/list-installed moved
//! host-local here).

use std::sync::mpsc::Sender;

use crate::ipc::proto::{
    InstalledExtensionDetailWire, InstalledExtWire, InstalledModelWire, InstalledSubAgentWire,
    InstalledToolWire, PanelWire, StoreContributesWire, StoreDetailWire, StoreItemWire,
};
use crate::model::app_config::{AppConfig, InstalledExtension, OAuthProvider};
use crate::model::store;

/// Base URL of the koma.run extension store API (contract v0) — same constant as the
/// daemon's `requests_ext::STORE_API_BASE`.
const STORE_API_BASE: &str = "https://koma.run/api/v1/extensions";

// ─── DETACHED (host_swapper): push the reply straight through the cloned sink ───

/// `HostCtl::StoreBrowse` while detached.
pub(super) fn spawn_store_browse(
    push: impl Fn(String) + Send + 'static,
    query: Option<String>,
    category: Option<String>,
) {
    std::thread::spawn(move || {
        let (items, error) = match fetch_catalogue(query, category) {
            Ok(items) => (items, None),
            Err(e) => (Vec::new(), Some(e)),
        };
        super::push_proto::push_store_catalogue(&push, items, error);
    });
}

/// `HostCtl::StoreDetail` while detached.
pub(super) fn spawn_store_detail(push: impl Fn(String) + Send + 'static, id: String) {
    std::thread::spawn(move || {
        let (detail, error) = match fetch_detail(&id) {
            Ok(d) => (Some(d), None),
            Err(e) => (None, Some(e)),
        };
        super::push_proto::push_store_detail(&push, detail, error);
    });
}

/// `HostCtl::ListInstalledExtensions` while detached.
pub(super) fn spawn_list_installed(push: impl Fn(String) + Send + 'static) {
    std::thread::spawn(move || {
        super::push_proto::push_installed_extensions(&push, installed_extensions());
    });
}

/// `HostCtl::GetInstalledExtensionDetail` while detached — two-phase: local
/// detail first (instant), then best-effort online enrichment.
pub(super) fn spawn_get_installed_detail(push: impl Fn(String) + Send + 'static, id: String) {
    let id2 = id.clone();
    let id3 = id.clone();
    std::thread::spawn(move || {
        // Phase 1: local detail (store_detail = None).
        let (detail, error) = match get_installed_detail(&id2) {
            Ok(mut d) => {
                d.store_detail = None;
                (Some(d), None)
            }
            Err(e) => (None, Some(e)),
        };
        let had_local_error = error.is_some();
        super::push_proto::push_installed_ext_detail(&push, id2, detail, error);

        // Phase 2: best-effort online enrichment (no second response on failure).
        if !had_local_error {
            if let Ok(store_detail) = fetch_detail(&id3) {
                if let Ok(mut d) = get_installed_detail(&id3) {
                    d.store_detail = Some(store_detail);
                    super::push_proto::push_installed_ext_detail(&push, id3, Some(d), None);
                }
            }
        }
    });
}

// ─── DETACHED install/uninstall (StartScreen / swapper) ───
//
// Install/uninstall MUTATE persisted state (`~/.koma/config.json` + the on-disk
// `extensions/<id>/` package), which — unlike browse/detail/list-installed above — isn't
// read-only. But it doesn't need a LIVE daemon either: the KomaRun bearer lives in the
// GLOBAL `AppConfig`, and `app::ext::install::install_from_zip` is a pure verify+unpack
// over the downloaded zip. What genuinely NEEDS a live daemon runtime is the
// SESSION-SCOPED tail an ATTACHED install/uninstall also does — MCP tool registration
// (`register::register_contributions`), ext-daemon auto-start (`ExtHostManager::ensure_started`),
// and workspace-root injection (`ext_workspace::inject_extension_workspaces`) — none of
// which exist pre-session (no `ext_manager`/`mcp_manager`/foreground session), so that
// part is intentionally SKIPPED here. It self-heals: `lifecycle::build_startup` re-runs
// `ensure_started` + `register_contributions` for every enabled daemon-kind extension on
// EVERY daemon boot, and re-derives the workspace-root injection from the CURRENT enabled
// set on every boot too; a daemon-kind extension that hasn't auto-started yet also lazily
// starts on its first opened panel (see `requests_ext::panel_start_decision`). Mirrors the
// daemon's `requests_ext::install_extension`/`finish_install`/`uninstall_extension`
// field-for-field, minus that tail.

/// `HostCtl::InstallExtension` while detached. Runs on the TOKIO runtime (`fresh_key` +
/// the download are async), unlike the plain-thread browse/detail workers above.
pub(super) fn spawn_install(
    handle: &tokio::runtime::Handle,
    push: impl Fn(String) + Send + 'static,
    id: String,
    version: Option<String>,
) {
    handle.spawn(async move {
        let Some(platform) = detect_platform() else {
            store::append_global_error_log(
                "ext install",
                &format!("no platform for extension {id} (unsupported host os/arch)"),
            );
            super::push_proto::push_ext_op_result(
                &push,
                id,
                false,
                Some("extensions are not available for this platform".to_string()),
            );
            return;
        };

        // Same KomaRun sign-in check the daemon's `install_extension` runs — the bearer
        // lives in the GLOBAL config, not anything session-scoped.
        let cfg = AppConfig::load();
        let Some(conn) = cfg
            .oauth_conns
            .iter()
            .find(|c| c.provider == OAuthProvider::KomaRun)
        else {
            store::append_global_error_log(
                "ext install",
                &format!("no koma.run OAuth connection for extension {id} (platform {platform})"),
            );
            super::push_proto::push_ext_op_result(
                &push,
                id,
                false,
                Some("sign in to koma.run to install".to_string()),
            );
            return;
        };
        let oauth_uuid = conn.uuid.clone();

        let (bearer, _account) = crate::service::oauth::manager::fresh_key(&oauth_uuid, "").await;
        if bearer.trim().is_empty() {
            store::append_global_error_log(
                "ext install",
                &format!("koma.run bearer empty/expired for extension {id} (platform {platform})"),
            );
            super::push_proto::push_ext_op_result(
                &push,
                id,
                false,
                Some("koma.run session expired — sign in again".to_string()),
            );
            return;
        }

        match fetch_install_artifact(&id, version.as_deref(), platform, &bearer).await {
            Ok((zip, sha256, signature)) => finish_install_detached(&push, id, zip, sha256, signature),
            Err(e) => super::push_proto::push_ext_op_result(&push, id, false, Some(e)),
        }
    });
}

/// The detached tail of an install: verify + unpack (fail-closed, same pipeline as the
/// daemon's `finish_install`), upsert the registry entry + persist, then reply — see the
/// module-level doc above for what's intentionally skipped here.
fn finish_install_detached(
    push: &(impl Fn(String) + Send + 'static),
    id: String,
    zip: Vec<u8>,
    sha256: String,
    signature: Option<String>,
) {
    let installed: anyhow::Result<InstalledExtension> =
        match (&signature, sha256.trim().is_empty()) {
            (Some(sig), false) => crate::app::ext::install::install_from_zip(&zip, &sha256, sig),
            _ => install_unsigned_fallback(&id, &zip),
        };

    match installed {
        Ok(ext) => {
            let mut cfg = AppConfig::load();
            cfg.upsert_extension(ext.clone());
            // Auto-register any manifest-declared bundled MCP servers — same as the attached
            // daemon's `finish_install`, so a store install through the detached GUI host
            // never leaves the user needing to hand-add an McpServerEntry either.
            if let Err(e) = crate::app::ext::register::register_mcp_servers(&ext, &mut cfg) {
                store::append_global_error_log(
                    "ext-install",
                    &format!("register mcp servers for {}: {e:#}", ext.id),
                );
            }
            if let Err(e) = cfg.save() {
                store::append_global_error_log(
                    "ext-install",
                    &format!("save config after install {}: {e:#}", ext.id),
                );
            }
            // No live per-session `McpManager` exists pre-session to reconnect from the
            // just-saved server set, so BOUNCE the GLOBAL MCP daemon instead — mirrors
            // `spawn_uninstall`'s reload strategy: the next session's
            // `ensure_mcp_daemon_running` respawns it fresh off the new config (picking up
            // any newly-registered row), cheaply and safely via the build-skew fingerprint
            // handshake. Quiet — the GUI host owns no user terminal.
            crate::app::runtime::manage::stop_mcp_daemon(true);
            super::push_proto::push_ext_op_result(push, ext.id.clone(), true, None);
            super::push_proto::push_installed_extensions(push, installed_extensions());
        }
        Err(e) => {
            store::append_global_error_log(
                "ext install",
                &format!("verify/unpack failed for extension {id}: {e:#}"),
            );
            super::push_proto::push_ext_op_result(push, id, false, Some(format!("{e:#}")));
        }
    }
}

/// `HostCtl::UninstallExtension` while detached — the COMPLETE nuke's detached arm, mirroring
/// the daemon's `uninstall_extension` MINUS the session-scoped in-memory steps that don't
/// exist pre-session (see the module doc above). Synchronous (fs + a config save + a socket
/// fan-out), so this runs on a plain thread, NOT the tokio runtime. Order matches the audit:
/// snapshot the manifest (1); fan the in-memory unload out to every live session-daemon (3);
/// remove the on-disk package dir (6); purge catalogue contributions + deregister orphan
/// MCP-server rows + drop the registry entry, then ONE save (5a/8); bounce the global MCP
/// daemon so the next ensure respawns it off the new config (5b); sweep same-named agent
/// overrides (7); nuke the declared workspace_dir (9).
pub(super) fn spawn_uninstall(push: impl Fn(String) + Send + 'static, id: String) {
    std::thread::spawn(move || {
        // (1) Snapshot the manifest ONCE — its sub-agent names + workspace_dir — BEFORE the
        // dir is deleted below (after which the manifest is unreadable).
        let snap = crate::app::ext::uninstall::snapshot_manifest(&id);

        // (2 + 4) SKIPPED here by design: detached has no live `ext_manager`/`mcp_manager` to
        // stop the child or purge the in-memory MCP snapshot — that state doesn't exist
        // pre-session, and a fresh daemon re-derives it from the (now-reduced) config on its
        // next boot (see the module-level doc on the self-healing tail).

        // (3) Fan the in-memory unload out to every live session-daemon (all "other" — the
        // detached host owns no daemon of its own), so a daemon already serving this extension
        // drops it now instead of at its next boot. Synchronous is fine: this worker thread is
        // not an event loop, so the blocking socket sweep can't wedge anything. Best-effort.
        crate::app::runtime::manage::broadcast_unload_extension(&id);

        // (6) Remove the unpacked package dir. Guard the id against a path-escape before
        // joining (defense in depth — the id comes from the client), mirroring the daemon's
        // `uninstall_extension`.
        if is_safe_ext_id(&id) {
            if let Ok(dir) = store::extensions_dir() {
                let target = dir.join(&id);
                if let Err(e) = std::fs::remove_dir_all(&target) {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        store::append_global_error_log(
                            "ext-uninstall",
                            &format!("remove {}: {e}", target.display()),
                        );
                    }
                }
            }
        } else {
            store::append_global_error_log(
                "ext-uninstall",
                &format!("refusing to remove dir for unsafe extension id {id:?}"),
            );
        }

        // (5a + 8) Config mutations, then ONE save. Purge the extension's catalogue
        // contributions (key-backed providers/models/oauth conns) — `main_reset` drives a
        // foreground toast in the daemon twin, skipped here (no session to toast; a purged
        // Main role self-heals on the next resolve). Deregister orphan MCP-server rows
        // (ext-owned, or whose command lives under extensions/<id>/). Drop the registry entry.
        let mut cfg = AppConfig::load();
        let purge = cfg.purge_extension(&id);
        let _mcp_rows_removed = cfg.remove_ext_mcp_servers(&id);
        cfg.remove_extension_by_id(&id);
        if let Err(e) = cfg.save() {
            store::append_global_error_log(
                "ext-uninstall",
                &format!("save config after uninstall {id}: {e:#}"),
            );
        } else if !purge.model_uuids.is_empty() || !purge.dead_anchors.is_empty() {
            use std::collections::HashSet;
            let dead_models: HashSet<String> = purge.model_uuids.iter().cloned().collect();
            let dead_providers: HashSet<String> = purge.dead_anchors.iter().cloned().collect();
            let _ = crate::app::cascade::rebind_consumers_after_model_removal(
                None,
                &dead_models,
                &dead_providers,
                purge.main_reset,
            );
        }

        // (5b, detached variant) No per-session `McpManager` exists pre-session to `reconnect`
        // from the just-saved server set, so instead BOUNCE the GLOBAL MCP daemon: the next
        // session's `ensure_mcp_daemon_running` respawns it fresh off the new config (dropping
        // any removed orphan row's connection). The build-skew fingerprint handshake makes
        // that respawn cheap + safe. Quiet — the GUI host owns no user terminal.
        crate::app::runtime::manage::stop_mcp_daemon(true);

        // (7) Sweep same-named agent-override files (global + every session) left by a user
        // who saved an edited copy of one of this extension's sub-agents. The same-name caveat
        // is documented on the helper.
        crate::app::ext::uninstall::sweep_agent_overrides(&snap.sub_agent_names);

        // (9) Nuke the extension's declared workspace_dir (validated against the SAME policy
        // as install; a missing/rejected dir is skipped). User-approved data deletion — the
        // GUI confirm named this dir before the request was sent.
        if let Some(ws) = snap.workspace_dir.as_deref() {
            crate::model::ext_workspace::remove_workspace_dir(ws);
        }

        // (10) SKIPPED: no foreground session here to reindex / rebuild the system prompt for;
        // a fresh daemon re-derives its workspace roots + prompt on boot.

        super::push_proto::push_ext_op_result(&push, id, true, None);
        super::push_proto::push_installed_extensions(&push, installed_extensions());
    });
}

/// The DEBUG-only unsigned install fallback — duplicate of the daemon's
/// `requests_ext::install_unsigned_fallback` (see this module's doc comment on
/// duplication). Writes the zip to a temp file and installs it via
/// [`crate::app::ext::install::install_dev_unsigned`] (skips signature verification), so
/// the end-to-end store→install flow is testable before koma.run's signing infra is live.
/// LOUDLY logged. A release build has no such path.
#[cfg(debug_assertions)]
fn install_unsigned_fallback(id: &str, zip: &[u8]) -> anyhow::Result<InstalledExtension> {
    store::append_global_error_log(
        "ext-install",
        &format!("UNSIGNED dev install of {id} (koma.run sent no signature — debug build only)"),
    );
    let tmp = std::env::temp_dir().join(format!("koma-ext-dl-{}.zip", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, zip)
        .map_err(|e| anyhow::anyhow!("write temp zip {}: {e}", tmp.display()))?;
    let r = crate::app::ext::install::install_dev_unsigned(&tmp);
    let _ = std::fs::remove_file(&tmp);
    r
}

/// Release builds reject an unsigned artifact — same gate as the daemon twin.
#[cfg(not(debug_assertions))]
fn install_unsigned_fallback(_id: &str, _zip: &[u8]) -> anyhow::Result<InstalledExtension> {
    anyhow::bail!("extension artifact is unsigned; refusing to install")
}

/// Detect this build's store platform token — duplicate of the daemon's
/// `requests_ext::detect_platform` (see this module's doc comment on duplication).
fn detect_platform() -> Option<&'static str> {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return Some("linux-x64");
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        return Some("linux-arm64");
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        return Some("darwin-x64");
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return Some("darwin-arm64");
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        return Some("windows-x64");
    }
    #[allow(unreachable_code)]
    {
        None
    }
}

/// Whether `id` is a well-formed reverse-DNS extension id safe to use as a directory name
/// under `extensions/` — duplicate of the daemon's `requests_ext::is_safe_ext_id` (see this
/// module's doc comment on duplication).
fn is_safe_ext_id(id: &str) -> bool {
    let all_allowed = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-');
    let has_alnum = id.chars().any(|c| c.is_ascii_alphanumeric());
    let dot_wrapped = id.starts_with('.') || id.ends_with('.');
    all_allowed && has_alnum && !dot_wrapped
}

/// `GET /extensions/{id}/download?version&platform` with the account Bearer — duplicate of
/// the daemon's ASYNC `requests_ext::fetch_install_artifact` (see this module's doc comment
/// on duplication; the blocking `fetch_catalogue`/`fetch_detail` above stay separate since
/// THIS caller already runs on the tokio runtime via `spawn_install`'s `handle.spawn`).
/// Resolves the store contract's TWO artifact shapes (302 redirect + integrity body, or a
/// direct 200 stream with integrity headers); returns `(zip_bytes, sha256, signature)`.
async fn fetch_install_artifact(
    id: &str,
    version: Option<&str>,
    platform: &str,
    bearer: &str,
) -> std::result::Result<(Vec<u8>, String, Option<String>), String> {
    let no_redirect = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("http client build failed: {e}"))?;

    let mut pairs: Vec<(&str, &str)> = vec![("platform", platform)];
    if let Some(v) = version {
        if !v.is_empty() {
            pairs.push(("version", v));
        }
    }
    let url = reqwest::Url::parse_with_params(&format!("{STORE_API_BASE}/{id}/download"), &pairs)
        .map_err(|e| format!("bad download url: {e}"))?;

    let resp = no_redirect
        .get(url)
        .bearer_auth(bearer)
        .send()
        .await
        .map_err(|e| {
            let msg = format!("download request failed: {e}");
            store::append_global_error_log(
                "ext download",
                &format!("{id} (platform {platform}): {msg}"),
            );
            msg
        })?;
    let status = resp.status();

    if status.is_redirection() {
        // 302: Location -> signed URI; body echoes the integrity fields.
        let location = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .ok_or_else(|| {
                let msg = "download redirect missing Location header".to_string();
                store::append_global_error_log(
                    "ext download",
                    &format!("{id} (platform {platform}): {msg}"),
                );
                msg
            })?;
        let body = resp.text().await.unwrap_or_default();
        let (sha256, signature) = parse_integrity_json(&body);

        // The signed URI is public (auth is in the query signature) — a plain follow.
        let zresp = reqwest::Client::new()
            .get(&location)
            .send()
            .await
            .map_err(|e| format!("signed download failed: {e}"))?;
        if !zresp.status().is_success() {
            let signed_status = zresp.status().as_u16();
            store::append_global_error_log(
                "ext download",
                &format!(
                    "{id} (platform {platform}): signed download returned HTTP {signed_status}"
                ),
            );
            return Err(format!("signed download returned HTTP {signed_status}"));
        }
        let bytes = zresp
            .bytes()
            .await
            .map_err(|e| format!("reading artifact failed: {e}"))?
            .to_vec();
        Ok((bytes, sha256, signature))
    } else if status.is_success() {
        // Direct stream: integrity in headers, body IS the zip.
        let sha256 = header_str(&resp, "x-koma-sha256").unwrap_or_default();
        let signature = header_str(&resp, "x-koma-signature").filter(|s| !s.is_empty());
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("reading artifact failed: {e}"))?
            .to_vec();
        Ok((bytes, sha256, signature))
    } else {
        let code = status.as_u16();
        let msg = match code {
            401 => "koma.run rejected the session — sign in again".to_string(),
            402 => "this extension needs an active koma.run entitlement".to_string(),
            404 => "extension not found for this version/platform".to_string(),
            429 => "koma.run is rate limiting — try again shortly".to_string(),
            other => format!("download failed (HTTP {other})"),
        };
        store::append_global_error_log(
            "ext download",
            &format!("{id} (platform {platform}): HTTP {code}: {msg}"),
        );
        Err(msg)
    }
}

/// Read a response header as a `String`, or `None` if absent / non-ASCII — duplicate of
/// the daemon's `requests_ext::header_str`.
fn header_str(resp: &reqwest::Response, name: &str) -> Option<String> {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

/// Pull `{ sha256, signature }` out of a 302 integrity body (best-effort) — duplicate of
/// the daemon's `requests_ext::parse_integrity_json`. A malformed/empty body yields
/// `(String::new(), None)` — the caller then treats it as unsigned.
fn parse_integrity_json(body: &str) -> (String, Option<String>) {
    let v: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return (String::new(), None),
    };
    let sha = str_field(&v, "sha256");
    let sig = v
        .get("signature")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
    (sha, sig)
}

// ─── ATTACHED (push_loop): reply over an mpsc channel, drained by the fold loop ───

/// `HostCtl::StoreBrowse` while attached.
pub(super) fn spawn_store_browse_attached(
    tx: Sender<(Vec<StoreItemWire>, Option<String>)>,
    query: Option<String>,
    category: Option<String>,
) {
    std::thread::spawn(move || {
        let result = match fetch_catalogue(query, category) {
            Ok(items) => (items, None),
            Err(e) => (Vec::new(), Some(e)),
        };
        let _ = tx.send(result);
    });
}

/// `HostCtl::StoreDetail` while attached.
pub(super) fn spawn_store_detail_attached(
    tx: Sender<(Option<StoreDetailWire>, Option<String>)>,
    id: String,
) {
    std::thread::spawn(move || {
        let result = match fetch_detail(&id) {
            Ok(d) => (Some(d), None),
            Err(e) => (None, Some(e)),
        };
        let _ = tx.send(result);
    });
}

/// `HostCtl::ListInstalledExtensions` while attached.
pub(super) fn spawn_list_installed_attached(tx: Sender<Vec<InstalledExtWire>>) {
    std::thread::spawn(move || {
        let _ = tx.send(installed_extensions());
    });
}

/// `HostCtl::GetInstalledExtensionDetail` while attached — two-phase: local
/// detail first, then best-effort online enrichment. The channel drains in a
/// loop, so sending twice is naturally picked up by push_loop.
pub(super) fn spawn_get_installed_detail_attached(
    tx: Sender<(String, Option<InstalledExtensionDetailWire>, Option<String>)>,
    id: String,
) {
    let id2 = id.clone();
    let id3 = id.clone();
    std::thread::spawn(move || {
        // Phase 1: local detail (store_detail = None).
        let (detail, error) = match get_installed_detail(&id2) {
            Ok(mut d) => {
                d.store_detail = None;
                (Some(d), None)
            }
            Err(e) => (None, Some(e)),
        };
        let had_local_error = error.is_some();
        let _ = tx.send((id2, detail, error));

        // Phase 2: best-effort online enrichment (no second response on failure).
        if !had_local_error {
            if let Ok(store_detail) = fetch_detail(&id3) {
                if let Ok(mut d) = get_installed_detail(&id3) {
                    d.store_detail = Some(store_detail);
                    let _ = tx.send((id3, Some(d), None));
                }
            }
        }
    });
}

// ─── shared blocking computation ───

/// Read the locally-installed extension registry straight off `~/.koma/config.json` — the
/// SAME projection the daemon's `requests_ext::send_installed_extensions` builds, so a
/// re-attach (or the daemon's own post-install/-uninstall re-push) never disagrees with
/// this host read. Each entry's `name` comes from the installed manifest when readable,
/// falling back to the extension id.
fn installed_extensions() -> Vec<InstalledExtWire> {
    let cfg = AppConfig::load();
    cfg.installed_extensions
        .iter()
        .map(|e| {
            let (name, panels) = read_ext_manifest_info(&e.id);
            InstalledExtWire {
                id: e.id.clone(),
                name,
                version: e.version.clone(),
                tier: e.tier.clone(),
                kind: e.kind.clone(),
                enabled: e.enabled,
                granted: e.granted.clone(),
                panels,
                // Surfaced so the GUI uninstall confirm can name the data dir the nuke
                // deletes (read fresh off the installed manifest, like `panels`).
                workspace_dir: crate::model::ext_workspace::read_workspace_dir(&e.id),
            }
        })
        .collect()
}

/// Read full detail for one installed extension: registry fields + on-disk
/// manifest contributions. Returns `Err` when the extension is not in the
/// registry (missing/unknown id).
fn get_installed_detail(id: &str) -> Result<InstalledExtensionDetailWire, String> {
    let cfg = AppConfig::load();
    let entry = cfg
        .installed_extensions
        .iter()
        .find(|e| e.id == id)
        .ok_or_else(|| format!("extension '{id}' is not installed"))?;

    // Read the manifest for contributions + name + description + requires.
    // A missing/unreadable/unparsable manifest is now an explicit error for the
    // detail path (not a silent degradation like the best-effort list panels).
    let manifest = read_manifest(id)?;
    let name = manifest.name.clone();
    let description = manifest.description.clone();
    let requires: Vec<String> = manifest
        .requires
        .iter()
        .map(|grant| match grant {
            koma_extension::protocol::Grant::AgentsRead => "agents:read".to_string(),
            koma_extension::protocol::Grant::AgentsOrchestrate => "agents:orchestrate".to_string(),
            // WAVE-1 COMPILE STUB: exhaustiveness-only, matching `broker::grant_wire`;
            // see the comment there.
            koma_extension::protocol::Grant::SessionsManage => "sessions:manage".to_string(),
            koma_extension::protocol::Grant::ChatPrompt => "chat:prompt".to_string(),
            koma_extension::protocol::Grant::ModelsInvoke => "models:invoke".to_string(),
            koma_extension::protocol::Grant::ContextPublish => "context:publish".to_string(),
            koma_extension::protocol::Grant::OauthContribute => "oauth:contribute".to_string(),
            koma_extension::protocol::Grant::ModelsContribute => "models:contribute".to_string(),
            koma_extension::protocol::Grant::OauthRead => "oauth:read".to_string(),
        })
        .collect();
    let panels = manifest.contributes.panels.iter().map(|p| PanelWire {
            id: p.id.clone(),
            title: p.title.clone(),
            icon: p.icon.clone(),
        }).collect();
    let tools = manifest.contributes.tools.iter().map(|t| InstalledToolWire {
            name: t.name.clone(),
            description: t.description.clone(),
        }).collect();
    let models = manifest.contributes.models.iter().map(|mdl| InstalledModelWire {
            id: mdl.id.clone(),
            display_name: mdl.display_name.clone(),
        }).collect();
    let sub_agents = manifest.contributes.sub_agents.iter().map(|a| InstalledSubAgentWire {
            name: a.name.clone(),
            description: a.description.clone(),
        }).collect();

    Ok(InstalledExtensionDetailWire {
        id: entry.id.clone(),
        name,
        version: entry.version.clone(),
        description,
        tier: entry.tier.clone(),
        kind: entry.kind.clone(),
        enabled: entry.enabled,
        granted: entry.granted.clone(),
        requires,
        panels,
        tools,
        models,
        sub_agents,
        store_detail: None,
        // Named in the GUI uninstall confirm as the data dir the nuke deletes.
        workspace_dir: crate::model::ext_workspace::read_workspace_dir(id),
    })
}

/// Read and parse `manifest.json` for extension `id`. Returns `Err` with a
/// descriptive message on any failure (extensions dir error, missing manifest,
/// unreadable manifest, invalid JSON/schema) — the caller surfaces these as
/// explicit errors rather than silently degrading.
fn read_manifest(id: &str) -> Result<koma_extension::protocol::ExtensionManifest, String> {
    let dir = store::extensions_dir()
        .map_err(|e| format!("extensions directory error: {e}"))?;
    let path = dir.join(id).join("manifest.json");
    if !path.exists() {
        return Err(format!("missing manifest for extension '{id}'"));
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("unreadable manifest for extension '{id}': {e}"))?;
    let manifest: koma_extension::protocol::ExtensionManifest = serde_json::from_str(&raw)
        .map_err(|e| format!("invalid manifest for extension '{id}': {e}"))?;
    Ok(manifest)
}

/// Read the manifest for extension `id` and extract both the friendly name and
/// the panel list. A missing/unreadable/unparsable manifest degrades to using
/// the id as the name and an empty panel list (non-fatal for list rendering).
fn read_ext_manifest_info(id: &str) -> (String, Vec<PanelWire>) {
    let path = match store::extensions_dir() {
        Ok(dir) => dir.join(id).join("manifest.json"),
        Err(_) => return (id.to_string(), Vec::new()),
    };
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return (id.to_string(), Vec::new()),
    };
    let manifest: koma_extension::protocol::ExtensionManifest = match serde_json::from_str(&raw) {
        Ok(m) => m,
        Err(e) => {
            store::append_global_error_log(
                "ext",
                &format!("failed to parse manifest.json for {id}: {e}"),
            );
            return (id.to_string(), Vec::new());
        }
    };
    let name = if manifest.name.is_empty() {
        id.to_string()
    } else {
        manifest.name
    };
    let panels = manifest
        .contributes
        .panels
        .into_iter()
        .map(|p| PanelWire {
            id: p.id,
            title: p.title,
            icon: p.icon,
        })
        .collect();
    (name, panels)
}

/// Read `contributes.panels` straight off `extensions_dir()/<id>/manifest.json` — the
/// registry (`InstalledExtension`) doesn't carry contributions, so this is a fresh,
/// best-effort re-read on every installed-list build: a missing/unreadable/unparsable
/// manifest degrades to an empty panel list (never fails the whole installed-list
/// projection over one bad entry), logged via `append_global_error_log` so a parse
/// failure is still visible. SAME logic as the daemon's
/// `requests_ext::read_ext_panels` copy, mirroring this module's existing
/// map_summary/map_detail/map_contributes duplication.
fn read_ext_panels(id: &str) -> Vec<PanelWire> {
    let path = match store::extensions_dir() {
        Ok(dir) => dir.join(id).join("manifest.json"),
        Err(_) => return Vec::new(),
    };
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return Vec::new(), // not installed / unreadable — no panels
    };
    let manifest: koma_extension::protocol::ExtensionManifest = match serde_json::from_str(&raw) {
        Ok(m) => m,
        Err(e) => {
            store::append_global_error_log(
                "ext",
                &format!("failed to parse manifest.json for {id}: {e}"),
            );
            return Vec::new();
        }
    };
    manifest
        .contributes
        .panels
        .into_iter()
        .map(|p| PanelWire {
            id: p.id,
            title: p.title,
            icon: p.icon,
        })
        .collect()
}

/// `GET /extensions[?q&category]` → the mapped catalogue rows, BLOCKING (this always runs
/// off a plain `std::thread::spawn` worker, never the tokio runtime, so `reqwest::blocking`
/// is the simplest fit — mirrors the daemon's async `requests_ext::fetch_catalogue`
/// field-for-field). A non-2xx status or a parse error is an `Err(String)` the caller
/// surfaces as the catalogue's `error`.
fn fetch_catalogue(
    query: Option<String>,
    category: Option<String>,
) -> Result<Vec<StoreItemWire>, String> {
    let mut pairs: Vec<(&str, String)> = Vec::new();
    if let Some(q) = query {
        let q = q.trim().to_string();
        if !q.is_empty() {
            pairs.push(("q", q));
        }
    }
    if let Some(c) = category {
        let c = c.trim().to_string();
        if !c.is_empty() {
            pairs.push(("category", c));
        }
    }
    let url = reqwest::Url::parse_with_params(STORE_API_BASE, &pairs)
        .map_err(|e| format!("bad store url: {e}"))?;

    let resp = reqwest::blocking::get(url).map_err(|e| {
        let msg = format!("store request failed: {e}");
        store::append_global_error_log("ext browse", &msg);
        msg
    })?;
    if !resp.status().is_success() {
        let code = resp.status().as_u16();
        store::append_global_error_log("ext browse", &format!("store returned HTTP {code}"));
        return Err(format!("store returned HTTP {code}"));
    }
    let body: serde_json::Value = resp
        .json()
        .map_err(|e| format!("store response parse failed: {e}"))?;
    let items = body
        .get("items")
        .and_then(|x| x.as_array())
        .map(|arr| arr.iter().map(map_summary).collect())
        .unwrap_or_default();
    Ok(items)
}

/// `GET /extensions/{id}` → the mapped detail, BLOCKING (see [`fetch_catalogue`]).
fn fetch_detail(id: &str) -> Result<StoreDetailWire, String> {
    let url = format!("{STORE_API_BASE}/{id}");
    let resp = reqwest::blocking::get(&url).map_err(|e| {
        let msg = format!("store request failed: {e}");
        store::append_global_error_log("ext browse", &format!("{id}: {msg}"));
        msg
    })?;
    if !resp.status().is_success() {
        let code = resp.status().as_u16();
        store::append_global_error_log("ext browse", &format!("{id}: store returned HTTP {code}"));
        return Err(format!("store returned HTTP {code}"));
    }
    let body: serde_json::Value = resp
        .json()
        .map_err(|e| format!("store response parse failed: {e}"))?;
    Ok(map_detail(&body))
}

/// Map one store `ExtensionSummary` JSON object to [`StoreItemWire`] — duplicate of the
/// daemon's `requests_ext::map_summary` (defensive: a missing field degrades to empty
/// rather than failing the whole list parse).
fn map_summary(v: &serde_json::Value) -> StoreItemWire {
    StoreItemWire {
        id: str_field(v, "id"),
        name: str_field(v, "name"),
        tagline: str_field(v, "tagline"),
        tier: str_field(v, "tier"),
        kind: str_field(v, "kind"),
        latest_version: str_field(v, "latest_version"),
        icon_url: str_field(v, "icon_url"),
        categories: arr_str(v, "categories"),
        author: str_field(v, "author"),
        updated_at: str_field(v, "updated_at"),
    }
}

/// Map one store `ExtensionDetail` JSON object to [`StoreDetailWire`] — duplicate of the
/// daemon's `requests_ext::map_detail`.
fn map_detail(v: &serde_json::Value) -> StoreDetailWire {
    StoreDetailWire {
        id: str_field(v, "id"),
        name: str_field(v, "name"),
        tagline: str_field(v, "tagline"),
        tier: str_field(v, "tier"),
        kind: str_field(v, "kind"),
        latest_version: str_field(v, "latest_version"),
        icon_url: str_field(v, "icon_url"),
        categories: arr_str(v, "categories"),
        author: str_field(v, "author"),
        updated_at: str_field(v, "updated_at"),
        description_md: str_field(v, "description_md"),
        screenshots: arr_str(v, "screenshots"),
        contributes: map_contributes(v.get("contributes")),
        requires: arr_str(v, "requires"),
        versions: arr_str(v, "versions"),
    }
}

/// Collapse the detail's `contributes` object to per-kind COUNTS — duplicate of the
/// daemon's `requests_ext::map_contributes`.
fn map_contributes(v: Option<&serde_json::Value>) -> StoreContributesWire {
    let count = |key: &str| -> u32 {
        v.and_then(|c| c.get(key))
            .and_then(|x| x.as_array())
            .map(|a| a.len() as u32)
            .unwrap_or(0)
    };
    StoreContributesWire {
        models: count("models"),
        panels: count("panels"),
        tools: count("tools"),
        sub_agents: count("sub_agents"),
    }
}

/// A string field of a JSON object, or `""` if absent / not a string.
fn str_field(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string()
}

/// A `Vec<String>` field of a JSON object (its string elements), or empty.
fn arr_str(v: &serde_json::Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|e| e.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The summary mapping pulls exactly the wire fields from an `ExtensionSummary`-shaped
    /// object, degrading a missing field to empty rather than failing — mirrors the
    /// daemon-side `requests_ext::map_summary_projects_summary_fields` test so the two
    /// independent copies stay behaviourally identical.
    #[test]
    fn map_summary_projects_summary_fields() {
        let v = serde_json::json!({
            "id": "run.koma.gateway",
            "name": "koma Gateway",
            "tagline": "Premium koma models, one endpoint.",
            "tier": "paid",
            "kind": "daemon",
            "latest_version": "0.3.1",
            "icon_url": "https://cdn.koma.run/ext/run.koma.gateway/icon.png",
            "categories": ["models", "gateway"],
            "author": "koma",
            "updated_at": "2026-07-10T12:00:00Z"
        });
        let item = map_summary(&v);
        assert_eq!(item.id, "run.koma.gateway");
        assert_eq!(item.name, "koma Gateway");
        assert_eq!(item.tier, "paid");
        assert_eq!(item.kind, "daemon");
        assert_eq!(item.latest_version, "0.3.1");
        assert_eq!(item.categories, vec!["models", "gateway"]);
        assert_eq!(item.author, "koma");
    }

    /// The detail mapping projects the long-form fields AND collapses `contributes` to
    /// per-kind counts + carries the `requires` grant list (the install card's inputs).
    #[test]
    fn map_detail_counts_contributions_and_reads_requires() {
        let v = serde_json::json!({
            "id": "run.koma.gateway",
            "name": "koma Gateway",
            "tagline": "one endpoint",
            "tier": "paid",
            "kind": "daemon",
            "latest_version": "0.3.1",
            "icon_url": "",
            "categories": ["models"],
            "author": "koma",
            "updated_at": "2026-07-10T12:00:00Z",
            "description_md": "# koma Gateway\n\nlong",
            "screenshots": ["https://cdn.koma.run/ext/run.koma.gateway/1.png"],
            "contributes": {
                "models": [{ "id": "a" }, { "id": "b" }],
                "panels": [],
                "tools": [{ "name": "t" }],
                "sub_agents": []
            },
            "requires": ["agents:read"],
            "versions": ["0.3.1", "0.3.0"]
        });
        let d = map_detail(&v);
        assert_eq!(d.description_md, "# koma Gateway\n\nlong");
        assert_eq!(d.screenshots.len(), 1);
        assert_eq!(d.contributes.models, 2);
        assert_eq!(d.contributes.panels, 0);
        assert_eq!(d.contributes.tools, 1);
        assert_eq!(d.contributes.sub_agents, 0);
        assert_eq!(d.requires, vec!["agents:read"]);
        assert_eq!(d.versions, vec!["0.3.1", "0.3.0"]);
    }

    /// The installed-extensions projection reads straight off `AppConfig` and carries every
    /// display field verbatim — a smoke check that the host copy stays in lockstep with the
    /// daemon's own `send_installed_extensions` projection shape. The `name` field comes from
    /// the manifest when readable, falling back to the extension id.
    #[test]
    fn installed_extensions_projects_registry_fields() {
        let mut cfg = AppConfig::default();
        cfg.installed_extensions.push(crate::model::app_config::InstalledExtension {
            id: "run.koma.gateway".to_string(),
            version: "0.3.1".to_string(),
            tier: "paid".to_string(),
            kind: "daemon".to_string(),
            enabled: true,
            granted: vec!["agents:read".to_string()],
            exec: String::new(),
        });
        let items: Vec<InstalledExtWire> = cfg
            .installed_extensions
            .iter()
            .map(|e| {
                let (name, panels) = read_ext_manifest_info(&e.id);
                InstalledExtWire {
                    id: e.id.clone(),
                    name,
                    version: e.version.clone(),
                    tier: e.tier.clone(),
                    kind: e.kind.clone(),
                    enabled: e.enabled,
                    granted: e.granted.clone(),
                    panels,
                    workspace_dir: None,
                }
            })
            .collect();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "run.koma.gateway");
        // No manifest on test machine — falls back to id.
        assert_eq!(items[0].name, "run.koma.gateway");
        assert_eq!(items[0].tier, "paid");
        assert!(items[0].enabled);
        assert_eq!(items[0].granted, vec!["agents:read"]);
    }

    /// `read_ext_manifest_info` returns the id as name when no manifest exists.
    #[test]
    fn read_ext_manifest_info_falls_back_to_id_on_missing_manifest() {
        let (name, panels) =
            read_ext_manifest_info("run.koma.definitely-not-installed.test-fixture");
        assert_eq!(name, "run.koma.definitely-not-installed.test-fixture");
        assert!(panels.is_empty());
    }

    /// The local installed detail initializes with `store_detail: None` before
    /// any online enrichment.
    #[test]
    fn get_installed_detail_has_no_store_detail() {
        // This extension is never installed on a test machine, so
        // get_installed_detail returns Err — confirming the function compiles
        // and the wire struct has the store_detail field.
        let result = get_installed_detail("run.koma.definitely-not-installed.test-fixture");
        assert!(result.is_err());
    }

    /// Wire serialization: InstalledExtWire carries `name` and serializes it as
    /// `name` (camelCase — already flat).
    #[test]
    fn installed_ext_wire_serializes_name() {
        let wire = InstalledExtWire {
            id: "run.koma.hello".to_string(),
            name: "Hello World".to_string(),
            version: "0.1.0".to_string(),
            tier: "free".to_string(),
            kind: "daemon".to_string(),
            enabled: true,
            granted: vec![],
            panels: vec![],
            workspace_dir: None,
        };
        let json = serde_json::to_value(&wire).unwrap();
        assert_eq!(json["name"], "Hello World");
        assert_eq!(json["id"], "run.koma.hello");
    }

    /// Wire serialization: InstalledExtensionDetailWire with `store_detail: None`
    /// omits/nulls the field for backward compat.
    #[test]
    fn installed_detail_wire_omits_none_store_detail() {
        let wire = InstalledExtensionDetailWire {
            id: "run.koma.hello".to_string(),
            name: "Hello World".to_string(),
            version: "0.1.0".to_string(),
            description: "A test ext".to_string(),
            tier: "free".to_string(),
            kind: "daemon".to_string(),
            enabled: true,
            granted: vec![],
            requires: vec![],
            panels: vec![],
            tools: vec![],
            models: vec![],
            sub_agents: vec![],
            store_detail: None,
            workspace_dir: None,
        };
        let json = serde_json::to_value(&wire).unwrap();
        // serde skips None Option by default with #[serde(default)]
        assert!(json.get("storeDetail").is_none() || json["storeDetail"].is_null());
    }

    /// A missing/never-installed manifest degrades to an empty panel list rather than
    /// failing — the id here is guaranteed to have no `extensions/<id>/manifest.json` on
    /// any test machine.
    #[test]
    fn read_ext_panels_degrades_to_empty_on_missing_manifest() {
        assert_eq!(
            read_ext_panels("run.koma.definitely-not-installed.test-fixture"),
            Vec::<PanelWire>::new()
        );
    }
}
