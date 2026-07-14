//! Action handlers for settings editor: SaveSettings, SaveEffort, EffortCancel,
//! FetchModelEndpoints.

use std::sync::Arc;

use anyhow::Result;

use crate::app::mode::Mode;
use crate::app::state::AppState;
use crate::model::app_config::{ModelEntry, ProviderConn};
use crate::model::store;
use crate::service::openrouter::OpenRouterClient;

/// Handle `Action::SaveSettings`: pull all setting drafts from the modal, apply
/// them to the session and global config, persist, and return to Chat.
pub(super) fn handle_save_settings(state: &mut AppState) -> Result<()> {
    // 1. Pull drafts out of the mode first so the borrow of `state.mode`
    //    is released before we mutate `state.rest` / `state.mode` below.
    let drafts = match state.mode() {
        Mode::Settings(s) => Some((
            s.api_key.clone(),
            s.model.clone(),
            s.provider.clone(),
            s.name.clone(),
            s.theme.clone(),
            s.accent.clone(),
            s.palette.clone(),
            s.workdir.clone(),
            s.awareness_enabled,
            s.awareness_inherit,
            s.awareness_model.clone(),
            s.awareness_provider.clone(),
            s.classifier_enabled,
            s.classifier_model.clone(),
            s.classifier_provider.clone(),
            s.allowed_folders.clone(),
            s.short_send_enabled,
            s.sliding_cache,
            s.bash_saving,
            s.coding_autosave,
            s.internet_mode,
            s.providers.clone(),
            s.oauth_drafts.clone(),
            s.models.clone(),
        )),
        _ => None,
    };
    if let Some((
        api_key,
        model,
        provider,
        name,
        theme,
        accent,
        palette,
        workdir,
        awareness_enabled,
        awareness_inherit,
        awareness_model,
        awareness_provider,
        classifier_enabled,
        classifier_model,
        classifier_provider,
        allowed_folders,
        short_send_enabled,
        sliding_cache,
        bash_saving,
        coding_autosave,
        internet_mode,
        provider_drafts,
        oauth_drafts,
        model_drafts,
    )) = drafts
    {
        // No client rebuild is keyed off creds/model/provider changes
        // anymore: the client is KEYLESS and every request resolves its
        // connection/model/effort per-call via `resolve_role`, so the
        // existing Arc keeps serving (and keeps its cache-stable plan_word).
        // a) Apply the text drafts to the session settings. The
        //    awareness settings ride along here too; they don't affect
        //    the chat client (the awareness call uses `complete_with`
        //    per invocation), so no client rebuild is keyed off them.
        // Normalise both path-list drafts: trim each entry, drop empties.
        // (They're already `Vec<String>` from the managed list editor — no
        // comma-splitting anymore.)
        let allowed_folders_vec: Vec<String> = allowed_folders
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        // Workdir must keep at least one entry; if the draft normalises to
        // nothing, fall back to the launch cwd so `Session::workdir` still
        // resolves and the reindex below has a real directory.
        let mut workdir_vec: Vec<String> = workdir
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if workdir_vec.is_empty() {
            workdir_vec = std::env::current_dir()
                .map(|p| vec![p.display().to_string()])
                .unwrap_or_default();
        }
        // Map provider drafts -> persisted ProviderConn (preserve uuid;
        // mint one only if a draft somehow arrived without it).
        let mut provider_conns: Vec<ProviderConn> = provider_drafts
            .iter()
            .map(|d| ProviderConn {
                uuid: if d.uuid.is_empty() {
                    uuid::Uuid::new_v4().to_string()
                } else {
                    d.uuid.clone()
                },
                name: d.name.clone(),
                api_type: d.api_type,
                endpoint: d.endpoint.clone(),
                api_key: d.api_key.clone(),
                // W12b: round-trip the extension-ownership tag so a settings save never
                // silently converts an ext-managed key-backed provider into a native one.
                ext_id: d.ext_id.clone(),
            })
            .collect();
        // W12b HOST GUARD (the A3 whole-Vec-replace deletion path): an EXTENSION-managed
        // provider is IMMUTABLE via settings (only the extension may change it, via
        // `providers.register`; only uninstall removes it). RESTORE each ext provider verbatim
        // from the existing config — replacing a round-tripped draft (in case a stale/older
        // client dropped its `ext_id` or edited it) or re-appending one the draft omitted (a
        // delete slipping through). Natives (no ext providers) → no-op, so a zero-extension
        // config is byte-identical.
        for ep in state
            .rest
            .config
            .providers
            .iter()
            .filter(|p| p.ext_id.is_some())
        {
            match provider_conns.iter_mut().find(|p| p.uuid == ep.uuid) {
                Some(slot) => *slot = ep.clone(),
                None => provider_conns.push(ep.clone()),
            }
        }
        // Map model drafts -> persisted ModelEntry, resolving the draft's
        // positional `provider_idx` back to a `provider_uuid` against the
        // FRESHLY built provider_conns (so a model added in this same edit
        // session that points at a brand-new provider still resolves). A
        // dangling idx yields an empty provider_uuid (surfaces for re-pick).
        // Resolve `provider_idx` back to a `provider_uuid` via the SAME
        // providers-then-oauth-offset merge as the load side: a real provider's
        // uuid when the idx is within `provider_conns`, else the OAuth draft's uuid
        // (idx offset by `provider_conns.len()`). A dangling idx yields an empty
        // provider_uuid (surfaces for re-pick). `oauth_drafts` is passed in so the
        // OAuth catalogue never leaks into `config.providers`.
        let to_entry = |d: &crate::app::mode::settings::ModelDraft, oauth_drafts: &[crate::app::mode::settings::OAuthDraft]| ModelEntry {
            uuid: if d.uuid.is_empty() {
                uuid::Uuid::new_v4().to_string()
            } else {
                d.uuid.clone()
            },
            name: d.name.clone(),
            model_id: d.model_id.clone(),
            provider_uuid: if d.provider_idx < provider_conns.len() {
                provider_conns.get(d.provider_idx).map(|p| p.uuid.clone()).unwrap_or_default()
            } else {
                oauth_drafts
                    .get(d.provider_idx - provider_conns.len())
                    .map(|o| o.uuid.clone())
                    .unwrap_or_default()
            },
            route: d.route.clone(),
            // Persist the multi-role list; leave the legacy single-role
            // field None so it stops being serialized (migration on save).
            roles: d.roles.clone(),
            role: None,
            // Preserve the clone-source identity through the save so a /settings save
            // that never opened this override keeps the GUI picker's exact match
            // (a modal edit re-authors the draft to None — see save_model_modal).
            source_uuid: d.source_uuid.clone(),
        };
        // Global catalogue: session_only == false. Session override layer:
        // session_only == true (persisted to settings.json, never config).
        let model_entries: Vec<ModelEntry> = model_drafts
            .iter()
            .filter(|d| !d.session_only)
            .map(|d| to_entry(d, &oauth_drafts))
            .collect();
        let session_model_entries: Vec<ModelEntry> = model_drafts
            .iter()
            .filter(|d| d.session_only)
            .map(|d| to_entry(d, &oauth_drafts))
            .collect();
        // Capture old internet mode before overwriting, so we can toast only
        // on actual change (avoids a spurious toast on every settings save).
        let old_internet = state.rest.fg().session.as_ref().map(|s| s.settings.internet_mode);
        // BUG FIX: this save can reassign the Main role two ways at once — the
        // session-local `session_models` write below, AND the global
        // `config.models` write further down (b) — either of which can change
        // what THIS session's Main resolves to. Snapshot before, compare after
        // both writes land, and reset a now-stale `effort` iff the resolved
        // Main model actually swapped (never on a re-save of the same model,
        // and never for the many OTHER fields this same save also touches).
        let before_main = state.rest.main_identity_now();
        if let Some(sess) = state.rest.fg_mut().session.as_mut() {
            sess.settings.api_key = api_key;
            sess.settings.model = model;
            sess.settings.provider = provider;
            sess.settings.workdir = workdir_vec;
            sess.settings.awareness_enabled = awareness_enabled;
            sess.settings.awareness_inherit = awareness_inherit;
            sess.settings.awareness_model = awareness_model;
            sess.settings.awareness_provider = awareness_provider;
            // Harness settings ride along here too; like awareness they
            // don't affect the chat client (the classifier uses
            // `complete_with` per invocation), so no client rebuild is
            // keyed off them.
            sess.settings.classifier_enabled = classifier_enabled;
            sess.settings.classifier_model = classifier_model;
            sess.settings.classifier_provider = classifier_provider;
            sess.settings.allowed_folders = allowed_folders_vec;
            // Short-send kill switch: no client rebuild needed; the
            // shape() call reads this flag per-send.
            sess.settings.short_send_enabled = short_send_enabled;
            // Sliding-cache toggle: no client rebuild needed; a later
            // wave's summarization logic reads this flag per-send.
            sess.settings.sliding_cache = sliding_cache;
            // Bash-saving toggle: no client rebuild needed; the tool
            // context reads this flag per-spawn.
            sess.settings.bash_saving = bash_saving;
            // Coding autosave: GUI-only preference; no client rebuild needed.
            sess.settings.coding_autosave = coding_autosave;
            // Internet-mode toggle: no client rebuild needed; the tool
            // dispatch layer reads this flag per-request.
            sess.settings.internet_mode = internet_mode;
            // But DO refresh the system-prompt roster so any mode-gated agents
            // stay in sync on a mid-session mode change (rebuild reads in-memory
            // settings; nothing else here rebuilds).
            sess.rebuild_system();
            // Session-only models live in the per-session override layer,
            // never in the global config. Persisted via sess.save() below.
            sess.settings.session_models = session_model_entries;
        }
        // b) Apply global theme/accent + the provider/model catalogue and
        //    persist config.json in one write. Best-effort: a write failure
        //    surfaces to the status line but does not abort the rest of the
        //    save.
        state.rest.config.theme = theme;
        state.rest.config.accent = accent;
        state.rest.config.palette = palette;
        state.rest.config.providers = provider_conns;
        state.rest.config.models = model_entries;
        if let Err(e) = state.rest.config.save() {
            state.rest.fg_mut().status = format!("config save failed: {e}");
        }
        // c) Persist the session's settings.json.
        if let Some(sess) = state.rest.fg_mut().session.as_mut() {
            if let Err(e) = sess.save() {
                state.rest.fg_mut().status = format!("error: {e}");
            }
        }
        // c1) BUG FIX: both Main-affecting writes (a + b above) have landed and
        // persisted — now check whether the resolved Main model actually
        // swapped, and if so reset the stale `effort` (this call is its own
        // save, on top of (c)'s, but ONLY fires on an actual swap).
        state.rest.reset_effort_if_main_changed(before_main);
        // c2) Reindex the dir cache against the (possibly changed) workdirs.
        //     Spawns a background thread; non-blocking.
        let roots = state.rest.fg().session.as_ref().map(|s| s.workdirs());
        let dir_cache = state.rest.fg().dir_cache.clone();
        if let Some(r) = roots {
            crate::tool::dircache::reindex(r, dir_cache);
        }
        // d) Rename LAST, and only when the name actually changed and is
        //    non-empty. Doing it last means a rename failure can't lose the
        //    other drafts (they're already saved above).
        let needs_rename = state
            .rest
            .fg()
            .session
            .as_ref()
            .map(|s| !name.trim().is_empty() && name.trim() != s.name)
            .unwrap_or(false);
        if needs_rename {
            if let Some(sess) = state.rest.fg_mut().session.as_mut() {
                if let Err(e) = store::rename_session(sess, name.trim()) {
                    state.rest.fg_mut().status = format!("rename failed: {e}");
                }
            }
        }
        // e) No client rebuild: creds/model/provider are read per-call via
        //    `resolve_role`, so the existing keyless Arc serves the new
        //    settings on the next request. (This also keeps the cache-stable
        //    plan_word intact across a settings save.)
        // f) Brief status line flash when switching internet mode (shared
        //    helper so the Full-needs-install line matches the `/internet`
        //    and Ctrl+E paths exactly).  Only fires when the mode actually
        //    changed; it resets to "ready" on the next action.
        crate::app::runtime::commands::internet::flash_internet_feedback(
            state,
            old_internet,
            internet_mode,
        );
    }
    *state.mode_mut() = Mode::Chat;
    Ok(())
}

