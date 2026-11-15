//! Team setup: `room.team.get`, `room.models.set`, `room.avatars.upsert` and
//! `room.avatars.remove`.
//!
//! A room's team comes from three registry layers, lowest first: the embedded
//! files, the template at `<koma data dir>/room/` and the room's own
//! `<session dir>/room/`. A view builds the registry of each step with the
//! daemon's own loader and diffs them, so every avatar field says which layer
//! it comes from. Roles are shown, never written.
//!
//! A write edits `avatars.toml` of one layer (`target`): under the layer's SDK
//! lock it checks the layer revision, edits the parsed file keeping every other
//! entry and key, loads the candidate layers with the daemon's loader and
//! refuses a change the loader would drop, then writes a temp file, renames it
//! over the old one and reads it back. A live room that can reload is told to,
//! and the change is checked in its answer; when the room does not take it, the
//! old bytes are put back.
use super::{bound_room, object, out_of_range, RoomEnv};
use crate::app::runtime::sdk::binding::{refusal, Binding, BindingState, Family};
use crate::app::runtime::sdk::host::config::ConfigStore;
use crate::app::runtime::sdk::host::lock::{self, ConfigLock};
use crate::app::runtime::sdk::methods::HandlerFuture;
use crate::app::runtime::sdk::protocol::{Reason, Request, RpcError, CONFIG_LOCK_WAIT};
use crate::app::runtime::sdk::Adapter;
use crate::model::app_config::AppConfig;
use crate::model::settings::Settings;
use crate::room::registry::{
    blocks_avatar_write, layer_revision, read_overlay, validate_layers, Avatar, LaneKind,
    LaneModel, Registry, Tree, TEMPLATE_WARNING_PREFIX,
};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// `Team` caps.
const MAX_AVATARS: usize = 128;
const MAX_ROLES: usize = 64;
const MAX_CHOICES: usize = 2000;
const MAX_WARNINGS: usize = 100;
/// The effective team holds at most this many avatars.
const TEAM_CAP: usize = 64;
const DISPLAY_CHARS: usize = 64;
const EFFORT_CHARS: usize = 64;
const ID_CHARS: usize = 64;
const AVATARS_FILE: &str = "avatars.toml";
const LOCK_FILE: &str = ".sdk-lock";
/// Beside `avatars.toml`; the loader reads only `.md`, `.toml` and `.json`.
const TMP_FILE: &str = ".avatars.toml.sdk-tmp";
const ROOM_STATES: &[BindingState] = &[BindingState::RoomLive, BindingState::RoomClosed];

