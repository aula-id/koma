//! Tests for the config methods and `session_info`. Every test uses a config
//! directory of its own and, for a bound session, a peer that plays the daemon.
use super::host::config::{commit, prepare, Change, ConfigStore};
use super::host::lock;
use super::protocol::{Policy, Request, RpcError, State, CONFIG_LOCK_WAIT};
use super::*;
use crate::ipc::proto::{DaemonEvent as E, DaemonFrame};
use crate::model::app_config::{
    ApiType, AppConfig, ModelEntry, ModelRole, OAuthConn, ProviderConn,
};
use std::path::{Path, PathBuf};

const PLANTED_KEY: &str = "sk-planted-key-0123456789";
const PLANTED_TOKEN: &str = "oauth-planted-token-0123456789";
const PLANTED_PASSWORD: &str = "planted-password-0123";
const PLANTED_QUERY: &str = "planted-query-0123";

/// A fresh directory under the system temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("koma-cfg-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn config(&self) -> PathBuf {
        self.0.join("config.json")
    }
    fn bytes(&self) -> Option<Vec<u8>> {
        std::fs::read(self.config()).ok()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write_config(path: &Path, config: &AppConfig) {
    let key = crate::model::secret_box::key_for_write().unwrap();
    let bytes = config.to_disk_bytes(&key).unwrap();
    crate::model::secret_box::write_private(path, &bytes).unwrap();
}

fn provider(uuid: &str, name: &str, endpoint: &str) -> ProviderConn {
    ProviderConn {
        uuid: uuid.into(),
        name: name.into(),
        api_type: ApiType::OpenAiCompatible,
        endpoint: endpoint.into(),
        api_key: PLANTED_KEY.into(),
        ext_id: None,
    }
}

fn model(uuid: &str, provider: &str, roles: Vec<ModelRole>) -> ModelEntry {
    ModelEntry {
        uuid: uuid.into(),
        name: uuid.into(),
        model_id: format!("{uuid}-id"),
        provider_uuid: provider.into(),
        roles,
        ..ModelEntry::default()
    }
}

/// Two providers: `p-local` serves the Main model, `p-proxy` (an endpoint
/// with credentials) serves the Awareness model. One OAuth connection.
fn fixture() -> AppConfig {
    AppConfig {
        providers: vec![
            provider("p-local", "local", "http://127.0.0.1:9/v1"),
            provider(
                "p-proxy",
                "proxy",
                &format!("https://user:{PLANTED_PASSWORD}@proxy.example/v1?key={PLANTED_QUERY}"),
            ),
        ],
        models: vec![
            model("m-main", "p-local", vec![ModelRole::Main]),
            model("m-aware", "p-proxy", vec![ModelRole::Awareness]),
            model("m-spare", "p-local", vec![]),
        ],
        oauth_conns: vec![OAuthConn {
            uuid: "o-1".into(),
            name: "login".into(),
            access_token: PLANTED_TOKEN.into(),
            refresh_token: PLANTED_TOKEN.into(),
            ..OAuthConn::default()
        }],
        ..AppConfig::default()
    }
}

fn store_with_fixture() -> (TempDir, ConfigStore) {
    let dir = TempDir::new();
    write_config(&dir.config(), &fixture());
    let store = ConfigStore::at(&dir.0);
    (dir, store)
}

fn unbound(store: &ConfigStore) -> Adapter {
    let (events, _) = watch::channel(None);
    let mut adapter = Adapter::new(events);
    adapter.config = store.clone();
    adapter
}

async fn call(adapter: &mut Adapter, method: &str, params: Value) -> Result<Value, RpcError> {
    dispatch(
        Request {
            id: method.into(),
            method: method.into(),
            params,
        },
        adapter,
    )
    .await
}

fn reason(error: &RpcError) -> Option<&'static str> {
    error.extra().and_then(|extra| extra.reason)
}

fn assert_no_secret(text: &str) {
    for planted in [PLANTED_KEY, PLANTED_TOKEN, PLANTED_PASSWORD, PLANTED_QUERY] {
        assert!(!text.contains(planted), "{planted} in {text}");
    }
}

// ── in-process writes ──────────────────────────────────────────────────────

#[tokio::test]
async fn a_missing_file_is_an_empty_view() {
    let dir = TempDir::new();
    let mut adapter = unbound(&ConfigStore::at(&dir.0));
    let view = call(&mut adapter, "config.get", json!({})).await.unwrap();
    assert_eq!(view["file"], "missing");
    assert_eq!(view["revision"], "absent");
    assert_eq!(view["view"], "global");
    assert_eq!(view["providers"], json!([]));
    assert_eq!(view["roles"].as_array().unwrap().len(), 5);
    assert_eq!(view["main_fallback"], "unconfigured");
    let error = call(
        &mut adapter,
        "config.providers.delete",
        json!({"provider_uuid":"x"}),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "not_found");
    assert!(dir.bytes().is_none());
}

#[tokio::test]
async fn a_corrupt_config_is_unreadable_and_left_untouched() {
    let dir = TempDir::new();
    std::fs::write(dir.config(), b"{\"providers\": [ not json").unwrap();
    let before = dir.bytes();
    let mut adapter = unbound(&ConfigStore::at(&dir.0));
    let calls = [
        ("config.get", json!({})),
        (
            "config.providers.delete",
            json!({"provider_uuid":"p-local"}),
        ),
        ("config.models.delete", json!({"model_uuid":"m-main"})),
        ("config.roles.unassign", json!({"role":"main"})),
    ];
    for (method, params) in calls {
        let error = call(&mut adapter, method, params).await.unwrap_err();
        assert_eq!(error.code, "unavailable", "{method}");
        assert_eq!(error.delivery, "not_sent", "{method}");
        assert_eq!(reason(&error), Some("config_unreadable"), "{method}");
    }
    assert_eq!(dir.bytes(), before);
}

#[tokio::test]
async fn an_unreadable_config_error_never_quotes_the_file() {
    let dir = TempDir::new();
    // A string where a list of pairs belongs: serde's error quotes the value.
    let secret = "KEY=MCPENVSECRET1234";
    std::fs::write(
        dir.config(),
        format!("{{\"mcp_servers\":[{{\"env\":\"{secret}\"}}]}}"),
    )
    .unwrap();
    let mut adapter = unbound(&ConfigStore::at(&dir.0));
    let calls = [
        ("config.get", json!({})),
        (
            "config.providers.delete",
            json!({"provider_uuid":"p-local"}),
        ),
        ("config.models.delete", json!({"model_uuid":"m-main"})),
        ("config.roles.unassign", json!({"role":"planner"})),
    ];
    for (method, params) in calls {
        let error = call(&mut adapter, method, params).await.unwrap_err();
        assert_eq!(reason(&error), Some("config_unreadable"), "{method}");
        assert!(
            error
                .message
                .starts_with("config.json is not valid (data error"),
            "{}",
            error.message
        );
        let wire = serde_json::to_string(&error).unwrap();
        assert!(!wire.contains("MCPENVSECRET"), "{method}: {wire}");
    }
}