/// Handle `Action::SaveEffort`: persist the chosen effort level and return to
/// Chat. No client rebuild: effort is resolved per-call.
pub(super) fn handle_save_effort(choice: String, state: &mut AppState) -> Result<()> {
    // Store the chosen effort ("default" → empty = model default) and
    // persist. No client rebuild: effort is now resolved per-call (it flows
    // only into the streaming path via the Main route's `effort`), so the
    // existing keyless client applies the new directive on the next request
    // WITHOUT busting its cache-stable plan_word.
    let effort = if choice == "default" { String::new() } else { choice };
    if let Some(sess) = state.rest.fg_mut().session.as_mut() {
        sess.settings.effort = effort.clone();
        if let Err(e) = sess.save() {
            state.rest.fg_mut().status = format!("error: {e}");
        }
    }
    let label = if effort.is_empty() { "default" } else { &effort };
    state.rest.fg_mut().status = format!("effort: {label}");
    *state.mode_mut() = Mode::Chat;
    Ok(())
}

/// Handle `Action::FetchModelEndpoints`: spawn a background task to fetch the
/// per-model provider endpoints from the OpenRouter catalogue API.
pub(super) fn handle_fetch_model_endpoints(
    model_id: String,
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    // Spawn the per-model provider-endpoints fetch on a background task
    // (mirrors the advisory prompt-classifier spawn in `Action::Submit`):
    // open a fresh channel, stash its receiver (replacing any in-flight
    // older fetch — dropping that receiver is the desired stale-cancel),
    // and send one EndpointsLoaded / EndpointsError when the request
    // resolves. The drain in `run_loop` folds it into the modal.
    //
    // Endpoints-API gate: `list_model_endpoints` is an OpenRouter-only GET,
    // and only an OpenAI-compatible provider has it (an Anthropic-typed
    // provider has no equivalent catalogue endpoint). When the modal's
    // SELECTED provider isn't an OpenRouter OpenAI-compatible one, don't
    // fire a doomed request: resolve the modal to an EMPTY endpoints list
    // (the view renders "no providers found" / Auto-only routing) and clear
    // loading. This keeps non-OpenRouter + Anthropic providers from
    // spinning on a request that would 404/400.
    if !matches!(state.mode(), Mode::Settings(s) if s.mm_provider_has_endpoints_api()) {
        if let Mode::Settings(s) = state.mode_mut() {
            if let Some(m) = s.model_modal.as_mut() {
                m.endpoints = Some(Vec::new());
                m.endpoints_loading = false;
            }
        }
        return Ok(());
    }
    // No client, or no connection for the modal's OWN provider → nothing
    // to fetch against; clear the loading flag so the UI doesn't spin.
    // The endpoints GET must go against the EDITED MODEL's provider
    // connection (OpenRouter), NOT the Main role's connection (which may
    // be on a completely different provider). Pull (endpoint, api_key)
    // from `mm_provider_conn` and MOVE the owned Strings into the task
    // (no borrow of `state` crosses the spawn boundary).
    let (provider_conn, oauth_uuid) = if let Mode::Settings(s) = state.mode() {
        (s.mm_provider_conn(), s.mm_provider_oauth_uuid())
    } else {
        (None, String::new())
    };
    let (Some(c), Some((endpoint, api_key))) = (client.as_ref(), provider_conn) else {
        if let Mode::Settings(s) = state.mode_mut() {
            if let Some(m) = s.model_modal.as_mut() {
                m.endpoints_loading = false;
            }
        }
        return Ok(());
    };
    if endpoint.trim().is_empty() {
        if let Mode::Settings(s) = state.mode_mut() {
            if let Some(m) = s.model_modal.as_mut() {
                m.endpoints_loading = false;
            }
        }
        return Ok(());
    }
    let c = Arc::clone(c);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    state.rest.endpoints_rx = Some(rx);
    handle.spawn(async move {
        // A dropped receiver (modal closed / a newer fetch superseded
        // this one) makes the send a no-op — same contract as the
        // streaming + harness channels.
        let conn = crate::service::openrouter::Conn {
            endpoint: &endpoint,
            api_key: &api_key,
            // Endpoints fetch is OpenAI-compatible. `oauth_uuid` is threaded through
            // so an OAuth-backed catalogue refreshes its bearer via `fresh_key`;
            // empty for a static-key provider. `account_id` stays empty — a
            // catalogue GET needs no org header.
            api_type: crate::model::app_config::ApiType::OpenAiCompatible,
            account_id: "",
            oauth_uuid: &oauth_uuid,
            // Catalogue GET, never koma-free — no X-Koma header needed.
            install_id: "",
        };
        let _ = match c.list_model_endpoints(conn, &model_id).await {
            Ok(eps) => tx.send(crate::service::StreamEvent::EndpointsLoaded {
                model_id,
                endpoints: eps,
            }),
            Err(e) => tx.send(crate::service::StreamEvent::EndpointsError {
                model_id,
                error: e.to_string(),
            }),
        };
    });
    Ok(())
}
