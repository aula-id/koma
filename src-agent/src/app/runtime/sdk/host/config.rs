//! The `config.*` methods. Global writes in an unbound adapter, and global
//! role release in any binding, run in-process under the SDK lock. A
//! bound coding session's catalogue deletes and every per-session write go
//! through its daemon and are confirmed by a correlated readback.
use super::{lock, redact};
use crate::app::resolve::{role_resolutions, RoleResolution};
use crate::app::runtime::manage::BroadcastReport;
use crate::app::runtime::sdk::{
    binding::{refusal, BindingState, Family},
    configure::{self, Refusal, Removal, ViewOf},
    connection::Connection,
    dto::{self, Propagation, MAX_EFFORT_OPTIONS},
    methods::HandlerFuture,
    protocol::{Reason, Request, RpcError, CONFIG_LOCK_WAIT},
    reconcile::public_effort,
    Adapter,
};
use crate::ipc::proto::{ClientRequest, RunState};
use crate::model::app_config::{ApiType, AppConfig, ModelRole, SubagentRisky};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const LOCK_FILE: &str = "config.json.sdk-lock";
/// `detail.diff` lists at most this many uuids.
const MAX_DIFF: usize = 100;
const EFFORT_WAIT_DEFAULT: Duration = Duration::from_millis(3000);
const EFFORT_WAIT_MAX_MS: u64 = 10_000;
const MAX_ID_CHARS: usize = 64;
const MAX_OUTPUT_TOKENS: u32 = 1_000_000;
const MIN_OUTPUT_TOKENS: u32 = 256;
const MIN_CONTEXT_WINDOW: u64 = 1024;
const MAX_CONTEXT_WINDOW: u64 = 100_000_000;
const MAX_ALIAS_CHARS: usize = 200;

type SaveFn = dyn Fn(&Path, &AppConfig) -> anyhow::Result<()> + Send + Sync;
type RebindFn = dyn Fn(&AppConfig, &HashSet<String>, &HashSet<String>, bool) + Send + Sync;
type BroadcastFn = dyn Fn() -> BroadcastReport + Send + Sync;

/// Where `config.json` and its SDK lock live, and the side effects of a
/// committed write. The adapter uses koma's data dir; tests point it at a
/// directory of their own and replace the side effects.
#[derive(Clone)]
pub struct ConfigStore {
    /// `None` is koma's data dir, resolved as every koma reader resolves it.
    dir: Option<PathBuf>,
    save: Arc<SaveFn>,
    rebind: Arc<RebindFn>,
    broadcast: Arc<BroadcastFn>,
}

impl ConfigStore {
    pub fn home() -> Self {
        Self {
            dir: None,
            save: Arc::new(|_, config| config.save()),
            rebind: Arc::new(|config, models, providers, main_reset| {
                crate::app::cascade::rebind_consumers_after_model_removal(
                    None, config, models, providers, main_reset,
                );
            }),
            broadcast: Arc::new(
                crate::app::runtime::manage::broadcast_reload_global_catalogue_report,
            ),
        }
    }

    /// A store in `dir` whose rebind and broadcast do nothing.
    #[cfg(test)]
    pub fn at(dir: &Path) -> Self {
        Self {
            dir: Some(dir.to_path_buf()),
            save: Arc::new(|path, config| {
                let bytes = config.to_disk_bytes(&crate::model::secret_box::key_for_write()?)?;
                crate::model::secret_box::write_private(path, &bytes)?;
                Ok(())
            }),
            rebind: Arc::new(|_, _, _, _| {}),
            broadcast: Arc::new(BroadcastReport::default),
        }
    }

    #[cfg(test)]
    pub fn with_save(
        mut self,
        save: impl Fn(&Path, &AppConfig) -> anyhow::Result<()> + Send + Sync + 'static,
    ) -> Self {
        self.save = Arc::new(save);
        self
    }

    fn dir(&self) -> Result<PathBuf, RpcError> {
        match &self.dir {
            Some(dir) => Ok(dir.clone()),
            None => crate::model::store::base_dir().map_err(|e| {
                RpcError::reason(Reason::IoError, format!("koma data dir: {e}"))
            }),
        }
    }

    pub fn config_path(&self) -> Result<PathBuf, RpcError> {
        Ok(self.dir()?.join("config.json"))
    }

    pub fn lock_path(&self) -> Result<PathBuf, RpcError> {
        Ok(self.dir()?.join(LOCK_FILE))
    }

    /// `AppConfig::try_load`, which never turns a corrupt file into defaults.
    fn load_config(&self, path: &Path) -> anyhow::Result<AppConfig> {
        match &self.dir {
            None => AppConfig::try_load(),
            Some(_) => {
                let read = crate::model::secret_box::key_for_read();
                AppConfig::load_at(
                    path,
                    read.as_ref(),
                    !cfg!(test),
                    !cfg!(test),
                    crate::model::secret_box::key_for_write,
                )
            }
        }
    }

    /// Load `config.json`, then read its bytes for the revision. The bytes are
    /// read after loading because loading may seal plain-text secrets on disk.
    pub fn read(&self) -> Result<Loaded, RpcError> {
        let path = self.config_path()?;
        let config = match self.load_config(&path) {
            Ok(config) => config,
            Err(e) if is_not_found(&e) => return Ok(Loaded::missing()),
            Err(e) => return Err(RpcError::reason(Reason::ConfigUnreadable, load_failure(&e))),
        };
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                return Err(RpcError::reason(
                    Reason::IoError,
                    format!("config.json could not be read ({:?})", e.kind()),
                ))
            }
        };
        Ok(Loaded {
            missing: bytes.is_none(),
            revision: bytes
                .as_deref()
                .map_or_else(|| "absent".to_string(), revision),
            config,
            bytes,
        })
    }
}