#[tokio::test]
async fn a_stale_revision_is_refused_and_nothing_is_written() {
    let (dir, store) = store_with_fixture();
    let before = dir.bytes();
    let mut adapter = unbound(&store);
    let error = call(
        &mut adapter,
        "config.providers.delete",
        json!({"provider_uuid":"p-proxy","if_revision":"0000000000000000"}),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "invalid_state");
    assert_eq!(reason(&error), Some("stale"));
    assert_eq!(
        error.message,
        "config.json changed since revision 0000000000000000"
    );
    assert_eq!(dir.bytes(), before);
}

#[tokio::test]
async fn a_dry_run_reports_the_effect_and_writes_nothing() {
    let (dir, store) = store_with_fixture();
    let before = dir.bytes();
    let mut adapter = unbound(&store);
    let view = call(&mut adapter, "config.get", json!({})).await.unwrap();
    let result = call(
        &mut adapter,
        "config.providers.delete",
        json!({"provider_uuid":"p-proxy","dry_run":true,"if_revision":view["revision"]}),
    )
    .await
    .unwrap();
    assert_eq!(result["dry_run"], true);
    assert_eq!(result["removed"]["models_removed"], json!(["m-aware"]));
    assert_eq!(result["removed"]["main_reset"], false);
    assert_eq!(result["config"]["revision"], view["revision"]);
    assert_eq!(result["propagation"], json!({}));
    assert_eq!(dir.bytes(), before);
    let result = call(
        &mut adapter,
        "config.models.delete",
        json!({"model_uuid":"m-main","dry_run":true}),
    )
    .await
    .unwrap();
    assert_eq!(result["removed"]["roles_released"], json!(["main"]));
    assert_eq!(result["removed"]["main_reset"], true);
    assert_eq!(dir.bytes(), before);
}

#[tokio::test]
async fn a_provider_delete_commits_and_reports_propagation() {
    let (dir, store) = store_with_fixture();
    let mut adapter = unbound(&store);
    let before = call(&mut adapter, "config.get", json!({})).await.unwrap();
    let result = call(
        &mut adapter,
        "config.providers.delete",
        json!({"provider_uuid":"p-proxy","if_revision":before["revision"]}),
    )
    .await
    .unwrap();
    assert_eq!(result["dry_run"], false);
    assert_eq!(
        result["propagation"],
        json!({"bound":"not_bound","broadcast":"sent","rebind":"done","rooms":"next_start"})
    );
    let config = &result["config"];
    assert_ne!(config["revision"], before["revision"]);
    assert_eq!(config["providers"].as_array().unwrap().len(), 1);
    assert!(config["models"]
        .as_array()
        .unwrap()
        .iter()
        .all(|m| m["model_uuid"] != "m-aware"));
    let on_disk = store.read().unwrap();
    assert!(!on_disk.config.providers.iter().any(|p| p.uuid == "p-proxy"));
    assert!(on_disk
        .config
        .providers
        .iter()
        .any(|p| p.api_key == PLANTED_KEY));
    // A retry after success means "already gone".
    let error = call(
        &mut adapter,
        "config.providers.delete",
        json!({"provider_uuid":"p-proxy"}),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "not_found");
    assert_eq!(error.message, "provider p-proxy not found");
    drop(dir);
}

#[tokio::test]
async fn a_managed_provider_is_refused() {
    let dir = TempDir::new();
    let mut config = fixture();
    config.providers[0].ext_id = Some("ext".into());
    write_config(&dir.config(), &config);
    let before = dir.bytes();
    let mut adapter = unbound(&ConfigStore::at(&dir.0));
    let error = call(
        &mut adapter,
        "config.providers.delete",
        json!({"provider_uuid":"p-local"}),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "invalid_state");
    assert_eq!(reason(&error), Some("managed"));
    assert_eq!(dir.bytes(), before);
}

#[tokio::test]
async fn role_release_changes_only_what_it_names() {
    let (dir, store) = store_with_fixture();
    let mut adapter = unbound(&store);
    let before = dir.bytes();
    let result = call(
        &mut adapter,
        "config.roles.unassign",
        json!({"role":"planner"}),
    )
    .await
    .unwrap();
    assert_eq!(result["changed"], false);
    assert_eq!(result["previous_model_uuid"], Value::Null);
    assert_eq!(result["propagation"], json!({}));
    assert_eq!(dir.bytes(), before);
    let result = call(
        &mut adapter,
        "config.roles.unassign",
        json!({"role":"main"}),
    )
    .await
    .unwrap();
    assert_eq!(result["changed"], true);
    assert_eq!(result["previous_model_uuid"], "m-main");
    assert_eq!(result["propagation"]["rebind"], "not_needed");
    assert_eq!(result["config"]["main_fallback"], "unconfigured");
    let config = store.read().unwrap().config;
    assert!(config.models.iter().any(|m| m.uuid == "m-main"));
    assert!(config
        .models
        .iter()
        .all(|m| !m.effective_roles().contains(&ModelRole::Main)));
}

#[tokio::test]
async fn session_scope_params_are_checked_before_anything_else() {
    let (dir, store) = store_with_fixture();
    let before = dir.bytes();
    let mut adapter = unbound(&store);
    let error = call(
        &mut adapter,
        "config.roles.unassign",
        json!({"role":"planner","scope":"session"}),
    )
    .await
    .unwrap_err();
    assert_eq!(
        (error.code, reason(&error)),
        ("invalid_request", Some("out_of_range"))
    );
    assert_eq!(error.message, "scope session accepts only role main");
    let error = call(
        &mut adapter,
        "config.roles.unassign",
        json!({"role":"main","scope":"session","if_revision":"0000000000000000"}),
    )
    .await
    .unwrap_err();
    assert_eq!(
        (error.code, reason(&error)),
        ("invalid_request", Some("conflicting_params"))
    );
    // A well-formed session-scope call outside a coding binding is E2.
    let error = call(
        &mut adapter,
        "config.roles.unassign",
        json!({"role":"main","scope":"session"}),
    )
    .await
    .unwrap_err();
    assert_eq!(reason(&error), Some("wrong_binding"));
    assert_eq!(
        error.message,
        "config.roles.unassign requires binding coding; this adapter is unbound"
    );
    let error = call(&mut adapter, "config.get", json!({"view":"session"}))
        .await
        .unwrap_err();
    assert_eq!(reason(&error), Some("wrong_binding"));
    for scope in [json!("session"), json!("local"), json!(null), json!(1)] {
        let error = call(
            &mut adapter,
            "config.models.delete",
            json!({"model_uuid":"m-main","scope":scope}),
        )
        .await
        .unwrap_err();
        assert_eq!(
            (error.code, reason(&error)),
            ("invalid_request", Some("out_of_range"))
        );
    }
    assert_eq!(dir.bytes(), before);
    let result = call(
        &mut adapter,
        "config.models.delete",
        json!({"model_uuid":"m-spare","scope":"global"}),
    )
    .await
    .unwrap();
    assert_eq!(result["removed"]["roles_released"], json!([]));
}