// ── DTOs ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, PartialEq)]
struct Field {
    value: Option<String>,
    origin: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct LaneView {
    lane: &'static str,
    provider_uuid: Field,
    model: Field,
    effort: Field,
    model_label: Option<String>,
    resolves: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct AvatarView {
    id: String,
    role: String,
    defined_in: &'static str,
    removable: bool,
    display: Field,
    display_effective: String,
    alias: Field,
    alias_effective: String,
    lanes: Vec<LaneView>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct RoleLaneView {
    provider_uuid: Option<String>,
    model: Option<String>,
    effort: Option<String>,
}

impl From<&LaneModel> for RoleLaneView {
    fn from(lane: &LaneModel) -> Self {
        Self {
            provider_uuid: lane.provider.clone(),
            model: lane.model.clone(),
            effort: lane.effort.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct RoleView {
    id: String,
    display: String,
    shelf_allow: Vec<String>,
    /// `None`: every skill.
    skills_allow: Option<Vec<String>>,
    chat: RoleLaneView,
    work: RoleLaneView,
    defined_in: &'static str,
}

/// One model a lane can be pointed at.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Choice {
    pub id: String,
    pub name: String,
    pub model_id: String,
    pub provider_name: Option<String>,
    pub provider_uuid: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct Warning {
    layer: &'static str,
    path: String,
    message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct Team {
    target: &'static str,
    revision: String,
    layer_present: bool,
    template_revision: String,
    template_pending_restart: bool,
    avatars: Vec<AvatarView>,
    avatars_truncated: bool,
    roles: Vec<RoleView>,
    roles_truncated: bool,
    choices: Vec<Choice>,
    choices_truncated: bool,
    choices_source: &'static str,
    warnings: Vec<Warning>,
    warnings_truncated: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct Propagation {
    room: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct TeamWrite {
    team: Team,
    applies: &'static str,
    propagation: Propagation,
    /// `room.avatars.remove` only.
    #[serde(skip_serializing_if = "Option::is_none")]
    reverted_to: Option<Option<&'static str>>,
}

// ── errors ──────────────────────────────────────────────────────────────

fn invalid(reason: Reason, message: impl Into<String>) -> RpcError {
    RpcError::reason(reason, message)
}

fn not_found(reason: Reason, message: impl Into<String>) -> RpcError {
    RpcError::reason(reason, message)
}

fn io_error(what: &str, e: impl std::fmt::Display) -> RpcError {
    RpcError::reason(Reason::IoError, format!("{what}: {e}"))
}

fn warnings_detail(warnings: &[String]) -> Value {
    let list: Vec<Warning> = warnings
        .iter()
        .take(MAX_WARNINGS)
        .map(|w| warning_of(w, None))
        .collect();
    json!({"warnings": list, "truncated": warnings.len() > MAX_WARNINGS})
}

fn layer_rejected(message: impl Into<String>, warnings: &[String]) -> RpcError {
    invalid(Reason::LayerRejected, message).with_detail(warnings_detail(warnings))
}

// ── targets and scope ───────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Template,
    Room,
}

impl Target {
    fn name(self) -> &'static str {
        match self {
            Target::Template => "template",
            Target::Room => "room",
        }
    }
}

/// `target`, defaulting for reads to the room when one is bound. A room
/// target outside a room binding gets the room family's binding error.
fn target_param(
    map: &Map<String, Value>,
    method: &str,
    state: BindingState,
    write: bool,
) -> Result<Target, RpcError> {
    let in_room = ROOM_STATES.contains(&state);
    let target = match map.get("target") {
        None | Some(Value::Null) if write => {
            return Err(invalid(Reason::TargetRequired, "target is required"))
        }
        None | Some(Value::Null) if in_room => Target::Room,
        None | Some(Value::Null) => Target::Template,
        Some(Value::String(t)) if t == "template" => Target::Template,
        Some(Value::String(t)) if t == "room" => Target::Room,
        Some(_) => return Err(out_of_range("target must be template or room")),
    };
    if target == Target::Room && !in_room {
        return Err(refusal(method, Family::Room, ROOM_STATES, state));
    }
    Ok(target)
}

/// Where one team call reads and writes.
#[derive(Clone)]
struct Scope {
    target: Target,
    template_dir: Option<PathBuf>,
    /// The room's own layer; a room target only.
    overlay: Option<PathBuf>,
    parent: Option<String>,
    /// The template revision the bound room read at start; a room target only.
    frozen: Option<String>,
    env: RoomEnv,
    store: ConfigStore,
}

impl Scope {
    fn of(adapter: &Adapter, target: Target) -> Result<Self, RpcError> {
        let env = adapter.room_env.clone();
        let mut scope = Scope {
            target,
            template_dir: (env.template_dir)(),
            overlay: None,
            parent: None,
            frozen: None,
            env,
            store: adapter.config.clone(),
        };
        if target == Target::Room {
            let room = bound_room(adapter)?;
            scope.overlay = Some(crate::room::registry::overlay_dir_for(room.session_dir()));
            scope.parent = room.parent_session_id().map(str::to_string);
            scope.frozen = room.template_revision().map(str::to_string);
        }
        Ok(scope)
    }

    fn layer_dir(&self) -> Result<PathBuf, RpcError> {
        match self.target {
            Target::Template => self.template_dir.clone(),
            Target::Room => self.overlay.clone(),
        }
        .ok_or_else(|| {
            RpcError::reason(Reason::IoError, "the koma data directory cannot be resolved")
        })
    }

    /// The settings of the room's parent session, whose session models the
    /// room offers too. Defaults when there is none or it cannot be read.
    fn parent_settings(&self) -> Settings {
        let Some(parent) = &self.parent else {
            return Settings::default();
        };
        let Ok(rows) = (self.env.sessions)() else {
            return Settings::default();
        };
        rows.iter()
            .find(|row| &row.uuid == parent)
            .and_then(|row| (self.env.session_dir)(&row.pwd_hash, &row.uuid).ok())
            .and_then(|dir| Settings::load(&dir.join("settings.json")).ok())
            .unwrap_or_default()
    }
}

// ── layers and registries ───────────────────────────────────────────────

/// One layer directory as the loader reads it.
#[derive(Debug, Clone, Default)]
struct Layer {
    tree: Tree,
    read_warnings: Vec<String>,
}

impl Layer {
    fn read(dir: Option<&Path>) -> Self {
        let mut read_warnings = Vec::new();
        let tree = dir.map_or_else(Tree::new, |dir| read_overlay(dir, &mut read_warnings));
        Self {
            tree,
            read_warnings,
        }
    }

    fn revision(&self) -> String {
        layer_revision(&self.tree)
    }

    /// The avatar ids `avatars.toml` of this layer has an entry for.
    fn entry_ids(&self) -> BTreeSet<String> {
        let Some(text) = self.tree.get(AVATARS_FILE) else {
            return BTreeSet::new();
        };
        let Ok(doc) = toml::from_str::<toml::Table>(text) else {
            return BTreeSet::new();
        };
        doc.get("avatar")
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.get("id").and_then(toml::Value::as_str))
            .map(str::to_string)
            .collect()
    }
}

/// The registries of base, base + template and, for a room, base + template +
/// session, with the loader's warnings for the whole stack.
struct Stack {
    base: Registry,
    template: Registry,
    session: Option<Registry>,
    warnings: Vec<String>,
}

impl Stack {
    fn build(template: &Tree, session: Option<&Tree>) -> Self {
        let (base, _) = validate_layers(&Tree::new(), None);
        let (with_template, template_warnings) = validate_layers(template, None);
        let (session, warnings) = match session {
            Some(tree) => {
                let (registry, warnings) = validate_layers(template, Some(tree));
                (Some(registry), warnings)
            }
            None => (None, template_warnings),
        };
        Self {
            base,
            template: with_template,
            session,
            warnings,
        }
    }

    fn top(&self) -> &Registry {
        self.session.as_ref().unwrap_or(&self.template)
    }

    /// The registry under the target layer.
    fn below(&self, target: Target) -> &Registry {
        match target {
            Target::Template => &self.base,
            Target::Room => &self.template,
        }
    }

    /// The layer whose registry first holds `get`'s value at the top.
    fn origin<T: PartialEq>(&self, get: impl Fn(&Registry) -> Option<T>) -> &'static str {
        let top = get(self.top());
        if self.session.is_some() && get(&self.template) != top {
            return "session";
        }
        if get(&self.base) != get(&self.template) {
            return "template";
        }
        "base"
    }

    /// The warnings about the target layer, as the loader words them for that
    /// layer alone (template warnings without their prefix).
    fn layer_warnings(&self, target: Target) -> Vec<String> {
        self.warnings
            .iter()
            .filter_map(|w| match (target, w.strip_prefix(TEMPLATE_WARNING_PREFIX)) {
                (Target::Template, Some(rest)) => Some(rest.to_string()),
                (Target::Room, None) => Some(w.clone()),
                _ => None,
            })
            .collect()
    }
}

/// Everything a view or a write reads from disk.
struct Inputs {
    config: AppConfig,
    settings: Settings,
    template: Layer,
    session: Option<Layer>,
}

impl Inputs {
    fn read(scope: &Scope) -> Result<Self, RpcError> {
        Ok(Self {
            config: scope.store.read()?.config,
            settings: scope.parent_settings(),
            template: Layer::read(scope.template_dir.as_deref()),
            session: scope.overlay.as_deref().map(|dir| Layer::read(Some(dir))),
        })
    }

    fn target_layer(&self, target: Target) -> &Layer {
        match (target, &self.session) {
            (Target::Room, Some(session)) => session,
            _ => &self.template,
        }
    }

    fn stack(&self) -> Stack {
        Stack::build(
            &self.template.tree,
            self.session.as_ref().map(|layer| &layer.tree),
        )
    }
}

/// The chief only talks and the clerk only works.
fn lane_exists(avatar: &str, lane: LaneKind) -> bool {
    !matches!(
        (avatar, lane),
        ("chief", LaneKind::Work) | ("clerk", LaneKind::Chat)
    )
}

fn lane_name(lane: LaneKind) -> &'static str {
    match lane {
        LaneKind::Chat => "chat",
        LaneKind::Work => "work",
    }
}

fn lane_of(avatar: &Avatar, lane: LaneKind) -> &LaneModel {
    match lane {
        LaneKind::Chat => &avatar.chat,
        LaneKind::Work => &avatar.work,
    }
}

// ── choices ─────────────────────────────────────────────────────────────

/// The models a lane can run, by the room's own rule: the parent session's
/// models first, then the global catalogue, a session copy of a global entry
/// standing for it once.
fn config_choices(config: &AppConfig, settings: &Settings) -> Vec<Choice> {
    let mut choices: Vec<Choice> = Vec::new();
    for entry in settings.session_models.iter().chain(config.models.iter()) {
        let id = entry
            .source_uuid
            .clone()
            .unwrap_or_else(|| entry.uuid.clone());
        if choices.iter().any(|c| c.id == id || c.id == entry.uuid) {
            continue;
        }
        let provider = config
            .providers
            .iter()
            .find(|p| p.uuid == entry.provider_uuid);
        let name = if entry.name.trim().is_empty() {
            entry.model_id.clone()
        } else {
            entry.name.clone()
        };
        choices.push(Choice {
            id,
            name,
            model_id: entry.model_id.clone(),
            provider_name: provider.map(|p| p.name.clone()).filter(|n| !n.is_empty()),
            provider_uuid: Some(entry.provider_uuid.clone()).filter(|p| !p.is_empty()),
        });
    }
    choices
}

/// The choices of a live room's `models` answer, with provider ids joined from
/// the config.
fn daemon_choices(models: &Value, config: &AppConfig, settings: &Settings) -> Vec<Choice> {
    let known = config_choices(config, settings);
    let text = |c: &Value, key: &str| {
        c.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    models
        .get("choices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let id = text(c, "id");
            if id.is_empty() {
                return None;
            }
            let provider = text(c, "provider");
            let provider_uuid = known
                .iter()
                .find(|k| k.id == id)
                .and_then(|k| k.provider_uuid.clone())
                .or_else(|| {
                    config
                        .providers
                        .iter()
                        .find(|p| !provider.is_empty() && p.name == provider)
                        .map(|p| p.uuid.clone())
                });
            Some(Choice {
                id,
                name: text(c, "name"),
                model_id: text(c, "model_id"),
                provider_name: Some(provider).filter(|p| !p.is_empty()),
                provider_uuid,
            })
        })
        .collect()
}

/// The label a room shows for a lane, by the rule of its `models` answer.
fn lane_label(registry: &Registry, choices: &[Choice], avatar: &str, lane: LaneKind) -> String {
    let model = registry.lane_model(avatar, lane);
    match model.model.as_deref().filter(|m| !m.is_empty()) {
        None => "session default".to_string(),
        Some(id) => {
            let shown = choices
                .iter()
                .find(|c| c.id == id || c.model_id == id || c.name == id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| id.to_string());
            match model.effort.as_deref().filter(|e| !e.is_empty()) {
                Some(effort) => format!("{shown} · {effort}"),
                None => shown,
            }
        }
    }
}

// ── the view ────────────────────────────────────────────────────────────

fn warning_of(warning: &str, layer: Option<&'static str>) -> Warning {
    let (prefixed, rest) = match warning.strip_prefix(TEMPLATE_WARNING_PREFIX) {
        Some(rest) => ("template", rest),
        None => ("session", warning),
    };
    let (path, message) = match rest.split_once(": ") {
        Some((path, message)) if !path.is_empty() && !path.contains(' ') => {
            (path.to_string(), message.to_string())
        }
        _ => (String::new(), rest.to_string()),
    };
    Warning {
        layer: layer.unwrap_or(prefixed),
        path,
        message,
    }
}

struct ViewOf<'a> {
    stack: &'a Stack,
    inputs: &'a Inputs,
    choices: &'a [Choice],
    entries: BTreeSet<String>,
}

impl ViewOf<'_> {
    fn field(
        &self,
        id: &str,
        own: Option<String>,
        get: impl Fn(&Avatar) -> Option<String>,
        fallback: (Option<String>, &'static str),
    ) -> Field {
        match own {
            Some(value) => Field {
                value: Some(value),
                origin: self.stack.origin(|r| r.avatar(id).and_then(&get)),
            },
            None => Field {
                value: fallback.0,
                origin: fallback.1,
            },
        }
    }

    fn lane(&self, avatar: &Avatar, lane: LaneKind) -> LaneView {
        let top = self.stack.top();
        let role_lane = top.role(&avatar.role).map(|r| match lane {
            LaneKind::Chat => &r.chat,
            LaneKind::Work => &r.work,
        });
        let pick = |part: fn(&LaneModel) -> &Option<String>| {
            let own = part(lane_of(avatar, lane)).clone();
            let fallback = match role_lane.and_then(|l| part(l).clone()) {
                Some(value) => (Some(value), "role"),
                None => (None, "default"),
            };
            self.field(&avatar.id, own, |a| part(lane_of(a, lane)).clone(), fallback)
        };
        let effective = top.lane_model(&avatar.id, lane);
        let model = effective.model.as_deref().filter(|m| !m.is_empty());
        let provider = effective
            .provider
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty());
        let resolves = match (provider, model) {
            (Some(provider), model) => crate::app::resolve::resolve_lane_route(
                &self.inputs.config,
                &self.inputs.settings,
                provider,
                model,
            )
            .is_some(),
            (None, Some(model)) => self
                .choices
                .iter()
                .any(|c| c.id == model || c.model_id == model),
            (None, None) => true,
        };
        LaneView {
            lane: lane_name(lane),
            provider_uuid: pick(|l| &l.provider),
            model: pick(|l| &l.model),
            effort: pick(|l| &l.effort),
            model_label: model.map(|m| {
                self.choices
                    .iter()
                    .find(|c| c.id == m || c.model_id == m || c.name == m)
                    .map_or_else(|| m.to_string(), |c| c.name.clone())
            }),
            resolves,
        }
    }

    fn avatar(&self, avatar: &Avatar) -> AvatarView {
        let top = self.stack.top();
        let id = avatar.id.as_str();
        let defined_in = if self.stack.base.avatar(id).is_some() {
            "base"
        } else if self.stack.template.avatar(id).is_some() {
            "template"
        } else {
            "session"
        };
        let role_display = top.role(&avatar.role).map(|r| r.display.clone());
        AvatarView {
            id: avatar.id.clone(),
            role: avatar.role.clone(),
            defined_in,
            removable: self.entries.contains(id),
            display: self.field(
                id,
                avatar.display.clone(),
                |a| a.display.clone(),
                (role_display, "role"),
            ),
            display_effective: top.display_of(id).to_string(),
            alias: self.field(
                id,
                avatar.alias.clone(),
                |a| a.alias.clone(),
                (None, "default"),
            ),
            alias_effective: top.alias_of(id),
            lanes: [LaneKind::Chat, LaneKind::Work]
                .into_iter()
                .filter(|lane| lane_exists(id, *lane))
                .map(|lane| self.lane(avatar, lane))
                .collect(),
        }
    }

    fn role_defined_in(&self, id: &str) -> &'static str {
        let path = format!("roles/{id}/role.toml");
        let accepted = |layer: Option<&Layer>, prefix: &str| {
            layer.is_some_and(|l| l.tree.contains_key(&path))
                && !self
                    .stack
                    .warnings
                    .iter()
                    .any(|w| w.starts_with(&format!("{prefix}{path}: ")))
        };
        if accepted(self.inputs.session.as_ref(), "") {
            "session"
        } else if accepted(Some(&self.inputs.template), TEMPLATE_WARNING_PREFIX) {
            "template"
        } else {
            "base"
        }
    }
}

fn view(
    scope: &Scope,
    inputs: &Inputs,
    choices: Vec<Choice>,
    choices_source: &'static str,
) -> Team {
    let stack = inputs.stack();
    let layer = inputs.target_layer(scope.target);
    let of = ViewOf {
        stack: &stack,
        inputs,
        choices: &choices,
        entries: layer.entry_ids(),
    };
    let top = stack.top();
    let mut avatars: Vec<AvatarView> = top.all_avatars().map(|a| of.avatar(a)).collect();
    let avatars_truncated = avatars.len() > MAX_AVATARS;
    avatars.truncate(MAX_AVATARS);
    let mut roles: Vec<RoleView> = top
        .roles
        .values()
        .map(|role| RoleView {
            id: role.id.clone(),
            display: role.display.clone(),
            shelf_allow: role.shelf_allow.clone(),
            skills_allow: Some(role.skills_allow.clone()).filter(|s| !s.is_empty()),
            chat: RoleLaneView::from(&role.chat),
            work: RoleLaneView::from(&role.work),
            defined_in: of.role_defined_in(&role.id),
        })
        .collect();
    let roles_truncated = roles.len() > MAX_ROLES;
    roles.truncate(MAX_ROLES);
    let mut warnings: Vec<Warning> = inputs
        .template
        .read_warnings
        .iter()
        .map(|w| warning_of(w, Some("template")))
        .chain(
            inputs
                .session
                .iter()
                .flat_map(|s| s.read_warnings.iter())
                .map(|w| warning_of(w, Some("session"))),
        )
        .chain(stack.warnings.iter().map(|w| warning_of(w, None)))
        .collect();
    let warnings_truncated = warnings.len() > MAX_WARNINGS;
    warnings.truncate(MAX_WARNINGS);
    let template_revision = inputs.template.revision();
    let template_pending_restart = scope.target == Target::Room
        && scope
            .frozen
            .as_deref()
            .is_some_and(|frozen| frozen != template_revision);
    let mut choices = choices;
    let choices_truncated = choices.len() > MAX_CHOICES;
    choices.truncate(MAX_CHOICES);
    Team {
        target: scope.target.name(),
        revision: layer.revision(),
        layer_present: !layer.tree.is_empty(),
        template_revision,
        template_pending_restart,
        avatars,
        avatars_truncated,
        roles,
        roles_truncated,
        choices,
        choices_truncated,
        choices_source,
        warnings,
        warnings_truncated,
    }
}

/// Read the layers and build the view. `live` is a live room's `models`
/// answer; without one the choices come from the config.
fn read_view(scope: &Scope, live: Option<&Value>) -> Result<Team, RpcError> {
    let inputs = Inputs::read(scope)?;
    let (choices, source) = match live {
        Some(models) => (
            daemon_choices(models, &inputs.config, &inputs.settings),
            "room_daemon",
        ),
        None => (config_choices(&inputs.config, &inputs.settings), "config"),
    };
    Ok(view(scope, &inputs, choices, source))
}

/// The bound live room's `models` answer for a room target, or `None`. A
/// failed request leaves the binding in ROOM(closed), and the call goes on
/// from the config.
async fn live_models(adapter: &mut Adapter, target: Target) -> Option<Value> {
    if target != Target::Room {
        return None;
    }
    let room = adapter.binding.room_mut()?;
    if !room.is_live() {
        return None;
    }
    room.models().await.ok()
}

async fn scrubbed(adapter: &Adapter, value: &impl Serialize) -> Result<Value, RpcError> {
    let value =
        serde_json::to_value(value).map_err(|e| RpcError::new("internal", e.to_string()))?;
    Ok(super::scrub::scrub_value(&adapter.scrub, &adapter.config, value).await)
}

// ── room.team.get ───────────────────────────────────────────────────────

pub fn handle_get(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(get(r.params, adapter))
}

async fn get(params: Value, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let map = object(params)?;
    let target = target_param(&map, "room.team.get", adapter.binding.state(), false)?;
    let models = live_models(adapter, target).await;
    let scope = Scope::of(adapter, target)?;
    let team = adapter
        .blocking(move || read_view(&scope, models.as_ref()))
        .await?;
    scrubbed(adapter, &team).await
}

// ── params ──────────────────────────────────────────────────────────────

/// A key that is absent, `null` or a value.
fn nullable_string(map: &Map<String, Value>, key: &str) -> Result<Option<Option<String>>, RpcError> {
    match map.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(text)) => Ok(Some(Some(text.clone()))),
        Some(_) => Err(out_of_range(format!("{key} must be a string or null"))),
    }
}

