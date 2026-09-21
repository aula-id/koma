use super::*;
use crate::model::app_config::{AppConfig, ExtensionActivation};

pub(crate) struct ExtensionFixture {
    pub ext: InstalledExtension,
    pub workspace: PathBuf,
    pub agent: String,
    package: PathBuf,
}

impl ExtensionFixture {
    pub fn new() -> Self {
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let id = format!("run.koma.scope-test-{suffix}");
        let package = store::extensions_dir().unwrap().join(&id);
        let workspace = dirs::home_dir()
            .unwrap()
            .join(format!(".scope-test-{suffix}"));
        let agent = format!("scope-test-{suffix}");
        std::fs::create_dir_all(&package).unwrap();
        let manifest = serde_json::json!({
            "schema": "koma-extension/v0", "id": id, "name": "Scope test",
            "version": "1.0.0", "tier": "free", "kind": "oneshot",
            "runtime": {"exec":"bin/test"}, "workspace_dir":workspace,
            "contributes": {"sub_agents":[{"name":agent,"description":"Extension-specific context","tools":["read"]}],
                "tools":[{"name":"inspect","description":"Inspect extension state","input_schema":{"type":"object"}}]}
        });
        std::fs::write(
            package.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        Self {
            ext: InstalledExtension {
                id,
                enabled: true,
                kind: "oneshot".into(),
                ..Default::default()
            },
            workspace,
            package,
            agent,
        }
    }
}

impl Drop for ExtensionFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.package);
        let _ = std::fs::remove_dir_all(&self.workspace);
    }
}

#[test]
fn old_extension_entries_default_to_on_demand_and_upgrades_keep_policy() {
    let mut ext: InstalledExtension = serde_json::from_value(serde_json::json!({
        "id":"run.koma.example", "version":"1", "tier":"free"
    }))
    .unwrap();
    assert!(ext.enabled);
    assert_eq!(ext.activation, ExtensionActivation::OnDemand);
    assert!(!ext.active_in(&[]));
    assert!(ext.active_in(&[ext.id.clone()]));
    ext.activation = ExtensionActivation::Global;
    let mut config = AppConfig::default();
    config.upsert_extension(ext.clone());
    config.upsert_extension(InstalledExtension {
        version: "2".into(),
        activation: ExtensionActivation::OnDemand,
        ..ext
    });
    assert_eq!(
        config.installed_extensions[0].activation,
        ExtensionActivation::Global
    );
    assert_eq!(config.installed_extensions[0].version, "2");
}

#[test]
fn selection_is_session_local_persisted_and_unloads_only_managed_roots() {
    let fixture = ExtensionFixture::new();
    let installed = vec![fixture.ext.clone()];
    let mut a = Settings {
        workdir: vec!["/project".into(), "/user-extra".into()],
        ..Default::default()
    };
    let mut b = a.clone();
    assert!(!sync_extension_workspaces(&installed, &mut a));
    assert!(
        !fixture.workspace.exists(),
        "inactive extensions must not even create roots"
    );
    a.active_extensions.push(fixture.ext.id.clone());
    assert!(sync_extension_workspaces(&installed, &mut a));
    assert_eq!(a.workdir.len(), 3);
    assert!(!sync_extension_workspaces(&installed, &mut a));
    assert!(!sync_extension_workspaces(&installed, &mut b));
    assert_eq!(b.workdir.len(), 2);
    let mut resumed: Settings = serde_json::from_slice(&serde_json::to_vec(&a).unwrap()).unwrap();
    assert!(!sync_extension_workspaces(&installed, &mut resumed));
    resumed.active_extensions.clear();
    assert!(sync_extension_workspaces(&installed, &mut resumed));
    assert_eq!(resumed.workdir, b.workdir);

    let mut global = installed.clone();
    global[0].activation = ExtensionActivation::Global;
    assert!(sync_extension_workspaces(&global, &mut b));
    assert!(sync_extension_workspaces(&installed, &mut b));
    assert_eq!(b.workdir, resumed.workdir);
    // Uninstall removes tracked roots even after the manifest is gone.
    assert!(sync_extension_workspaces(&[], &mut a));
    assert_eq!(a.workdir, resumed.workdir);
}