#[test]
fn a_failed_save_with_the_pre_image_intact_is_not_sent() {
    let (dir, store) = store_with_fixture();
    let before = dir.bytes();
    let store = store.with_save(|_, _| Err(anyhow::anyhow!("disk full")));
    let change = Change::DeleteModel("m-spare".into());
    let prepared = prepare(&store, &change, None).unwrap();
    let Err(error) = commit(&store, &change, prepared) else {
        panic!("the save must fail")
    };
    assert_eq!(error.code, "unavailable");
    assert_eq!(error.delivery, "not_sent");
    assert_eq!(reason(&error), Some("save_failed"));
    assert_eq!(dir.bytes(), before);
}

#[test]
fn a_failed_save_that_changed_the_file_is_unknown_and_written() {
    let (dir, store) = store_with_fixture();
    let store = store.with_save(|path, _| {
        std::fs::write(path, b"{}")?;
        Err(anyhow::anyhow!("rename failed"))
    });
    let change = Change::DeleteModel("m-spare".into());
    let prepared = prepare(&store, &change, None).unwrap();
    let Err(error) = commit(&store, &change, prepared) else {
        panic!("the save must fail")
    };
    assert_eq!(error.delivery, "unknown");
    assert_eq!(reason(&error), Some("unverified"));
    assert_eq!(error.extra().and_then(|e| e.written), Some(true));
    // In-process work never ends the process.
    assert!(!error.ends_process(false));
    drop(dir);
}

#[test]
fn two_writers_are_serialized_by_the_lock() {
    let (dir, store) = store_with_fixture();
    let before = dir.bytes();
    let held = lock::acquire(&store.lock_path().unwrap(), CONFIG_LOCK_WAIT).unwrap();
    let writer = {
        let store = store.clone();
        std::thread::spawn(move || {
            let change = Change::DeleteModel("m-spare".into());
            let prepared = prepare(&store, &change, None)?;
            commit(&store, &change, prepared).map(|c| c.written)
        })
    };
    std::thread::sleep(std::time::Duration::from_millis(300));
    // The second writer waits for the lock and has not touched the file.
    assert!(!writer.is_finished());
    assert_eq!(dir.bytes(), before);
    drop(held);
    assert!(writer.join().unwrap().unwrap());
    let config = store.read().unwrap().config;
    assert!(!config.models.iter().any(|m| m.uuid == "m-spare"));
}

#[tokio::test]
async fn the_planted_secrets_never_appear_in_any_output() {
    let (dir, store) = store_with_fixture();
    let mut adapter = unbound(&store);
    let mut outputs = Vec::new();
    let calls = [
        ("config.get", json!({})),
        ("config.get", json!({"view":"global"})),
        (
            "config.providers.delete",
            json!({"provider_uuid":"p-proxy","dry_run":true}),
        ),
        (
            "config.providers.delete",
            json!({"provider_uuid":"p-proxy","if_revision":"0000000000000000"}),
        ),
        ("config.models.delete", json!({"model_uuid":"m-aware"})),
        ("config.roles.unassign", json!({"role":"awareness"})),
        ("config.roles.unassign", json!({"role":"main"})),
        (
            "config.providers.delete",
            json!({"provider_uuid":"p-proxy"}),
        ),
        (
            "config.providers.delete",
            json!({"provider_uuid":"p-proxy"}),
        ),
        ("config.effort.get", json!({})),
    ];
    for (method, params) in calls {
        let output = match call(&mut adapter, method, params).await {
            Ok(result) => serde_json::to_string(&result).unwrap(),
            Err(error) => serde_json::to_string(&error).unwrap(),
        };
        outputs.push(output);
    }
    for output in &outputs {
        assert_no_secret(output);
    }
    // The key is still stored: it was hidden, not lost.
    let config = store.read().unwrap().config;
    assert_eq!(config.providers[0].api_key, PLANTED_KEY);
    drop(dir);
}

#[tokio::test]
async fn endpoint_userinfo_and_query_are_stripped() {
    let (_dir, store) = store_with_fixture();
    let mut adapter = unbound(&store);
    let view = call(&mut adapter, "config.get", json!({})).await.unwrap();
    let providers = view["providers"].as_array().unwrap();
    assert_eq!(providers[0]["endpoint"], "http://127.0.0.1:9/v1");
    assert_eq!(providers[0]["endpoint_redacted"], false);
    assert_eq!(providers[1]["endpoint"], "https://proxy.example/v1");
    assert_eq!(providers[1]["endpoint_redacted"], true);
    assert_eq!(providers[1]["key_state"], "set");
    assert_eq!(providers[1]["model_count"], 1);
    assert_eq!(view["oauth"][0]["has_token"], true);
    assert_eq!(view["models"][0]["provider_kind"], "provider");
    assert_eq!(view["models"][0]["provider_name"], "local");
    assert_eq!(view["roles"][0]["role"], "main");
    assert_eq!(view["roles"][0]["origin"], "global");
    assert_eq!(view["roles"][0]["model_uuid"], "m-main");
    assert_eq!(view["roles"][0]["provider_name"], "local");
}

// ── config.skill_roots.set ─────────────────────────────────────────────────

fn skill_root_dir(parent: &TempDir, name: &str) -> PathBuf {
    let dir = parent.0.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::canonicalize(dir).unwrap()
}

#[tokio::test]
async fn skill_roots_set_replaces_only_that_key_and_is_idempotent() {
    let (dir, store) = store_with_fixture();
    let one = skill_root_dir(&dir, "team-skills");
    let two = skill_root_dir(&dir, "more-skills");
    let mut adapter = unbound(&store);
    let before = store.read().unwrap();

    let first = call(
        &mut adapter,
        "config.skill_roots.set",
        json!({"roots":[one, two]}),
    )
    .await
    .unwrap();
    assert_eq!(first["changed"], true);
    assert_eq!(first["roots"], json!([one, two]));
    assert_eq!(first["config"]["view"], "global");
    assert_no_secret(&first.to_string());
    let after = store.read().unwrap();
    assert_eq!(
        after.config.extra_skill_roots,
        vec![one.clone(), two.clone()]
    );
    assert_eq!(after.config.providers.len(), before.config.providers.len());
    assert_eq!(after.config.models.len(), before.config.models.len());
    assert_eq!(after.config.providers[0].api_key, PLANTED_KEY);

    let bytes = dir.bytes();
    let again = call(
        &mut adapter,
        "config.skill_roots.set",
        json!({"roots":[one, two], "if_revision": after.revision}),
    )
    .await
    .unwrap();
    assert_eq!(again["changed"], false);
    assert_eq!(dir.bytes(), bytes);

    let cleared = call(&mut adapter, "config.skill_roots.set", json!({"roots":[]}))
        .await
        .unwrap();
    assert_eq!(cleared["changed"], true);
    assert_eq!(cleared["roots"], json!([]));
    assert!(store.read().unwrap().config.extra_skill_roots.is_empty());
}

