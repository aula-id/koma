//! Cascade consumer rebind after catalogue model/provider removal.
//!
//! When a provider, oauth conn, or model is deleted from the global catalogue (or a
//! session-local model is dropped), every consumer that held the dead model uuid is
//! nudged back to **inherit** (`model_uuid = None`) rather than left dangling as
//! `name @ ?`. Session-local model rows whose provider anchor vanished are dropped.
//!
//! Pure best-effort: a single bad agent/session file is logged and skipped.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::app::mode::Mode;
use crate::app::state::AppState;
use crate::model::agent_def::{load_agent_file, AgentSource};
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
/// rewrite every consumer of `dead_model_uuids` back to inherit. Also drops
/// session_models rows whose `provider_uuid` is in `dead_provider_uuids`.
///
/// When `state` is `None` (pre-session host path), only the on-disk walk runs.
pub fn rebind_consumers_after_model_removal(
    state: Option<&mut AppState>,
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

    let mut report = CascadeReport {
        models_removed: dead_model_uuids.iter().cloned().collect(),
        agents_cleared: 0,
        sessions_touched: 0,
        main_reset,
    };

    // Paths already handled in-memory — skip on the disk walk to avoid double-write races.
    let mut skip_session_paths: HashSet<PathBuf> = HashSet::new();

    if let Some(state) = state {
        // A. Open sessions (in-memory)
        for sess_rt in state.rest.sessions.iter_mut() {
            let Some(sess) = sess_rt.session.as_mut() else {
                continue;
            };
            let before = sess.settings.session_models.len();
            sess.settings.session_models.retain(|m| {
                !dead_model_uuids.contains(&m.uuid) && !dead_provider_uuids.contains(&m.provider_uuid)
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
            skip_session_paths.insert(sess.path.clone());
        }

        // C (in-memory). Agents mode draft + list, if open on any session.
        for sess_rt in state.rest.sessions.iter_mut() {
            if let Mode::Agents(agents) = &mut sess_rt.mode {
                if let Some(u) = agents.draft_model_uuid.as_ref() {
                    if dead_model_uuids.contains(u) {
                        agents.draft_model_uuid = None;
                    }
                }
                for a in agents.agents.iter_mut() {
                    if let Some(u) = a.model_uuid.as_ref() {
                        if dead_model_uuids.contains(u) {
                            a.model_uuid = None;
                            a.model = None;
                            a.provider_uuid = None;
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

    // C. Agent files on disk → inherit
    report.agents_cleared += rebind_agent_files(dead_model_uuids, None);

    report
}

/// Scoped rebind for a session-local model delete: only that session's agents dir +
/// in-memory agents for that session. Does not walk the global agents tree or other
/// sessions (a session-local model uuid is only meaningful inside that session).
pub fn rebind_after_local_model_removal(
    state: &mut AppState,
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

    // In-memory: the open session that matches this path (session_models already
    // retained by the caller) + its agents mode draft if open.
    for sess_rt in state.rest.sessions.iter_mut() {
        let Some(sess) = sess_rt.session.as_ref() else {
            continue;
        };
        if sess.path != session_path {
            continue;
        }
        if let Mode::Agents(agents) = &mut sess_rt.mode {
            if agents.draft_model_uuid.as_deref() == Some(dead_model_uuid) {
                agents.draft_model_uuid = None;
            }
            for a in agents.agents.iter_mut() {
                if a.model_uuid.as_deref() == Some(dead_model_uuid) {
                    a.model_uuid = None;
                    a.model = None;
                    a.provider_uuid = None;
                    report.agents_cleared += 1;
                }
            }
        }
    }

    // Disk: only this session's agents/
    report.agents_cleared += rebind_agent_files(&dead_models, Some(session_path));
    report
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

/// Rewrite agent `.md` files so dead `model_uuid` becomes inherit (`None`).
///
/// - `session_scope = None`: global agents + every session's `agents/`.
/// - `session_scope = Some(path)`: only `<path>/agents/`.
fn rebind_agent_files(dead_models: &HashSet<String>, session_scope: Option<&Path>) -> usize {
    let mut cleared = 0;

    match session_scope {
        Some(session_dir) => {
            cleared += rebind_agents_in_dir(
                &session_dir.join("agents"),
                AgentSource::Session,
                dead_models,
            );
        }
        None => {
            if let Ok(dir) = crate::model::agent_def::global_agents_dir() {
                cleared += rebind_agents_in_dir(&dir, AgentSource::Global, dead_models);
            }
            if let Ok(sessions_root) = store::sessions_dir() {
                for bucket in read_subdirs(&sessions_root) {
                    for session_dir in read_subdirs(&bucket) {
                        cleared += rebind_agents_in_dir(
                            &session_dir.join("agents"),
                            AgentSource::Session,
                            dead_models,
                        );
                    }
                }
            }
        }
    }
    cleared
}

fn rebind_agents_in_dir(
    dir: &Path,
    source: AgentSource,
    dead_models: &HashSet<String>,
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
        if !dead_models.contains(u) {
            continue;
        }
        agent.model_uuid = None;
        // Full clean: legacy slug/provider fields that went with the binding.
        agent.model = None;
        agent.provider_uuid = None;
        agent.provider = None;
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
    let mut s = format!("removed {label} · {n} models · {m} agents → inherit");
    if report.main_reset {
        s.push_str(" · main model reset");
    }
    s
}

#[cfg(test)]
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
        let n = rebind_agents_in_dir(&agents, AgentSource::Global, &dead);
        assert_eq!(n, 1);

        let cleared = load_agent_file(&agents.join("explore.md"), AgentSource::Global).unwrap();
        assert!(cleared.model_uuid.is_none());
        assert!(cleared.model.is_none());
        assert!(cleared.provider_uuid.is_none());

        let kept = load_agent_file(&agents.join("general.md"), AgentSource::Global).unwrap();
        assert_eq!(kept.model_uuid.as_deref(), Some("keep-model"));
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
        let n = rebind_agent_files(&dead, Some(&sess_a));
        assert_eq!(n, 1);

        let a = load_agent_file(&sess_a.join("agents/explore.md"), AgentSource::Session).unwrap();
        assert!(a.model_uuid.is_none());
        let b = load_agent_file(&sess_b.join("agents/explore.md"), AgentSource::Session).unwrap();
        assert_eq!(b.model_uuid.as_deref(), Some("local-m"));
        let _ = fs::remove_dir_all(&tmp);
    }
}
