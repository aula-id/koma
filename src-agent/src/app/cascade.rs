//! Cascade consumer rebind after catalogue model/provider removal.
//!
//! When a provider, oauth conn, or model is deleted from the global catalogue (or a
//! session-local model is dropped), every consumer that held the dead model uuid is
//! nudged back to **inherit main** (`model_uuid = None`) rather than left dangling as
//! `name @ ?`. Session-local model rows whose provider anchor vanished are dropped.
//!
//! Agent `.md` files (`~/.koma/agents/*.md` and `<session>/agents/*.md`) are rewritten
//! whenever their `model_uuid` is absent from the **live** catalogue (global models ∪
//! that session's `session_models`) — not only when the uuid is in the just-deleted set.
//! That way a missing model always falls back to inherit main on disk.
//!
//! Pure best-effort: a single bad agent/session file is logged and skipped.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::app::mode::Mode;
use crate::app::state::AppState;
use crate::model::agent_def::{load_agent_file, AgentSource};
use crate::model::app_config::AppConfig;
use crate::model::settings::Settings;
use crate::model::store;

/// Report from [`rebind_consumers_after_model_removal`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CascadeReport {
    pub models_removed: Vec<String>,
    pub agents_cleared: usize,
    pub sessions_touched: usize,
    pub main_reset: bool,
}

/// After models have been removed from config (and config saved or about to be),
/// rewrite every consumer of dead models back to **inherit main**. Also drops
/// session_models rows whose `provider_uuid` is in `dead_provider_uuids`.
///
/// `config` must already reflect the post-removal catalogue — agent `.md` files
/// whose `model_uuid` is not in `config.models` (∪ the relevant session's
/// `session_models`) are cleared to inherit even if that uuid was not listed in
/// `dead_model_uuids` (repairs pre-existing dangling bindings).
///
/// When `state` is `None` (pre-session host path), only the on-disk walk runs.
pub fn rebind_consumers_after_model_removal(
    state: Option<&mut AppState>,
    config: &AppConfig,
    dead_model_uuids: &HashSet<String>,
    dead_provider_uuids: &HashSet<String>,
    main_reset: bool,
) -> CascadeReport {
    if dead_model_uuids.is_empty() && dead_provider_uuids.is_empty() {
        return CascadeReport {
            models_removed: Vec::new(),
            agents_cleared: 0,
            sessions_touched: 0,
            main_reset,
        };
    }

    let alive_global = alive_model_set(config, None);

    let mut report = CascadeReport {
        models_removed: dead_model_uuids.iter().cloned().collect(),
        agents_cleared: 0,
        sessions_touched: 0,
        main_reset,
    };

    // Paths already handled in-memory — skip on the disk walk to avoid double-write races.
    let mut skip_session_paths: HashSet<PathBuf> = HashSet::new();
    // Per-session alive model sets (global ∪ session_models) after retain, for agent rebind.
    let mut session_alive: Vec<(PathBuf, HashSet<String>)> = Vec::new();

    if let Some(state) = state {
        // A. Open sessions (in-memory)
        for sess_rt in state.rest.sessions.iter_mut() {
            let Some(sess) = sess_rt.session.as_mut() else {
                continue;
            };
            let before = sess.settings.session_models.len();
            sess.settings.session_models.retain(|m| {
                !dead_model_uuids.contains(&m.uuid)
                    && !dead_provider_uuids.contains(&m.provider_uuid)
            });
            let models_changed = sess.settings.session_models.len() != before;
            if models_changed {
                if let Err(e) = sess.save() {
                    store::append_global_error_log(
                        "cascade",
                        &format!("save open session {}: {e:#}", sess.path.display()),
                    );
                } else {
                    report.sessions_touched += 1;
                }
            }
            let mut alive = alive_global.clone();
            for m in &sess.settings.session_models {
                alive.insert(m.uuid.clone());
            }
            session_alive.push((sess.path.clone(), alive));
            skip_session_paths.insert(sess.path.clone());
        }

        // C (in-memory). Agents mode draft + list, if open on any session.
        for sess_rt in state.rest.sessions.iter_mut() {
            let sess_path = sess_rt.session.as_ref().map(|s| s.path.clone());
            let alive_for_sess = sess_path
                .as_ref()
                .and_then(|p| {
                    session_alive
                        .iter()
                        .find(|(path, _)| path == p)
                        .map(|(_, a)| a)
                })
                .unwrap_or(&alive_global);

            if let Mode::Agents(agents) = &mut sess_rt.mode {
                if let Some(u) = agents.draft_model_uuid.as_ref() {
                    if model_binding_is_dead(u, dead_model_uuids, alive_for_sess) {
                        agents.draft_model_uuid = None;
                        agents.draft_model_legacy = None;
                    }
                }
                for a in agents.agents.iter_mut() {
                    if let Some(u) = a.model_uuid.as_ref() {
                        if model_binding_is_dead(u, dead_model_uuids, alive_for_sess) {
                            clear_agent_model_to_inherit(a);
                            report.agents_cleared += 1;
                        }
                    }
                }
            }
        }
    }

    // B. Offline sessions (settings.json only)
    report.sessions_touched += rebind_offline_sessions(
        dead_model_uuids,
        dead_provider_uuids,
        &skip_session_paths,
    );

    // C. Agent files on disk → inherit main when model no longer exists
    report.agents_cleared += rebind_agent_files(
        dead_model_uuids,
        &alive_global,
        None,
        &session_alive,
    );

    report
}