#[tokio::test]
async fn skill_roots_set_keeps_a_key_another_writer_saved() {
    let (dir, store) = store_with_fixture();
    let root = skill_root_dir(&dir, "team-skills");
    let mut adapter = unbound(&store);
    // Another writer saves a new provider after the adapter started.
    let mut other = store.read().unwrap().config;
    other
        .providers
        .push(provider("p-late", "late", "http://127.0.0.1:10/v1"));
    write_config(&dir.config(), &other);
    call(
        &mut adapter,
        "config.skill_roots.set",
        json!({"roots":[root]}),
    )
    .await
    .unwrap();
    let after = store.read().unwrap().config;
    assert!(after.providers.iter().any(|p| p.uuid == "p-late"));
    assert_eq!(after.extra_skill_roots, vec![root]);
}

#[tokio::test]
async fn skill_roots_set_refuses_bad_roots_before_writing() {
    let (dir, store) = store_with_fixture();
    let good = skill_root_dir(&dir, "team-skills");
    let file = dir.0.join("a-file");
    std::fs::write(&file, b"x").unwrap();
    let bytes = dir.bytes();
    let mut adapter = unbound(&store);
    let cases = [
        (json!({}), "missing_params"),
        (json!({"roots":"/x"}), "out_of_range"),
        (json!({"roots":[1]}), "out_of_range"),
        (json!({"roots":[""]}), "out_of_range"),
        (json!({"roots":["relative/skills"]}), "out_of_range"),
        (json!({"roots":["~/skills"]}), "out_of_range"),
        (json!({"roots":[dir.0.join("absent")]}), "out_of_range"),
        (json!({"roots":[file]}), "out_of_range"),
        (json!({"roots":[good, good]}), "out_of_range"),
    ];
    for (params, expected) in cases {
        let error = call(&mut adapter, "config.skill_roots.set", params.clone())
            .await
            .unwrap_err();
        assert_eq!(reason(&error), Some(expected), "{params}");
        assert_eq!(error.delivery, "not_sent", "{params}");
    }
    assert_eq!(dir.bytes(), bytes);
}

#[tokio::test]
async fn skill_roots_set_refuses_a_stale_revision() {
    let (dir, store) = store_with_fixture();
    let root = skill_root_dir(&dir, "team-skills");
    let bytes = dir.bytes();
    let mut adapter = unbound(&store);
    let error = call(
        &mut adapter,
        "config.skill_roots.set",
        json!({"roots":[root], "if_revision":"0000000000000000"}),
    )
    .await
    .unwrap_err();
    assert_eq!(reason(&error), Some("stale"));
    assert_eq!(dir.bytes(), bytes);
}

// ── session_info ───────────────────────────────────────────────────────────

fn routes(main_origin: &str, main_uuid: &str) -> Value {
    json!([
        {"role":"main","origin":main_origin,"assignment_uuid":main_uuid,"source_uuid":null,
         "configured_model":"m-main-id","provider_uuid":"p-local","effective_model":"m-main-id","reason":null},
        {"role":"awareness","origin":"global","assignment_uuid":"m-aware","source_uuid":null,
         "configured_model":"m-aware-id","provider_uuid":"p-proxy","effective_model":"m-aware-id","reason":null},
        {"role":"safeguard","origin":"legacy","assignment_uuid":null,"source_uuid":null,
         "configured_model":"m-main-id","provider_uuid":"p-local","effective_model":"m-main-id","reason":null},
        {"role":"compactor","origin":"inherit_main","assignment_uuid":null,"source_uuid":null,
         "configured_model":"m-main-id","provider_uuid":"p-local","effective_model":"m-main-id","reason":null},
        {"role":"planner","origin":"inherit_main","assignment_uuid":null,"source_uuid":null,
         "configured_model":"m-main-id","provider_uuid":"p-local","effective_model":"m-main-id","reason":null}
    ])
}

/// A full snapshot of session `session`. `providers` and `models` are the
/// daemon's in-memory catalogue uuids.
fn snapshot_event(
    name: &str,
    effort: &str,
    routes: Value,
    providers: &[&str],
    models: &[&str],
) -> E {
    let providers: Vec<Value> = providers
        .iter()
        .map(|uuid| json!({"uuid":uuid,"name":format!("{uuid}-name"),"api_key":PLANTED_KEY}))
        .collect();
    let models: Vec<Value> = models.iter().map(|uuid| json!({"uuid":uuid})).collect();
    let value = json!({"foreground_id":"session","sessions":[{
     "id":"session","name":name,"cwd":"/work","messages":[],"streaming":null,"stream_reasoning":"",
     "tokens_in":0,"tokens_out":0,"tokens_cached":0,"cost":0.0,"waiting":false,"awaiting_approval":false,
     "approval_reason":null,"pending_tool_calls":[],"tool_idx":0,"working":false,"finished_unseen":false,
     "subagents":[],"pending_subagents":[],"resolved_model_id":"m-main-id","effort":effort,
     "model_routes":routes,
     "context_usage":{"prompt_tokens":10,"effective_window":131072,"estimated":true}
    }],"global":{
     "input":"","cursor":0,"scroll":0,"follow":true,"status":"","work_elapsed_ms":null,"theme":"dark","accent":"blue","palette":"default","mode":"Chat","toast":null,"models_cache":null,"models_cache_endpoint":null,"models_cache_failed":null,"agent_viewer":null,"agent_viewer_scroll":0,"agent_viewer_follow":true,"subagents_open":false,"subagent_sel":0,"palette_sel":0,"pending_attachments":[],"file_palette":null,"agent_mode":"auto","latest_version":null,
     "providers":providers,"config_models":models
    }});
    E::Snapshot(Box::new(serde_json::from_value(value).unwrap()))
}