fn id_param(map: &Map<String, Value>, key: &str) -> Result<String, RpcError> {
    match map.get(key) {
        Some(Value::String(id)) if !id.is_empty() && id.chars().count() <= ID_CHARS => {
            Ok(id.clone())
        }
        _ => Err(out_of_range(format!(
            "{key} must be a string of 1 to {ID_CHARS} characters"
        ))),
    }
}

fn revision_param(map: &Map<String, Value>) -> Result<Option<String>, RpcError> {
    match map.get("if_revision") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(revision)) => Ok(Some(revision.clone())),
        Some(_) => Err(out_of_range("if_revision must be a string")),
    }
}

/// The loader's avatar id: `[a-z0-9][a-z0-9_-]{0,47}`.
fn valid_avatar_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 48
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_' || *b == b'-')
}

fn has_control(text: &str) -> bool {
    text.chars().any(|c| c.is_control())
}

fn effort_param(map: &Map<String, Value>) -> Result<Option<Option<String>>, RpcError> {
    let effort = nullable_string(map, "effort")?;
    if let Some(Some(text)) = &effort {
        let count = text.chars().count();
        if count == 0
            || count > EFFORT_CHARS
            || text.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(out_of_range(format!(
                "effort must be 1 to {EFFORT_CHARS} characters without whitespace"
            )));
        }
    }
    Ok(effort)
}