/// Scoped rebind for a session-local model delete: that session's agents dir +
/// in-memory agents. A session-local model uuid is only meaningful inside that session.
pub fn rebind_after_local_model_removal(
    state: &mut AppState,
    config: &AppConfig,
    session_path: &Path,
    dead_model_uuid: &str,
) -> CascadeReport {
    let mut dead_models: HashSet<String> = HashSet::new();
    dead_models.insert(dead_model_uuid.to_string());

    let mut report = CascadeReport {
        models_removed: vec![dead_model_uuid.to_string()],
        agents_cleared: 0,
        sessions_touched: 0,
        main_reset: false,
    };

    let mut alive = alive_model_set(config, None);
    // Fold this session's remaining session_models into alive.
    for sess_rt in state.rest.sessions.iter() {
        if let Some(sess) = sess_rt.session.as_ref() {
            if sess.path == session_path {
                for m in &sess.settings.session_models {
                    alive.insert(m.uuid.clone());
                }
                break;
            }
        }
    }

    // In-memory agents mode for this session.
    for sess_rt in state.rest.sessions.iter_mut() {
        let Some(sess) = sess_rt.session.as_ref() else {
            continue;
        };
        if sess.path != session_path {
            continue;
        }
        if let Mode::Agents(agents) = &mut sess_rt.mode {
            if let Some(u) = agents.draft_model_uuid.as_ref() {
                if model_binding_is_dead(u, &dead_models, &alive) {
                    agents.draft_model_uuid = None;
                    agents.draft_model_legacy = None;
                }
            }
            for a in agents.agents.iter_mut() {
                if let Some(u) = a.model_uuid.as_ref() {
                    if model_binding_is_dead(u, &dead_models, &alive) {
                        clear_agent_model_to_inherit(a);
                        report.agents_cleared += 1;
                    }
                }
            }
        }
    }

    // Disk: only this session's agents/ — inherit main when model missing.
    let session_alive = vec![(session_path.to_path_buf(), alive.clone())];
    report.agents_cleared += rebind_agent_files(
        &dead_models,
        &alive_model_set(config, None),
        Some(session_path),
        &session_alive,
    );
    report
}

/// True when `uuid` should be cleared to inherit: explicitly dead, or absent from alive.
fn model_binding_is_dead(
    uuid: &str,
    dead: &HashSet<String>,
    alive: &HashSet<String>,
) -> bool {
    dead.contains(uuid) || !alive.contains(uuid)
}

/// Strip model binding fields so the agent inherits Main.
fn clear_agent_model_to_inherit(agent: &mut crate::model::agent_def::AgentDef) {
    agent.model_uuid = None;
    agent.model = None;
    agent.provider_uuid = None;
    agent.provider = None;
}

fn alive_model_set(config: &AppConfig, extra: Option<&[crate::model::app_config::ModelEntry]>) -> HashSet<String> {
    let mut s: HashSet<String> = config.models.iter().map(|m| m.uuid.clone()).collect();
    if let Some(extra) = extra {
        for m in extra {
            s.insert(m.uuid.clone());
        }
    }
    s
}

/// Walk every offline session's `settings.json` and drop dead session_models rows.
fn rebind_offline_sessions(
    dead_models: &HashSet<String>,
    dead_providers: &HashSet<String>,
    skip: &HashSet<PathBuf>,
) -> usize {
    let Ok(sessions_root) = store::sessions_dir() else {
        return 0;
    };
    let mut touched = 0;
    for bucket in read_subdirs(&sessions_root) {
        for session_dir in read_subdirs(&bucket) {
            if skip.contains(&session_dir) {
                continue;
            }
            let settings_path = session_dir.join("settings.json");
            if !settings_path.exists() {
                continue;
            }
            let mut settings = match Settings::load(&settings_path) {
                Ok(s) => s,
                Err(e) => {
                    store::append_global_error_log(
                        "cascade",
                        &format!("load {}: {e:#}", settings_path.display()),
                    );
                    continue;
                }
            };
            let before = settings.session_models.len();
            settings.session_models.retain(|m| {
                !dead_models.contains(&m.uuid) && !dead_providers.contains(&m.provider_uuid)
            });
            if settings.session_models.len() == before {
                continue;
            }
            match settings.save(&settings_path) {
                Ok(()) => touched += 1,
                Err(e) => store::append_global_error_log(
                    "cascade",
                    &format!("save {}: {e:#}", settings_path.display()),
                ),
            }
        }
    }
    touched
}