#[test]
fn a_session_info_change_emits_no_event_and_bumps_no_revision() {
    let mut r = reconcile::Reconcile::new("session".into());
    r.policy = Some(Policy::default());
    r.apply(&DaemonFrame {
        seq: 1,
        event: snapshot_event("first", "", routes("global", "m-main"), &[], &[]),
    });
    let first = r.state().unwrap();
    let info = r.session_info(|_| None);
    assert_eq!(info.effort, "default");
    assert_eq!(info.name, "first");
    let mut previous = Some(first.clone());
    let mut seq = 0;
    r.apply(&DaemonFrame {
        seq: 2,
        event: snapshot_event("renamed", "high", routes("session", "m-local"), &[], &[]),
    });
    let second = r.state().unwrap();
    assert_eq!(second, first);
    assert_eq!(second.revision, first.revision);
    assert!(event_frames(&mut previous, second, &mut seq).is_empty());
    let info = r.session_info(|_| None);
    assert_eq!(info.effort, "high");
    assert_eq!(info.name, "renamed");
    assert_eq!(info.routes[0].origin, "session");
    assert_eq!(info.limits.effective_window, Some(131072));
    // Usage is reported from the snapshot but changes neither state nor events.
    assert!(info.usage.is_some());
}

#[test]
fn state_objects_stay_v1() {
    // `session_info` is a sibling key; the v1 `State` serializes as before.
    let state = State {
        revision: 1,
        resync: 0,
        external_approvals: false,
        pending_approval: None,
        gate: None,
        session_id: "s".into(),
        cwd: "/".into(),
        model_id: "m".into(),
        policy: Policy::default(),
        turn: None,
        working: false,
        awaiting_approval: false,
        approval_reason: None,
        pending_count: 0,
        messages: vec![],
        streaming: None,
    };
    let wire = serde_json::to_value(&state).unwrap();
    assert!(wire.get("session_info").is_none());
    assert_eq!(wire.as_object().unwrap().len(), 15);
}

/// A bound coding session, played by a peer on a socket pair.
#[cfg(unix)]
mod bound {
    use super::*;
    use crate::ipc::{client, frame::FrameReader, proto::ClientRequest, proto::RunState};

    fn run_state(effort: &str, main_origin: &str, main_uuid: &str) -> RunState {
        RunState {
            session_id: "session".into(),
            policy: Some(crate::ipc::proto::turn::SessionPolicy {
                plan: crate::ipc::proto::PlanPolicy::Forbid,
                approval: crate::ipc::proto::ApprovalPolicy::Reject,
            }),
            name: "renamed".into(),
            effort: effort.into(),
            model_routes: serde_json::from_value(routes(main_origin, main_uuid)).unwrap(),
            max_output_tokens: 4096,
            ..RunState::default()
        }
    }

    fn settings_values(effort: &str) -> E {
        serde_json::from_value(json!({"SettingsValues":{
            "name":"renamed","workdir":[],"short_send":false,"sliding_cache":false,"bash_saving":false,
            "coding_autosave":false,"internet_mode":"simple","palette":"default","effort":effort,
            "subagent_max_turns":1,"extra_skill_roots":[]
        }}))
        .unwrap()
    }

    /// What the fake daemon does with one request: the frames it sends back.
    type Script = Box<dyn FnMut(&ClientRequest) -> Vec<E> + Send>;

    fn encode(event: E, seq: &mut u64) -> Vec<u8> {
        *seq += 1;
        serde_json::to_vec(&DaemonFrame { seq: *seq, event }).unwrap()
    }

    /// A connection whose peer answers every request with `script` and records
    /// what it received. The peer has already sent one snapshot.
    fn fake_daemon(
        mut script: Script,
    ) -> (
        connection::Connection,
        tokio::task::JoinHandle<Vec<ClientRequest>>,
    ) {
        let (a, mut b) = tokio::net::UnixStream::pair().unwrap();
        let mut c = connection::Connection::for_test(a, "session");
        c.projection.policy = Some(Policy::default());
        let peer = tokio::spawn(async move {
            let mut seq = 0u64;
            let mut seen = Vec::new();
            let first = encode(
                snapshot_event(
                    "renamed",
                    "",
                    routes("global", "m-main"),
                    &["p-local", "p-proxy"],
                    &["m-main", "m-aware", "m-spare"],
                ),
                &mut seq,
            );
            if client::send_frame(&mut b, &first).await.is_err() {
                return seen;
            }
            let mut reader = FrameReader::new();
            while let Ok(bytes) = client::recv_frame(&mut b, &mut reader).await {
                let request: ClientRequest = serde_json::from_slice(&bytes).unwrap();
                let replies = script(&request);
                seen.push(request);
                for event in replies {
                    let frame = encode(event, &mut seq);
                    if client::send_frame(&mut b, &frame).await.is_err() {
                        return seen;
                    }
                }
            }
            seen
        });
        (c, peer)
    }

    /// Bind `c` once its first snapshot has arrived.
    async fn coding(mut c: connection::Connection, store: &ConfigStore) -> Adapter {
        while c.projection.session.is_none() {
            c.receive().await.unwrap();
        }
        let mut adapter = unbound(store);
        adapter.binding = Binding::Coding(Box::new(c));
        adapter
    }

    /// Detach the adapter and return every request the fake daemon received.
    async fn requests(
        adapter: Adapter,
        peer: tokio::task::JoinHandle<Vec<ClientRequest>>,
    ) -> Vec<ClientRequest> {
        drop(adapter);
        peer.await.unwrap()
    }

    /// Replies for the readback and resync requests of `state`.
    fn readback(request: &ClientRequest, effort: &str) -> Option<Vec<E>> {
        match request {
            ClientRequest::GetRunState { req_seq } => Some(vec![E::RunState {
                req_seq: *req_seq,
                state: run_state(effort, "global", "m-main"),
            }]),
            ClientRequest::Resync => Some(vec![snapshot_event(
                "renamed",
                effort,
                routes("global", "m-main"),
                &["p-local", "p-proxy"],
                &["m-main", "m-aware", "m-spare"],
            )]),
            _ => None,
        }
    }

    #[tokio::test]
    async fn the_next_state_result_carries_session_info() {
        let (_dir, store) = store_with_fixture();
        let effort = std::sync::Arc::new(std::sync::Mutex::new(""));
        let current = effort.clone();
        let (c, peer) = fake_daemon(Box::new(move |request| {
            readback(request, *current.lock().unwrap()).unwrap_or_default()
        }));
        let mut adapter = coding(c, &store).await;
        let first = call(&mut adapter, "state", json!({})).await.unwrap();
        assert_eq!(first["session_info"]["effort"], "default");
        assert!(first["state"].get("session_info").is_none());
        *effort.lock().unwrap() = "high";
        let second = call(&mut adapter, "state", json!({})).await.unwrap();
        assert_eq!(second["state"], first["state"]);
        assert_eq!(second["session_info"]["effort"], "high");
        assert_eq!(second["session_info"]["limits"]["max_output_tokens"], 4096);
        assert_eq!(
            second["session_info"]["routes"][1]["provider_name"],
            "p-proxy-name"
        );
        assert!(
            second["session_info"]["observed_at_ms"].as_u64()
                >= first["session_info"]["observed_at_ms"].as_u64()
        );
        let info = call(&mut adapter, "sessions.info", json!({}))
            .await
            .unwrap();
        assert_eq!(info["session_info"], second["session_info"]);
        assert_no_secret(&second.to_string());
        requests(adapter, peer).await;
    }