fn display_param(map: &Map<String, Value>) -> Result<Option<Option<String>>, RpcError> {
    match nullable_string(map, "display")? {
        Some(Some(text)) => {
            if has_control(&text) {
                return Err(invalid(Reason::InvalidName, "display holds control characters"));
            }
            let text = text.trim().to_string();
            if text.is_empty() || text.chars().count() > DISPLAY_CHARS {
                return Err(out_of_range(format!(
                    "display must be 1 to {DISPLAY_CHARS} characters"
                )));
            }
            Ok(Some(Some(text)))
        }
        other => Ok(other),
    }
}

fn alias_param(map: &Map<String, Value>) -> Result<Option<Option<String>>, RpcError> {
    match nullable_string(map, "alias")? {
        Some(Some(text)) => {
            let count = text.chars().count();
            if !(1..=3).contains(&count) || !text.chars().all(|c| c.is_ascii_alphanumeric()) {
                return Err(out_of_range("alias must be 1 to 3 ASCII letters or digits"));
            }
            Ok(Some(Some(text.to_uppercase())))
        }
        other => Ok(other),
    }
}

// ── edits ───────────────────────────────────────────────────────────────

/// One change to a layer's `avatars.toml`. `Option<Option<_>>` keeps an
/// omitted key (`None`) apart from `null` (`Some(None)`).
#[derive(Debug, Clone)]
enum Edit {
    Model {
        avatar: String,
        lane: LaneKind,
        model: Option<Option<String>>,
        effort: Option<Option<String>>,
    },
    Upsert {
        id: String,
        role: Option<String>,
        display: Option<Option<String>>,
        alias: Option<Option<String>>,
    },
    Remove {
        id: String,
    },
}

