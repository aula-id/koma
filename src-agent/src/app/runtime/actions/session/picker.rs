use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;

use crate::app::mode::{KeyInputForm, Mode};
use crate::app::state::AppState;
use crate::config::DEFAULT_MODEL;
use crate::model::{session::Session, store};
use crate::service::openrouter::OpenRouterClient;

use crate::app::runtime::build_client;

/// Handle `Action::PickerSelect`: NON-DESTRUCTIVE `--resume` startup-picker
/// selection. Extracts the highlighted session's path from the picker, then runs
/// the shared [`open_disk_session`] load path (append-or-swap, never destructive).
pub fn handle_picker_select(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    // Guard: can't append onto an unstable session tail while a /new KeyInput is
    // pending confirmation. The modal state should prevent this, but make the
    // invariant explicit.
    if state.rest.spawn_pending {
        return Ok(());
    }

    // Extract selected path first (borrow of mode released before mutating
    // rest/mode below). The picked path is the canonical identity: a session dir
    // is `sessions/<pwd_hash>/<uuid>`, so equal paths ⇒ equal id, and every
    // lock/load API here is already path-keyed.
    let path = match state.mode() {
        Mode::SessionPicker(p) => p.selected_meta().map(|m| m.path.clone()),
        _ => None,
    };
    let Some(path) = path else {
        state.rest.fg_mut().status = "no session selected".into();
        return Ok(());
    };

    open_disk_session(state, client, handle, path)
}

/// Handle `Action::HubOpenHistory`: NON-DESTRUCTIVE open of the session hub's
/// HISTORY-pane selection. Resolves the carried row index to its on-disk path
/// from the hub state, then runs the same shared [`open_disk_session`] load path.
///
/// History rows are de-duplicated against the live sessions at hub-open time, so
/// the path normally won't match a live tab — but `open_disk_session`'s case 1
/// still handles it (falls back to a foreground SWAP) if it somehow does.
pub fn handle_hub_open_history(
    idx: usize,
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    // Guard: don't append onto an unstable session tail mid /new-KeyInput
    // confirmation (mirrors handle_picker_select).
    if state.rest.spawn_pending {
        return Ok(());
    }

    // Pull the row's path out of the hub state (borrow released before mutating).
    let path = match state.mode() {
        Mode::SessionHub(h) => h.history.get(idx).map(|e| e.path.clone()),
        _ => None,
    };
    let Some(path) = path else {
        state.rest.fg_mut().status = "no session selected".into();
        return Ok(());
    };

    open_disk_session(state, client, handle, path)
}