    #[tokio::test]
    async fn a_stale_settings_values_never_satisfies_a_readback() {
        let (c, peer) = fake_daemon(Box::new(|request| match request {
            // A trailing `SettingsValues` and an older readback arrive first.
            ClientRequest::GetRunState { req_seq } => vec![
                settings_values("high"),
                E::RunState {
                    req_seq: req_seq.wrapping_sub(1),
                    state: run_state("high", "global", "m-main"),
                },
                E::RunState {
                    req_seq: *req_seq,
                    state: run_state("", "global", "m-main"),
                },
            ],
            _ => vec![],
        }));
        let mut c = c;
        let state = c.readback().await.unwrap();
        assert_eq!(state.effort, "");
        assert_eq!(c.session_info().effort, "default");
        drop(c);
        peer.await.unwrap();
    }

    #[tokio::test]
    async fn effort_set_after_a_main_change_is_a_non_fatal_main_changed() {
        let (_dir, store) = store_with_fixture();
        let mut readbacks = 0;
        let (c, peer) = fake_daemon(Box::new(move |request| match request {
            ClientRequest::GetRunState { req_seq } => {
                readbacks += 1;
                // The second readback sees another Main, which reset the effort.
                let state = if readbacks == 1 {
                    run_state("", "global", "m-main")
                } else {
                    run_state("", "session", "m-other")
                };
                vec![E::RunState {
                    req_seq: *req_seq,
                    state,
                }]
            }
            ClientRequest::GetEffortOptions => vec![E::EffortOptions {
                options: vec!["default".into(), "low".into(), "high".into()],
                selected: 0,
                note: String::new(),
                state: "ready".into(),
            }],
            ClientRequest::SetEffort { .. } => vec![settings_values("high")],
            _ => vec![],
        }));
        let mut adapter = coding(c, &store).await;
        let error = call(
            &mut adapter,
            "config.effort.set",
            json!({"effort":"high","if_main_model":"m-main"}),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "invalid_state");
        assert_eq!(error.delivery, "not_sent");
        assert_eq!(reason(&error), Some("main_changed"));
        assert!(!error.ends_process(false));
        let seen = requests(adapter, peer).await;
        assert!(seen
            .iter()
            .any(|r| matches!(r, ClientRequest::SetEffort { effort } if effort == "high")));
    }

    #[tokio::test]
    async fn effort_get_while_loading_still_lists_default_first() {
        let (_dir, store) = store_with_fixture();
        let (c, _peer) = fake_daemon(Box::new(move |request| match request {
            ClientRequest::GetRunState { req_seq } => vec![E::RunState {
                req_seq: *req_seq,
                state: run_state("", "global", "m-main"),
            }],
            ClientRequest::GetEffortOptions => vec![E::EffortOptions {
                options: vec![],
                selected: 0,
                note: "loading".into(),
                state: "loading".into(),
            }],
            _ => vec![],
        }));
        let mut adapter = coding(c, &store).await;
        let options = call(&mut adapter, "config.effort.get", json!({"wait_ms":0}))
            .await
            .unwrap();
        assert_eq!(options["state"], "loading");
        assert_eq!(options["options"], json!(["default"]));
        assert_eq!(options["selected"], "default");
    }

    #[tokio::test]
    async fn effort_set_checks_the_offered_options_and_confirms_by_readback() {
        let (_dir, store) = store_with_fixture();
        let effort = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let stored = effort.clone();
        let (c, peer) = fake_daemon(Box::new(move |request| match request {
            ClientRequest::GetRunState { req_seq } => vec![E::RunState {
                req_seq: *req_seq,
                state: run_state(&stored.lock().unwrap(), "global", "m-main"),
            }],
            ClientRequest::GetEffortOptions => vec![
                E::EffortOptions {
                    options: vec![],
                    selected: 0,
                    note: "loading".into(),
                    state: "loading".into(),
                },
                E::EffortOptions {
                    options: vec!["default".into(), "high".into()],
                    selected: 0,
                    note: String::new(),
                    state: "ready".into(),
                },
            ],
            ClientRequest::SetEffort { effort } => {
                *stored.lock().unwrap() = if effort == "default" {
                    String::new()
                } else {
                    effort.clone()
                };
                vec![settings_values(effort)]
            }
            _ => vec![],
        }));
        let mut adapter = coding(c, &store).await;
        let options = call(&mut adapter, "config.effort.get", json!({"wait_ms":1000}))
            .await
            .unwrap();
        assert_eq!(options["state"], "ready");
        assert_eq!(options["options"], json!(["default", "high"]));
        assert_eq!(options["selected"], "default");
        assert_eq!(options["note"], Value::Null);
        assert_eq!(options["main_model_uuid"], "m-main");
        let error = call(&mut adapter, "config.effort.set", json!({"effort":"max"}))
            .await
            .unwrap_err();
        assert_eq!(reason(&error), Some("not_offered"));
        let error = call(&mut adapter, "config.effort.set", json!({"effort":"a b"}))
            .await
            .unwrap_err();
        assert_eq!(reason(&error), Some("out_of_range"));
        let result = call(&mut adapter, "config.effort.set", json!({"effort":"high"}))
            .await
            .unwrap();
        assert_eq!(result["effort"], "high");
        assert_eq!(result["superseded"], false);
        assert_eq!(result["session_info"]["effort"], "high");
        let result = call(
            &mut adapter,
            "config.effort.set",
            json!({"effort":"default"}),
        )
        .await
        .unwrap();
        assert_eq!(result["effort"], "default");
        let seen = requests(adapter, peer).await;
        // The options were fetched once and then served from the cache.
        assert_eq!(
            seen.iter()
                .filter(|r| matches!(r, ClientRequest::GetEffortOptions))
                .count(),
            1
        );
        drop(effort);
    }

