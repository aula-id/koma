// The `koma --daemon-selftest` end-to-end harness (`run_daemon_selftest` +
// its fallible body) lives in the sibling `selftest` module (file size);
// re-exported here so `crate::app::runtime::lifecycle::run_daemon_selftest`
// keeps resolving unchanged for the `app::runtime` / `crate::app` re-export
// chain above it.
mod selftest;
pub use selftest::run_daemon_selftest;

use std::io::stdout;
use std::sync::Arc;

use anyhow::Result;
use ratatui::{backend::CrosstermBackend, Terminal};

use crate::app::mode::{KeyInputForm, Mode, OnboardState, PickerState};
use crate::app::resolve::resolve_role;
use crate::app::state::{AppState, SessionRuntime};
use crate::config::DEFAULT_MODEL;
use crate::model::app_config::ModelRole;
use crate::model::session::Session;
use crate::model::session_registry;
use crate::model::{app_config::AppConfig, settings::Settings, store};
use crate::service::openrouter::OpenRouterClient;

use super::event_loop::daemon::{daemon_loop, DaemonHub};
use super::event_loop::run_loop;
use super::session_mgmt::{build_client, warm_session};
use super::signals::install_daemon_signals;
use super::terminal::TerminalGuard;

/// Best-effort prefill of (api_key, model, provider) from the most-recently-modified
/// session that has a non-empty key. Ignores all errors.
pub(super) fn prefill_creds() -> (Option<String>, Option<String>, Option<String>) {
    let metas = match store::list_sessions() {
        Ok(m) => m,
        Err(_) => return (None, None, None),
    };
    let Some(meta) = metas.into_iter().next() else {
        return (None, None, None);
    };
    let settings = match Settings::load(&meta.path.join("settings.json")) {
        Ok(s) => s,
        Err(_) => return (None, None, None),
    };
    if settings.api_key.is_empty() {
        (None, None, None)
    } else {
        (
            Some(settings.api_key),
            Some(settings.model),
            Some(settings.provider),
        )
    }
}

