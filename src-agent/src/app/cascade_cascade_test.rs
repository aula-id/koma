#![allow(clippy::unwrap_used, clippy::expect_used)]
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
    let settings = Settings {
        session_models: vec![
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
        ],
        ..Default::default()
    };
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