/// What is safe to say about a failed load: the kind of failure and where it
/// is, never the error text, which can quote a value from the file, secrets
/// included.
fn load_failure(error: &anyhow::Error) -> String {
    for cause in error.chain() {
        if let Some(e) = cause.downcast_ref::<serde_json::Error>() {
            use serde_json::error::Category;
            let kind = match e.classify() {
                Category::Io => "read error",
                Category::Syntax => "syntax error",
                Category::Data => "data error",
                Category::Eof => "unexpected end",
            };
            // Errors from converting an already parsed value have no position.
            return if e.line() == 0 {
                format!("config.json is not valid ({kind})")
            } else {
                format!(
                    "config.json is not valid ({kind} at line {}, column {})",
                    e.line(),
                    e.column()
                )
            };
        }
        if let Some(e) = cause.downcast_ref::<std::io::Error>() {
            return format!("config.json could not be read ({:?})", e.kind());
        }
    }
    "config.json could not be read".to_string()
}

/// A failed save, named by its io error kind only.
fn save_failure(error: &anyhow::Error) -> String {
    match error
        .chain()
        .find_map(|cause| cause.downcast_ref::<std::io::Error>())
    {
        Some(e) => format!("config.json was not saved ({:?})", e.kind()),
        None => "config.json was not saved".to_string(),
    }
}

fn is_not_found(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound)
}

/// One read of `config.json`.
pub struct Loaded {
    pub config: AppConfig,
    pub missing: bool,
    pub revision: String,
    pub bytes: Option<Vec<u8>>,
}

impl Loaded {
    /// A missing file is an empty config.
    fn missing() -> Self {
        Self {
            config: AppConfig::default(),
            missing: true,
            revision: "absent".into(),
            bytes: None,
        }
    }
}