impl Edit {
    fn avatar(&self) -> &str {
        match self {
            Edit::Model { avatar, .. } => avatar,
            Edit::Upsert { id, .. } | Edit::Remove { id } => id,
        }
    }
}

/// The parsed `avatars.toml` of one layer: its `[[avatar]]` entries and
/// whatever else the file holds.
#[derive(Debug, Clone, Default)]
struct AvatarsDoc {
    rest: toml::Table,
    list: Vec<toml::Value>,
}

impl AvatarsDoc {
    /// Parse the file's bytes; `None` is an empty document. A file that does
    /// not parse is the layer's own fault and is never overwritten.
    fn parse(before: Option<&[u8]>) -> Result<Self, RpcError> {
        let Some(bytes) = before else {
            return Ok(Self::default());
        };
        let broken = |e: &dyn std::fmt::Display| {
            layer_rejected(
                format!("{AVATARS_FILE} of this layer does not parse; fix it by hand first"),
                &[format!("{AVATARS_FILE}: {e}")],
            )
        };
        let text = std::str::from_utf8(bytes).map_err(|e| broken(&e))?;
        let mut rest: toml::Table = toml::from_str(text).map_err(|e| broken(&e))?;
        let list = match rest.remove("avatar") {
            None => Vec::new(),
            Some(toml::Value::Array(list)) if list.iter().all(toml::Value::is_table) => list,
            Some(_) => return Err(broken(&"`avatar` is not a list of tables")),
        };
        Ok(Self { rest, list })
    }

    fn position(&self, id: &str) -> Option<usize> {
        self.list
            .iter()
            .position(|v| v.get("id").and_then(toml::Value::as_str) == Some(id))
    }

    /// The entry for `id`, created as `{id, role}` when there is none. An
    /// existing entry is given the avatar's effective role, which is the only
    /// role the loader accepts for it.
    fn entry(&mut self, id: &str, role: &str) -> Result<&mut toml::Table, RpcError> {
        if self.position(id).is_none() {
            self.list.push(toml::Value::Table(toml::Table::new()));
        }
        let at = self.position(id).unwrap_or(self.list.len() - 1);
        let entry = self.list[at].as_table_mut().ok_or_else(|| {
            RpcError::new("internal", format!("{AVATARS_FILE}: the entry for '{id}' is not a table"))
        })?;
        entry.insert("id".into(), id.to_string().into());
        entry.insert("role".into(), role.to_string().into());
        Ok(entry)
    }

    /// Drop `id`'s entry when it holds nothing but `{id, role}` and the avatar
    /// exists below, where such an entry changes nothing.
    fn tidy(&mut self, id: &str, below: &Registry) {
        let Some(at) = self.position(id) else {
            return;
        };
        let bare = self.list[at]
            .as_table()
            .is_some_and(|t| t.keys().all(|k| k == "id" || k == "role"));
        if bare && below.avatar(id).is_some() {
            self.list.remove(at);
        }
    }

    /// The file text, or `None` when nothing is left in it.
    fn render(self) -> Result<Option<String>, RpcError> {
        let mut doc = self.rest;
        if !self.list.is_empty() {
            doc.insert("avatar".into(), toml::Value::Array(self.list));
        }
        if doc.is_empty() {
            return Ok(None);
        }
        toml::to_string(&doc)
            .map(Some)
            .map_err(|e| RpcError::new("internal", format!("{AVATARS_FILE}: {e}")))
    }
}

