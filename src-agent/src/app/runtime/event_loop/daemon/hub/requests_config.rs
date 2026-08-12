//! Config-mutation arm bodies for [`super::core::DaemonHub`] — split out of
//! `requests.rs` for file size (pure code motion, no behaviour change). Every
//! method here is called from `requests.rs`'s `handle_controller_mutation` match,
//! one method per moved `ClientRequest` variant, taking exactly the parameters the
//! original arm body used (never `client` — none of these arms touch the model
//! client). The three MCP arms (`set_mcp_server`/`delete_mcp_server`/
//! `enable_mcp_server`) DO take `handle`: `save_and_reload_mcp` needs it to
//! construct `mcp_manager` on demand when the daemon booted with zero MCP servers
//! (see `actions::mcp::ensure_mcp_manager`).

use crate::app::state::AppState;

use super::core::DaemonHub;

impl DaemonHub {
    // GUI MCP CRUD (McpPanel). Build an `McpServerEntry` from the panel's form
    // (mapping the single-line args/env STRING forms into the daemon's array/pair
    // forms via the SAME `parse_args`/`parse_env` the TUI editor uses), upsert it
    // into `config.mcp_servers` by uuid (a `None`/empty uuid mints a new one), then
    // persist + live-reconnect the MCP manager via the mode-independent
    // `save_and_reload_mcp`. Any client may drive this (config is global; the C2
    // bracket is irrelevant here).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn set_mcp_server(
        &mut self,
        idx: usize,
        state: &mut AppState,
        handle: &tokio::runtime::Handle,
        uuid: Option<String>,
        name: String,
        enabled: bool,
        transport: String,
        command: String,
        args: String,
        env: String,
        url: String,
    ) {
        let entry = crate::model::app_config::McpServerEntry {
            uuid: uuid.unwrap_or_default(),
            name: name.trim().to_string(),
            enabled,
            transport: if transport == "http" {
                crate::model::app_config::McpTransport::Http
            } else {
                crate::model::app_config::McpTransport::Stdio
            },
            command: command.trim().to_string(),
            args: crate::app::mode::mcp::parse_args(&args),
            env: crate::app::mode::mcp::parse_env(&env),
            url: url.trim().to_string(),
            // A GUI-configured server is user-owned — no extension provenance.
            ext_id: None,
        };
        state.rest.config.upsert_mcp_server(entry);
        let result = crate::app::runtime::actions::save_and_reload_mcp(state, handle);
        self.ack_or_error(idx, result);
    }

    // GUI MCP delete: drop the server by uuid, persist + live-reconnect.
    pub(super) fn delete_mcp_server(
        &mut self,
        idx: usize,
        state: &mut AppState,
        handle: &tokio::runtime::Handle,
        uuid: String,
    ) {
        state.rest.config.remove_mcp_server_by_uuid(&uuid);
        let result = crate::app::runtime::actions::save_and_reload_mcp(state, handle);
        self.ack_or_error(idx, result);
    }

    // GUI MCP enable toggle: set the `enabled` flag by uuid, persist + reconnect.
    pub(super) fn enable_mcp_server(
        &mut self,
        idx: usize,
        state: &mut AppState,
        handle: &tokio::runtime::Handle,
        uuid: String,
        enabled: bool,
    ) {
        state.rest.config.set_mcp_enabled_by_uuid(&uuid, enabled);
        let result = crate::app::runtime::actions::save_and_reload_mcp(state, handle);
        self.ack_or_error(idx, result);
    }

    // GUI provider CRUD (Connector ProviderForm). Upsert by uuid via the
    // config-layer setter (preserving wire type on edit, minting OpenAI-compatible
    // on create), then persist. Config-global; any client may drive it.
    pub(super) fn set_provider(
        &mut self,
        idx: usize,
        state: &mut AppState,
        uuid: Option<String>,
        name: String,
        endpoint: String,
        api_key: String,
    ) {
        state.rest.config.upsert_provider(
            uuid,
            name.trim().to_string(),
            endpoint.trim().to_string(),
            api_key,
        );
        let result = crate::app::runtime::actions::save_config_and_broadcast(&state.rest.config);
        self.ack_or_error(idx, result);
    }

    // GUI provider delete: cascade-drop models that pointed at it, rebind consumers
    // (agents/sessions → inherit), persist.
    //
    // W12b HOST-ENFORCED GUARD: an EXTENSION-managed key-backed provider
    // (`ProviderConn::ext_id` set) can never be deleted by the user — only uninstalling the
    // owning extension removes it. Reject with a structured `DaemonEvent::Error` (the shape
    // `ack_or_error` replies) so no client-side edit can orphan an extension's gateway.
    pub(super) fn delete_provider(&mut self, idx: usize, state: &mut AppState, uuid: String) {
        if let Some(ext) = state
            .rest
            .config
            .providers
            .iter()
            .find(|p| p.uuid == uuid)
            .and_then(|p| p.ext_id.as_deref())
        {
            // Reuse the same `DaemonEvent::Error` reply shape as the success/failure ack.
            self.ack_or_error(
                idx,
                Err(anyhow::anyhow!(
                    "managed by extension {ext} — uninstall to remove"
                )),
            );
            return;
        }
        let purge = state.rest.config.cascade_remove_provider(&uuid);
        let result = crate::app::runtime::actions::save_config_and_broadcast(&state.rest.config);
        // Always rebind agent .md → inherit main when a provider went away (heals
        // orphans even if this provider had zero catalogue models).
        if result.is_ok() {
            use std::collections::HashSet;
            let dead_models: HashSet<String> = purge.models_removed.iter().cloned().collect();
            let mut dead_providers = HashSet::new();
            dead_providers.insert(uuid.clone());
            let cfg = state.rest.config.clone();
            let report = crate::app::cascade::rebind_consumers_after_model_removal(
                Some(state),
                &cfg,
                &dead_models,
                &dead_providers,
                purge.main_reset,
            );
            if !purge.models_removed.is_empty() || report.agents_cleared > 0 || purge.main_reset {
                state
                    .rest
                    .fg_mut()
                    .set_toast_info(crate::app::cascade::cascade_status_line(
                        "provider", &report,
                    ));
            }
        }
        self.ack_or_error(idx, result);
    }

    // GUI model CRUD (Connector ModelForm). Build a `ModelEntry` (parsing the
    // lowercase role tokens; an empty `route` → `None`), then upsert with per-scope
    // role-steal into either the GLOBAL catalogue (`config.models`, persisted via
    // `config.save`) or the foreground session's LOCAL override layer
    // (`settings.session_models`, persisted via `sess.save`). The two scopes keep
    // the role invariant independently — same split the TUI Settings save uses.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn set_model(
        &mut self,
        idx: usize,
        state: &mut AppState,
        uuid: Option<String>,
        name: String,
        model_id: String,
        provider_uuid: String,
        route: Option<String>,
        roles: Vec<String>,
        scope: String,
    ) {
        let roles = roles.iter().filter_map(|r| parse_model_role(r)).collect();
        let entry = crate::model::app_config::ModelEntry {
            uuid: uuid.unwrap_or_default(),
            name: name.trim().to_string(),
            model_id: model_id.trim().to_string(),
            provider_uuid,
            route: crate::model::app_config::ModelEntry::normalize_route(route),
            roles,
            role: None,
            // A directly-authored ModelForm entry, not a clone of a global — no source.
            source_uuid: None,
        };
        // BUG FIX: an upsert that claims the Main role steals it from whatever
        // OTHER entry held it (`upsert_model_entry`'s role-invariant, or the
        // global-scope equivalent) — which can change what THIS session's Main
        // resolves to even though this request never touches `session_models`
        // directly (a global-scope edit changes it too, when the session has no
        // local override). Snapshot before the upsert, compare after, and reset
        // a now-stale `effort` iff the resolved Main model actually swapped.
        let before_main = state.rest.main_identity_now();
        let result = if scope == "local" {
            if let Some(sess) = state.rest.fg_mut().session.as_mut() {
                crate::model::app_config::upsert_model_entry(
                    &mut sess.settings.session_models,
                    entry,
                );
                sess.save()
            } else {
                Ok(()) // no foreground session to hold a local override
            }
        } else {
            // BUG FIX (parity with the TUI settings modal's `save_model_modal`
            // directional steal, PR#83 / commit f1c500f): `config.upsert_model`
            // only steals the claimed roles from OTHER entries within
            // `config.models` — it never touches `session_models`. But
            // `resolve_role` checks a session's LOCAL overrides FIRST, so a
            // session-local entry that already holds one of these roles would
            // keep shadowing the new global assignment forever. Strip the
            // claimed roles from every entry in the foreground session's local
            // overrides too (both the `roles` vec and the legacy `role` field,
            // via `strip_role`, so an old-format entry can't keep shadowing via
            // its legacy field — mirrors `upsert_model_entry`'s own same-scope
            // steal), then persist that session alongside the config. Entries
            // left with zero roles are kept in place (not removed) — same as
            // `strip_role`/`upsert_model_entry` and `save_model_modal` — so
            // stripping can only ever narrow what an entry claims, never widen it.
            let claimed = entry.effective_roles();
            let session_result = if let Some(sess) = state.rest.fg_mut().session.as_mut() {
                for other in sess.settings.session_models.iter_mut() {
                    for role in &claimed {
                        crate::model::app_config::strip_role(other, *role);
                    }
                }
                sess.save()
            } else {
                Ok(()) // no foreground session to hold a local override
            };
            state.rest.config.upsert_model(entry);
            let config_result =
                crate::app::runtime::actions::save_config_and_broadcast(&state.rest.config);
            session_result.and(config_result)
        };
        state.rest.reset_effort_if_main_changed(before_main);
        self.ack_or_error(idx, result);
    }

    // GUI model delete: remove by uuid from the addressed scope, rebind consumers → inherit,
    // persist.
    pub(super) fn delete_model(
        &mut self,
        idx: usize,
        state: &mut AppState,
        uuid: String,
        scope: String,
    ) {
        use std::collections::HashSet;
        let result = if scope == "local" {
            if let Some(sess) = state.rest.fg_mut().session.as_mut() {
                let path = sess.path.clone();
                sess.settings.session_models.retain(|m| m.uuid != uuid);
                let save = sess.save();
                if save.is_ok() {
                    let cfg = state.rest.config.clone();
                    let _ = crate::app::cascade::rebind_after_local_model_removal(
                        state, &cfg, &path, &uuid,
                    );
                }
                save
            } else {
                Ok(())
            }
        } else {
            let mut dead = HashSet::new();
            dead.insert(uuid.clone());
            let purge = state.rest.config.cascade_remove_models(&dead);
            let save = crate::app::runtime::actions::save_config_and_broadcast(&state.rest.config);
            if save.is_ok() && !purge.models_removed.is_empty() {
                let dead_models: HashSet<String> = purge.models_removed.iter().cloned().collect();
                let empty = HashSet::new();
                let cfg = state.rest.config.clone();
                let report = crate::app::cascade::rebind_consumers_after_model_removal(
                    Some(state),
                    &cfg,
                    &dead_models,
                    &empty,
                    purge.main_reset,
                );
                if purge.main_reset || report.agents_cleared > 0 {
                    state
                        .rest
                        .fg_mut()
                        .set_toast_info(crate::app::cascade::cascade_status_line("model", &report));
                }
            }
            save
        };
        self.ack_or_error(idx, result);
    }

    // GUI theme picker (onboarding step 1 + the future Settings gear): set the
    // active palette registry key + persist. Config-global; any client may drive
    // it. Only `config.palette` (the live theme key) is touched — the deprecated
    // `theme`/`accent` legacy fields are left as-is. The palette change is picked
    // up by the snapshot diff (`ipc::snapshot::diff` gates a full snapshot on
    // `palette`), so the GUI host re-derives + re-pushes its Config palette live.
    pub(super) fn set_theme(&mut self, idx: usize, state: &mut AppState, name: String) {
        state.rest.config.palette = name;
        let result = crate::app::runtime::actions::save_config_and_broadcast(&state.rest.config);
        self.ack_or_error(idx, result);
    }

    // GUI Settings tab (Session section): partial-update the foreground session's
    // GUI-editable prefs. Only the `Some` fields are applied, EACH through the SAME
    // per-field apply logic the TUI settings save uses
    // (`actions::settings::handle_save_settings`):
    //   - short-send / sliding-cache / bash-saving: plain field sets (:185-191) — no
    //     client rebuild needed (each flag is read per-send / per-spawn).
    //   - internet_mode: capture-old + set + the SHARED `flash_internet_feedback`
    //     (status line + optional install toast, only on an actual change) — the exact
    //     helper the settings save calls (:194 + feedback path).
    //   - workdir: normalized (trim + drop empties + cwd fallback, :84-101) then a
    //     dir-cache reindex (:221-227).
    // The C2 LOAD bracket already pointed `fg()` at this client's session. After
    // applying, `rebuild_system` refreshes the mode-gated roster + `sess.save()`
    // persists (mirrors :198/:216), then a fresh `SettingsValues` is re-pushed so the
    // GUI reflects reality, and the request is acked.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn set_session_prefs(
        &mut self,
        idx: usize,
        state: &mut AppState,
        short_send: Option<bool>,
        sliding_cache: Option<bool>,
        bash_saving: Option<bool>,
        coding_autosave: Option<bool>,
        internet_mode: Option<String>,
        workdir: Option<Vec<String>>,
        subagent_max_turns: Option<u32>,
    ) {
        use crate::model::settings::InternetMode;
        // Capture the old internet mode BEFORE the set, for the shared change-gated
        // feedback below (mirrors handle_save_settings' `old_internet`).
        let old_internet = state
            .rest
            .fg()
            .session
            .as_ref()
            .map(|s| s.settings.internet_mode);
        let internet_target = internet_mode.as_deref().and_then(InternetMode::from_token);
        // Normalize the workdir draft exactly like actions/settings.rs:84-101.
        let workdir_vec = workdir.map(|dirs| {
            let mut v: Vec<String> = dirs
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if v.is_empty() {
                v = std::env::current_dir()
                    .map(|p| vec![p.display().to_string()])
                    .unwrap_or_default();
            }
            v
        });
        // Only a workdir change needs a dir-cache reindex (a full workspace
        // re-walk) — so gate it below rather than firing it on every toggle.
        let did_workdir = workdir_vec.is_some();
        // Snapshot the old workdirs BEFORE the set so we can detect an actual
        // change and trigger linker registration only when the roots differ.
        #[cfg(feature = "linker")]
        let old_workdirs: Vec<std::path::PathBuf> = state
            .rest
            .fg()
            .session
            .as_ref()
            .map(|s| s.workdirs())
            .unwrap_or_default();
        #[cfg(feature = "linker")]
        let mut session_save_ok = true;
        if let Some(sess) = state.rest.fg_mut().session.as_mut() {
            if let Some(v) = short_send {
                sess.settings.short_send_enabled = v;
            }
            if let Some(v) = sliding_cache {
                sess.settings.sliding_cache = v;
            }
            if let Some(v) = bash_saving {
                sess.settings.bash_saving = v;
            }
            if let Some(v) = coding_autosave {
                sess.settings.coding_autosave = v;
            }
            if let Some(m) = internet_target {
                sess.settings.internet_mode = m;
            }
            if let Some(v) = workdir_vec {
                sess.settings.workdir = v;
            }
            if let Some(v) = subagent_max_turns {
                sess.settings.subagent_max_turns = v.max(1);
            }
            // Refresh the mode-gated system-prompt roster, then persist — mirrors
            // handle_save_settings (:198 rebuild + :216 save). A save error just
            // leaves the on-disk file stale; the `SettingsValues` re-push below still
            // reflects the in-memory state, and this GUI path has no Ack/Error channel
            // to surface it to (the store ignores those frames).
            sess.rebuild_system();
            if sess.save().is_err() {
                #[cfg(feature = "linker")]
                {
                    session_save_ok = false;
                }
            }
        }
        // internet feedback (status + optional install toast) only on an actual
        // change — the SAME shared helper the TUI settings save uses.
        if let Some(m) = internet_target {
            crate::app::runtime::commands::internet::flash_internet_feedback(
                state,
                old_internet,
                m,
            );
        }
        if did_workdir {
            // BUG FIX: the workdir write above may have just dropped the dir a
            // live `/cd` (`active_cwd`) points at — leaving effective_cwd outside
            // every allowed root, which would WC-deny every subsequent tool spawn
            // until a manual `/cd`. Clamp it back to the primary workdir (`None`)
            // when that happens, regardless of harness mode (mirrors
            // actions/settings.rs's `handle_save_settings`).
            let launch_dir = state.rest.launch_dir.clone();
            state.rest.fg_mut().clamp_active_cwd(&launch_dir);
            // Reindex the dir cache against the changed workdirs (actions/settings.rs:
            // 221-227), but ONLY when workdir was actually part of this update — a
            // toggle/internet change never touches the workspace roots.
            let roots = state.rest.fg().session.as_ref().map(|s| s.workdirs());
            let dir_cache = state.rest.fg().dir_cache.clone();
            if let Some(r) = roots {
                crate::tool::dircache::reindex(r, dir_cache);
            }
            // Linker: register changed workdirs on a background thread so
            // blocking IPC never stalls the event loop. Only fires when the
            // persisted workdirs actually differ from the snapshot taken
            // above AND the session save succeeded.  Revision is allocated
            // synchronously here so the background worker carries a
            // deterministic revision that the daemon uses to reject stale
            // out-of-order registrations.
            #[cfg(feature = "linker")]
            if session_save_ok {
                let new_workdirs = state
                    .rest
                    .fg()
                    .session
                    .as_ref()
                    .map(|s| s.workdirs())
                    .unwrap_or_default();
                if new_workdirs != old_workdirs {
                    let session_id = state.rest.fg().id.clone();
                    let revision = crate::linker::client::next_registration_revision(&session_id);
                    std::thread::Builder::new()
                        .name("linker-register".to_string())
                        .spawn(move || {
                            let _ = crate::linker::client::ensure_and_register_with_revision(
                                &new_workdirs,
                                &session_id,
                                revision,
                            );
                        })
                        .ok();
                }
            }
        }
        // The `SettingsValues` re-push IS the reply (one-shot framing, like
        // ListModels / ListRoutes / GetSettings) — the store has no Ack/Error case,
        // so an extra Ack frame would only burn a per-client seq slot.
        self.send_settings_values(idx, state);
    }

    // GUI composer EFFORT picker pick: persist the chosen effort level with the
    // SAME field-level sanitization `handle_save_effort` applies ("default" ->
    // empty = model default) — but mutate the session field DIRECTLY rather than
    // going through `Action::SaveEffort`, because that action ALSO does
    // `*state.mode_mut() = Mode::Chat` at the end. `Mode` is per-SESSION, so
    // routing through it would silently kick any OTHER client viewing this
    // session (TUI in Settings/Agents/an approval, or its own `/effort` picker)
    // back to Chat — exactly the bug `SetModel`/`SetSessionPrefs` avoid by
    // replicating field effects directly instead of calling a mode-mutating
    // action. No client rebuild needed: effort is resolved per-call. The C2 LOAD
    // bracket already pointed `fg()` at this client's session. Reply framing
    // mirrors `SetSessionPrefs`: a fresh `SettingsValues` re-push IS the reply
    // (the effort-picker label rides the same settings channel), not a bare Ack.
    pub(super) fn set_effort(&mut self, idx: usize, state: &mut AppState, effort: String) {
        let effort = if effort == "default" {
            String::new()
        } else {
            effort
        };
        if let Some(sess) = state.rest.fg_mut().session.as_mut() {
            sess.settings.effort = effort;
            let _ = sess.save();
        }
        self.send_settings_values(idx, state);
    }

    // GUI onboarding "koma free": mint/reuse the keyless Koma Free provider + a
    // Main-role koma-free model in the GLOBAL config (the non-key equivalent of the
    // TUI's `Action::SetupKomaFree`), then persist. Only the CONFIG mutation is
    // shared with the TUI path (via `ensure_koma_free_config`) — the daemon owns no
    // first-run session-create / mode-switch here (a GUI session already exists on
    // this attached path). Config-global; any client may drive it. The config change
    // forces a full snapshot, so the GUI host re-pushes `Config` (clearing `firstRun`).
    pub(super) fn setup_koma_free(&mut self, idx: usize, state: &mut AppState) {
        crate::service::koma_free::ensure_koma_free_config(&mut state.rest.config);
        let result = crate::app::runtime::actions::save_config_and_broadcast(&state.rest.config);
        self.ack_or_error(idx, result);
    }

    // GUI model quick-picker: set (or clear) the foreground session's LOCAL Main
    // override. `Some(uuid)` CLONES the matching GLOBAL `config.models` entry into a
    // session-local Main `ModelEntry` (reusing an existing matching local override
    // rather than duplicating); `None` REMOVES the override (inherit the global
    // Main). Only `session_models` is touched — the global catalogue is untouched, so
    // the global Main resurfaces the instant the override is dropped. Mirrors the
    // `/free` clone-or-reuse path (`commands::free`). `resolve_role` scans
    // `session_models` first, so the change takes effect next turn.
    pub(super) fn set_session_main(
        &mut self,
        idx: usize,
        state: &mut AppState,
        model_uuid: Option<String>,
    ) {
        use crate::model::app_config::{new_uuid, ModelEntry, ModelRole};
        // BUG FIX: this whole request exists to reassign the session's Main
        // role (every branch below either pins, clones, or drops a local Main
        // override). Snapshot the resolved Main route BEFORE any branch runs,
        // so a stale `effort` (whose scale/support may not even apply to the
        // NEW model) gets reset back to model-default once we know whether the
        // model actually swapped — see `reset_effort_if_main_changed`.
        let before_main = state.rest.main_identity_now();
        // Free-pin (wave-3+4 D): the SYNTHETIC "advertised free" row carries the
        // dedicated `KOMA_FREE_SENTINEL` id (never a real `config.models` uuid), so
        // route it through the SAME `/free` find-or-create-and-pin flow the slash
        // command uses instead of the global-clone path below. Handled first so the
        // sentinel can never fall into the "unknown uuid" no-op.
        if model_uuid.as_deref() == Some(crate::service::koma_free::KOMA_FREE_SENTINEL) {
            let result = crate::app::runtime::commands::free::set_session_koma_free(state);
            state.rest.reset_effort_if_main_changed(before_main);
            self.ack_or_error(idx, result);
            return;
        }
        // Resolve + CLONE the chosen global entry first (owned) so the later
        // `fg_mut()` mutable borrow doesn't overlap the config read.
        let chosen = model_uuid.as_ref().and_then(|u| {
            state
                .rest
                .config
                .models
                .iter()
                .find(|m| &m.uuid == u)
                .cloned()
        });
        let result = if let Some(sess) = state.rest.fg_mut().session.as_mut() {
            if model_uuid.is_none() {
                // Inherit: drop any local Main override; the global Main resurfaces.
                sess.settings
                    .session_models
                    .retain(|e| !e.effective_roles().contains(&ModelRole::Main));
                sess.save()
            } else if let Some(chosen) = chosen {
                // Reuse: no-op ONLY when the current local Main override was cloned
                // from THIS exact global (source_uuid identity). Two globals that
                // share model_id+provider but differ by uuid/route (the user's XAI vs
                // grpk grok-4.5 twins) are DISTINCT picks — matching on
                // model_id+provider alone would wrongly no-op the switch, leaving the
                // old source_uuid + route pinned. A `None` source (a pre-identity or
                // koma-free override) never matches, so it's always replaced (and thus
                // upgraded to carry the source identity).
                let already = sess.settings.session_models.iter().any(|e| {
                    e.effective_roles().contains(&ModelRole::Main)
                        && e.source_uuid.as_deref() == Some(chosen.uuid.as_str())
                });
                if !already {
                    // Drop any OTHER local Main override (one local Main per scope),
                    // then push the cloned global entry as the new local Main.
                    sess.settings
                        .session_models
                        .retain(|e| !e.effective_roles().contains(&ModelRole::Main));
                    sess.settings.session_models.push(ModelEntry {
                        uuid: new_uuid(),
                        name: chosen.name.clone(),
                        model_id: chosen.model_id.clone(),
                        provider_uuid: chosen.provider_uuid.clone(),
                        route: chosen.route.clone(),
                        roles: vec![ModelRole::Main],
                        role: None,
                        // Remember EXACTLY which global this local Main override was
                        // cloned from, so the GUI ModelPicker can light that global's
                        // row by identity (source uuid) rather than a fuzzy name match.
                        source_uuid: Some(chosen.uuid.clone()),
                    });
                }
                sess.save()
            } else {
                // Unknown uuid (not in the global catalogue) — leave overrides as-is.
                Ok(())
            }
        } else {
            Ok(()) // no foreground session to hold a local override
        };
        state.rest.reset_effort_if_main_changed(before_main);
        self.ack_or_error(idx, result);
    }
}

/// Map a lowercase role token (`"main"`/`"awareness"`/`"safeguard"`/`"compactor"`/
/// `"planner"`) from the GUI `SetModel` request to its [`ModelRole`]. Unknown tokens
/// yield `None` and are dropped (a forgiving parse — the ModelForm only emits valid
/// tokens, but a version-skewed webview never crashes the daemon).
fn parse_model_role(s: &str) -> Option<crate::model::app_config::ModelRole> {
    use crate::model::app_config::ModelRole;
    match s {
        "main" => Some(ModelRole::Main),
        "awareness" => Some(ModelRole::Awareness),
        "safeguard" => Some(ModelRole::Safeguard),
        "compactor" => Some(ModelRole::Compactor),
        "planner" => Some(ModelRole::Planner),
        _ => None,
    }
}