/// The shared startup prefix for BOTH the interactive TUI ([`run`]) and the
/// headless daemon ([`daemon_run`]).
///
/// Does everything that is independent of the terminal: ensure the config dirs
/// exist, build the tokio runtime + clone its handle, decide the initial
/// [`AppState`] (resume picker / returning-user chat / first-run wizard), load the
/// global config, capture the launch cwd for the harness workspace check, build
/// the keyless per-session client when a usable Main route resolves, and warm the
/// active session (workspace reindex + awareness). Returns the owned runtime (kept
/// alive + dropped LAST by the caller), its handle, the constructed state, and the
/// optional client (the no-client-no-send gate).
///
/// SAFE FOR HEADLESS USE: nothing here touches stdout / the terminal. `warm_session`
/// only spawns background tasks + mutates state + does best-effort lock IO, so the
/// daemon path can call this identically to the TUI path.
fn build_startup(
    opts: &crate::cli::Opts,
) -> Result<(
    tokio::runtime::Runtime,
    tokio::runtime::Handle,
    AppState,
    Option<Arc<OpenRouterClient>>,
)> {
    store::ensure_dirs()?;

    let rt = tokio::runtime::Runtime::new()?;
    let handle = rt.handle().clone();

    // Load the global config UP FRONT — before the first-run decision below — so the
    // gate can ask the real question ("does the user have a usable Main route?")
    // against the global provider/model catalogue, not just the legacy
    // `settings.api_key` field. `ensure_dirs` has already run (so the dir exists if we
    // later persist config.json); `AppConfig::load()` is a pure read and falls back to
    // `AppConfig::default()` on any error. Stashed into `state.rest.config` below at the
    // point the old code loaded it (so the MCP wiring that reads it is unchanged).
    let config = AppConfig::load();

    // Decide initial state.
    let mut state = if opts.daemon {
        // Daemon-per-session: `install_daemon_session` (called right after build_startup
        // in run_daemon) owns create/load for this daemon's keyed session id. Do NOT
        // create a throwaway returning-user session here (install would orphan it every
        // launch) and do NOT run the resume picker / `list_sessions` scan — just stash
        // last-used creds on a sessionless placeholder; install_daemon_session sets the
        // real session + mode before the first tick, so the placeholder mode is moot.
        let (lk, lm, lp) = prefill_creds();
        let mut state = AppState::new(Mode::Chat);
        state.rest.last_key = lk;
        state.rest.last_model = lm;
        state.rest.last_provider = lp;
        state
    } else if opts.resume {
        let metas = store::list_sessions()?;
        let (lk, lm, lp) = prefill_creds();
        let mut state = AppState::new(Mode::SessionPicker(PickerState::new(metas)));
        state.rest.last_key = lk;
        state.rest.last_model = lm;
        state.rest.last_provider = lp;
        state
    } else {
        let (lk, lm, lp) = prefill_creds();
        // Route to onboarding when Main doesn't resolve to a usable route. Empty config
        // → legacy fallback with no key is not usable → onboard. A configured/usable Main
        // skips the chooser. Probe with a Settings reflecting the prefilled legacy creds
        // so resolve_role can evaluate the route against them.
        let probe = Settings {
            api_key: lk.clone().unwrap_or_default(),
            model: lm.clone().unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            provider: lp.clone().unwrap_or_default(),
            ..Default::default()
        };
        let unconfigured =
            resolve_role(&config, &probe, ModelRole::Main).is_none_or(|r| !r.is_usable());
        let mut state = if !unconfigured {
            // Returning user: spawn a fresh session pre-loaded with the last
            // creds and drop straight into chat. The credential prompt only
            // appears on the very first run. Per-session changes via /settings.
            let mut st = AppState::new(Mode::Chat);
            match store::create_session() {
                Ok(mut sess) => {
                    sess.settings.api_key = lk.clone().unwrap_or_default();
                    sess.settings.model = lm.clone().unwrap_or_else(|| DEFAULT_MODEL.to_string());
                    sess.settings.provider = lp.clone().unwrap_or_default();
                    let _ = sess.save();
                    let sess_path = sess.path.clone();
                    st.rest.fg_mut().session = Some(sess);
                    // Fresh startup session → seed ITS OWN counters (0 here, since a
                    // brand-new session has no ledger yet); harmless and explicit.
                    let fg = st.rest.foreground;
                    st.rest.load_token_totals(fg, &sess_path);
                    // Every session spawn kicks a fresh NON-BLOCKING version check;
                    // the result lands in `latest_version` when (if) it succeeds.
                    if let Some(tx) = st.rest.version_tx.as_ref() {
                        crate::app::version::spawn_check(tx.clone());
                    }
                }
                Err(e) => {
                    // Couldn't create the session dir — fall back to the prompt.
                    // Per-session status (C6); startup has the single foreground session.
                    st.rest.fg_mut().status = format!("error: {e}");
                    *st.mode_mut() = Mode::KeyInput(KeyInputForm::prefilled(
                        lk.clone().unwrap_or_default(),
                        lm.clone().unwrap_or_else(|| DEFAULT_MODEL.to_string()),
                        true,
                        false,
                    ));
                }
            }
            st
        } else {
            // First ever run on this machine: show the connection CHOOSER (koma free /
            // provider / custom) — lazy, no session dir is created until the user picks
            // a path. Each choice routes to its own setup action (see `Mode::Onboard`).
            AppState::new(Mode::Onboard(Box::new(OnboardState { cursor: 0 })))
        };
        state.rest.last_key = lk;
        state.rest.last_model = lm;
        state.rest.last_provider = lp;
        state
    };

    // Stash the global config loaded up front (before the first-run gate). Moved here
    // — at the original load point — so the MCP wiring below that reads
    // `state.rest.config.mcp_servers` is unchanged. No second AppConfig::load().
    state.rest.config = config;

    // Build the MCP client manager from the configured servers and stash it in
    // AppStateRest (cloned into every ToolCtx so `mcp__*` calls can dispatch).
    //
    // In DAEMON mode this is left `None` here: `run_daemon` sets it up right after,
    // PROXYING to the global MCP daemon (with a local fallback) so N session-daemons
    // share one copy of every heavyweight server instead of each spawning their own.
    // Building a LOCAL manager here too would spawn a duplicate set that the proxy
    // then supersedes — so skip it for `--daemon` and let `run_daemon` decide.
    //
    // In the STANDALONE/`--local` (and returning-user TUI) path there is no global
    // daemon, so build the LOCAL manager exactly as before. NON-BLOCKING: `connect_all`
    // returns immediately and connects each enabled server in a background task; tools
    // appear once a server is ready. With no `mcp_servers` configured this spawns
    // nothing and advertises no tools — identical to a build without MCP.
    if opts.daemon {
        state.rest.mcp_manager = None;
    } else {
        state.rest.mcp_manager = Some(crate::app::mcp::McpManager::connect_all(
            &handle,
            &state.rest.config.mcp_servers,
        ));
    }

    // Build the security-daemon client. Mint a per-process token and, if the
    // daemon is installed, auto-start it (M1: gated only on install; a later
    // milestone adds the /security enabled-toggle gate). Non-blocking.
    let sec_token = uuid::Uuid::new_v4().to_string();
    let sec = crate::app::sec::SecDaemonManager::new(&handle);
    // Auto-start is gated on BOTH install and the runtime enable flag. The flag
    // starts `false` so the daemon stays off by default; the `/security` panel's
    // toggle key (`t`) sets it and calls `.start()` explicitly.
    if crate::security::is_installed() && state.rest.security_enabled {
        sec.start(sec_token.clone());
    }
    state.rest.sec_token = sec_token;
    state.rest.sec_manager = Some(sec);

    // Build the extension host manager and auto-start every ENABLED daemon-kind
    // extension recorded in the config registry. Best-effort: each start is offloaded
    // onto the blocking pool (`ensure_started` blocks on its handshake, so running it on
    // the main thread would stall a slow/hung extension into boot) and any failure is
    // logged to `~/.koma/error.log` — never eprintln/println (this is TUI-owning code).
    // With no installed extensions this builds an empty, inert manager and spawns
    // nothing — byte-identical to a build without the extension host.
    let ext = crate::app::ext::ExtHostManager::new(&handle);
    // Wire the grant-broker lane BEFORE any reader task is spawned below, so a
    // fast-connecting extension's first `agents.*` `Call` can reach the event loop
    // (a call arriving before this is set is answered "grant broker not initialized"
    // rather than hanging). The sender is cloned; the receiver stays on `AppStateRest`
    // for `service_global`'s `drain_ext_calls`.
    ext.set_ext_call_tx(state.rest.ext_call_tx.clone());
    // Same reasoning for the notify lane: wired before any reader task is spawned so
    // a fast-connecting extension's first `Notify` can reach the event loop rather
    // than being silently dropped by `ext_notify_tx()` still reading `None`.
    ext.set_ext_notify_tx(state.rest.ext_notify_tx.clone());
    // Captured for the registration hook below: `Option<Arc<McpManager>>` is
    // `None` in `--daemon` mode at this point (`run_daemon` builds its — possibly
    // `Proxy` — manager AFTER `build_startup` returns), so extension tools are
    // simply not registered for that process yet; see
    // `app::ext::register::register_contributions`'s docs for the "later wave"
    // note on routing extension tools through the global MCP daemon's proxy wire.
    let mcp_for_ext = state.rest.mcp_manager.clone();
    for installed in &state.rest.config.installed_extensions {
        if installed.enabled && installed.kind == "daemon" {
            let mgr = Arc::clone(&ext);
            let installed = installed.clone();
            let mcp_for_ext = mcp_for_ext.clone();
            handle.spawn_blocking(move || {
                match mgr.ensure_started(&installed) {
                    Ok(()) => {
                        // Wire `contributes.tools` (extension-owned MCP) now that the
                        // daemon is live. `contributes.sub_agents` needs no action
                        // here — `AgentRegistry::load` picks it up on its own; see
                        // `register_contributions`'s docs. A future install/enable
                        // command handler should call this too.
                        if let Err(e) = crate::app::ext::register::register_contributions(
                            &installed,
                            mcp_for_ext.as_ref(),
                            &mgr,
                        ) {
                            crate::model::store::append_global_error_log(
                                "extensions",
                                &format!(
                                    "failed to register contributions for '{}': {e:#}",
                                    installed.id
                                ),
                            );
                        }
                    }
                    Err(e) => {
                        crate::model::store::append_global_error_log(
                            "extensions",
                            &format!("failed to start extension '{}': {e:#}", installed.id),
                        );
                    }
                }
            });
        }
    }
    state.rest.ext_manager = Some(ext);

    // Widen the active session's workspace roots with every ENABLED extension's declared
    // `workspace_dir` (validated + created), so agent writes into an extension's state dir
    // pass the harness. In `--daemon` mode there is NO session here yet (install_daemon_session
    // sets it and re-runs this same injection), so this only fires for the TUI returning-user
    // path. In-memory only (no save): the roots are re-derived from the CURRENT enabled set on
    // every boot, and `warm_session` below reindexes the dir cache over the widened roots.
    if state.rest.fg().session.is_some() {
        let installed = state.rest.config.installed_extensions.clone();
        if let Some(sess) = state.rest.fg_mut().session.as_mut() {
            crate::model::ext_workspace::inject_extension_workspaces(
                &installed,
                &mut sess.settings.workdir,
            );
        }
    }

    // Capture the process launch directory for the harness workspace check (WC).
    // This folder is always an allowed workspace regardless of the allow-list.
    if let Ok(cwd) = std::env::current_dir() {
        state.rest.launch_dir = cwd;
    }

    // If startup opened a session straight into chat (returning user), build its
    // client now; otherwise it's built when the user confirms credentials. The
    // None-gate is the "is there a usable session/key?" signal the whole runtime
    // relies on. Because the key now lives in config/settings (read per-call), the
    // condition is whether the MAIN role resolves to a usable route (a route with a
    // non-empty api_key) — NOT the old `!settings.api_key.is_empty()`. The client
    // itself is keyless; the gate just preserves the no-client-no-send invariant.
    let client: Option<Arc<OpenRouterClient>> = state
        .rest
        .fg()
        .session
        .as_ref()
        .filter(|s| {
            resolve_role(&state.rest.config, &s.settings, ModelRole::Main)
                .is_some_and(|r| r.is_usable())
        })
        .map(|_| build_client());

    // Warm the session (reindex workspace + compute awareness summary) so a
    // cold launch is fully primed before the first keystroke. Picker / first-run
    // paths have no session yet; warm_session is a no-op for them and fires
    // later when a session becomes active (picker-select / creds-confirm / /new).
    warm_session(&mut state, &client, &handle);

    Ok((rt, handle, state, client))
}