/// Check `edit` against the effective team and apply it to `doc`. Returns
/// `reverted_to` for a removal.
fn apply(
    edit: &Edit,
    doc: &mut AvatarsDoc,
    stack: &Stack,
    target: Target,
    choices: &[Choice],
) -> Result<Option<Option<&'static str>>, RpcError> {
    let top = stack.top();
    let below = stack.below(target);
    match edit {
        Edit::Model {
            avatar,
            lane,
            model,
            effort,
        } => {
            let Some(member) = top.avatar(avatar) else {
                return Err(not_found(Reason::UnknownAvatar,
                    format!("the team has no avatar '{avatar}'"),
                ));
            };
            let picked = match model {
                Some(Some(id)) => Some(choices.iter().find(|c| &c.id == id).ok_or_else(|| {
                    not_found(Reason::UnknownModel, format!("'{id}' is not one of the choices"))
                })?),
                _ => None,
            };
            let role = member.role.clone();
            let name = lane_name(*lane);
            let entry = doc.entry(avatar, &role)?;
            let mut unit = entry
                .get(name)
                .and_then(toml::Value::as_table)
                .cloned()
                .unwrap_or_default();
            match (model, picked) {
                (Some(Some(_)), Some(choice)) => {
                    // The lane is one unit: the model brings its own provider,
                    // and the layer keeps the effort it had.
                    let kept = unit.get("effort").cloned();
                    unit.clear();
                    match &choice.provider_uuid {
                        Some(provider) => {
                            unit.insert("provider".into(), provider.clone().into());
                            unit.insert("model".into(), choice.id.clone().into());
                        }
                        None => {
                            unit.insert("model".into(), choice.model_id.clone().into());
                        }
                    }
                    if let Some(kept) = kept {
                        unit.insert("effort".into(), kept);
                    }
                }
                (Some(None), _) => unit.clear(),
                _ => {}
            }
            match effort {
                Some(Some(effort)) => {
                    unit.insert("effort".into(), effort.clone().into());
                }
                Some(None) => {
                    unit.remove("effort");
                }
                None => {}
            }
            if unit.is_empty() {
                entry.remove(name);
            } else {
                entry.insert(name.into(), toml::Value::Table(unit));
            }
            doc.tidy(avatar, below);
            Ok(None)
        }
        Edit::Upsert {
            id,
            role,
            display,
            alias,
        } => {
            let effective_role = match top.avatar(id) {
                Some(existing) => {
                    if role.as_deref().is_some_and(|r| r != existing.role) {
                        return Err(invalid(Reason::RoleImmutable,
                            format!("the role of '{id}' is '{}' and cannot change", existing.role),
                        ));
                    }
                    if role.is_none() && display.is_none() && alias.is_none() {
                        return Err(invalid(Reason::MissingParams, "give role, display or alias"));
                    }
                    existing.role.clone()
                }
                None => {
                    let Some(role) = role else {
                        return Err(invalid(Reason::MissingParams, "a new avatar needs a role"));
                    };
                    if crate::room::registry::SINGLE_ROLES
                        .iter()
                        .any(|single| single == role || single == id)
                    {
                        return Err(invalid(Reason::ReservedId,
                            "the chief and the clerk are single; their ids and roles are reserved",
                        ));
                    }
                    if top.role(role).is_none() {
                        return Err(not_found(Reason::UnknownRole, format!("there is no role '{role}'")));
                    }
                    if top.avatars.len() >= TEAM_CAP {
                        return Err(out_of_range(format!(
                            "the team holds at most {TEAM_CAP} avatars"
                        )));
                    }
                    role.clone()
                }
            };
            let entry = doc.entry(id, &effective_role)?;
            for (key, value) in [("display", display), ("alias", alias)] {
                match value {
                    Some(Some(value)) => {
                        entry.insert(key.into(), value.clone().into());
                    }
                    Some(None) => {
                        entry.remove(key);
                    }
                    None => {}
                }
            }
            doc.tidy(id, below);
            Ok(None)
        }
        Edit::Remove { id } => {
            let Some(at) = doc.position(id) else {
                return Err(not_found(Reason::UnknownAvatar,
                    format!("this layer has no entry for '{id}'"),
                ));
            };
            doc.list.remove(at);
            let reverted_to = match below.avatar(id) {
                None => None,
                Some(_) if target == Target::Room && stack.template.avatar(id) != stack.base.avatar(id) => {
                    Some("template")
                }
                Some(_) => Some("base"),
            };
            Ok(Some(reverted_to))
        }
    }
}

// ── the write ───────────────────────────────────────────────────────────

/// What a live room must show once it has reloaded the written layer.
#[derive(Debug, Clone, PartialEq)]
enum Expected {
    /// The label of one lane in the room's `models` answer.
    Lane {
        avatar: String,
        lane: &'static str,
        label: String,
    },
    /// The avatar's display name, or `None` when it must be gone.
    Avatar { id: String, display: Option<String> },
}

impl Expected {
    fn holds(&self, models: &Value) -> bool {
        let lanes: Vec<&Value> = models
            .get("lanes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .collect();
        let text = |row: &Value, key: &str| row.get(key).and_then(Value::as_str).map(str::to_string);
        match self {
            Expected::Lane {
                avatar,
                lane,
                label,
            } => lanes.iter().any(|row| {
                text(row, "avatar").as_deref() == Some(avatar)
                    && text(row, "lane").as_deref() == Some(lane)
                    && text(row, "model").as_deref() == Some(label)
            }),
            Expected::Avatar { id, display } => {
                let rows: Vec<&&Value> = lanes
                    .iter()
                    .filter(|row| text(row, "avatar").as_deref() == Some(id))
                    .collect();
                match display {
                    None => rows.is_empty(),
                    Some(display) => {
                        !rows.is_empty()
                            && rows
                                .iter()
                                .all(|row| text(row, "display").as_deref() == Some(display))
                    }
                }
            }
        }
    }
}

/// A committed layer write. The lock is held until it is dropped.
struct Committed {
    _lock: ConfigLock,
    dir: PathBuf,
    before: Option<Vec<u8>>,
    /// The target layer's warnings before the write.
    existing: Vec<String>,
    /// `None` when the room's own view cannot be predicted.
    expected: Option<Expected>,
    reverted_to: Option<Option<&'static str>>,
}

fn read_file(path: &Path) -> Result<Option<Vec<u8>>, RpcError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_error(&format!("reading {}", path.display()), e)),
    }
}

fn unverified(message: impl Into<String>) -> RpcError {
    RpcError::reason(Reason::Unverified, message).in_process()
        .written()
}