/// Rewrite agent `.md` files so a missing/dead `model_uuid` becomes inherit main (`None`).
///
/// - `session_scope = None`: global agents + every session's `agents/`.
/// - `session_scope = Some(path)`: only `<path>/agents/`.
///
/// For session agents, alive = that session's entry in `session_alive` if present,
/// else `alive_global` plus models loaded from that session's `settings.json`.
fn rebind_agent_files(
    dead_models: &HashSet<String>,
    alive_global: &HashSet<String>,
    session_scope: Option<&Path>,
    session_alive: &[(PathBuf, HashSet<String>)],
) -> usize {
    let mut cleared = 0;

    match session_scope {
        Some(session_dir) => {
            let alive = session_alive
                .iter()
                .find(|(p, _)| p == session_dir)
                .map(|(_, a)| a.clone())
                .unwrap_or_else(|| {
                    let mut a = alive_global.clone();
                    extend_alive_from_session_settings(&mut a, session_dir);
                    a
                });
            cleared += rebind_agents_in_dir(
                &session_dir.join("agents"),
                AgentSource::Session,
                dead_models,
                &alive,
            );
        }
        None => {
            if let Ok(dir) = crate::model::agent_def::global_agents_dir() {
                cleared += rebind_agents_in_dir(
                    &dir,
                    AgentSource::Global,
                    dead_models,
                    alive_global,
                );
            }
            if let Ok(sessions_root) = store::sessions_dir() {
                for bucket in read_subdirs(&sessions_root) {
                    for session_dir in read_subdirs(&bucket) {
                        let alive = session_alive
                            .iter()
                            .find(|(p, _)| p == &session_dir)
                            .map(|(_, a)| a.clone())
                            .unwrap_or_else(|| {
                                let mut a = alive_global.clone();
                                extend_alive_from_session_settings(&mut a, &session_dir);
                                a
                            });
                        cleared += rebind_agents_in_dir(
                            &session_dir.join("agents"),
                            AgentSource::Session,
                            dead_models,
                            &alive,
                        );
                    }
                }
            }
        }
    }
    cleared
}

fn extend_alive_from_session_settings(alive: &mut HashSet<String>, session_dir: &Path) {
    let path = session_dir.join("settings.json");
    if let Ok(settings) = Settings::load(&path) {
        for m in settings.session_models {
            alive.insert(m.uuid);
        }
    }
}

fn rebind_agents_in_dir(
    dir: &Path,
    source: AgentSource,
    dead_models: &HashSet<String>,
    alive_models: &HashSet<String>,
) -> usize {
    if !dir.is_dir() {
        return 0;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    let mut cleared = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let mut agent = match load_agent_file(&path, source) {
            Ok(a) => a,
            Err(e) => {
                store::append_global_error_log(
                    "cascade",
                    &format!("load agent {}: {e:#}", path.display()),
                );
                continue;
            }
        };
        let Some(u) = agent.model_uuid.as_ref() else {
            continue;
        };
        if !model_binding_is_dead(u, dead_models, alive_models) {
            continue;
        }
        // Model no longer exists → inherit main.
        clear_agent_model_to_inherit(&mut agent);
        match std::fs::write(&path, agent.to_markdown()) {
            Ok(()) => cleared += 1,
            Err(e) => store::append_global_error_log(
                "cascade",
                &format!("rewrite agent {}: {e:#}", path.display()),
            ),
        }
    }
    cleared
}

fn read_subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.push(p);
            }
        }
    }
    out
}