/// The first 16 hex chars of sha256 over the document bytes.
pub fn revision(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The parts of a config that its catalogue writes can change, keyed by uuid.
/// Secrets are left out: two documents are compared by what they route to.
#[derive(Debug, PartialEq)]
struct Catalogue {
    providers: BTreeMap<String, (String, String, ApiType, Option<String>)>,
    models: BTreeMap<String, ModelFacts>,
    oauth: BTreeSet<String>,
}

type ModelFacts = (
    String,
    String,
    String,
    Option<String>,
    Vec<ModelRole>,
    Option<String>,
);

impl Catalogue {
    fn of(config: &AppConfig) -> Self {
        Self {
            providers: config
                .providers
                .iter()
                .map(|p| {
                    let facts = (
                        p.name.clone(),
                        p.endpoint.clone(),
                        p.api_type,
                        p.ext_id.clone(),
                    );
                    (p.uuid.clone(), facts)
                })
                .collect(),
            models: config
                .models
                .iter()
                .map(|m| {
                    let facts = (
                        m.name.clone(),
                        m.model_id.clone(),
                        m.provider_uuid.clone(),
                        m.route.clone(),
                        m.effective_roles(),
                        m.source_uuid.clone(),
                    );
                    (m.uuid.clone(), facts)
                })
                .collect(),
            oauth: config.oauth_conns.iter().map(|c| c.uuid.clone()).collect(),
        }
    }

    /// Every uuid whose entry differs between the two documents.
    fn diff(&self, other: &Self) -> Vec<String> {
        fn keys<V: PartialEq>(a: &BTreeMap<String, V>, b: &BTreeMap<String, V>) -> Vec<String> {
            a.keys()
                .chain(b.keys())
                .filter(|k| a.get(*k) != b.get(*k))
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        }
        let mut out = keys(&self.providers, &other.providers);
        out.extend(keys(&self.models, &other.models));
        out.extend(self.oauth.symmetric_difference(&other.oauth).cloned());
        out
    }
}

fn diff_detail(uuids: Vec<String>) -> Value {
    let truncated = uuids.len() > MAX_DIFF;
    json!({"diff": uuids.into_iter().take(MAX_DIFF).collect::<Vec<_>>(), "truncated": truncated})
}

/// A global catalogue write.
#[derive(Debug, Clone)]
pub enum Change {
    DeleteProvider(String),
    DeleteModel(String),
    UnassignRole(ModelRole),
    /// Replace `subagent_risky`.
    SetSubagentRisky(SubagentRisky),
}

/// What a write did, or would do on a dry run.
#[derive(Debug, Clone)]
pub enum Effect {
    Removed(Removal),
    /// The entry that held the role, or `None` when nobody did.
    Released(Option<String>),
    /// Whether `subagent_risky` differed from the one stored.
    SubagentRisky { changed: bool },
}

impl Change {
    fn apply(&self, config: &mut AppConfig) -> Result<Effect, RpcError> {
        let refused = |refusal: Refusal, kind: &str, uuid: &str| {
            match refusal {
            Refusal::NotFound => {
                RpcError::reason(Reason::NotFound, format!("{kind} {uuid} not found"))
            }
            Refusal::Managed => RpcError::reason(
                Reason::Managed,
                format!("{kind} {uuid} is managed by an extension; uninstall the extension to remove it"),
            ),
        }
        };
        match self {
            Change::DeleteProvider(uuid) => configure::delete_provider(config, uuid)
                .map(Effect::Removed)
                .map_err(|r| refused(r, "provider", uuid)),
            Change::DeleteModel(uuid) => configure::delete_model(config, uuid)
                .map(Effect::Removed)
                .map_err(|r| refused(r, "model", uuid)),
            Change::UnassignRole(role) => {
                Ok(Effect::Released(configure::unassign_role(config, *role)))
            }
            Change::SetSubagentRisky(mode) => {
                let changed = config.subagent_risky != *mode;
                config.subagent_risky = *mode;
                Ok(Effect::SubagentRisky { changed })
            }
        }
    }

    /// Whether `config` shows this change in effect.
    fn holds(&self, effect: &Effect, config: &AppConfig) -> bool {
        match (self, effect) {
            (Change::DeleteProvider(uuid), Effect::Removed(removal)) => {
                !config.providers.iter().any(|p| &p.uuid == uuid)
                    && !config
                        .models
                        .iter()
                        .any(|m| removal.models_removed.contains(&m.uuid))
            }
            (Change::DeleteModel(uuid), _) => !config.models.iter().any(|m| &m.uuid == uuid),
            (Change::UnassignRole(role), _) => !config
                .models
                .iter()
                .any(|m| m.effective_roles().contains(role)),
            (Change::SetSubagentRisky(mode), _) => config.subagent_risky == *mode,
            _ => false,
        }
    }

    /// Provider and model uuids that a rebind must release.
    fn dead(&self, effect: &Effect) -> Option<(HashSet<String>, HashSet<String>, bool)> {
        let Effect::Removed(removal) = effect else {
            return None;
        };
        let models: HashSet<String> = removal.models_removed.iter().cloned().collect();
        let providers: HashSet<String> = match self {
            Change::DeleteProvider(uuid) => std::iter::once(uuid.clone()).collect(),
            _ => HashSet::new(),
        };
        Some((models, providers, removal.main_reset))
    }
}

/// The first half of an in-process write: lock, load, revision check and
/// the pure change, applied to a copy. Nothing is written.
pub struct Prepared {
    lock: lock::ConfigLock,
    pub before: Loaded,
    /// The pre-image with the change applied.
    pub after: AppConfig,
    pub effect: Effect,
}

pub fn prepare(
    store: &ConfigStore,
    change: &Change,
    if_revision: Option<&str>,
) -> Result<Prepared, RpcError> {
    let lock = lock::acquire(&store.lock_path()?, CONFIG_LOCK_WAIT)?;
    let before = store.read()?;
    if let Some(expected) = if_revision {
        if expected != before.revision {
            return Err(RpcError::reason(
                Reason::Stale,
                format!("config.json changed since revision {expected}"),
            ));
        }
    }
    let mut after = before.config.clone();
    let effect = change.apply(&mut after)?;
    Ok(Prepared {
        lock,
        before,
        after,
        effect,
    })
}

/// An in-process write after the save. The lock stays held until it is dropped.
pub struct Committed {
    lock: lock::ConfigLock,
    pub effect: Effect,
    /// The document the result shows: re-read after a save, or the pre-image
    /// when there was nothing to write.
    pub view: Loaded,
    pub written: bool,
    pub rebind: &'static str,
}

/// Save, verify by re-reading, then rebind consumers of what was removed.
pub fn commit(
    store: &ConfigStore,
    change: &Change,
    prepared: Prepared,
) -> Result<Committed, RpcError> {
    let Prepared {
        lock,
        before,
        after,
        effect,
    } = prepared;
    if matches!(
        effect,
        Effect::Released(None) | Effect::SubagentRisky { changed: false }
    ) {
        return Ok(Committed {
            lock,
            effect,
            view: before,
            written: false,
            rebind: "not_needed",
        });
    }
    let path = store.config_path()?;
    if let Err(error) = (store.save)(&path, &after) {
        let now = std::fs::read(&path).ok();
        if now == before.bytes {
            return Err(RpcError::reason(Reason::SaveFailed, save_failure(&error)));
        }
        return Err(unverified_host(
            "config.json changed although the save reported an error",
        ));
    }
    let view = store
        .read()
        .map_err(|_| unverified_host("config.json could not be read back after the save"))?;
    if !change.holds(&effect, &view.config) {
        return Err(unverified_host(
            "config.json does not show the change after the save",
        ));
    }
    let rebind = match change.dead(&effect) {
        Some((models, providers, main_reset)) => {
            (store.rebind)(&view.config, &models, &providers, main_reset);
            "done"
        }
        None => "not_needed",
    };
    Ok(Committed {
        lock,
        effect,
        view,
        written: true,
        rebind,
    })
}

fn unverified_host(message: &str) -> RpcError {
    RpcError::reason(Reason::Unverified, message).in_process()
}

/// A write on the bound coding connection whose readback shows something
/// other than the intended document. It ends the process, as every unknown
/// outcome on that connection does.
fn unverified_bound(message: &str, detail: Value) -> RpcError {
    RpcError::reason(Reason::Unverified, message).with_detail(detail)
}

// ── views ──────────────────────────────────────────────────────────────────

pub fn global_view(loaded: &Loaded) -> dto::ConfigView {
    let settings = crate::model::settings::Settings::default();
    let roles = role_resolutions(&loaded.config, &settings);
    configure::describe_all(
        &loaded.config,
        ViewOf {
            view: "global",
            revision: loaded.revision.clone(),
            missing: loaded.missing,
            roles: &roles,
            main_fallback: configure::global_main_fallback(&loaded.config),
            extra_names: &|_| None,
        },
    )
}

/// The catalogue with the bound session's own role routes.
pub fn session_view(loaded: &Loaded, connection: &Connection) -> dto::ConfigView {
    let info = &connection.projection.info;
    let daemon_names = |uuid: &str| {
        info.providers
            .iter()
            .find(|(id, _)| id == uuid)
            .map(|(_, name)| name.clone())
    };
    configure::describe_all(
        &loaded.config,
        ViewOf {
            view: "session",
            revision: loaded.revision.clone(),
            missing: loaded.missing,
            roles: &info.routes,
            main_fallback: configure::session_main_fallback(&loaded.config, info.main()),
            extra_names: &daemon_names,
        },
    )
}

/// Keep the connection's provider and OAuth names current for `session_info`.
fn remember_names(connection: &mut Connection, config: &AppConfig) {
    connection.names = config
        .providers
        .iter()
        .map(|p| (p.uuid.clone(), p.name.clone()))
        .chain(
            config
                .oauth_conns
                .iter()
                .map(|c| (c.uuid.clone(), c.name.clone())),
        )
        .collect();
}

fn main_uuid(routes: &[RoleResolution]) -> Option<String> {
    routes
        .iter()
        .find(|r| r.role == ModelRole::Main)
        .and_then(|r| r.assignment_uuid.clone())
}

/// Adds `session_info` to a result when a coding session is bound.
fn with_session_info(mut result: Value, adapter: &mut Adapter) -> Value {
    if let Some(connection) = adapter.binding.coding_mut() {
        result["session_info"] = json!(connection.session_info());
    }
    result
}

fn to_value<T: serde::Serialize>(value: &T) -> Result<Value, RpcError> {
    serde_json::to_value(value).map_err(|e| RpcError::new("internal", e.to_string()))
}

// ── params ─────────────────────────────────────────────────────────────────

fn out_of_range(message: impl Into<String>) -> RpcError {
    RpcError::reason(Reason::OutOfRange, message)
}

fn object(params: Value) -> Result<Map<String, Value>, RpcError> {
    match params {
        Value::Object(map) => Ok(map),
        _ => Err(out_of_range("params must be an object")),
    }
}

fn id_param(map: &Map<String, Value>, key: &str) -> Result<String, RpcError> {
    match map.get(key) {
        Some(Value::String(id)) if !id.is_empty() && id.chars().count() <= MAX_ID_CHARS => {
            Ok(id.clone())
        }
        Some(_) => Err(out_of_range(format!(
            "{key} must be a string of 1 to {MAX_ID_CHARS} characters"
        ))),
        None => Err(out_of_range(format!("{key} is required"))),
    }
}

fn revision_param(map: &Map<String, Value>) -> Result<Option<String>, RpcError> {
    match map.get("if_revision") {
        None => Ok(None),
        Some(Value::String(rev)) if !rev.is_empty() && rev.chars().count() <= MAX_ID_CHARS => {
            Ok(Some(rev.clone()))
        }
        Some(_) => Err(out_of_range("if_revision must be a revision string")),
    }
}

fn bool_param(map: &Map<String, Value>, key: &str) -> Result<bool, RpcError> {
    match map.get(key) {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(out_of_range(format!("{key} must be a boolean"))),
    }
}

fn str_param<'a>(map: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>, RpcError> {
    match map.get(key) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(out_of_range(format!("{key} must be a string"))),
    }
}