    #[tokio::test]
    async fn limits_set_sends_only_the_limit_fields_and_reads_them_back() {
        let (_dir, store) = store_with_fixture();
        let limit = std::sync::Arc::new(std::sync::Mutex::new(0u32));
        let stored = limit.clone();
        let (c, peer) = fake_daemon(Box::new(move |request| match request {
            ClientRequest::GetRunState { req_seq } => {
                let mut state = run_state("", "global", "m-main");
                state.max_output_tokens = *stored.lock().unwrap();
                vec![E::RunState {
                    req_seq: *req_seq,
                    state,
                }]
            }
            ClientRequest::SetSessionPrefs {
                max_output_tokens, ..
            } => {
                if let Some(v) = max_output_tokens {
                    *stored.lock().unwrap() = *v;
                }
                vec![settings_values("")]
            }
            _ => vec![],
        }));
        let mut adapter = coding(c, &store).await;
        let error = call(&mut adapter, "config.limits.set", json!({}))
            .await
            .unwrap_err();
        assert_eq!(reason(&error), Some("missing_params"));
        let error = call(
            &mut adapter,
            "config.limits.set",
            json!({"max_output_tokens":100}),
        )
        .await
        .unwrap_err();
        assert_eq!(reason(&error), Some("out_of_range"));
        let result = call(
            &mut adapter,
            "config.limits.set",
            json!({"max_output_tokens":4096}),
        )
        .await
        .unwrap();
        assert_eq!(result["max_output_tokens"], 4096);
        assert_eq!(result["effective_window"], 131072);
        assert_eq!(result["session_info"]["limits"]["max_output_tokens"], 4096);
        let got = call(&mut adapter, "config.limits.get", json!({}))
            .await
            .unwrap();
        assert_eq!(got["max_output_tokens"], 4096);
        // A read returns `Limits` alone.
        assert!(got.get("session_info").is_none());
        let seen = requests(adapter, peer).await;
        let prefs: Vec<&ClientRequest> = seen
            .iter()
            .filter(|r| matches!(r, ClientRequest::SetSessionPrefs { .. }))
            .collect();
        assert_eq!(prefs.len(), 1);
        assert!(matches!(
            prefs[0],
            ClientRequest::SetSessionPrefs {
                max_output_tokens: Some(4096),
                context_window_limit: None,
                context_model_alias: None,
                short_send: None,
                workdir: None,
                internet_mode: None,
                ..
            }
        ));
    }

    // ── daemon-mediated deletes ────────────────────────────────────────────────

    #[tokio::test]
    async fn a_dry_run_on_the_daemon_path_sends_no_ipc() {
        let (dir, store) = store_with_fixture();
        let before = dir.bytes();
        let (c, peer) = fake_daemon(Box::new(|_| vec![]));
        let mut adapter = coding(c, &store).await;
        let result = call(
            &mut adapter,
            "config.providers.delete",
            json!({"provider_uuid":"p-proxy","dry_run":true}),
        )
        .await
        .unwrap();
        assert_eq!(result["removed"]["models_removed"], json!(["m-aware"]));
        assert_eq!(result["propagation"], json!({}));
        assert!(result.get("session_info").is_some());
        let result = call(
            &mut adapter,
            "config.models.delete",
            json!({"model_uuid":"m-main","dry_run":true}),
        )
        .await
        .unwrap();
        assert_eq!(result["removed"]["main_reset"], true);
        assert!(requests(adapter, peer).await.is_empty());
        assert_eq!(dir.bytes(), before);
    }

    /// A fake daemon holding its own in-memory config, as the hub does: it
    /// reloads from disk on `ReloadGlobalCatalogue` and saves the whole struct
    /// after a delete.
    fn catalogue_daemon(store: ConfigStore, path: PathBuf, fail_save: bool) -> Script {
        let mut memory = store.read().unwrap().config;
        Box::new(move |request| {
            if let Some(replies) = readback(request, "") {
                if matches!(request, ClientRequest::Resync) {
                    let providers: Vec<&str> =
                        memory.providers.iter().map(|p| p.uuid.as_str()).collect();
                    let models: Vec<&str> = memory.models.iter().map(|m| m.uuid.as_str()).collect();
                    return vec![snapshot_event(
                        "renamed",
                        "",
                        routes("global", "m-main"),
                        &providers,
                        &models,
                    )];
                }
                return replies;
            }
            match request {
                ClientRequest::ReloadGlobalCatalogue => {
                    memory = store.read().unwrap().config;
                    vec![E::Ack]
                }
                ClientRequest::DeleteProvider { uuid } => {
                    memory.cascade_remove_provider(uuid);
                    if fail_save {
                        return vec![E::Error(format!("save failed with key {PLANTED_KEY}"))];
                    }
                    write_config(&path, &memory);
                    vec![E::Ack]
                }
                _ => vec![E::Error("unexpected".into())],
            }
        })
    }

    #[tokio::test]
    async fn a_daemon_delete_reloads_first_and_keeps_an_unloaded_change() {
        let (dir, store) = store_with_fixture();
        let script = catalogue_daemon(store.clone(), dir.config(), false);
        // A v1 `configure` the daemon never heard of adds a provider on disk.
        let mut unloaded = store.read().unwrap().config;
        unloaded
            .providers
            .push(provider("p-new", "new", "http://127.0.0.1:9/v2"));
        write_config(&dir.config(), &unloaded);
        let (c, peer) = fake_daemon(script);
        let mut adapter = coding(c, &store).await;
        let result = call(
            &mut adapter,
            "config.providers.delete",
            json!({"provider_uuid":"p-proxy"}),
        )
        .await
        .unwrap();
        assert_eq!(
            result["propagation"],
            json!({"bound":"applied_by_daemon","broadcast":"sent_by_daemon","rebind":"done_by_daemon"})
        );
        assert_eq!(result["config"]["view"], "global");
        let config = store.read().unwrap().config;
        assert!(config.providers.iter().any(|p| p.uuid == "p-new"));
        assert!(!config.providers.iter().any(|p| p.uuid == "p-proxy"));
        let seen = requests(adapter, peer).await;
        let reload = seen
            .iter()
            .position(|r| matches!(r, ClientRequest::ReloadGlobalCatalogue));
        let delete = seen
            .iter()
            .position(|r| matches!(r, ClientRequest::DeleteProvider { .. }));
        assert!(reload.is_some() && reload < delete, "{seen:?}");
    }

    #[tokio::test]
    async fn a_daemon_error_with_the_change_in_memory_is_partial_apply() {
        let (dir, store) = store_with_fixture();
        let before = dir.bytes();
        let script = catalogue_daemon(store.clone(), dir.config(), true);
        let (c, peer) = fake_daemon(script);
        let mut adapter = coding(c, &store).await;
        let error = call(
            &mut adapter,
            "config.providers.delete",
            json!({"provider_uuid":"p-proxy"}),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "unavailable");
        assert_eq!(error.delivery, "unknown");
        assert_eq!(reason(&error), Some("partial_apply"));
        let detail = error.extra().and_then(|e| e.detail.clone()).unwrap();
        assert_eq!(detail["in_memory"], true);
        assert_eq!(detail["on_disk"], false);
        assert!(error.extra().and_then(|e| e.written).is_none());
        // The bound connection's outcome is unknown, so the process ends.
        assert!(error.ends_process(false));
        assert_no_secret(&serde_json::to_string(&error).unwrap());
        assert_eq!(dir.bytes(), before);
        requests(adapter, peer).await;
    }