/// NON-DESTRUCTIVE load of an on-disk session by `path`. Shared by the `--resume`
/// startup picker ([`handle_picker_select`]) and the session hub's history pane
/// ([`handle_hub_open_history`]). The current foreground is NEVER aborted, never
/// loses its lock, and keeps cooking in its own slot:
///
/// 1. **Already open in THIS process** (a `sessions` slot's `session.path`
///    matches `path`) → just SWAP foreground to it (flat-UI reset, mirroring
///    [`super::attach::handle_live_switch`]), no reload, no lock churn.
/// 2. **Locked by ANOTHER live process** → refuse (a lock held by US is always
///    covered by case 1, so a still-live lock here is necessarily another PID).
/// 3. **Free to load** → load the [`Session`] from disk, hydrate a FRESH
///    [`SessionRuntime`], acquire ITS lock, APPEND to `sessions`, make it the
///    foreground (`/new`'s flat-UI reset + warm). The previous foreground stays
///    live in its slot, lock held.
///
/// INVARIANT (this stage): `sessions` is only ever APPENDED to + the foreground
/// changes — never reordered/removed — and no other session's lock is released.
pub fn open_disk_session(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
    path: PathBuf,
) -> Result<()> {
    // --- Case 1: already open as a live tab in THIS process → SWAP, don't reload.
    // Match on the on-disk session path (stable identity). If found, behave exactly
    // like a cooking-pane switch (handle_live_switch): set foreground, reset the flat foreground-UI,
    // rebuild the keyless client for the target's route, status from is_working().
    if let Some(idx) = state
        .rest
        .sessions
        .iter()
        .position(|rt| rt.session.as_ref().map(|s| &s.path) == Some(&path))
    {
        // Per-session mode (C3): this swap is reached from the open SessionPicker /
        // SessionHub, so the CURRENT (leaving) foreground's mode is that overlay. Reset it
        // to Chat BEFORE repointing so switching back doesn't resurrect the picker/hub; the
        // target keeps its OWN stored mode (mirrors `handle_live_switch`).
        state.rest.fg_mut().mode = Mode::Chat;
        state.rest.foreground = idx;
        // Per-session composer + view reset for the now-shown tab (mirror
        // handle_live_switch): empty composer + caret, pinned-to-bottom scroll, no
        // staged attachments, and a fresh transcript cache so the target's
        // conversation renders instead of the previous tab's cached blocks. No
        // token reseed — each slot owns its counters.
        {
            let fg = state.rest.fg_mut();
            fg.input.clear();
            fg.cursor = 0;
            fg.pending_attachments.clear();
        }
        state.rest.reset_scroll();
        state.rest.transcript_cache.borrow_mut().blocks.clear();
        // KEYLESS client → rebuild for a fresh plan_word at this session boundary,
        // gated on the target having a usable Main route (no-client-no-send).
        let usable = state
            .rest
            .fg()
            .session
            .as_ref()
            .map(|s| s.settings.clone())
            .is_some_and(|settings| {
                crate::app::resolve::resolve_role(
                    &state.rest.config,
                    &settings,
                    crate::model::app_config::ModelRole::Main,
                )
                .is_some_and(|r| r.is_usable())
            });
        *client = if usable { Some(build_client()) } else { None };
        // Per-session status (C6): compute the working flag first (it borrows
        // `sessions` immutably), then write the foreground session's own status.
        let working = state.rest.fg().is_ui_busy();
        state.rest.fg_mut().status = if working {
            "working".into()
        } else {
            "ready".into()
        };
        // No `mode = Chat` on the target (C3): it shows its OWN stored mode. The leaving
        // session was reset to Chat above before the repoint.
        return Ok(());
    }

    // --- Case 2: not in our process but locked by a LIVE process → refuse.
    // Re-check the lock live (don't trust the cached row flag) so a race — the
    // session getting opened elsewhere after the list was built — can't slip
    // through. Since case 1 already ruled out OUR own tabs, any live lock here is
    // necessarily another process's (`is_locked` does the PID-liveness check and
    // sweeps stale locks). Stay in the picker; the row already shows the marker.
    if store::is_locked(&path) {
        state.rest.fg_mut().status = "session in use by another process".into();
        return Ok(());
    }

    // --- Case 3: free to load → APPEND a fresh tab; previous foreground stays live.
    let sess = match Session::load(&path) {
        Ok(s) => s,
        Err(e) => {
            state.rest.fg_mut().status = format!("error: {e}");
            return Ok(());
        }
    };

    // Acquire THIS session's lock immediately — every live session holds its own
    // lock for its lifetime. Build a fresh runtime that owns the loaded session +
    // lock, then APPEND it and make it the foreground. The OLD foreground keeps its
    // own slot, lock, and in-flight turn (we never abort or take it).
    store::write_lock(&sess.path);
    let mut runtime = crate::app::state::SessionRuntime::new();
    runtime.held_lock = Some(sess.path.clone());
    // Show KeyInput only when NO usable Main route exists for this loaded session —
    // resolve against the GLOBAL config (providers/models) plus the legacy fallback,
    // not just the bare `settings.api_key`. A populated global config => straight to
    // chat even if this session has no legacy key. Computed before `sess` is moved
    // into `runtime` below; only borrows `&state.rest.config` + `&sess.settings`.
    let no_creds = crate::app::resolve::resolve_role(
        &state.rest.config,
        &sess.settings,
        crate::model::app_config::ModelRole::Main,
    )
    .is_none_or(|r| !r.is_usable());
    let sess_path = sess.path.clone();
    runtime.session = Some(sess);

    // Remember where to return if the (creds-less) KeyInput below is cancelled,
    // then APPEND + make foreground.
    state.rest.spawn_prev_fg = state.rest.foreground;
    state.rest.sessions.push(runtime);
    state.rest.foreground = state.rest.sessions.len() - 1;

    // Reset the per-session composer + view for a clean slate on the new tab
    // (mirror /new): empty composer + caret, pinned-to-bottom scroll, no staged
    // attachments (so the previous session's images don't leak in), fresh
    // transcript cache.
    {
        let fg = state.rest.fg_mut();
        fg.input.clear();
        fg.cursor = 0;
        fg.pending_attachments.clear();
    }
    state.rest.reset_scroll();
    state.rest.transcript_cache.borrow_mut().blocks.clear();
    state.rest.fg_mut().status = "ready".into();

    // Existing session: seed THIS (new foreground) slot's OWN counters from its full
    // sqlite log so the readout reflects prior usage. Never touches another slot.
    let new_fg = state.rest.foreground;
    state.rest.load_token_totals(new_fg, &sess_path);

    if no_creds {
        // Loaded session has no creds — prompt FOR THE NEW (appended) SESSION. Mark
        // it spawn-pending and open KeyInput with from_picker = false so Esc routes
        // to CancelKeyInput, whose spawn_pending branch POPS this just-appended tab,
        // releases its lock, and restores the previous foreground (reusing /new's
        // proven cancel machinery — leaving a valid foreground either way).
        let lk = state.rest.last_key.clone().unwrap_or_default();
        let lm = state
            .rest
            .last_model
            .clone()
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());
        *client = None;
        state.rest.spawn_pending = true;
        // Fail-closed SDLC restore even while waiting for creds: mode/phase must
        // not stay plain Auto over an active execute/integrate mission on disk.
        restore_sdlc_on_open(state);
        *state.mode_mut() = Mode::KeyInput(KeyInputForm::prefilled(lk, lm, false, false));
    } else {
        state.rest.spawn_pending = false;
        let (key, model, provider) = state
            .rest
            .fg()
            .session
            .as_ref()
            .map(|s| {
                (
                    s.settings.api_key.clone(),
                    s.settings.model.clone(),
                    s.settings.provider.clone(),
                )
            })
            .unwrap_or_default();
        state.rest.remember_creds(&key, &model, &provider);
        // KEYLESS client → fresh plan_word at this session boundary. This branch
        // already gated on a non-empty key above, so build directly.
        *client = Some(build_client());
        // Land in Chat first, THEN warm: `warm_session` is non-blocking and may
        // upgrade the mode to `Mode::Loading` (animated splash) when it has warm
        // work to spawn, so it must run LAST. With no warm work it leaves the mode
        // as the Chat we just set. warm_session -> reconcile_session_lock only ever
        // touches the (new) foreground's lock, which already matches the on-disk
        // lock we just wrote — a no-op for locks; no other session's lock is freed.
        *state.mode_mut() = Mode::Chat;

        // SDLC local open/resume: same fail-closed restoration as daemon lifecycle.
        // Only a validated active binding may resume execute/integrate; missing or
        // invalid binding lands in assess with reapproval required — never plain
        // Auto with an active mission on disk (unrestricted execution).
        restore_sdlc_on_open(state);

        super::super::super::warm_session(state, client, handle);
    }
    Ok(())
}