/// The refusal for a session-scope call made outside a coding binding.
fn needs_coding(method: &'static str, adapter: &Adapter) -> Result<(), RpcError> {
    let state = adapter.binding.state();
    if state == BindingState::Coding {
        Ok(())
    } else {
        Err(refusal(
            method,
            Family::Coding,
            &[BindingState::Coding],
            state,
        ))
    }
}

fn bound(adapter: &mut Adapter) -> Result<&mut Connection, RpcError> {
    if adapter.lost.is_some() {
        return Err(RpcError::reason(
            Reason::IoError,
            "the session connection was lost",
        ));
    }
    adapter
        .binding
        .coding_mut()
        .ok_or_else(|| RpcError::reason(Reason::WrongBinding, "no session bound"))
}

// ── handlers ───────────────────────────────────────────────────────────────

pub fn handle_get(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(get(r.params, adapter))
}
pub fn handle_providers_delete(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(providers_delete(r.params, adapter))
}
pub fn handle_models_delete(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(models_delete(r.params, adapter))
}
pub fn handle_roles_unassign(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(roles_unassign(r.params, adapter))
}
pub fn handle_subagent_risky_set(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(subagent_risky_set(r.params, adapter))
}
pub fn handle_effort_get(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(effort_get(r.params, adapter))
}
pub fn handle_effort_set(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(effort_set(r.params, adapter))
}
pub fn handle_limits_get(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(limits_get(r.params, adapter))
}
pub fn handle_limits_set(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(limits_set(r.params, adapter))
}

async fn read_config(adapter: &mut Adapter) -> Result<Loaded, RpcError> {
    let store = adapter.config.clone();
    adapter.blocking(move || store.read()).await
}

async fn get(params: Value, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let map = object(params)?;
    let coding = adapter.binding.state() == BindingState::Coding;
    let view = match str_param(&map, "view")? {
        None if coding => "session",
        None => "global",
        Some("global") => "global",
        Some("session") => "session",
        Some(_) => return Err(out_of_range("view must be \"global\" or \"session\"")),
    };
    if view == "session" {
        needs_coding("config.get", adapter)?;
    }
    let loaded = read_config(adapter).await?;
    if let Some(connection) = adapter.binding.coding_mut() {
        remember_names(connection, &loaded.config);
        if view == "session" {
            return to_value(&session_view(&loaded, connection));
        }
    }
    to_value(&global_view(&loaded))
}

async fn providers_delete(params: Value, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let map = object(params)?;
    let uuid = id_param(&map, "provider_uuid")?;
    let if_revision = revision_param(&map)?;
    let dry_run = bool_param(&map, "dry_run")?;
    let change = Change::DeleteProvider(uuid.clone());
    let (effect, view, propagation) = write(adapter, change, if_revision, dry_run).await?;
    let Effect::Removed(removal) = effect else {
        return Err(RpcError::new("internal", "provider delete released a role"));
    };
    let result = json!({
        "dry_run": dry_run,
        "removed": {
            "provider_uuid": uuid,
            "models_removed": removal.models_removed,
            "main_reset": removal.main_reset,
        },
        "config": to_value(&view)?,
        "propagation": to_value(&propagation)?,
    });
    Ok(with_session_info(result, adapter))
}