/// Create-or-LOAD the session `<id>` and install it as the daemon's SINGLE foreground
/// session (daemon-per-session). Called once at daemon startup, AFTER [`build_startup`]
/// and BEFORE the accept/daemon loops.
///
/// The daemon owns exactly one session — the one keyed to its socket. The client minted
/// `session_id` and passed it via `--session`, so:
/// - if the registry already has `session_id` → [`Session::load`] it from disk (resume,
///   exercised by a LATER commit);
/// - else → create a NEW session WITH THAT id via [`store::create_session_in_with_id`],
///   rooted at the daemon's current working dir (it inherited the client's cwd at spawn,
///   so `current_dir()` is the right pwd bucket). At THIS commit the minted id is always
///   new, so only the create branch runs.
///
/// Construction MIRRORS the Attach-create path `create_session_for_pwd` (inherit
/// last-used creds for a fresh session, acquire the session's lock into a fresh
/// [`SessionRuntime`], reset the flat foreground UI, seed token counters, then either
/// open KeyInput when no usable Main route resolves or land in Chat + warm) — with ONE
/// structural difference: it REPLACES the single foreground slot (`sessions[0]`) instead
/// of appending a tab, because the daemon serves one session, not a multiplexed set. Any
/// lock `build_startup` already grabbed for its placeholder/returning-user session is
/// released first so we never strand a stale lock.
///
/// Best-effort and infallible at the type level: a create/load error degrades to a
/// status line + KeyInput rather than aborting daemon startup, so a bad session can
/// never wedge the daemon before it can even report the problem to a client.
fn install_daemon_session(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
    session_id: &str,
) {
    // Release any lock build_startup acquired (its placeholder / returning-user session);
    // we are about to overwrite the foreground slot with OUR keyed session.
    if let Some(old) = state.rest.fg_mut().held_lock.take() {
        store::remove_lock(&old);
    }

    // NOTE: for a returning user, `build_startup` already minted a throwaway session
    // (its `create_session()`), which this replace orphans on disk/registry. That extra
    // empty session is a pre-existing wart (the global daemon's `build_startup` seeded one
    // every launch too) and is harmless; the multiplexing-rip-out commit removes the
    // shared `build_startup` seeding, so it is intentionally left as-is here.

    // --- create-or-load resolution (quote-worthy: the create-vs-load oracle) ---
    // The registry is the source of truth for "does this session already exist?". A
    // present row → load from its on-disk dir; an absent row (or any registry error) →
    // create fresh with this exact id, rooted at the daemon's cwd.
    let loaded: Result<Session> = match session_registry::get(session_id) {
        Ok(Some(row)) => {
            store::session_dir(&row.pwd_hash, session_id).and_then(|dir| Session::load(&dir))
        }
        _ => {
            let workdir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            store::create_session_in_with_id(&workdir, session_id)
        }
    };

    let mut sess = match loaded {
        Ok(s) => s,
        Err(e) => {
            // Couldn't build the session — surface it on the (placeholder) foreground and
            // drop into KeyInput so the client still reaches a usable screen.
            state.rest.fg_mut().status = format!("error: could not open session: {e}");
            *client = None;
            *state.mode_mut() = Mode::KeyInput(KeyInputForm::prefilled(
                state.rest.last_key.clone().unwrap_or_default(),
                state
                    .rest
                    .last_model
                    .clone()
                    .unwrap_or_else(|| DEFAULT_MODEL.to_string()),
                true,  // first_run framing
                false, // not from picker
            ));
            return;
        }
    };

    // A FRESH session inherits the last-used creds (so it drops straight into chat, same
    // as `/new` and the attach-create path); a LOADED session already carries its own
    // persisted creds, so only fill blanks from last-used as a convenience.
    if sess.settings.api_key.is_empty() {
        sess.settings.api_key = state.rest.last_key.clone().unwrap_or_default();
    }
    if sess.settings.provider.is_empty() {
        sess.settings.provider = state.rest.last_provider.clone().unwrap_or_default();
    }
    if sess.settings.model.is_empty() {
        sess.settings.model = state
            .rest
            .last_model
            .clone()
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());
    }
    let _ = sess.save();

    // Acquire THIS session's lock into a fresh runtime keyed by the SAME uuid as the
    // socket — so the SessionRuntime id, the session id, and the socket key all agree.
    store::write_lock(&sess.path);
    let mut runtime = SessionRuntime::new();
    runtime.id = session_id.to_string();
    runtime.held_lock = Some(sess.path.clone());

    // Onboard when Main resolves but is not usable (or doesn't resolve). Computed
    // before `sess` moves in.
    let unconfigured = resolve_role(&state.rest.config, &sess.settings, ModelRole::Main)
        .is_none_or(|r| !r.is_usable());
    let sess_path = sess.path.clone();
    runtime.session = Some(sess);

    // Wave-5 restore: rehydrate this session's persisted per-session records so
    // the GUI Explore sidepanel survives close/reopen. The cumulative file-change
    // log (#24) is read straight into the in-memory mirror; the bg-bash + sub-agent
    // records (#25) are restored INERT — the live workers died with the previous
    // daemon, so a record that was still "running" at close comes back settled-stale,
    // never running. All best-effort (empty when the session has no such records).
    runtime.file_changes = crate::model::msglog::read_file_changes(&sess_path);
    // Rehydrate the session's CURRENT todo checklist — mirrors `file_changes`'s
    // load-time refresh so a reattached/resumed session's GUI Explore "PLAN"
    // section is correct from the very first snapshot, not just after the next
    // checklist/plan_ready. Mode-aware: `plan_todos.md` while in Plan mode, else
    // the per-directory `memory/TODO.md` (the regular working list `checklist`
    // writes to outside Plan mode) — same selection `/todo` itself uses, so the
    // GUI shows execution-phase todos too, not just mid-plan ones.
    runtime.plan_todos = runtime
        .session
        .as_ref()
        .map(|s| {
            crate::app::mode::todo::load_current_todos(
                s,
                state.rest.agent_mode == crate::app::state::AgentMode::Plan,
            )
        })
        .unwrap_or_default();
    super::bg_persist::restore_bg_records(&mut runtime, &sess_path, handle);

    // Install as the SINGLE foreground session (replace the slot; never append).
    state.rest.sessions = vec![runtime];
    state.rest.foreground = 0;

    // Clean slate for the flat foreground UI (mirror create_session_for_pwd / /new).
    {
        let fg = state.rest.fg_mut();
        fg.input.clear();
        fg.cursor = 0;
        fg.pending_attachments.clear();
    }
    state.rest.reset_scroll();
    state.rest.transcript_cache.borrow_mut().blocks.clear();
    state.rest.fg_mut().status = "ready".into();

    // Seed this session's cumulative token counters from its own (possibly empty) ledger.
    state.rest.load_token_totals(0, &sess_path);

    // Widen this daemon session's workspace roots with enabled extensions' `workspace_dir`s
    // (see `build_startup` — this is the daemon-path equivalent, where the session finally
    // exists). Done BEFORE `warm_session` so its reindex covers the new roots. In-memory only.
    {
        let installed = state.rest.config.installed_extensions.clone();
        if let Some(sess) = state.rest.fg_mut().session.as_mut() {
            crate::model::ext_workspace::inject_extension_workspaces(
                &installed,
                &mut sess.settings.workdir,
            );
        }
    }

    // SDLC daemon restart: if this session has an existing mission (approved or
    // mid-assess), restore Sdlc mode so the keeper + phase-aware tooling resume
    // where they left off. Also snapshot the worktree cwd so tools bind to the
    // right root immediately.
    if crate::model::sdlc::Mission::load(&sess_path).is_some() {
        if let Some(sess) = state.rest.fg().session.as_ref() {
            if sess.settings.workdir_saved.is_some() {
                if let Some(wt) = sess.settings.workdir.first().cloned() {
                    let p = std::path::PathBuf::from(&wt);
                    if p.is_dir() {
                        state.rest.fg_mut().active_cwd = Some(p);
                    }
                }
            }
        }
        state.rest.set_agent_mode(crate::app::state::AgentMode::Sdlc);
    }

    if unconfigured {
        // Nothing configured yet — show the connection CHOOSER through the client. The
        // client renders `Mode::Onboard` and forwards the pick to the daemon; each
        // choice routes to its own setup action (koma free / provider / custom).
        *client = None;
        *state.mode_mut() = Mode::Onboard(Box::new(OnboardState { cursor: 0 }));
    } else {
        *client = Some(build_client());
        // Land in Chat first, THEN warm (warm_session may upgrade to Loading).
        *state.mode_mut() = Mode::Chat;
        warm_session(state, client, handle);
    }
}