    #[tokio::test]
    async fn a_daemon_error_without_a_change_is_daemon_refused() {
        let (dir, store) = store_with_fixture();
        let (c, peer) = fake_daemon(Box::new(|request| {
            readback(request, "").unwrap_or_else(|| match request {
                ClientRequest::ReloadGlobalCatalogue => vec![E::Ack],
                _ => vec![E::Error("not now".into())],
            })
        }));
        let mut adapter = coding(c, &store).await;
        let error = call(
            &mut adapter,
            "config.providers.delete",
            json!({"provider_uuid":"p-proxy"}),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "invalid_state");
        assert_eq!(error.delivery, "not_sent");
        assert_eq!(error.message, "not now");
        assert_eq!(reason(&error), Some("daemon_refused"));
        requests(adapter, peer).await;
        drop(dir);
    }

    #[tokio::test]
    async fn effort_and_limits_scrub_the_daemon_error_text() {
        let (_dir, store) = store_with_fixture();
        let leak = format!(
            "Could not save: key {PLANTED_KEY} at https://user:{PLANTED_PASSWORD}@proxy.example/v1"
        );
        let text = leak.clone();
        let (c, peer) = fake_daemon(Box::new(move |request| match request {
            ClientRequest::GetEffortOptions => vec![E::EffortOptions {
                options: vec!["default".into(), "high".into()],
                selected: 0,
                note: String::new(),
                state: "ready".into(),
            }],
            ClientRequest::SetEffort { .. } | ClientRequest::SetSessionPrefs { .. } => {
                vec![E::Error(text.clone())]
            }
            _ => readback(request, "").unwrap_or_default(),
        }));
        let mut adapter = coding(c, &store).await;
        let effort = call(&mut adapter, "config.effort.set", json!({"effort":"high"}))
            .await
            .unwrap_err();
        let limits = call(
            &mut adapter,
            "config.limits.set",
            json!({"max_output_tokens":2048}),
        )
        .await
        .unwrap_err();
        for error in [effort, limits] {
            assert_eq!(reason(&error), Some("daemon_refused"));
            assert!(error.message.contains("[redacted]"), "{}", error.message);
            assert_no_secret(&serde_json::to_string(&error).unwrap());
        }
        requests(adapter, peer).await;
    }

    #[tokio::test]
    async fn session_main_release_is_confirmed_by_the_readback_route() {
        let (_dir, store) = store_with_fixture();
        let pinned = std::sync::Arc::new(std::sync::Mutex::new(true));
        let state = pinned.clone();
        let (c, peer) = fake_daemon(Box::new(move |request| match request {
            ClientRequest::GetRunState { req_seq } => {
                let origin = if *state.lock().unwrap() {
                    "session"
                } else {
                    "global"
                };
                vec![E::RunState {
                    req_seq: *req_seq,
                    state: run_state("", origin, "m-main"),
                }]
            }
            ClientRequest::SetSessionMain { model_uuid: None } => {
                *state.lock().unwrap() = false;
                vec![E::Ack]
            }
            _ => vec![],
        }));
        // The adapter's last frame says Main is global; the daemon's
        // readback says the session pins it. The readback decides.
        let mut adapter = coding(c, &store).await;
        assert_ne!(
            adapter
                .binding
                .coding_mut()
                .and_then(|c| c.projection.info.main())
                .map(|main| main.origin.as_str()),
            Some("session")
        );
        let result = call(
            &mut adapter,
            "config.roles.unassign",
            json!({"role":"main","scope":"session"}),
        )
        .await
        .unwrap();
        assert_eq!(result["changed"], true);
        assert_eq!(result["previous_model_uuid"], "m-main");
        assert_eq!(result["config"]["view"], "session");
        assert_eq!(result["propagation"], json!({"bound":"applied_by_daemon"}));
        assert_eq!(result["session_info"]["routes"][0]["origin"], "global");
        let again = call(
            &mut adapter,
            "config.roles.unassign",
            json!({"role":"main","scope":"session"}),
        )
        .await
        .unwrap();
        assert_eq!(again["changed"], false);
        let seen = requests(adapter, peer).await;
        assert_eq!(
            seen.iter()
                .filter(|r| matches!(r, ClientRequest::SetSessionMain { .. }))
                .count(),
            1
        );
        drop(pinned);
    }

    #[tokio::test]
    async fn a_global_release_in_a_coding_binding_reloads_the_bound_daemon() {
        let (_dir, store) = store_with_fixture();
        let (c, peer) = fake_daemon(Box::new(|request| match request {
            ClientRequest::ReloadGlobalCatalogue => vec![E::Ack],
            _ => readback(request, "").unwrap_or_default(),
        }));
        let mut adapter = coding(c, &store).await;
        let result = call(
            &mut adapter,
            "config.roles.unassign",
            json!({"role":"awareness"}),
        )
        .await
        .unwrap();
        assert_eq!(result["changed"], true);
        assert_eq!(result["config"]["view"], "global");
        assert_eq!(result["propagation"]["bound"], "reloaded");
        assert!(result.get("session_info").is_some());
        let session = call(&mut adapter, "config.get", json!({})).await.unwrap();
        assert_eq!(session["view"], "session");
        assert_eq!(session["roles"][0]["provider_name"], "local");
        let seen = requests(adapter, peer).await;
        assert!(seen
            .iter()
            .any(|r| matches!(r, ClientRequest::ReloadGlobalCatalogue)));
    }

    #[tokio::test]
    async fn skill_roots_in_a_coding_binding_are_written_here_and_reloaded_by_the_daemon() {
        let (dir, store) = store_with_fixture();
        let root = skill_root_dir(&dir, "team-skills");
        let (c, peer) = fake_daemon(Box::new(|request| match request {
            ClientRequest::ReloadGlobalCatalogue => vec![E::Ack],
            _ => readback(request, "").unwrap_or_default(),
        }));
        let mut adapter = coding(c, &store).await;
        let result = call(
            &mut adapter,
            "config.skill_roots.set",
            json!({"roots":[root]}),
        )
        .await
        .unwrap();
        assert_eq!(result["changed"], true);
        assert_eq!(result["propagation"]["bound"], "reloaded");
        assert!(result.get("session_info").is_some());
        assert_eq!(store.read().unwrap().config.extra_skill_roots, vec![root]);
        let seen = requests(adapter, peer).await;
        assert!(seen
            .iter()
            .any(|r| matches!(r, ClientRequest::ReloadGlobalCatalogue)));
        assert!(!seen
            .iter()
            .any(|r| matches!(r, ClientRequest::SetExtraSkillRoots { .. })));
    }
}