async fn models_delete(params: Value, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let map = object(params)?;
    let uuid = id_param(&map, "model_uuid")?;
    // There is no session-scope delete. A caller that asks for one must not
    // delete the global entry instead.
    if map
        .get("scope")
        .is_some_and(|scope| scope.as_str() != Some("global"))
    {
        return Err(out_of_range("config.models.delete has global scope only"));
    }
    let if_revision = revision_param(&map)?;
    let dry_run = bool_param(&map, "dry_run")?;
    let change = Change::DeleteModel(uuid.clone());
    let (effect, view, propagation) = write(adapter, change, if_revision, dry_run).await?;
    let Effect::Removed(removal) = effect else {
        return Err(RpcError::new("internal", "model delete released a role"));
    };
    let roles: Vec<&str> = removal
        .roles_released
        .iter()
        .filter_map(|role| redact::public_role(*role))
        .collect();
    let result = json!({
        "dry_run": dry_run,
        "removed": {
            "model_uuid": uuid,
            "roles_released": roles,
            "main_reset": removal.main_reset,
        },
        "config": to_value(&view)?,
        "propagation": to_value(&propagation)?,
    });
    Ok(with_session_info(result, adapter))
}

async fn roles_unassign(params: Value, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let map = object(params)?;
    let role = match str_param(&map, "role")? {
        Some(name) => configure::parse_public_role(name)
            .ok_or_else(|| out_of_range(format!("unknown role '{name}'")))?,
        None => return Err(out_of_range("role is required")),
    };
    let session = match str_param(&map, "scope")? {
        None | Some("global") => false,
        Some("session") => true,
        Some(_) => return Err(out_of_range("scope must be \"global\" or \"session\"")),
    };
    let if_revision = revision_param(&map)?;
    if session {
        if role != ModelRole::Main {
            return Err(out_of_range("scope session accepts only role main"));
        }
        if if_revision.is_some() {
            return Err(RpcError::reason(
                Reason::ConflictingParams,
                "if_revision applies to global scope only",
            ));
        }
        needs_coding("config.roles.unassign", adapter)?;
        return clear_session_main(adapter).await;
    }
    let (effect, view, propagation) =
        write(adapter, Change::UnassignRole(role), if_revision, false).await?;
    let previous = match effect {
        Effect::Released(previous) => previous,
        Effect::Removed(_) | Effect::SubagentRisky { .. } => None,
    };
    let result = json!({
        "changed": previous.is_some(),
        "previous_model_uuid": previous,
        "config": to_value(&view)?,
        "propagation": to_value(&propagation)?,
    });
    Ok(with_session_info(result, adapter))
}

/// `config.subagent_risky.set`: choose what a sub-agent's risky tool call goes
/// through. Only that key changes, under the SDK lock on a fresh read of the
/// file. Running daemons pick it up through the catalogue reload broadcast, and
/// it applies to sub-agents spawned after that.
async fn subagent_risky_set(params: Value, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let map = object(params)?;
    let mode = match map.get("mode") {
        None => return Err(RpcError::reason(Reason::MissingParams, "mode is required")),
        Some(Value::String(value)) => SubagentRisky::parse(value)
            .ok_or_else(|| out_of_range("mode must be 'classifier', 'parent' or 'deny'"))?,
        Some(_) => return Err(out_of_range("mode must be a string")),
    };
    let if_revision = revision_param(&map)?;
    let (effect, view, propagation) =
        write(adapter, Change::SetSubagentRisky(mode), if_revision, false).await?;
    let changed = matches!(effect, Effect::SubagentRisky { changed: true });
    let result = json!({
        "changed": changed,
        "mode": mode.as_str(),
        "config": to_value(&view)?,
        "propagation": to_value(&propagation)?,
    });
    Ok(with_session_info(result, adapter))
}

/// A global catalogue write: through the bound daemon for deletes in a coding
/// binding, else in-process. Returns the effect, the global view to show and
/// what happened after the commit.
async fn write(
    adapter: &mut Adapter,
    change: Change,
    if_revision: Option<String>,
    dry_run: bool,
) -> Result<(Effect, dto::ConfigView, Propagation), RpcError> {
    let through_daemon = adapter.binding.state() == BindingState::Coding
        && matches!(change, Change::DeleteProvider(_) | Change::DeleteModel(_));
    if through_daemon {
        daemon_write(adapter, change, if_revision, dry_run).await
    } else {
        local_write(adapter, change, if_revision, dry_run).await
    }
}

/// The write runs in this process.
async fn local_write(
    adapter: &mut Adapter,
    change: Change,
    if_revision: Option<String>,
    dry_run: bool,
) -> Result<(Effect, dto::ConfigView, Propagation), RpcError> {
    let store = adapter.config.clone();
    let staged = adapter
        .blocking(move || {
            let prepared = prepare(&store, &change, if_revision.as_deref())?;
            if dry_run {
                return Ok(Err(prepared));
            }
            commit(&store, &change, prepared).map(Ok)
        })
        .await?;
    let committed = match staged {
        Err(prepared) => {
            return Ok((
                prepared.effect,
                global_view(&prepared.before),
                Propagation::default(),
            ))
        }
        Ok(committed) => committed,
    };
    if let Some(connection) = adapter.binding.coding_mut() {
        remember_names(connection, &committed.view.config);
    }
    let view = global_view(&committed.view);
    if !committed.written {
        return Ok((committed.effect, view, Propagation::default()));
    }
    let mut propagation = Propagation {
        rebind: Some(committed.rebind),
        rooms: Some("next_start"),
        ..Propagation::default()
    };
    propagation.bound = Some(reload_bound(adapter).await);
    let store = adapter.config.clone();
    let report = adapter
        .blocking(move || Ok((store.broadcast)()))
        .await
        .unwrap_or(BroadcastReport {
            reached: 0,
            failed: 1,
        });
    propagation.broadcast = Some(if report.failed > 0 { "failed" } else { "sent" });
    drop(committed.lock);
    Ok((committed.effect, view, propagation))
}