/// Format a short status/toast line for a cascade outcome.
pub fn cascade_status_line(label: &str, report: &CascadeReport) -> String {
    let n = report.models_removed.len();
    let m = report.agents_cleared;
    let mut s = format!("removed {label} · {n} models · {m} agents → inherit main");
    if report.main_reset {
        s.push_str(" · main model reset");
    }
    s
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod cascade_test {
    use super::*;
    use crate::model::agent_def::AgentDef;
    use crate::model::app_config::ModelEntry;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Unique scratch dir under std temp (no `tempfile` crate dependency).
    fn scratch(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("koma-cascade-{label}-{nanos}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_agent(dir: &Path, name: &str, model_uuid: Option<&str>) {
        fs::create_dir_all(dir).unwrap();
        let body = AgentDef {
            name: name.to_string(),
            description: "test".into(),
            model_uuid: model_uuid.map(str::to_string),
            model: Some("slug/x".into()),
            provider_uuid: Some("prov".into()),
            prompt: "hi".into(),
            ..Default::default()
        };
        fs::write(dir.join(format!("{name}.md")), body.to_markdown()).unwrap();
    }

    #[test]
    fn agent_md_with_dead_model_uuid_rewritten_to_inherit() {
        let tmp = scratch("agent");
        let agents = tmp.join("agents");
        write_agent(&agents, "explore", Some("dead-model"));
        write_agent(&agents, "general", Some("keep-model"));

        let mut dead = HashSet::new();
        dead.insert("dead-model".into());
        let mut alive = HashSet::new();
        alive.insert("keep-model".into());
        let n = rebind_agents_in_dir(&agents, AgentSource::Global, &dead, &alive);
        assert_eq!(n, 1);

        let cleared = load_agent_file(&agents.join("explore.md"), AgentSource::Global).unwrap();
        assert!(cleared.model_uuid.is_none(), "must inherit main");
        assert!(cleared.model.is_none());
        assert!(cleared.provider_uuid.is_none());

        let kept = load_agent_file(&agents.join("general.md"), AgentSource::Global).unwrap();
        assert_eq!(kept.model_uuid.as_deref(), Some("keep-model"));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn agent_md_missing_from_alive_catalogue_inherits_even_without_dead_set() {
        // The model was already gone before this cascade run — still rewrite to inherit.
        let tmp = scratch("orphan");
        let agents = tmp.join("agents");
        write_agent(&agents, "explore", Some("already-gone"));

        let dead = HashSet::new(); // empty dead set
        let alive = HashSet::new(); // nothing alive
        let n = rebind_agents_in_dir(&agents, AgentSource::Global, &dead, &alive);
        assert_eq!(n, 1, "orphan model_uuid must clear to inherit main");
        let cleared = load_agent_file(&agents.join("explore.md"), AgentSource::Global).unwrap();
        assert!(cleared.model_uuid.is_none());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn session_settings_drops_dead_provider_and_model_rows() {
        let tmp = scratch("sess");
        let sess = tmp.join("bucket").join("sid");
        fs::create_dir_all(&sess).unwrap();
        let mut settings = Settings::default();
        settings.session_models = vec![
            ModelEntry {
                uuid: "m-dead".into(),
                provider_uuid: "p-alive".into(),
                ..Default::default()
            },
            ModelEntry {
                uuid: "m-alive".into(),
                provider_uuid: "p-dead".into(),
                ..Default::default()
            },
            ModelEntry {
                uuid: "m-ok".into(),
                provider_uuid: "p-ok".into(),
                ..Default::default()
            },
        ];
        let path = sess.join("settings.json");
        settings.save(&path).unwrap();

        let mut dead_models: HashSet<String> = HashSet::new();
        dead_models.insert("m-dead".into());
        let mut dead_providers: HashSet<String> = HashSet::new();
        dead_providers.insert("p-dead".into());

        let mut s = Settings::load(&path).unwrap();
        s.session_models.retain(|m| {
            !dead_models.contains(&m.uuid) && !dead_providers.contains(&m.provider_uuid)
        });
        s.save(&path).unwrap();

        let loaded = Settings::load(&path).unwrap();
        assert_eq!(loaded.session_models.len(), 1);
        assert_eq!(loaded.session_models[0].uuid, "m-ok");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn scoped_session_agent_rebind_only_touches_that_session() {
        let tmp = scratch("scope");
        let sess_a = tmp.join("a");
        let sess_b = tmp.join("b");
        write_agent(&sess_a.join("agents"), "explore", Some("local-m"));
        write_agent(&sess_b.join("agents"), "explore", Some("local-m"));

        let mut dead = HashSet::new();
        dead.insert("local-m".into());
        // Only sess_a is in scope; alive global is empty so local-m is dead there.
        let alive_global = HashSet::new();
        let session_alive = vec![(sess_a.clone(), HashSet::new())];
        let n = rebind_agent_files(&dead, &alive_global, Some(&sess_a), &session_alive);
        assert_eq!(n, 1);

        let a = load_agent_file(&sess_a.join("agents/explore.md"), AgentSource::Session).unwrap();
        assert!(a.model_uuid.is_none(), "sess_a must inherit main");
        let b = load_agent_file(&sess_b.join("agents/explore.md"), AgentSource::Session).unwrap();
        assert_eq!(b.model_uuid.as_deref(), Some("local-m"), "sess_b untouched");
        let _ = fs::remove_dir_all(&tmp);
    }
}