/// Put `bytes` in place of `avatars.toml` in `dir` (remove it for `None`),
/// through a synced temp file and a rename, then read it back. A failure
/// before the rename changed nothing.
fn write_layer_file(dir: &Path, bytes: Option<&[u8]>) -> Result<(), RpcError> {
    use std::io::Write as _;
    let path = dir.join(AVATARS_FILE);
    match bytes {
        None => match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_error(&format!("removing {}", path.display()), e)),
        },
        Some(bytes) => {
            let tmp = dir.join(TMP_FILE);
            let staged = std::fs::File::create(&tmp)
                .and_then(|mut file| file.write_all(bytes).and_then(|()| file.sync_all()));
            if let Err(e) = staged {
                let _ = std::fs::remove_file(&tmp);
                return Err(io_error(&format!("writing {}", tmp.display()), e));
            }
            // Windows can refuse a rename while another process has the file
            // open, so a few short retries.
            let mut renamed = std::fs::rename(&tmp, &path);
            for attempt in 1..5u64 {
                if renamed.is_ok() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(40 * attempt));
                renamed = std::fs::rename(&tmp, &path);
            }
            if let Err(e) = renamed {
                let _ = std::fs::remove_file(&tmp);
                return Err(io_error(&format!("replacing {}", path.display()), e));
            }
        }
    }
    match read_file(&path) {
        Ok(found) if found.as_deref() == bytes => Ok(()),
        Ok(_) => Err(unverified(format!(
            "{} does not hold the written bytes",
            path.display()
        ))),
        Err(e) => Err(unverified(format!("reading {} back: {}", path.display(), e.message))),
    }
}

/// Lock the target layer, check, edit, validate and write it.
fn commit(
    scope: &Scope,
    edit: &Edit,
    if_revision: Option<&str>,
    live: Option<&Value>,
) -> Result<Committed, RpcError> {
    let dir = scope.layer_dir()?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| io_error(&format!("creating {}", dir.display()), e))?;
    let held = lock::acquire(&dir.join(LOCK_FILE), CONFIG_LOCK_WAIT).map_err(|mut e| {
        if e.extra().and_then(|x| x.reason) == Some("lock_timeout") {
            e.message = format!(
                "another SDK team write held the lock for {} ms",
                CONFIG_LOCK_WAIT.as_millis()
            );
        }
        e
    })?;
    let inputs = Inputs::read(scope)?;
    let layer = inputs.target_layer(scope.target);
    let revision = layer.revision();
    if if_revision.is_some_and(|want| want != revision) {
        return Err(RpcError::reason(Reason::Stale, format!("the layer changed; its revision is {revision}")));
    }
    let choices = match live {
        Some(models) => daemon_choices(models, &inputs.config, &inputs.settings),
        None => config_choices(&inputs.config, &inputs.settings),
    };
    let before = read_file(&dir.join(AVATARS_FILE))?;
    let mut doc = AvatarsDoc::parse(before.as_deref())?;
    let current = inputs.stack();
    let reverted_to = apply(edit, &mut doc, &current, scope.target, &choices)?;
    let text = doc.render()?;

    let mut candidate = layer.tree.clone();
    match &text {
        Some(text) => candidate.insert(AVATARS_FILE.to_string(), text.clone()),
        None => candidate.remove(AVATARS_FILE),
    };
    let next = match scope.target {
        Target::Template => Stack::build(&candidate, None),
        Target::Room => Stack::build(&inputs.template.tree, Some(&candidate)),
    };
    let existing = current.layer_warnings(scope.target);
    let avatar = edit.avatar();
    let new: Vec<String> = next
        .layer_warnings(scope.target)
        .into_iter()
        .filter(|w| !existing.contains(w) && blocks_avatar_write(w, avatar))
        .collect();
    if !new.is_empty() {
        return Err(layer_rejected(
            format!("the loader would not take this change to '{avatar}'"),
            &new,
        ));
    }

    // A room shows the result only when it runs on the template the view was
    // computed with.
    let predictable = scope.frozen.as_deref().unwrap_or("absent") == inputs.template.revision();
    let expected = (live.is_some() && predictable).then(|| {
        let top = next.top();
        match edit {
            Edit::Model { avatar, lane, .. } => Expected::Lane {
                avatar: avatar.clone(),
                lane: lane_name(*lane),
                label: lane_label(top, &choices, avatar, *lane),
            },
            Edit::Upsert { id, .. } | Edit::Remove { id } => Expected::Avatar {
                id: id.clone(),
                display: top.avatar(id).map(|_| top.display_of(id).to_string()),
            },
        }
    });

    write_layer_file(&dir, text.as_deref().map(str::as_bytes))?;
    Ok(Committed {
        _lock: held,
        dir,
        before,
        existing,
        expected,
        reverted_to,
    })
}

/// The reload warnings that show the room did not take the change to
/// `avatar`: about it or its file as a whole, and new since before the write.
fn refused_by(warnings: &[String], existing: &[String], avatar: &str) -> Vec<String> {
    warnings
        .iter()
        .filter(|w| !w.starts_with(TEMPLATE_WARNING_PREFIX))
        .filter(|w| blocks_avatar_write(w, avatar) && !existing.contains(w))
        .cloned()
        .collect()
}

async fn off_thread<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, RpcError> + Send + 'static,
) -> Result<T, RpcError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| RpcError::new("internal", e.to_string()))?
}