/// Bounded window between firing the ext-owned sub-agent death notices
/// ([`notify_ext_owned_subagents_on_shutdown`]) and stopping the extension host, so the
/// per-extension async writer tasks get a moment to write+flush those queued frames to
/// their sockets — and the still-live extension children a moment to READ them — before
/// `stop_all` SIGKILLs the children. [`ExtHostManager::notify`](crate::app::ext::ExtHostManager::notify)
/// only QUEUES a frame onto an unbounded mpsc; the socket write happens LATER on the
/// async `writer_task`, so without this pause `drop(rt)` could cancel that writer before
/// the notice ever left the process. Only paid when at least one notice was emitted, so a
/// shutdown with no ext-owned work adds zero latency.
const EXT_SHUTDOWN_FLUSH_GRACE: std::time::Duration = std::time::Duration::from_millis(100);

/// Fire a `killed` death notice to the spawner of every EXTENSION-OWNED sub-agent still
/// in flight, because the host is shutting down. Returns the number notified.
///
/// Runs from [`shutdown_runtime`] BEFORE the extension host is torn down, while the
/// duplex ext wire (and the runtime) are still live, so an `agents.done {
/// status:"killed", error:"daemon restart" }` frame can reach the spawner. That lets a
/// restart-resilient extension respawn its agent under the fresh daemon (the build-skew
/// auto-restart after a binary upgrade is the motivating case) instead of seeing a bare
/// "killed" it can't tell from a real failure.
///
/// The reason is GENERIC by necessity: a skew-restart's `QuitDaemon` is indistinguishable
/// on the wire from a `koma daemon kill` / `/quit` `QuitDaemon` (the request carries no
/// skew context), and a first-`SIGTERM` graceful stop flips the same flag — so every
/// graceful shutdown reports the same respawnable `"daemon restart"`.
///
/// Covers both in-flight kinds:
/// - RUNNING ext-owned [`SubAgent`](crate::app::subagent::SubAgent)s — flipped to
///   [`Killed`](crate::app::subagent::SubAgentStatus::Killed) first (an honest in-memory
///   record), then emitted.
/// - QUEUED ext-owned [`PendingSubagent`](crate::app::subagent::PendingSubagent)s — they
///   never ran, but `broker_spawn` registered each in `ext_agents` at ENQUEUE time (with
///   its `notify` flag), so the owned `agents.done` still correlates to the spawner;
///   emitted too so a queued delegation isn't silently lost across the restart.
///
/// The `Running` gate is also the de-dupe: an agent already settled (by `broker_kill`, by
/// `drain_subagents` observing its terminal edge, or by `close()`'s
/// `abort_running_subagents`) is no longer `Running`, so it is skipped here and never
/// double-emitted. RESTORED-from-disk records likewise never match — `restore_bg_records`
/// coerces any still-"running" record to `Killed` AND sets `ext_owned = false`, so they
/// stay terminal + un-emitted on both this path and the `drain_subagents` was-running edge.
fn notify_ext_owned_subagents_on_shutdown(state: &mut AppState) -> usize {
    use crate::app::subagent::SubAgentStatus;

    // Ext-only: with no extension host there are no ext-owned agents at all, so this is a
    // pure no-op and the common (no-extensions) shutdown stays byte-identical.
    if state.rest.ext_manager.is_none() {
        return 0;
    }

    // The single, generic, respawnable reason for EVERY graceful shutdown (see fn docs:
    // a skew-restart is indistinguishable from a plain quit on the wire).
    const REASON: &str = "daemon restart";

    // Collect-then-emit: the status flip needs `&mut`, the emit needs `&AppState`, so
    // gather (session_uuid, local_id, agent_name) first — flipping running ones to Killed
    // as we go — then emit once the mutable walk is done.
    let mut targets: Vec<(String, usize, String)> = Vec::new();
    for si in 0..state.rest.sessions.len() {
        let session_uuid = state.rest.sessions[si].id.clone();
        for ai in 0..state.rest.sessions[si].subagents.len() {
            let sa = &mut state.rest.sessions[si].subagents[ai];
            if sa.ext_owned && matches!(sa.status, SubAgentStatus::Running) {
                sa.status = SubAgentStatus::Killed;
                targets.push((session_uuid.clone(), sa.id, sa.agent_name.clone()));
            }
        }
        // Queued ext-owned delegations that never started (no status field — always
        // "queued"): emit for each so its spawner learns it died with the daemon.
        for p in &state.rest.sessions[si].pending_subagents {
            if p.ext_owned {
                targets.push((session_uuid.clone(), p.id, p.agent_name.clone()));
            }
        }
    }

    for (session_uuid, local_id, agent) in &targets {
        crate::app::ext::events::emit_subagent_terminal(
            state,
            session_uuid,
            *local_id,
            agent,
            "killed",
            Some(REASON),
        );
    }

    targets.len()
}