/// Make the bound daemon reload the catalogue after an in-process write. A
/// failed connection does not undo the committed write: the result is still
/// sent, and then the process ends.
async fn reload_bound(adapter: &mut Adapter) -> &'static str {
    if adapter.lost.is_some() {
        return "failed";
    }
    let Some(connection) = adapter.binding.coding_mut() else {
        return "not_bound";
    };
    match connection.reload_catalogue().await {
        Ok(()) => "reloaded",
        Err(error) => {
            if error.delivery == "unknown" {
                adapter.lost = Some(anyhow::anyhow!(error.message));
            }
            "failed"
        }
    }
}

/// A global delete in a coding binding: the daemon reloads the file,
/// deletes, saves and broadcasts; the adapter confirms the whole document.
async fn daemon_write(
    adapter: &mut Adapter,
    change: Change,
    if_revision: Option<String>,
    dry_run: bool,
) -> Result<(Effect, dto::ConfigView, Propagation), RpcError> {
    let store = adapter.config.clone();
    let prepared = {
        let change = change.clone();
        adapter
            .blocking(move || prepare(&store, &change, if_revision.as_deref()))
            .await?
    };
    if dry_run {
        if let Some(connection) = adapter.binding.coding_mut() {
            remember_names(connection, &prepared.before.config);
        }
        return Ok((
            prepared.effect,
            global_view(&prepared.before),
            Propagation::default(),
        ));
    }
    let secrets = redact::secret_values(&prepared.before.config);
    let connection = bound(adapter)?;
    // The daemon saves its whole in-memory config, so it must hold the file's
    // current content first, or it would revert writes it has not loaded.
    if let Err(error) = connection.reload_catalogue().await {
        return Err(refused_or_fatal(error, &secrets));
    }
    let request = match &change {
        Change::DeleteProvider(uuid) => ClientRequest::DeleteProvider { uuid: uuid.clone() },
        Change::DeleteModel(uuid) => ClientRequest::DeleteModel {
            uuid: uuid.clone(),
            scope: "global".into(),
        },
        Change::UnassignRole(_) | Change::SetSubagentRisky(_) => {
            return Err(RpcError::new(
                "internal",
                "this change has no daemon request",
            ))
        }
    };
    if let Err(error) = connection.ack(request).await {
        if error.delivery == "unknown" {
            return Err(error);
        }
        return Err(classify_catalogue_error(adapter, &prepared, &error.message, &secrets).await);
    }
    bound(adapter)?.readback().await?;
    let store = adapter.config.clone();
    let after = adapter.blocking(move || store.read()).await.map_err(|_| {
        unverified_bound(
            "config.json could not be read back after the daemon's write",
            json!({}),
        )
    })?;
    let differs = Catalogue::of(&prepared.after).diff(&Catalogue::of(&after.config));
    if !differs.is_empty() {
        return Err(unverified_bound(
            "config.json does not match the intended document after the daemon's write",
            diff_detail(differs),
        ));
    }
    if let Some(connection) = adapter.binding.coding_mut() {
        remember_names(connection, &after.config);
    }
    let view = global_view(&after);
    drop(prepared.lock);
    Ok((prepared.effect, view, Propagation::by_daemon()))
}

/// Daemon error text with every configured secret value and URL userinfo
/// replaced. Every daemon text a new method returns passes through here.
struct DaemonText(String);

impl DaemonText {
    fn scrubbed(message: &str, secrets: &[String]) -> Self {
        Self(redact::scrub(message, secrets))
    }

    /// Scrub with the secrets of the current `config.json`. When they cannot
    /// be read, the text is withheld.
    async fn read(adapter: &mut Adapter, message: &str) -> Self {
        match read_config(adapter).await {
            Ok(loaded) => Self::scrubbed(message, &redact::secret_values(&loaded.config)),
            Err(_) => Self("the daemon reported an error".to_string()),
        }
    }

    /// The readback shows none of the change in effect.
    fn refused(self) -> RpcError {
        RpcError::reason(Reason::DaemonRefused, self.0)
    }

    /// The readback shows the change in effect despite the daemon's error.
    fn partial(self, in_memory: bool, on_disk: bool) -> RpcError {
        let error = RpcError::reason(
            Reason::PartialApply,
            format!(
                "the daemon reported an error, but the change is in effect: {}",
                self.0
            ),
        )
        .with_detail(json!({"in_memory": in_memory, "on_disk": on_disk, "daemon_error": self.0}));
        if on_disk {
            error.written()
        } else {
            error
        }
    }
}

/// The configured secret values that daemon text is scrubbed of, or `None`
/// when `config.json` cannot be read, in which case the text is withheld.
pub async fn daemon_secrets(adapter: &mut Adapter) -> Option<Vec<String>> {
    read_config(adapter)
        .await
        .ok()
        .map(|loaded| redact::secret_values(&loaded.config))
}

/// Daemon error text for a new method's result, scrubbed of `secrets`, or
/// withheld when they could not be read.
pub fn daemon_text(message: &str, secrets: Option<&[String]>) -> String {
    match secrets {
        Some(secrets) => DaemonText::scrubbed(message, secrets).0,
        None => "the daemon reported an error".to_string(),
    }
}

/// A daemon `Error` before anything of ours was in effect, or a fatal
/// connection failure.
fn refused_or_fatal(error: RpcError, secrets: &[String]) -> RpcError {
    if error.delivery == "unknown" {
        return error;
    }
    DaemonText::scrubbed(&error.message, secrets).refused()
}