/// Tell a room target about the committed write. Returns `applies` and
/// `propagation.room`, or the error when the room did not take the change.
async fn apply_room(
    adapter: &mut Adapter,
    committed: &Committed,
    avatar: &str,
) -> Result<(&'static str, &'static str), RpcError> {
    let env = adapter.room_env.clone();
    let Binding::Room(room) = &mut adapter.binding else {
        return Ok(("next_start", "not_live"));
    };
    if !room.is_live() {
        let id = room.session_id().to_string();
        let alive = off_thread(move || Ok(env.alive(&id))).await?;
        return Ok(if alive {
            ("next_reload", "not_live")
        } else {
            ("next_start", "not_live")
        });
    }
    if !room.has_feature("sdk_reload") {
        return Ok(("next_reload", "not_supported"));
    }
    let Ok(reloaded) = room.reload().await else {
        return Ok(("next_reload", "failed"));
    };
    let mut refused = refused_by(&reloaded.warnings, &committed.existing, avatar);
    if !reloaded.ok {
        refused.push(
            reloaded
                .error
                .clone()
                .unwrap_or_else(|| "the room refused to reload".to_string()),
        );
    }
    let models = if refused.is_empty() {
        match room.models().await {
            Ok(models) => Some(models),
            Err(_) => return Ok(("next_reload", "failed")),
        }
    } else {
        None
    };
    let holds = match (&models, &committed.expected) {
        (Some(models), Some(expected)) => expected.holds(models),
        (Some(_), None) => true,
        (None, _) => false,
    };
    if refused.is_empty() && holds {
        if let Some(models) = &models {
            room.set_roster(models);
        }
        if !reloaded.warnings.is_empty() {
            room.notice("layer_warnings", reloaded.warnings.join("; "));
        }
        return Ok(("live", "reloaded"));
    }
    if refused.is_empty() {
        refused.push(format!(
            "{AVATARS_FILE}: the room does not show the change to '{avatar}'"
        ));
    }

    // Put the old bytes back and have the room read them again.
    let dir = committed.dir.clone();
    let before = committed.before.clone();
    let restored = off_thread(move || write_layer_file(&dir, before.as_deref())).await;
    let back = match restored {
        Ok(()) => match room.reload().await {
            Ok(again) => {
                again.ok && refused_by(&again.warnings, &committed.existing, avatar).is_empty()
            }
            Err(_) => false,
        },
        Err(_) => false,
    };
    if back {
        let _ = room.refresh_roster().await;
        Err(RpcError::reason(Reason::RejectedByRoom, format!("the room did not take the change to '{avatar}'; the layer is as it was"))
        .with_detail(warnings_detail(&refused)))
    } else {
        Err(RpcError::reason(Reason::RestoreFailed, format!(
            "the room did not take the change to '{avatar}', and the layer could not be put back"
        )).in_process()
        .written()
        .with_detail(warnings_detail(&refused)))
    }
}

async fn team_write(
    method: &'static str,
    map: Map<String, Value>,
    edit: Edit,
    adapter: &mut Adapter,
) -> Result<Value, RpcError> {
    let target = target_param(&map, method, adapter.binding.state(), true)?;
    let if_revision = revision_param(&map)?;
    if let (Edit::Remove { id }, Target::Room, Some(room)) =
        (&edit, target, adapter.binding.room())
    {
        let running = room.is_live()
            && room.cached_status().is_some_and(|status| {
                status
                    .lanes
                    .iter()
                    .any(|lane| &lane.avatar == id && lane.phase == "running")
            });
        if running {
            return Err(RpcError::reason(Reason::LaneRunning, format!("a lane of '{id}' is running; stop it first")));
        }
    }
    let models = live_models(adapter, target).await;
    let scope = Scope::of(adapter, target)?;
    let committed = {
        let (scope, edit, models) = (scope.clone(), edit.clone(), models.clone());
        adapter
            .blocking(move || commit(&scope, &edit, if_revision.as_deref(), models.as_ref()))
            .await?
    };
    let (applies, room) = match target {
        Target::Template => ("next_start", "not_live"),
        Target::Room => apply_room(adapter, &committed, edit.avatar()).await?,
    };
    let reverted_to = committed.reverted_to;
    // A lost stream on the way leaves the view to the config's choices.
    let models = models.filter(|_| room != "failed");
    let team = adapter
        .blocking(move || read_view(&scope, models.as_ref()))
        .await;
    drop(committed);
    let result = TeamWrite {
        team: team?,
        applies,
        propagation: Propagation { room },
        reverted_to,
    };
    scrubbed(adapter, &result).await
}

// ── the write methods ───────────────────────────────────────────────────

pub fn handle_models_set(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(models_set(r.params, adapter))
}

pub fn handle_avatars_upsert(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(avatars_upsert(r.params, adapter))
}

pub fn handle_avatars_remove(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(avatars_remove(r.params, adapter))
}

async fn models_set(params: Value, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let map = object(params)?;
    target_param(&map, "room.models.set", adapter.binding.state(), true)?;
    let avatar = id_param(&map, "avatar")?;
    let lane = match map.get("lane").and_then(Value::as_str) {
        Some("chat") => LaneKind::Chat,
        Some("work") => LaneKind::Work,
        _ => return Err(out_of_range("lane must be chat or work")),
    };
    let model = nullable_string(&map, "model")?;
    if model.as_ref().is_some_and(|m| m.as_deref() == Some("")) {
        return Err(out_of_range("model must be a choice id or null"));
    }
    let effort = effort_param(&map)?;
    if model.is_none() && effort.is_none() {
        return Err(invalid(Reason::MissingParams, "give model or effort"));
    }
    if !lane_exists(&avatar, lane) {
        return Err(invalid(Reason::NoSuchLane,
            format!("'{avatar}' has no {} lane", lane_name(lane)),
        ));
    }
    let edit = Edit::Model {
        avatar,
        lane,
        model,
        effort,
    };
    team_write("room.models.set", map, edit, adapter).await
}

async fn avatars_upsert(params: Value, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let map = object(params)?;
    target_param(&map, "room.avatars.upsert", adapter.binding.state(), true)?;
    let id = avatar_id_param(&map)?;
    let role = match map.get("role") {
        None | Some(Value::Null) => None,
        Some(Value::String(role)) if !role.is_empty() && role.chars().count() <= ID_CHARS => {
            Some(role.clone())
        }
        Some(_) => return Err(out_of_range("role must be a role id")),
    };
    let edit = Edit::Upsert {
        id,
        role,
        display: display_param(&map)?,
        alias: alias_param(&map)?,
    };
    team_write("room.avatars.upsert", map, edit, adapter).await
}

async fn avatars_remove(params: Value, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let map = object(params)?;
    target_param(&map, "room.avatars.remove", adapter.binding.state(), true)?;
    let id = avatar_id_param(&map)?;
    team_write("room.avatars.remove", map, Edit::Remove { id }, adapter).await
}

fn avatar_id_param(map: &Map<String, Value>) -> Result<String, RpcError> {
    match map.get("id") {
        Some(Value::String(id)) if valid_avatar_id(id) => Ok(id.clone()),
        Some(Value::String(_)) => Err(invalid(Reason::InvalidName,
            "id must match [a-z0-9][a-z0-9_-]{0,47}",
        )),
        _ => Err(out_of_range("id must be a string")),
    }
}