/// Release every live session's on-disk lock, then drop the tokio runtime LAST.
///
/// Shared clean-exit teardown for both the TUI and daemon paths. Multi-session
/// aware — a quit (kill-all OR detach) can leave several sessions holding locks,
/// so releasing only the foreground's would strand the rest until PID-liveness
/// staleness kicked in. Dropping `rt` last cancels every spawned task; each task
/// owns the sender of its own per-request channel, and `let _ =` on each send
/// makes a post-drop send a safe no-op (no panic, no deadlock). A crash that skips
/// this is covered by PID-liveness staleness in `store::is_locked`.
fn shutdown_runtime(state: &mut AppState, rt: tokio::runtime::Runtime) {
    crate::model::store::append_global_error_log("daemon-exit", "shutdown_runtime: entering");
    // Death-notice pass FIRST — while the duplex ext wire AND the runtime are still live,
    // and BEFORE `stop_all` kills the extension children: tell every ext-owned in-flight
    // sub-agent's spawner it is dying to a host shutdown/restart, so a restart-resilient
    // extension respawns it rather than seeing a bare "killed". Returns 0 (pure no-op)
    // when no extension host is built, so a normal shutdown is byte-identical.
    let notified = notify_ext_owned_subagents_on_shutdown(state);
    if notified > 0 {
        crate::model::store::append_global_error_log(
            "subagent",
            &format!("killed by daemon shutdown: {notified} ext-owned agents notified"),
        );
        // Give the async writer tasks (still scheduled on the not-yet-dropped runtime) a
        // bounded moment to flush those queued notice frames to the ext sockets — and the
        // still-live children a moment to read them — before `stop_all` below SIGKILLs
        // them. Gated on `notified > 0`, so a shutdown with nothing to flush adds nothing.
        std::thread::sleep(EXT_SHUTDOWN_FLUSH_GRACE);
    }

    // Stop every running extension BEFORE the runtime drops. `ExtHostManager::stop`
    // (called per-extension by `stop_all`) takes the child out of its entry and drops
    // it locally; with `kill_on_drop(true)` that drop needs a LIVE runtime to actually
    // reap the process, so this must run while `rt` is still alive, not after. It also
    // unlinks every extension's `.sock` file (routed through the same `stop()` the
    // single-extension path uses), closing the leaked-socket gap on every shutdown
    // path (TUI `run` and the headless `run_daemon`, both of which call this). A `None`
    // manager (extension host never built) is a no-op.
    if let Some(ext) = state.rest.ext_manager.as_ref() {
        crate::model::store::append_global_error_log(
            "daemon-exit",
            "shutdown_runtime: stopping all extensions (kill_on_drop)",
        );
        ext.stop_all();
        crate::model::store::append_global_error_log(
            "daemon-exit",
            "shutdown_runtime: all extensions stopped",
        );
        // Every extension is stopped, so every extension's grant-broker spawn
        // registry (`app::ext::broker::ExtAgentRegistry`) is now dangling —
        // clear it here, the one place a whole-app extension stop has `AppState`
        // access (see `AppStateRest::ext_agents`'s doc for the per-extension
        // uninstall gap this doesn't cover).
        state.rest.ext_agents.clear();
    }
    // TODO sec stop: `state.rest.sec_manager` has the identical stop()-before-runtime-
    // drop shape (same kill_on_drop child, same reason to run before `drop(rt)`) but is
    // NOT wired here yet. Left as a follow-up rather than risk the security daemon's
    // shutdown behavior in this pass — confirm first that calling `stop()`
    // unconditionally on every TUI + daemon shutdown path doesn't regress anything the
    // `/security` panel relies on.
    // Best-effort: unregister this process from the linker daemon so its
    // root refcounts decrement and the daemon can idle-reap.
    for s in &mut state.rest.sessions {
        crate::linker::client::unregister_client(&s.id);
    }
    for s in &mut state.rest.sessions {
        if let Some(p) = s.held_lock.take() {
            crate::model::store::remove_lock(&p);
        }
    }
    crate::model::store::append_global_error_log(
        "daemon-exit",
        "shutdown_runtime: runtime dropped, locks released, done",
    );
    drop(rt);
}