/// A daemon `Error` proves nothing: several handlers change memory before a
/// save that fails. Read the daemon's memory and the file back and classify.
async fn classify_catalogue_error(
    adapter: &mut Adapter,
    prepared: &Prepared,
    message: &str,
    secrets: &[String],
) -> RpcError {
    let text = DaemonText::scrubbed(message, secrets);
    let connection = match bound(adapter) {
        Ok(connection) => connection,
        Err(error) => return error,
    };
    // Without a fresh snapshot the daemon's memory is unknown, and an unknown
    // memory counts as changed.
    let fresh = match connection.state().await {
        Ok(_) => true,
        Err(error) if error.delivery == "unknown" => return error,
        Err(_) => false,
    };
    let info = &connection.projection.info;
    let before = &prepared.before.config;
    let memory_models: BTreeSet<&String> = info.models.iter().collect();
    let memory_providers: BTreeSet<&String> = info.providers.iter().map(|(id, _)| id).collect();
    let expected_models: BTreeSet<&String> = before.models.iter().map(|m| &m.uuid).collect();
    let expected_providers: BTreeSet<&String> = before.providers.iter().map(|p| &p.uuid).collect();
    let in_memory =
        !fresh || memory_models != expected_models || memory_providers != expected_providers;
    let store = adapter.config.clone();
    let on_disk = match adapter.blocking(move || store.read()).await {
        Ok(after) => Catalogue::of(&after.config) != Catalogue::of(before),
        Err(_) => true,
    };
    if in_memory || on_disk {
        text.partial(in_memory, on_disk)
    } else {
        text.refused()
    }
}

/// `config.roles.unassign{role:"main", scope:"session"}`: drop the session's
/// Main pin, so it inherits the global Main. Whether a pin exists is decided
/// by a readback, never by the last frame the adapter happened to receive.
async fn clear_session_main(adapter: &mut Adapter) -> Result<Value, RpcError> {
    let loaded = read_config(adapter).await?;
    let connection = bound(adapter)?;
    remember_names(connection, &loaded.config);
    let first = connection.readback().await?;
    let Some(previous) = session_pin(&first) else {
        return Ok(json!({
            "changed": false,
            "previous_model_uuid": null,
            "config": to_value(&session_view(&loaded, connection))?,
            "propagation": {},
            "session_info": connection.session_info(),
        }));
    };
    let sent = connection
        .ack(ClientRequest::SetSessionMain { model_uuid: None })
        .await;
    let run = connection.readback().await?;
    let still_pinned = session_pin(&run).is_some();
    if let Err(error) = sent {
        if error.delivery == "unknown" {
            return Err(error);
        }
        let text = DaemonText::scrubbed(&error.message, &redact::secret_values(&loaded.config));
        return Err(if still_pinned {
            text.refused()
        } else {
            text.partial(true, false)
        });
    }
    if still_pinned {
        return Err(unverified_bound(
            "the session still pins Main after the daemon's reply",
            json!({}),
        ));
    }
    Ok(json!({
        "changed": true,
        "previous_model_uuid": previous,
        "config": to_value(&session_view(&loaded, connection))?,
        "propagation": {"bound": "applied_by_daemon"},
        "session_info": connection.session_info(),
    }))
}

/// The model the session pins as Main, or `None` when it inherits Main.
fn session_pin(run: &RunState) -> Option<Option<String>> {
    run.model_routes
        .iter()
        .find(|r| r.role == ModelRole::Main && r.origin == "session")
        .map(|r| r.source_uuid.clone().or_else(|| r.assignment_uuid.clone()))
}

// ── effort ─────────────────────────────────────────────────────────────────

/// The menu `options` as the SDK reports them: `"default"` first, and only
/// `"default"` while the menu is loading or unsupported.
fn public_options(menu: &crate::app::runtime::sdk::connection::EffortMenu) -> Vec<String> {
    let mut options = if menu.state == "ready" {
        menu.options.clone()
    } else {
        Vec::new()
    };
    options.retain(|option| option != "default");
    options.insert(0, "default".to_string());
    options.truncate(MAX_EFFORT_OPTIONS);
    options
}

async fn effort_get(params: Value, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let map = object(params)?;
    let wait = match map.get("wait_ms") {
        None => EFFORT_WAIT_DEFAULT,
        Some(value) => match value.as_u64() {
            Some(ms) if ms <= EFFORT_WAIT_MAX_MS => Duration::from_millis(ms),
            _ => return Err(out_of_range("wait_ms must be 0 to 10000")),
        },
    };
    let connection = bound(adapter)?;
    let run = connection.readback().await?;
    let main = main_uuid(&run.model_routes);
    let menu = connection.effort_options(wait).await?;
    let options = public_options(&menu);
    if menu.state == "ready" {
        connection.effort_cache = Some((main.clone(), options.clone()));
    }
    Ok(json!({
        "state": menu.state,
        "options": options,
        "selected": public_effort(&run.effort),
        "note": (!menu.note.is_empty()).then_some(menu.note),
        "main_model_uuid": main,
    }))
}

/// An effort level: 1 to 64 characters, no whitespace or control characters.
fn effort_param(map: &Map<String, Value>) -> Result<String, RpcError> {
    let effort = str_param(map, "effort")?.ok_or_else(|| out_of_range("effort is required"))?;
    let chars = effort.chars().count();
    if chars == 0
        || chars > MAX_EFFORT_OPTIONS
        || effort.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(out_of_range(
            "effort must be 1 to 64 characters without whitespace",
        ));
    }
    Ok(effort.to_string())
}

fn main_changed() -> RpcError {
    RpcError::reason(Reason::MainChanged, "the session's Main model changed")
}