/// Restore SDLC mode for a freshly-opened local session when `mission.json`
/// exists AND passes the canonical [`should_auto_resume`] gate. Mirrors the
/// daemon lifecycle path: optional worktree cwd snap, then
/// `set_agent_mode(Sdlc)` which fail-closes invalid/missing bindings into
/// assess + needs_reapproval rather than resuming unrestricted Auto execution.
///
/// Missions that do NOT pass `should_auto_resume` (draft / assess / paused /
/// done / stale / invalid / needs_reapproval / missing binding) are left as
/// plain Auto — the user must explicitly re-enter SDLC via `/mode sdlc`.
fn restore_sdlc_on_open(state: &mut AppState) {
    let sess_path = match state.rest.fg().session.as_ref().map(|s| s.path.clone()) {
        Some(p) => p,
        None => return,
    };
    let mission = match crate::model::sdlc::Mission::load(&sess_path) {
        Some(m) => m,
        None => return,
    };
    if !crate::model::sdlc::mission::should_auto_resume(&mission) {
        return;
    }
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
    state
        .rest
        .set_agent_mode(crate::app::state::AgentMode::Sdlc);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod restore_sdlc_tests {
    use super::restore_sdlc_on_open;
    use crate::app::state::{AgentMode, AppState};
    use crate::model::conversation::Conversation;
    use crate::model::sdlc::Mission;
    use crate::model::session::Session;
    use crate::model::settings::Settings;

    fn scratch_session(tag: &str) -> (std::path::PathBuf, Session) {
        let dir = std::env::temp_dir().join(format!(
            "koma-open-sdlc-{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let sess = Session::new(
            format!("s-{tag}"),
            dir.clone(),
            "pwd".into(),
            Settings::default(),
            Conversation::from_messages(vec![]),
        );
        (dir, sess)
    }

    fn active_execute_mission(worktree_path: &str) -> Mission {
        let goal = "ship X";
        let acceptance = vec!["tests pass".into()];
        let non_goals = vec!["rewrite Y".into()];
        let lane = "standard";
        let verify_plan = vec!["cargo test".into()];
        let human_gates: Vec<String> = vec![];
        let risks = vec!["api churn".into()];
        let rationale = "match house style";
        let worktree_name = Some("sdlc-test".into());
        let branch = Some("sdlc/ship-x".into());
        let wt = Some(worktree_path.to_string());
        let target_worktree_path = Some("/tmp/primary".into());
        let target_branch = Some("main".into());
        let target_head = Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into());
        let hash =
            Mission::compute_contract_hash_full(crate::model::sdlc::mission::ContractHashInput {
                goal,
                acceptance: &acceptance,
                non_goals: &non_goals,
                lane,
                verify_plan: &verify_plan,
                human_gates: &human_gates,
                risks: &risks,
                rationale,
                graph_hash: None,
                worktree_name: worktree_name.as_deref(),
                branch: branch.as_deref(),
                worktree_path: wt.as_deref(),
                target_worktree_path: target_worktree_path.as_deref(),
                target_branch: target_branch.as_deref(),
                target_head: target_head.as_deref(),
            });
        Mission {
            contract_version: crate::model::sdlc::mission::CURRENT_CONTRACT_VERSION,
            id: "m1".into(),
            goal: goal.into(),
            non_goals,
            acceptance,
            lane: lane.into(),
            verify_plan,
            human_gates,
            human_gates_approved: vec![],
            risks,
            worktree_name,
            branch,
            worktree_path: wt,
            target_worktree_path,
            target_branch,
            target_head,
            rationale: rationale.into(),
            phase: "execute".into(),
            approved: true,
            hash,
            graph_hash: None,
            needs_reapproval: false,
            amendment_note: None,
        }
    }

    #[test]
    fn open_with_invalid_execute_binding_fails_closed_to_assess() {
        let (dir, sess) = scratch_session("bad-bind");
        // Bound path does not exist → re-entry must fail closed.
        let m = active_execute_mission(&format!(
            "/tmp/koma-missing-wt-{}-{}",
            std::process::id(),
            "nope"
        ));
        m.save(&dir).unwrap();

        let mut state = AppState::new(crate::app::mode::Mode::Chat);
        state.rest.fg_mut().session = Some(sess);
        // Default is Auto — without restore this would leave unrestricted Auto
        // over an active execute mission on disk.
        assert_eq!(state.rest.fg().agent_mode, AgentMode::Auto);

        restore_sdlc_on_open(&mut state);

        assert_eq!(state.rest.fg().agent_mode, AgentMode::Sdlc);
        assert_eq!(state.rest.fg().sdlc_phase.as_deref(), Some("assess"));
        let loaded = Mission::load(&dir).unwrap();
        assert!(!loaded.approved);
        assert!(loaded.needs_reapproval);
        assert_eq!(loaded.phase, "assess");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_without_mission_leaves_mode_untouched() {
        let (dir, sess) = scratch_session("no-mission");
        let mut state = AppState::new(crate::app::mode::Mode::Chat);
        state.rest.fg_mut().session = Some(sess);
        assert_eq!(state.rest.fg().agent_mode, AgentMode::Auto);
        restore_sdlc_on_open(&mut state);
        assert_eq!(state.rest.fg().agent_mode, AgentMode::Auto);
        assert!(state.rest.fg().sdlc_phase.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Helper: build a mission with given phase/approved/needs_reapproval fields
    /// and a valid (current-version, frozen-target, hash-valid) contract.
    fn mission_with_phase(phase: &str, approved: bool, needs_reapproval: bool) -> Mission {
        let goal = "reopen test";
        let acceptance = vec!["ok".into()];
        let non_goals = vec![];
        let lane = "standard";
        let verify_plan = vec![];
        let human_gates: Vec<String> = vec![];
        let risks = vec![];
        let rationale = "test";
        let worktree_name = Some("sdlc-reopen".into());
        let branch = Some("sdlc/reopen".into());
        let worktree_path = Some("/tmp/sdlc-reopen-test".into());
        let target_worktree_path = Some("/tmp/sdlc-primary-test".into());
        let target_branch = Some("develop".into());
        let target_head = Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into());
        let graph_hash = Some("gh-reopen".into());
        let hash =
            Mission::compute_contract_hash_full(crate::model::sdlc::mission::ContractHashInput {
                goal,
                acceptance: &acceptance,
                non_goals: &non_goals,
                lane,
                verify_plan: &verify_plan,
                human_gates: &human_gates,
                risks: &risks,
                rationale,
                graph_hash: graph_hash.as_deref(),
                worktree_name: worktree_name.as_deref(),
                branch: branch.as_deref(),
                worktree_path: worktree_path.as_deref(),
                target_worktree_path: target_worktree_path.as_deref(),
                target_branch: target_branch.as_deref(),
                target_head: target_head.as_deref(),
            });
        Mission {
            contract_version: crate::model::sdlc::mission::CURRENT_CONTRACT_VERSION,
            id: "m-reopen".into(),
            goal: goal.into(),
            non_goals,
            acceptance,
            lane: lane.into(),
            verify_plan,
            human_gates,
            human_gates_approved: vec![],
            risks,
            worktree_name,
            branch,
            worktree_path,
            target_worktree_path,
            target_branch,
            target_head,
            rationale: rationale.into(),
            phase: phase.into(),
            approved,
            hash,
            graph_hash,
            needs_reapproval,
            amendment_note: None,
        }
    }

    /// Reopen matrix: approved valid prepare/execute/integrate → should auto-resume SDLC.
    /// Note: since the mock worktree paths don't exist on disk, set_agent_mode
    /// fail-closes from prepare/execute/integrate into assess. The critical assertion
    /// is that the MODE is Sdlc (auto-resume gate passed), not the exact phase.
    #[test]
    fn reopen_matrix_approved_valid_resume() {
        for phase in ["prepare", "execute", "integrate"] {
            let (dir, sess) = scratch_session(&format!("resume-{phase}"));
            let m = mission_with_phase(phase, true, false);
            m.save(&dir).unwrap();

            let mut state = AppState::new(crate::app::mode::Mode::Chat);
            state.rest.fg_mut().session = Some(sess);
            assert_eq!(state.rest.fg().agent_mode, AgentMode::Auto);

            restore_sdlc_on_open(&mut state);
            assert_eq!(
                state.rest.fg().agent_mode,
                AgentMode::Sdlc,
                "phase={phase} should auto-resume to SDLC mode"
            );
            // Phase may be the original or assess (if worktree re-entry fails
            // because the mock path doesn't exist on disk).
            let p = state.rest.fg().sdlc_phase.as_deref();
            assert!(
                p == Some(phase) || p == Some("assess"),
                "phase={phase}: expected {phase} or assess, got {p:?}"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// Reopen matrix: denial for draft/assess/unapproved/needs_reapproval/paused/done/invalid/stale/missing/bad binding.
    #[test]
    fn reopen_matrix_denies_non_resume() {
        // draft → denied
        let (dir, sess) = scratch_session("deny-draft");
        let m = mission_with_phase("draft", false, false);
        m.save(&dir).unwrap();
        let mut state = AppState::new(crate::app::mode::Mode::Chat);
        state.rest.fg_mut().session = Some(sess);
        restore_sdlc_on_open(&mut state);
        assert_eq!(
            state.rest.fg().agent_mode,
            AgentMode::Auto,
            "draft must not resume"
        );
        let _ = std::fs::remove_dir_all(&dir);

        // assess (unapproved) → denied
        let (dir, sess) = scratch_session("deny-assess");
        let m = mission_with_phase("assess", false, false);
        m.save(&dir).unwrap();
        let mut state = AppState::new(crate::app::mode::Mode::Chat);
        state.rest.fg_mut().session = Some(sess);
        restore_sdlc_on_open(&mut state);
        assert_eq!(
            state.rest.fg().agent_mode,
            AgentMode::Auto,
            "assess must not resume"
        );
        let _ = std::fs::remove_dir_all(&dir);

        // approved but needs_reapproval → denied
        let (dir, sess) = scratch_session("deny-reapproval");
        let m = mission_with_phase("execute", true, true);
        m.save(&dir).unwrap();
        let mut state = AppState::new(crate::app::mode::Mode::Chat);
        state.rest.fg_mut().session = Some(sess);
        restore_sdlc_on_open(&mut state);
        assert_eq!(
            state.rest.fg().agent_mode,
            AgentMode::Auto,
            "needs_reapproval must not resume"
        );
        let _ = std::fs::remove_dir_all(&dir);

        // paused → denied
        let (dir, sess) = scratch_session("deny-paused");
        let m = mission_with_phase("paused", true, false);
        m.save(&dir).unwrap();
        let mut state = AppState::new(crate::app::mode::Mode::Chat);
        state.rest.fg_mut().session = Some(sess);
        restore_sdlc_on_open(&mut state);
        assert_eq!(
            state.rest.fg().agent_mode,
            AgentMode::Auto,
            "paused must not resume"
        );
        let _ = std::fs::remove_dir_all(&dir);

        // done → denied
        let (dir, sess) = scratch_session("deny-done");
        let m = mission_with_phase("done", true, false);
        m.save(&dir).unwrap();
        let mut state = AppState::new(crate::app::mode::Mode::Chat);
        state.rest.fg_mut().session = Some(sess);
        restore_sdlc_on_open(&mut state);
        assert_eq!(
            state.rest.fg().agent_mode,
            AgentMode::Auto,
            "done must not resume"
        );
        let _ = std::fs::remove_dir_all(&dir);

        // invalid hash → denied
        let (dir, sess) = scratch_session("deny-invalid");
        let mut m = mission_with_phase("execute", true, false);
        m.hash = "deadbeef".repeat(4);
        m.save(&dir).unwrap();
        let mut state = AppState::new(crate::app::mode::Mode::Chat);
        state.rest.fg_mut().session = Some(sess);
        restore_sdlc_on_open(&mut state);
        assert_eq!(
            state.rest.fg().agent_mode,
            AgentMode::Auto,
            "invalid hash must not resume"
        );
        let _ = std::fs::remove_dir_all(&dir);

        // stale contract version → denied
        let (dir, sess) = scratch_session("deny-stale");
        let mut m = mission_with_phase("execute", true, false);
        m.contract_version = 1;
        m.hash = m.recompute_hash();
        m.save(&dir).unwrap();
        let mut state = AppState::new(crate::app::mode::Mode::Chat);
        state.rest.fg_mut().session = Some(sess);
        restore_sdlc_on_open(&mut state);
        assert_eq!(
            state.rest.fg().agent_mode,
            AgentMode::Auto,
            "stale contract must not resume"
        );
        let _ = std::fs::remove_dir_all(&dir);

        // missing mission.json → no-op (stays Auto)
        let (dir, sess) = scratch_session("deny-missing");
        let mut state = AppState::new(crate::app::mode::Mode::Chat);
        state.rest.fg_mut().session = Some(sess);
        restore_sdlc_on_open(&mut state);
        assert_eq!(
            state.rest.fg().agent_mode,
            AgentMode::Auto,
            "missing mission must not resume"
        );
        let _ = std::fs::remove_dir_all(&dir);

        // bad binding (worktree_path does not exist on disk) → denied
        let (dir, sess) = scratch_session("deny-bad-binding");
        let m = mission_with_phase("execute", true, false);
        // m.worktree_path = Some("/tmp/sdlc-reopen-test") which doesn't exist →
        // should_auto_resume sees worktree_path non-empty, so it WILL auto-resume.
        // But the set_agent_mode path will fail-closed into assess. So verify
        // that auto-resume is attempted (mode = Sdlc) but phase = assess from
        // fail-closed.
        m.save(&dir).unwrap();
        let mut state = AppState::new(crate::app::mode::Mode::Chat);
        state.rest.fg_mut().session = Some(sess);
        restore_sdlc_on_open(&mut state);
        // should_auto_resume sees valid fields → enters SDLC; set_agent_mode
        // tries re-entry which fails → lands in assess
        assert_eq!(state.rest.fg().agent_mode, AgentMode::Sdlc);
        assert_eq!(state.rest.fg().sdlc_phase.as_deref(), Some("assess"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Reopen: unapproved execute mission (valid contract but not approved) → denied.
    #[test]
    fn reopen_matrix_unapproved_execute_denied() {
        let (dir, sess) = scratch_session("deny-unapproved");
        let m = mission_with_phase("execute", false, false);
        m.save(&dir).unwrap();
        let mut state = AppState::new(crate::app::mode::Mode::Chat);
        state.rest.fg_mut().session = Some(sess);
        restore_sdlc_on_open(&mut state);
        assert_eq!(
            state.rest.fg().agent_mode,
            AgentMode::Auto,
            "unapproved execute must not resume"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