pub fn run(opts: crate::cli::Opts) -> Result<()> {
    // Shared, terminal-independent startup (dirs, runtime, state, client, warm).
    let (rt, handle, mut state, mut client) = build_startup(&opts)?;

    // Terminal setup. Guard created BEFORE the Terminal so its Drop covers a
    // failing Terminal::new, any later `?`-error, and panic-unwind.
    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    // Clear the alternate screen so no shell scrollback bleeds through the
    // cells the UI never paints (e.g. the empty part of the transcript).
    terminal.clear()?;
    // Apply the mouse-capture mode from the foreground session's settings.
    // Auto-detects touch terminals (Termux) vs desktop; resolved once here
    // and re-applied on settings save.
    let mc = state
        .rest
        .fg()
        .session
        .as_ref()
        .map(|s| s.settings.mouse_capture)
        .unwrap_or_default();
    crate::app::runtime::actions::apply_mouse_capture(mc);

    let result = run_loop(&mut terminal, &mut state, &handle, &mut client);

    // Terminal teardown is handled by `_guard`'s Drop at function scope.
    // Release all session locks, then drop the runtime LAST (runs on Ok and Err).
    shutdown_runtime(&mut state, rt);

    result
}

/// Headless entry point: run the koma-daemon event loop with NO terminal.
///
/// Shares [`build_startup`] with the TUI [`run`] (same dirs / runtime / state /
/// client / warm), then — instead of the terminal + `run_loop` — ignores SIGPIPE,
/// installs the SIGHUP-survive + graceful/double-SIGTERM signal task, records the
/// pidfile, binds the unix socket, spawns the per-client accept loop, and enters
/// [`daemon_loop`]. The accept loop runs on the tokio runtime (async socket I/O);
/// `daemon_loop` runs synchronously on this thread and drains the bridge each tick
/// (critique #6). It returns when a controller sends `QuitDaemon` OR the process is
/// signalled (SIGTERM/SIGINT, via the polled `shutting_down` flag); the shared
/// teardown then releases every session lock, drops the runtime, and unlinks the
/// socket + pidfile.
pub fn run_daemon(opts: crate::cli::Opts) -> Result<()> {
    // Critique #10: writing to a dead client must never kill the daemon. Ignore
    // SIGPIPE process-wide BEFORE any socket IO so a broken-pipe write returns
    // EPIPE (handled per-write) instead of terminating the process. `libc` is a
    // direct dependency; this is the one tiny unsafe FFI call it is needed for.
    // SAFETY: `signal` with SIG_IGN on SIGPIPE is async-signal-safe and the
    // canonical way to opt out of SIGPIPE; it touches no Rust state.
    // SIGPIPE doesn't exist on Windows (no broken-pipe signal to ignore there).
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    // Windows has no SIGPIPE; instead arm the kill-on-close Job Object safety net NOW —
    // before `build_startup` spawns any child (e.g. an auto-started extension daemon) —
    // so every child auto-joins the job and a hard `TerminateProcess` of this daemon
    // tears the whole tree down. Not needed on unix (setsid + the signal/`QuitDaemon`
    // teardown release the tree there).
    #[cfg(windows)]
    super::signals::install_killtree_job();

    // Daemon-per-session: `--session <id>` is REQUIRED. This daemon binds the keyed
    // socket `run/<id>.sock` and owns exactly session `<id>` (the client minted the id
    // and passed it here, so both agree on the key). Erroring clearly beats binding a
    // wrong/global socket — the spawn machinery always passes it.
    let session_id = opts.session.clone().ok_or_else(|| {
        anyhow::anyhow!("`koma --daemon` requires `--session <id>` (daemon-per-session)")
    })?;

    // Shared, terminal-independent startup — identical to the TUI path.
    let (rt, handle, mut state, mut client) = build_startup(&opts)?;

    // Take ownership of session `<id>`: create-or-load it, wrap in a SessionRuntime,
    // acquire its lock, and install it as the daemon's single foreground session —
    // BEFORE the daemon loop / accept loop start. This REPLACES whatever placeholder /
    // returning-user session `build_startup` seeded, so the daemon serves exactly the
    // session keyed to its socket. Attach no longer creates a session (the daemon
    // already owns this one); see the hub `Attach` handler.
    install_daemon_session(&mut state, &mut client, &handle, &session_id);

    // MCP for the session-daemon: PROXY to the global MCP daemon when possible, with a
    // LOCAL fallback that is never worse than today. `build_startup` left
    // `mcp_manager = None` in daemon mode so this is the sole owner of the decision.
    //
    // - No `mcp_servers` configured → leave it `None` (no manager, no global daemon
    //   spawned): byte-identical to a build without MCP.
    // - Servers configured → ensure the singleton global MCP daemon is up and connect a
    //   PROXY to it, so N session-daemons share ONE copy of every heavyweight server
    //   (e.g. `serena`) instead of each spawning their own. If EITHER the ensure/spawn OR
    //   the proxy connect fails, FALL BACK to a LOCAL `connect_all` — MCP must always
    //   work; a missing/broken global daemon degrades to today's per-session behaviour.
    if !state.rest.config.mcp_servers.is_empty() {
        let proxy = crate::model::store::mcp_daemon_sock_path().and_then(|sock| {
            super::manage::ensure_mcp_daemon_running()
                .and_then(|()| crate::app::mcp::McpManager::connect_proxy(&handle, sock))
        });
        state.rest.mcp_manager = Some(match proxy {
            // Proxying to the shared global daemon: the dedup win.
            Ok(proxy) => proxy,
            // FALLBACK: any ensure/connect failure ⇒ own the connections locally, so
            // this daemon still has working MCP (just not shared).
            Err(e) => {
                crate::model::store::append_global_error_log(
                    "mcp",
                    &format!("global daemon unavailable ({e:#}); using local servers"),
                );
                crate::app::mcp::McpManager::connect_all(&handle, &state.rest.config.mcp_servers)
            }
        });
    }

    // Ensure the OAuth keep-alive daemon is running when there are OAuth connections.
    if !state.rest.config.oauth_conns.is_empty() {
        if let Err(e) = super::manage::ensure_oauth_daemon_running() {
            crate::model::store::append_global_error_log(
                "oauth",
                &format!("failed to start OAuth daemon: {e:#}"),
            );
        }
    }

    // Install the SIGHUP-survive + graceful/double-SIGTERM signal handling and get
    // the flag the SYNC loop polls. Done BEFORE binding the socket so a signal that
    // arrives during startup is already accounted for (it sets the flag the loop
    // checks on its very first tick). The daemon now ignores SIGHUP, so closing the
    // launching terminal can't kill it.
    let shutting_down = install_daemon_signals(&handle);

    // Record the advisory pidfile (diagnostics / `kill`), keyed by this session.
    // Best-effort: a write failure must not stop the daemon (the bound socket, not this
    // file, is the liveness oracle), so the error is swallowed. The teardown unlinks it.
    let pid_path = crate::model::store::daemon_pid_path(&session_id)?;
    let _ = crate::model::store::write_daemon_pid(&session_id);

    // Sync-loop <-> per-client-task bridge (critique #1/#6). The runner holds the
    // paired `req_tx` (which the accept loop clones into each connection task) for
    // the daemon's lifetime so `req_rx` never observes a premature `Disconnected`
    // before any client connects.
    let (mut hub, req_tx) = DaemonHub::new();

    // Bind the unix listener (this process becomes the live daemon — bind is the
    // liveness oracle) and spawn the accept loop onto the tokio runtime. Each
    // accepted connection gets a per-client task bridging its socket to `req_tx`.
    // `UnixListener::bind` + `handle.spawn` need a tokio reactor in scope, so enter
    // the runtime context for them. The keyed socket path is unlinked at teardown below.
    let sock_path = crate::model::store::daemon_sock_path(&session_id)?;
    {
        let _enter = handle.enter();
        let listener = crate::ipc::server::bind(&sock_path)?;
        handle.spawn(crate::ipc::server::accept_loop(listener, req_tx));
    }

    // Enter the headless loop: service_all_sessions + service_global + the request-
    // bridge drain (apply mutations) + delta streaming on the adaptive cadence.
    // Returns when a controller's QuitDaemon latches the hub flag OR a signal flips
    // `shutting_down` (both observed each tick).
    crate::model::store::append_global_error_log(
        "daemon-exit",
        "daemon_loop entering (this is normal startup)",
    );
    daemon_loop(&mut state, &mut client, &handle, &mut hub, &shutting_down);
    crate::model::store::append_global_error_log(
        "daemon-exit",
        &format!(
            "daemon_loop returned → teardown begins [sessions={}, clients={}]",
            state.rest.sessions.len(),
            hub.client_count(),
        ),
    );

    // Graceful teardown (QuitDaemon, SIGTERM/SIGINT, or a future self-exit). Dropping
    // the runtime in `shutdown_runtime` cancels the accept loop and every per-client
    // task, so no new client is serviced past this point ("stop accepting new
    // clients"); it also releases every session lock and drops the runtime LAST. Then
    // remove the socket + pidfile so the next spawn binds fresh. (A second SIGTERM
    // during this window hard-exits via the signal task instead of reaching here.)
    shutdown_runtime(&mut state, rt);
    // Unix-only: a unix socket is a filesystem object to unlink; a Windows named pipe
    // is released when the runtime (its owning handles) drops above, so there is no
    // socket file to remove. The pidfile is a real file on both platforms.
    #[cfg(unix)]
    let _ = std::fs::remove_file(&sock_path);
    let _ = std::fs::remove_file(&pid_path);

    Ok(())
}