async fn effort_set(params: Value, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let map = object(params)?;
    let effort = effort_param(&map)?;
    let if_main = match map.get("if_main_model") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(uuid)) => Some(Some(uuid.clone())),
        Some(_) => return Err(out_of_range("if_main_model must be a string or null")),
    };
    let connection = bound(adapter)?;
    let first = connection.readback().await?;
    let main = main_uuid(&first.model_routes);
    if if_main.is_some_and(|expected| expected != main) {
        return Err(main_changed());
    }
    let cached = connection
        .effort_cache
        .as_ref()
        .filter(|(uuid, _)| *uuid == main)
        .map(|(_, options)| options.clone());
    let options = match cached {
        Some(options) => options,
        None => {
            let menu = connection.effort_options(EFFORT_WAIT_DEFAULT).await?;
            match menu.state.as_str() {
                "ready" => {
                    let options = public_options(&menu);
                    connection.effort_cache = Some((main.clone(), options.clone()));
                    options
                }
                "unsupported" => vec!["default".to_string()],
                _ => {
                    return Err(RpcError::reason(
                        Reason::OptionsNotReady,
                        "the effort options are still loading; try again",
                    ))
                }
            }
        }
    };
    if !options.contains(&effort) {
        return Err(RpcError::reason(
            Reason::NotOffered,
            format!("effort '{effort}' is not offered for this model"),
        ));
    }
    let stored = if effort == "default" {
        String::new()
    } else {
        effort.clone()
    };
    let sent = connection.set_effort(effort).await;
    let after = connection.readback().await?;
    if let Err(error) = sent {
        if error.delivery == "unknown" {
            return Err(error);
        }
        let in_effect = after.effort == stored && first.effort != stored;
        let text = DaemonText::read(adapter, &error.message).await;
        return Err(if in_effect {
            text.partial(true, false)
        } else {
            text.refused()
        });
    }
    let main_after = main_uuid(&after.model_routes);
    let superseded = if after.effort == stored {
        false
    } else if main_after != main {
        // A Main change resets effort, so none of this write is in effect.
        return Err(main_changed());
    } else {
        true
    };
    Ok(json!({
        "effort": public_effort(&after.effort),
        "main_model_uuid": main_after,
        "superseded": superseded,
        "session_info": connection.session_info(),
    }))
}

// ── limits ─────────────────────────────────────────────────────────────────

/// The result of `config.limits.set`.
fn limits_result(connection: &Connection) -> Result<Value, RpcError> {
    let mut result = to_value(&connection.projection.info.limits())?;
    result["session_info"] = json!(connection.session_info());
    Ok(result)
}

/// A read: the result is `Limits` alone. Mutations add `session_info`.
async fn limits_get(params: Value, adapter: &mut Adapter) -> Result<Value, RpcError> {
    object(params)?;
    let connection = bound(adapter)?;
    connection.readback().await?;
    to_value(&connection.projection.info.limits())
}

struct LimitsIn {
    max_output_tokens: Option<u32>,
    context_window_limit: Option<u64>,
    context_model_alias: Option<String>,
}

fn limits_params(map: &Map<String, Value>) -> Result<LimitsIn, RpcError> {
    let max_output_tokens = match map.get("max_output_tokens") {
        None => None,
        Some(value) => match value.as_u64() {
            Some(0) => Some(0),
            Some(n)
                if (u64::from(MIN_OUTPUT_TOKENS)..=u64::from(MAX_OUTPUT_TOKENS)).contains(&n) =>
            {
                Some(n as u32)
            }
            _ => {
                return Err(out_of_range(
                    "max_output_tokens must be 0 or 256 to 1000000",
                ))
            }
        },
    };
    let context_window_limit = match map.get("context_window_limit") {
        None => None,
        Some(value) => match value.as_u64() {
            Some(0) => Some(0),
            Some(n) if (MIN_CONTEXT_WINDOW..=MAX_CONTEXT_WINDOW).contains(&n) => Some(n),
            _ => {
                return Err(out_of_range(
                    "context_window_limit must be 0 or 1024 to 100000000",
                ))
            }
        },
    };
    let context_model_alias = match str_param(map, "context_model_alias")? {
        None => None,
        Some(alias) if alias.chars().count() <= MAX_ALIAS_CHARS => Some(alias.trim().to_string()),
        Some(_) => {
            return Err(out_of_range(
                "context_model_alias must be at most 200 characters",
            ))
        }
    };
    if max_output_tokens.is_none()
        && context_window_limit.is_none()
        && context_model_alias.is_none()
    {
        return Err(RpcError::reason(
            Reason::MissingParams,
            "give max_output_tokens, context_window_limit or context_model_alias",
        ));
    }
    Ok(LimitsIn {
        max_output_tokens,
        context_window_limit,
        context_model_alias,
    })
}

impl LimitsIn {
    fn held_by(&self, run: &RunState) -> bool {
        self.max_output_tokens
            .is_none_or(|v| v == run.max_output_tokens)
            && self
                .context_window_limit
                .is_none_or(|v| v == run.context_window_limit)
            && self
                .context_model_alias
                .as_ref()
                .is_none_or(|v| *v == run.context_model_alias)
    }
}

async fn limits_set(params: Value, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let map = object(params)?;
    let wanted = limits_params(&map)?;
    let connection = bound(adapter)?;
    let first = connection.readback().await?;
    let sent = connection
        .set_limits(
            wanted.max_output_tokens,
            wanted.context_window_limit,
            wanted.context_model_alias.clone(),
        )
        .await;
    let after = connection.readback().await?;
    if let Err(error) = sent {
        if error.delivery == "unknown" {
            return Err(error);
        }
        let in_effect = wanted.held_by(&after) && !wanted.held_by(&first);
        let text = DaemonText::read(adapter, &error.message).await;
        return Err(if in_effect {
            text.partial(true, false)
        } else {
            text.refused()
        });
    }
    if !wanted.held_by(&after) {
        return Err(unverified_bound(
            "the session's limits do not match the request after the daemon's reply",
            json!({}),
        ));
    }
    limits_result(connection)
}