#[test]
fn extension_never_replaces_implicit_primary_workspace() {
    let fixture = ExtensionFixture::new();
    let mut settings = Settings {
        active_extensions: vec![fixture.ext.id.clone()],
        ..Default::default()
    };
    sync_extension_workspaces(&[fixture.ext.clone()], &mut settings);
    assert_eq!(
        PathBuf::from(&settings.workdir[0]),
        std::env::current_dir().unwrap()
    );
    settings.active_extensions.clear();
    sync_extension_workspaces(&[fixture.ext.clone()], &mut settings);
    assert_eq!(settings.workdir.len(), 1);
}

#[test]
fn legacy_secondary_roots_are_migrated_but_explicit_and_primary_roots_survive() {
    let fixture = ExtensionFixture::new();
    let path = fixture.workspace.to_string_lossy().into_owned();
    let mut legacy: Settings = serde_json::from_value(serde_json::json!({
        "workdir":["/project",path,"/user-extra"]
    }))
    .unwrap();
    assert!(sync_extension_workspaces(
        &[fixture.ext.clone()],
        &mut legacy
    ));
    assert_eq!(legacy.workdir, vec!["/project", "/user-extra"]);
    assert_eq!(legacy.extension_workspace_roots, Some(vec![]));
    let mut explicit = Settings {
        workdir: vec!["/project".into(), path.clone()],
        ..Default::default()
    };
    assert!(!sync_extension_workspaces(
        &[fixture.ext.clone()],
        &mut explicit
    ));
    let mut primary: Settings =
        serde_json::from_value(serde_json::json!({"workdir":[path]})).unwrap();
    assert!(!sync_extension_workspaces(
        &[fixture.ext.clone()],
        &mut primary
    ));
    assert_eq!(primary.workdir.len(), 1);
}

#[test]
fn inactive_extension_workspace_and_agent_are_absent_from_system_prompt() {
    use crate::model::{agent_def::AgentRegistry, conversation::Conversation, session::Session};
    let fixture = ExtensionFixture::new();
    let config = AppConfig {
        installed_extensions: vec![fixture.ext.clone()],
        ..Default::default()
    };
    let root = std::env::temp_dir().join(format!("scope-session-{}", uuid::Uuid::new_v4()));
    let mut session = Session::new(
        "scope-session".into(),
        root.clone(),
        "scope-test".into(),
        Settings {
            workdir: vec![root.display().to_string()],
            ..Default::default()
        },
        Conversation::from_messages(vec![]),
    );
    let registry = AgentRegistry::load_for_session(Some(&root), &config, &[]);
    assert!(registry.get(&fixture.agent).is_none());
    session.rebuild_system_with(&registry, &config);
    assert!(!session.conversation.messages()[0]
        .content
        .contains(&fixture.ext.id));
    session
        .settings
        .active_extensions
        .push(fixture.ext.id.clone());
    let registry =
        AgentRegistry::load_for_session(Some(&root), &config, &session.settings.active_extensions);
    assert!(registry.get(&fixture.agent).is_some());
    session.rebuild_system_with(&registry, &config);
    assert!(session.conversation.messages()[0]
        .content
        .contains(&fixture.ext.id));
    assert!(session.conversation.messages()[0]
        .content
        .contains(&fixture.agent));
    session.settings.active_extensions.clear();
    let registry = AgentRegistry::load_for_session(Some(&root), &config, &[]);
    session.rebuild_system_with(&registry, &config);
    assert!(!session.conversation.messages()[0]
        .content
        .contains(&fixture.ext.id));
    assert!(!session.conversation.messages()[0]
        .content
        .contains(&fixture.agent));
    let _ = std::fs::remove_dir_all(root);
}
