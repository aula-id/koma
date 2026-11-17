//! The `skills.*` methods. Every call runs in the adapter process, in every
//! binding, on koma's own skill code: discovery through `SkillRegistry`, and
//! writes through the ownership-checked persistence functions. No skill IPC is
//! sent to any daemon, and no event follows a write: the result is the only
//! signal.
//!
//! A skill is addressed by `(tier, name)` inside that tier's own root, so a
//! same-named skill in a later tier never hides the target of a write. Every
//! refusal is decided by the adapter's own checks before the core runs; when
//! the core still fails, the outcome is classified by re-reading the disk,
//! never by its error text.
use crate::app::runtime::sdk::{
    binding::{Binding, BindingState},
    host::config::ConfigStore,
    methods::HandlerFuture,
    protocol::{Reason, Request, RpcError},
    Adapter,
};
use crate::model::skill::{self, OwnedSkillTarget, SkillEdit, SkillRegistry, SkillSource};
use base64::Engine as _;
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// Most rows in one `skills.list` page.
pub const MAX_LIST: usize = 1000;
const DEFAULT_LIST: usize = 500;
/// Decoded bytes of an inline `data_b64` package; it fits the 40 MiB frame.
pub const INLINE_ZIP_BYTES: usize = 28 * 1024 * 1024;
/// The base64 length of `INLINE_ZIP_BYTES`, padding included. Checked before
/// anything is decoded.
pub const INLINE_B64_CHARS: usize = 39_146_840;
/// The core's caps on a package and its entries.
pub const ZIP_BYTES: u64 = 64 * 1024 * 1024;
pub const ENTRY_BYTES: u64 = 2 * 1024 * 1024;
pub const COMPANION_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_FILES: usize = 1000;
pub const MAX_DEPTH: usize = 16;
pub const NAME_BYTES: usize = 64;
/// `skills.get` `raw`, `skills.read_file` `text` and a new skill's text.
const TEXT_BYTES: u64 = 2 * 1024 * 1024;
const MAX_DIAGNOSTICS: usize = 200;
const MAX_DIAGNOSTIC_CHARS: usize = 1000;
const MAX_LISTED: usize = 64;
const MAX_COMPANIONS: usize = 1000;
const MAX_DESCRIPTION_CHARS: usize = 2000;
const MAX_TRIGGERS: usize = 32;
const MAX_TRIGGER_CHARS: usize = 200;
const MAX_TOOLS: usize = 64;
const MAX_TOOL_CHARS: usize = 100;
/// Room for the frontmatter keys and fences around the user's text.
const FRONTMATTER_SLACK: usize = 256;
const RESERVED_NAMES: &[&str] = &["skill", "readme"];
/// `FILE_FLAG_OPEN_REPARSE_POINT`: open a symbolic link itself, not its target.
#[cfg(windows)]
const OPEN_REPARSE_POINT: u32 = 0x0020_0000;

/// `hello.limits.skills`.
pub fn limits() -> Value {
    serde_json::json!({
        "inline_zip_bytes": INLINE_ZIP_BYTES,
        "zip_bytes": ZIP_BYTES,
        "entry_bytes": ENTRY_BYTES,
        "companion_bytes": COMPANION_BYTES,
        "files": MAX_FILES,
        "depth": MAX_DEPTH,
        "name_bytes": NAME_BYTES,
    })
}

// ── DTOs ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SkillRow {
    pub name: String,
    pub scope: &'static str,
    pub tier: String,
    pub description: String,
    pub triggers: Vec<String>,
    pub triggers_truncated: bool,
    pub tags: Vec<String>,
    pub tags_truncated: bool,
    pub pinned: bool,
    pub editable: bool,
    pub generation: String,
    /// Display only; calls address a skill by `(tier, name)`.
    pub path: String,
    pub effective: bool,
    /// The tier whose same-named skill wins over this one.
    pub shadowed_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SkillDetail {
    #[serde(flatten)]
    pub row: SkillRow,
    /// The skill's own text, verbatim.
    pub instruction: String,
    pub declared_tools: Vec<String>,
    pub declared_tools_truncated: bool,
    pub companion_files: Vec<String>,
    pub companion_files_truncated: bool,
    pub structured_save_supported: bool,
    /// The whole skill file, verbatim, when asked for.
    pub raw: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Diagnostic {
    pub path: String,
    pub error: String,
}

#[derive(Debug, Serialize)]
pub struct ListResult {
    pub project_workdir: Option<String>,
    pub skills: Vec<SkillRow>,
    pub diagnostics: Vec<Diagnostic>,
    pub diagnostics_truncated: bool,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FileResult {
    pub path: String,
    pub text: String,
    pub bytes: usize,
}

/// What a committed skill write reaches beyond the disk.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Propagation {
    pub skills: &'static str,
}

#[derive(Debug, Serialize)]
pub struct WriteResult {
    pub skill: SkillDetail,
    pub propagation: Propagation,
}

#[derive(Debug, Serialize)]
pub struct UpdateResult {
    pub skill: SkillDetail,
    pub renamed: bool,
    pub propagation: Propagation,
}

#[derive(Debug, Serialize)]
pub struct RemoveResult {
    pub removed: bool,
    pub now_effective: Option<SkillRow>,
    pub propagation: Propagation,
}

/// When a committed write reaches the sessions that use the catalogue.
///
/// A coding session holds a snapshot of the catalogue that is rebuilt with
/// its system prompt (on load, on an agent-mode change, after compaction, on
/// a memory-file change and after some tool calls), not before every prompt.
/// Adds, removes and renames reach it at that rebuild; a body edit to a skill
/// already in the snapshot is read from its file when the skill is next
/// loaded. A room freezes its catalogue when it starts.
pub fn propagation(state: BindingState) -> Propagation {
    let skills = match state {
        BindingState::Unbound | BindingState::Coding => "next_rebuild",
        BindingState::RoomLive | BindingState::RoomClosed => "room_restart",
    };
    Propagation { skills }
}

// ── tiers ──────────────────────────────────────────────────────────────────

/// A discovery tier. Later tiers win by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tier {
    Global,
    Claude,
    ProjectAgent,
    ProjectAgents,
    /// A configured extra root, by its index in `extra_skill_roots`.
    ExtraRoot(usize),
}

impl Tier {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "global" => Some(Self::Global),
            "claude" => Some(Self::Claude),
            "project-agent" => Some(Self::ProjectAgent),
            "project-agents" => Some(Self::ProjectAgents),
            other => {
                let index = other.strip_prefix("extra-root:")?;
                if index.is_empty() || !index.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                index.parse().ok().map(Self::ExtraRoot)
            }
        }
    }

    pub fn name(self) -> String {
        match self {
            Self::Global => "global".into(),
            Self::Claude => "claude".into(),
            Self::ProjectAgent => "project-agent".into(),
            Self::ProjectAgents => "project-agents".into(),
            Self::ExtraRoot(index) => format!("extra-root:{index}"),
        }
    }

    /// Discovery order. Extra roots follow their configuration order.
    fn rank(self) -> usize {
        match self {
            Self::Global => 0,
            Self::Claude => 1,
            Self::ProjectAgent => 2,
            Self::ProjectAgents => 3,
            Self::ExtraRoot(index) => 4 + index,
        }
    }

    pub fn scope(self) -> &'static str {
        match self {
            Self::Global => "computer",
            Self::ProjectAgent | Self::ProjectAgents => "project",
            Self::Claude | Self::ExtraRoot(_) => "external",
        }
    }

    pub fn editable(self) -> bool {
        matches!(self, Self::Global | Self::ProjectAgent | Self::ProjectAgents)
    }

    fn dir(self) -> Option<&'static str> {
        match self {
            Self::Claude => Some(".claude"),
            Self::ProjectAgent => Some(".agent"),
            Self::ProjectAgents => Some(".agents"),
            Self::Global | Self::ExtraRoot(_) => None,
        }
    }

    fn source(self) -> SkillSource {
        match self {
            Self::Global => SkillSource::Global,
            Self::Claude => SkillSource::Claude,
            Self::ProjectAgent => SkillSource::ProjectAgent,
            Self::ProjectAgents => SkillSource::ProjectAgents,
            Self::ExtraRoot(index) => SkillSource::ExtraRoot(index),
        }
    }
}

/// One discovered skill, copied out of the registry it was loaded in.
#[derive(Debug, Clone)]
struct Entry {
    tier: Tier,
    name: String,
    description: String,
    triggers: String,
    allowed_tools: Vec<String>,
    body: String,
    file_path: PathBuf,
    skill_dir: Option<PathBuf>,
    companion_files: Vec<String>,
    roundtrip_safe: bool,
    skill_id: String,
    generation: String,
    tags: Vec<String>,
    pinned: bool,
}

/// The skills of one tier and the registry they came from; the persistence
/// functions find a skill in that registry by its identity.
struct TierLoad {
    tier: Tier,
    registry: SkillRegistry,
    entries: Vec<Entry>,
    /// The skill files of this tier's root that could not be loaded.
    skipped: Vec<skill::SkippedSkill>,
}

impl TierLoad {
    fn find(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.name == name)
    }
}

/// The skills of `source` in `registry`, ordered by name.
fn entries_of(registry: &SkillRegistry, source: SkillSource, tier: Tier) -> Vec<Entry> {
    registry
        .list()
        .into_iter()
        .filter(|def| def.source == source)
        .map(|def| Entry {
            tier,
            name: def.name.clone(),
            description: def.description.clone(),
            triggers: def.triggers.clone(),
            allowed_tools: def.allowed_tools.clone(),
            body: def.body.clone(),
            file_path: def.file_path.clone(),
            skill_dir: def.skill_dir.clone(),
            companion_files: def.companion_files.clone(),
            roundtrip_safe: def.frontmatter_roundtrip_safe,
            skill_id: def.skill_id.clone(),
            generation: def.generation.clone(),
            tags: skill::search::tags_of(def),
            pinned: skill::search::is_pinned(def),
        })
        .collect()
}

/// Where the global tier lives: koma's data dir, or in tests a directory of
/// the test's own (its root is `<dir>/.agents/skills`, so the core writes it
/// with its project path and never touches the real data dir).
#[derive(Debug, Clone)]
enum GlobalRoot {
    #[cfg_attr(test, allow(dead_code))]
    Home,
    #[cfg(test)]
    At(PathBuf),
}

#[cfg(not(test))]
fn global_root(_store: &ConfigStore) -> Result<GlobalRoot, RpcError> {
    Ok(GlobalRoot::Home)
}

#[cfg(test)]
fn global_root(store: &ConfigStore) -> Result<GlobalRoot, RpcError> {
    let path = store.config_path()?;
    let dir = path
        .parent()
        .ok_or_else(|| io_error("the config path has no parent"))?;
    Ok(GlobalRoot::At(dir.to_path_buf()))
}

/// What one call works on.
#[derive(Debug, Clone)]
struct Context {
    /// Canonical, when the call has one.
    workdir: Option<PathBuf>,
    /// Valid configured extra roots: `(config_index, canonical path)`.
    extras: Vec<(usize, PathBuf)>,
    global: GlobalRoot,
    state: BindingState,
}

impl Context {
    fn build(
        given: Option<PathBuf>,
        bound: Option<PathBuf>,
        store: &ConfigStore,
        state: BindingState,
    ) -> Result<Self, RpcError> {
        let workdir = match given {
            Some(path) => Some(canonical_dir(&path)?),
            None => match bound {
                Some(path) => Some(crate::tool::winpath::canonical(&path).map_err(|e| {
                    io_error(format!("the session workdir could not be read: {e}"))
                })?),
                None => None,
            },
        };
        let mut extras = Vec::new();
        if let Some(workdir) = &workdir {
            let configured = store.read()?.config.extra_skill_roots;
            if !configured.is_empty() {
                extras = skill::valid_extra_skill_roots(Some(workdir), &configured)
                    .into_iter()
                    .map(|root| (root.config_index, root.path))
                    .collect();
            }
        }
        Ok(Self {
            workdir,
            extras,
            global: global_root(store)?,
            state,
        })
    }

    /// Every tier this call can see, in discovery order.
    fn tiers(&self) -> Vec<Tier> {
        let mut tiers = vec![Tier::Global];
        if self.workdir.is_some() {
            tiers.extend([Tier::Claude, Tier::ProjectAgent, Tier::ProjectAgents]);
            tiers.extend(self.extras.iter().map(|(index, _)| Tier::ExtraRoot(*index)));
        }
        tiers
    }

    /// The refusal for a tier this call cannot see.
    fn require(&self, tier: Tier) -> Result<(), RpcError> {
        if tier != Tier::Global && self.workdir.is_none() {
            return Err(workdir_required(&format!("the {} tier", tier.name())));
        }
        if let Tier::ExtraRoot(index) = tier {
            if !self.extras.iter().any(|(i, _)| *i == index) {
                return Err(not_found(format!("there is no usable extra skill root {index}")));
            }
        }
        Ok(())
    }

    fn global_dir(&self) -> Result<PathBuf, RpcError> {
        match &self.global {
            GlobalRoot::Home => Ok(crate::model::store::base_dir()
                .map_err(|e| io_error(format!("koma data dir: {e}")))?
                .join("skills")),
            #[cfg(test)]
            GlobalRoot::At(dir) => Ok(dir.join(".agents").join("skills")),
        }
    }

    fn root(&self, tier: Tier) -> Result<PathBuf, RpcError> {
        self.require(tier)?;
        match (tier, tier.dir(), &self.workdir) {
            (Tier::Global, _, _) => self.global_dir(),
            (Tier::ExtraRoot(index), _, _) => self
                .extras
                .iter()
                .find(|(i, _)| *i == index)
                .map(|(_, path)| path.clone())
                .ok_or_else(|| not_found(format!("there is no usable extra skill root {index}"))),
            (_, Some(dir), Some(workdir)) => Ok(workdir.join(dir).join("skills")),
            _ => Err(workdir_required(&format!("the {} tier", tier.name()))),
        }
    }

    /// Load one tier from its own root, shadowed skills included. Nothing
    /// is logged: what could not be loaded is kept in `skipped`.
    fn load(&self, tier: Tier) -> Result<TierLoad, RpcError> {
        let root = self.root(tier)?;
        let (defs, skipped) = skill::load_root_with_diagnostics(&root, tier.source());
        let registry = SkillRegistry::from_defs(defs);
        let entries = entries_of(&registry, tier.source(), tier);
        Ok(TierLoad {
            tier,
            registry,
            entries,
            skipped,
        })
    }

    /// Refuse a write to a project tier whose directory (`.agent`, `.agents`)
    /// is a symbolic link or whose root resolves outside the workdir: a
    /// write never leaves the project it was asked for.
    fn writable(&self, tier: Tier) -> Result<(), RpcError> {
        let (Some(dir), Some(workdir)) = (tier.dir(), &self.workdir) else {
            return Ok(());
        };
        let refuse = || {
            RpcError::reason(
                Reason::ReadOnly,
                format!(
                    "{dir} in the workdir is a symbolic link or leads outside it; the {} tier cannot be written",
                    tier.name()
                ),
            )
        };
        let base = workdir.join(dir);
        if std::fs::symlink_metadata(&base).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(refuse());
        }
        let root = base.join("skills");
        if std::fs::symlink_metadata(&root).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(refuse());
        }
        match crate::tool::winpath::canonical(&root) {
            Ok(resolved) if !resolved.starts_with(workdir) => Err(refuse()),
            _ => Ok(()),
        }
    }

    fn project_workdir(&self, tier: Tier) -> Result<PathBuf, RpcError> {
        self.workdir
            .clone()
            .ok_or_else(|| workdir_required(&format!("the {} tier", tier.name())))
    }

    fn load_all(&self) -> Result<Vec<TierLoad>, RpcError> {
        self.tiers().into_iter().map(|tier| self.load(tier)).collect()
    }

    /// Where the core writes a new skill of `tier`.
    fn owned_target(&self, tier: Tier) -> Result<(Option<PathBuf>, OwnedSkillTarget), RpcError> {
        match (tier, &self.global) {
            (Tier::Global, GlobalRoot::Home) => Ok((None, OwnedSkillTarget::Global)),
            #[cfg(test)]
            (Tier::Global, GlobalRoot::At(dir)) => Ok((Some(dir.clone()), OwnedSkillTarget::Project)),
            (Tier::ProjectAgents, _) => {
                Ok((Some(self.project_workdir(tier)?), OwnedSkillTarget::Project))
            }
            _ => Err(read_only(tier)),
        }
    }
}

/// The tier that wins each name: the last in discovery order.
fn winners(loads: &[TierLoad]) -> HashMap<String, Tier> {
    let mut winners = HashMap::new();
    for load in loads {
        for entry in &load.entries {
            winners.insert(entry.name.clone(), load.tier);
        }
    }
    winners
}

fn find_in<'a>(loads: &'a [TierLoad], tier: Tier, name: &str) -> Result<&'a Entry, RpcError> {
    let load = loads
        .iter()
        .find(|load| load.tier == tier)
        .ok_or_else(|| not_found(format!("there is no {} tier here", tier.name())))?;
    load.find(name).ok_or_else(|| not_found("no such skill"))
}

fn tier_load(loads: &[TierLoad], tier: Tier) -> Result<&TierLoad, RpcError> {
    loads
        .iter()
        .find(|load| load.tier == tier)
        .ok_or_else(|| not_found(format!("there is no {} tier here", tier.name())))
}

// ── projections ────────────────────────────────────────────────────────────

fn capped(mut items: Vec<String>, max: usize) -> (Vec<String>, bool) {
    let truncated = items.len() > max;
    items.truncate(max);
    (items, truncated)
}

/// `triggers` is free text in the skill file; a list is its comma- or
/// line-separated parts.
fn split_triggers(text: &str) -> Vec<String> {
    text.split([',', '\n'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn row(entry: &Entry, winners: &HashMap<String, Tier>) -> SkillRow {
    let winner = winners.get(&entry.name).copied().unwrap_or(entry.tier);
    let (triggers, triggers_truncated) = capped(split_triggers(&entry.triggers), MAX_LISTED);
    let (tags, tags_truncated) = capped(entry.tags.clone(), MAX_LISTED);
    SkillRow {
        name: entry.name.clone(),
        scope: entry.tier.scope(),
        tier: entry.tier.name(),
        description: entry.description.clone(),
        triggers,
        triggers_truncated,
        tags,
        tags_truncated,
        pinned: entry.pinned,
        editable: entry.tier.editable(),
        generation: entry.generation.clone(),
        path: display_path(&entry.file_path),
        effective: winner == entry.tier,
        shadowed_by: (winner != entry.tier).then(|| winner.name()),
    }
}

fn detail(entry: &Entry, winners: &HashMap<String, Tier>, raw: Option<String>) -> SkillDetail {
    let (declared_tools, declared_tools_truncated) =
        capped(entry.allowed_tools.clone(), MAX_LISTED);
    let (companion_files, companion_files_truncated) =
        capped(entry.companion_files.clone(), MAX_COMPANIONS);
    SkillDetail {
        row: row(entry, winners),
        instruction: entry.body.clone(),
        declared_tools,
        declared_tools_truncated,
        companion_files,
        companion_files_truncated,
        structured_save_supported: entry.tier.editable() && entry.roundtrip_safe,
        raw,
    }
}

/// The skill file, verbatim: a regular file, never through a symbolic link,
/// at most 2 MiB.
fn read_raw(entry: &Entry) -> Result<String, RpcError> {
    let metadata = std::fs::symlink_metadata(&entry.file_path)
        .map_err(|e| io_error(format!("the skill file could not be read: {e}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(io_error("the skill file is no longer a regular file"));
    }
    if metadata.len() > TEXT_BYTES {
        return Err(too_large("the skill file exceeds 2 MiB"));
    }
    std::fs::read_to_string(&entry.file_path)
        .map_err(|e| io_error(format!("the skill file could not be read: {e}")))
}

// ── errors ─────────────────────────────────────────────────────────────────

fn out_of_range(message: impl Into<String>) -> RpcError {
    RpcError::reason(Reason::OutOfRange, message)
}

fn invalid_name(message: impl Into<String>) -> RpcError {
    RpcError::reason(Reason::InvalidName, message)
}

fn invalid_package(message: impl Into<String>) -> RpcError {
    RpcError::reason(Reason::InvalidPackage, message)
}

fn too_large(message: impl Into<String>) -> RpcError {
    RpcError::reason(Reason::TooLarge, message)
}

fn workdir_required(what: &str) -> RpcError {
    RpcError::reason(Reason::WorkdirRequired, format!("workdir is required for {what} while unbound"))
}

fn not_found(message: impl Into<String>) -> RpcError {
    RpcError::reason(Reason::NotFound, message)
}

fn exists(name: &str) -> RpcError {
    RpcError::reason(Reason::Exists, format!("a skill named {name} exists"))
}

fn stale() -> RpcError {
    RpcError::reason(Reason::Stale, "the skill changed since it was read")
}

fn read_only(tier: Tier) -> RpcError {
    RpcError::reason(Reason::ReadOnly, format!("the {} tier is read-only", tier.name()))
}

fn io_error(message: impl Into<String>) -> RpcError {
    RpcError::reason(Reason::IoError, message)
}

/// The disk changed in a way the adapter cannot attribute.
fn unverified(message: impl Into<String>) -> RpcError {
    RpcError::reason(Reason::Unverified, message).in_process()
}

fn to_value<T: Serialize>(value: &T) -> Result<Value, RpcError> {
    serde_json::to_value(value).map_err(|e| RpcError::new("internal", e.to_string()))
}

// ── params ─────────────────────────────────────────────────────────────────

fn object(params: Value) -> Result<Map<String, Value>, RpcError> {
    match params {
        Value::Object(map) => Ok(map),
        _ => Err(RpcError::reason(Reason::OutOfRange, "params must be an object")),
    }
}

fn has_control(text: &str) -> bool {
    text.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}')
}

fn str_param<'a>(map: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>, RpcError> {
    match map.get(key) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(out_of_range(format!("{key} must be a string"))),
    }
}

fn bool_param(map: &Map<String, Value>, key: &str) -> Result<bool, RpcError> {
    match map.get(key) {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(out_of_range(format!("{key} must be a boolean"))),
    }
}

fn tier_param(map: &Map<String, Value>) -> Result<Option<Tier>, RpcError> {
    match str_param(map, "tier")? {
        None => Ok(None),
        Some(value) => Tier::parse(value).map(Some).ok_or_else(|| {
            out_of_range(
                "tier must be global, claude, project-agent, project-agents or extra-root:<n>",
            )
        }),
    }
}

fn required_tier(map: &Map<String, Value>) -> Result<Tier, RpcError> {
    tier_param(map)?.ok_or_else(|| out_of_range("tier is required"))
}

/// The tier a new skill goes to.
fn new_skill_tier(map: &Map<String, Value>) -> Result<Tier, RpcError> {
    match str_param(map, "scope")? {
        Some("computer") => Ok(Tier::Global),
        Some("project") => Ok(Tier::ProjectAgents),
        Some(_) => Err(out_of_range("scope must be \"computer\" or \"project\"")),
        None => Err(out_of_range("scope is required")),
    }
}

/// A given `workdir`: absolute. It is checked and canonicalized in
/// [`canonical_dir`].
fn workdir_param(map: &Map<String, Value>) -> Result<Option<PathBuf>, RpcError> {
    match str_param(map, "workdir")? {
        None => Ok(None),
        Some(value) => {
            let path = PathBuf::from(value);
            if has_control(value) || !path.is_absolute() {
                return Err(out_of_range("workdir must be an absolute existing directory"));
            }
            Ok(Some(path))
        }
    }
}

fn canonical_dir(path: &Path) -> Result<PathBuf, RpcError> {
    let canonical = crate::tool::winpath::canonical(path)
        .map_err(|_| out_of_range("workdir must be an absolute existing directory"))?;
    if !canonical.is_dir() {
        return Err(out_of_range("workdir must be an absolute existing directory"));
    }
    Ok(canonical)
}

/// The name of an existing skill: trimmed and matched exactly, so names
/// written by other tools stay addressable.
fn address_name(map: &Map<String, Value>) -> Result<String, RpcError> {
    let name = str_param(map, "name")?
        .ok_or_else(|| invalid_name("name is required"))?
        .trim();
    if name.is_empty() || has_control(name) {
        return Err(invalid_name("name must be a nonempty name without control characters"));
    }
    Ok(name.to_string())
}

/// The SDK rule for a new name: `^[a-z0-9]+(-[a-z0-9]+)*$`, at most 64 bytes,
/// and neither `skill` nor `readme`.
pub fn check_new_name(raw: &str) -> Result<String, RpcError> {
    let name = raw.trim();
    if has_control(name) {
        return Err(invalid_name("a skill name cannot hold control characters"));
    }
    let shaped = !name.is_empty()
        && name.len() <= NAME_BYTES
        && name
            .split('-')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()));
    if !shaped {
        return Err(invalid_name(
            "a skill name is 1 to 64 lowercase letters, digits and single inner dashes",
        ));
    }
    if RESERVED_NAMES.contains(&name) {
        return Err(invalid_name(format!("{name} is a reserved skill name")));
    }
    Ok(name.to_string())
}

fn new_name_param(map: &Map<String, Value>, key: &str) -> Result<Option<String>, RpcError> {
    match map.get(key) {
        None => Ok(None),
        Some(Value::String(value)) => check_new_name(value).map(Some),
        Some(_) => Err(invalid_name(format!("{key} must be a string"))),
    }
}

fn description_param(map: &Map<String, Value>) -> Result<Option<String>, RpcError> {
    let Some(value) = str_param(map, "description")? else {
        return Ok(None);
    };
    let chars = value.chars().count();
    if value.trim().is_empty() || chars > MAX_DESCRIPTION_CHARS {
        return Err(out_of_range(format!(
            "description must be 1 to {MAX_DESCRIPTION_CHARS} characters"
        )));
    }
    Ok(Some(value.to_string()))
}

fn instruction_param(map: &Map<String, Value>) -> Result<Option<String>, RpcError> {
    let Some(value) = str_param(map, "instruction")? else {
        return Ok(None);
    };
    if value.trim().is_empty() || value.len() as u64 > TEXT_BYTES {
        return Err(out_of_range("instruction must be 1 byte to 2 MiB of text"));
    }
    Ok(Some(value.to_string()))
}

/// A list of short strings. `,` is refused in an item because the skill file
/// keeps triggers as one comma-separated text.
fn list_param(
    map: &Map<String, Value>,
    key: &str,
    most: usize,
    chars: usize,
    no_comma: bool,
) -> Result<Option<Vec<String>>, RpcError> {
    let Some(value) = map.get(key) else {
        return Ok(None);
    };
    let message = || {
        out_of_range(format!(
            "{key} must be at most {most} strings of 1 to {chars} characters{}",
            if no_comma { " without commas" } else { "" }
        ))
    };
    let Value::Array(items) = value else {
        return Err(message());
    };
    if items.len() > most {
        return Err(message());
    }
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let Value::String(text) = item else {
            return Err(message());
        };
        let text = text.trim();
        if text.is_empty()
            || text.chars().count() > chars
            || has_control(text)
            || (no_comma && text.contains(','))
        {
            return Err(message());
        }
        out.push(text.to_string());
    }
    Ok(Some(out))
}

fn triggers_param(map: &Map<String, Value>) -> Result<Option<String>, RpcError> {
    Ok(list_param(map, "triggers", MAX_TRIGGERS, MAX_TRIGGER_CHARS, true)?
        .map(|items| items.join(", ")))
}

fn tools_param(map: &Map<String, Value>) -> Result<Option<Vec<String>>, RpcError> {
    list_param(map, "allowed_tools", MAX_TOOLS, MAX_TOOL_CHARS, false)
}

fn generation_param(map: &Map<String, Value>) -> Result<Option<String>, RpcError> {
    match str_param(map, "generation")? {
        Some("") => Err(out_of_range("generation must be nonempty")),
        other => Ok(other.map(ToOwned::to_owned)),
    }
}

fn check_generation(entry: &Entry, given: Option<&str>) -> Result<(), RpcError> {
    match given {
        Some(generation) if generation != entry.generation => Err(stale()),
        _ => Ok(()),
    }
}

/// A skill file this large would pass the core's 2 MiB cap.
fn check_edit_size(edit: &SkillEdit) -> Result<(), RpcError> {
    let bytes = edit.description.len()
        + edit.triggers.len()
        + edit.instruction.len()
        + edit.allowed_tools.iter().map(|tool| tool.len() + 4).sum::<usize>()
        + FRONTMATTER_SLACK;
    if bytes as u64 > TEXT_BYTES {
        return Err(out_of_range("the skill file would exceed 2 MiB"));
    }
    Ok(())
}

/// Whether `root` already holds a skill named `name`, in either form.
fn name_taken(root: &Path, name: &str) -> bool {
    std::fs::symlink_metadata(root.join(name)).is_ok()
        || std::fs::symlink_metadata(root.join(format!("{name}.md"))).is_ok()
}

// ── handlers ───────────────────────────────────────────────────────────────

pub fn handle_list(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(run(r.params, adapter, list))
}
pub fn handle_get(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(run(r.params, adapter, get))
}
pub fn handle_read_file(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(run(r.params, adapter, read_file))
}
pub fn handle_create(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(run(r.params, adapter, create))
}
pub fn handle_update(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(run(r.params, adapter, update))
}
pub fn handle_install(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(run(r.params, adapter, install))
}
pub fn handle_remove(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(run(r.params, adapter, remove))
}

type Operation = fn(Context, Map<String, Value>) -> Result<Value, RpcError>;

/// Resolve the call's workdir from its params and binding, then run `op` on
/// the blocking pool. A bound coding session keeps being drained meanwhile.
async fn run(params: Value, adapter: &mut Adapter, op: Operation) -> Result<Value, RpcError> {
    let map = object(params)?;
    let given = workdir_param(&map)?;
    let bound = match &adapter.binding {
        Binding::Unbound => None,
        Binding::Coding(connection) => connection
            .projection
            .session
            .as_ref()
            .map(|session| PathBuf::from(&session.cwd)),
        // The room session's registry workdir.
        Binding::Room(room) => room.info().workdir.as_ref().map(PathBuf::from),
    };
    let store = adapter.config.clone();
    let state = adapter.binding.state();
    adapter
        .blocking(move || {
            let context = Context::build(given, bound, &store, state)?;
            op(context, map)
        })
        .await
}

// ── skills.list ────────────────────────────────────────────────────────────

fn cursor_of(tier: Tier, name: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!("{}\n{name}", tier.rank()))
}

fn parse_cursor(cursor: &str) -> Result<(usize, String), RpcError> {
    let bad = || RpcError::reason(Reason::BadCursor, "the cursor does not decode");
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| bad())?;
    let text = String::from_utf8(bytes).map_err(|_| bad())?;
    let (rank, name) = text.split_once('\n').ok_or_else(bad)?;
    let rank = rank.parse().map_err(|_| bad())?;
    Ok((rank, name.to_string()))
}

fn list(context: Context, map: Map<String, Value>) -> Result<Value, RpcError> {
    let scope = str_param(&map, "scope")?.unwrap_or("all");
    if !matches!(scope, "computer" | "project" | "external" | "all") {
        return Err(out_of_range(
            "scope must be computer, project, external or all",
        ));
    }
    if matches!(scope, "project" | "external") && context.workdir.is_none() {
        return Err(workdir_required(&format!("{scope} skills")));
    }
    let tier = tier_param(&map)?;
    if let Some(tier) = tier {
        context.require(tier)?;
    }
    let include_diagnostics = bool_param(&map, "include_diagnostics")?;
    let limit = match map.get("limit") {
        None => DEFAULT_LIST,
        Some(value) => value
            .as_u64()
            .filter(|n| (1..=MAX_LIST as u64).contains(n))
            .map(|n| n as usize)
            .ok_or_else(|| out_of_range(format!("limit must be an integer from 1 to {MAX_LIST}")))?,
    };
    let after = str_param(&map, "cursor")?.map(parse_cursor).transpose()?;

    let loads = context.load_all()?;
    let winners = winners(&loads);
    let mut rows = loads
        .iter()
        .filter(|load| tier.is_none_or(|tier| tier == load.tier))
        .filter(|load| scope == "all" || load.tier.scope() == scope)
        .flat_map(|load| load.entries.iter())
        .filter(|entry| {
            after.as_ref().is_none_or(|(rank, name)| {
                (entry.tier.rank(), entry.name.as_str()) > (*rank, name.as_str())
            })
        });
    let skills: Vec<SkillRow> = rows.by_ref().take(limit).map(|e| row(e, &winners)).collect();
    let next_cursor = match (rows.next(), skills.last()) {
        (Some(_), Some(last)) => Tier::parse(&last.tier).map(|tier| cursor_of(tier, &last.name)),
        _ => None,
    };
    let (diagnostics, diagnostics_truncated) = if include_diagnostics {
        diagnostics(&loads)
    } else {
        (Vec::new(), false)
    };
    to_value(&ListResult {
        project_workdir: context.workdir.as_deref().map(display_path),
        skills,
        diagnostics,
        diagnostics_truncated,
        next_cursor,
    })
}

/// The skill files of each root that could not be loaded, with the
/// loader's reason, in discovery order.
fn diagnostics(loads: &[TierLoad]) -> (Vec<Diagnostic>, bool) {
    let mut all = loads.iter().flat_map(|load| load.skipped.iter());
    let out: Vec<Diagnostic> = all
        .by_ref()
        .take(MAX_DIAGNOSTICS)
        .map(|skipped| Diagnostic {
            path: display_path(&skipped.path),
            error: skipped.error.chars().take(MAX_DIAGNOSTIC_CHARS).collect(),
        })
        .collect();
    (out, all.next().is_some())
}

// ── skills.get / skills.read_file ──────────────────────────────────────────

/// The skill a read addresses: `(tier, name)`, or the winner for `name`.
fn addressed<'a>(
    context: &Context,
    loads: &'a [TierLoad],
    tier: Option<Tier>,
    name: &str,
) -> Result<&'a Entry, RpcError> {
    match tier {
        Some(tier) => {
            context.require(tier)?;
            find_in(loads, tier, name)
        }
        None => loads
            .iter()
            .rev()
            .find_map(|load| load.find(name))
            .ok_or_else(|| not_found("no such skill")),
    }
}

fn get(context: Context, map: Map<String, Value>) -> Result<Value, RpcError> {
    let tier = tier_param(&map)?;
    let name = address_name(&map)?;
    let include_raw = bool_param(&map, "include_raw")?;
    let loads = context.load_all()?;
    let winners = winners(&loads);
    let entry = addressed(&context, &loads, tier, &name)?;
    let raw = if include_raw { Some(read_raw(entry)?) } else { None };
    to_value(&detail(entry, &winners, raw))
}

/// A companion path: relative, inside the skill, no `..` and no `.`.
fn companion_param(map: &Map<String, Value>) -> Result<String, RpcError> {
    let refuse = || out_of_range("path must be a relative path inside the skill directory");
    let path = str_param(map, "path")?.ok_or_else(|| out_of_range("path is required"))?;
    let parsed = Path::new(path);
    if path.is_empty()
        || has_control(path)
        || parsed.is_absolute()
        || path.starts_with('/')
        || path.starts_with('\\')
        || parsed
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(refuse());
    }
    let normalized = path.replace('\\', "/");
    if normalized
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(refuse());
    }
    Ok(normalized)
}

fn read_file(context: Context, map: Map<String, Value>) -> Result<Value, RpcError> {
    let tier = tier_param(&map)?;
    let name = address_name(&map)?;
    let path = companion_param(&map)?;
    let generation = generation_param(&map)?;
    let loads = context.load_all()?;
    let entry = addressed(&context, &loads, tier, &name)?;
    check_generation(entry, generation.as_deref())?;
    let Some(dir) = entry.skill_dir.as_ref() else {
        return Err(not_found("a single-file skill has no companion files"));
    };
    let normalized = path;
    if !entry.companion_files.contains(&normalized) {
        return Err(not_found(format!("{normalized} is not a file of this skill")));
    }
    let root = crate::tool::winpath::canonical(dir)
        .map_err(|e| io_error(format!("the skill directory could not be read: {e}")))?;
    let candidate = crate::tool::winpath::canonical(&dir.join(&normalized))
        .map_err(|e| io_error(format!("the file could not be read: {e}")))?;
    if !candidate.starts_with(&root) {
        return Err(out_of_range("path must be a relative path inside the skill directory"));
    }
    let metadata = std::fs::symlink_metadata(&candidate)
        .map_err(|e| io_error(format!("the file could not be read: {e}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(not_found(format!("{normalized} is not a regular file")));
    }
    if metadata.len() > TEXT_BYTES {
        return Err(too_large("the file exceeds 2 MiB"));
    }
    let load = tier_load(&loads, entry.tier)?;
    match load
        .registry
        .read_companion_text(&entry.skill_id, &entry.generation, &normalized)
    {
        Ok(text) => to_value(&FileResult {
            path: normalized,
            bytes: text.len(),
            text,
        }),
        Err(error) => match std::fs::read(&candidate) {
            Ok(bytes) if std::str::from_utf8(&bytes).is_err() => Err(RpcError::reason(Reason::Binary, "the file is not UTF-8 text")),
            _ => Err(io_error(format!("the file could not be read: {error}"))),
        },
    }
}

// ── skills.create / skills.update ──────────────────────────────────────────

/// Reload after a committed write and return the written skill.
fn written(context: &Context, tier: Tier, name: &str) -> Result<(SkillDetail, Propagation), RpcError> {
    let loads = context
        .load_all()
        .map_err(|_| unverified("the skill was written but could not be read back").written())?;
    let winners = winners(&loads);
    let entry = find_in(&loads, tier, name)
        .map_err(|_| unverified("the skill was written but could not be read back").written())?;
    Ok((detail(entry, &winners, None), propagation(context.state)))
}

fn create(context: Context, map: Map<String, Value>) -> Result<Value, RpcError> {
    let tier = new_skill_tier(&map)?;
    context.require(tier)?;
    let name = new_name_param(&map, "name")?.ok_or_else(|| invalid_name("name is required"))?;
    let description =
        description_param(&map)?.ok_or_else(|| out_of_range("description is required"))?;
    let instruction =
        instruction_param(&map)?.ok_or_else(|| out_of_range("instruction is required"))?;
    let edit = SkillEdit {
        description,
        triggers: triggers_param(&map)?.unwrap_or_default(),
        allowed_tools: tools_param(&map)?.unwrap_or_default(),
        instruction,
    };
    check_edit_size(&edit)?;
    context.writable(tier)?;
    let root = context.root(tier)?;
    if name_taken(&root, &name) {
        return Err(exists(&name));
    }
    let (workdir, target) = context.owned_target(tier)?;
    if let Err(error) = skill::create_owned_skill(workdir.as_deref(), target, &name, &edit) {
        if name_taken(&root, &name) {
            return Err(exists(&name));
        }
        return Err(io_error(format!("the skill could not be created: {error}")));
    }
    let (skill, propagation) = written(&context, tier, &name)?;
    to_value(&WriteResult { skill, propagation })
}

/// Whether the skill on disk now carries `edit`, under either name.
fn holds_edit(context: &Context, tier: Tier, names: [&str; 2], edit: &SkillEdit) -> bool {
    let Ok(load) = context.load(tier) else {
        return false;
    };
    names.iter().filter_map(|name| load.find(name)).any(|entry| {
        entry.description == edit.description.trim()
            && entry.body == edit.instruction.trim()
            && entry.triggers == edit.triggers.trim()
    })
}

fn update(context: Context, map: Map<String, Value>) -> Result<Value, RpcError> {
    let tier = required_tier(&map)?;
    if !tier.editable() {
        return Err(read_only(tier));
    }
    context.require(tier)?;
    let name = address_name(&map)?;
    let generation = generation_param(&map)?;
    let patch = match map.get("patch") {
        Some(Value::Object(patch)) if !patch.is_empty() => patch.clone(),
        Some(Value::Object(_)) | None => {
            return Err(RpcError::reason(Reason::MissingParams, "patch must change at least one field"))
        }
        Some(_) => return Err(out_of_range("patch must be an object")),
    };
    let new_name = new_name_param(&patch, "name")?;
    let description = description_param(&patch)?;
    let triggers = triggers_param(&patch)?;
    let tools = tools_param(&patch)?;
    let instruction = instruction_param(&patch)?;
    context.writable(tier)?;

    let loads = context.load_all()?;
    let entry = find_in(&loads, tier, &name)?.clone();
    check_generation(&entry, generation.as_deref())?;
    if !entry.roundtrip_safe {
        return Err(RpcError::reason(Reason::UnsafeYaml, "the skill's frontmatter cannot be saved without losing YAML; edit the file by hand"));
    }
    let target = new_name.unwrap_or_else(|| entry.name.clone());
    let renamed = target != entry.name;
    let root = context.root(tier)?;
    if renamed && name_taken(&root, &target) {
        return Err(exists(&target));
    }
    let edit = SkillEdit {
        description: description.unwrap_or_else(|| entry.description.clone()),
        triggers: triggers.unwrap_or_else(|| entry.triggers.clone()),
        allowed_tools: tools.unwrap_or_else(|| entry.allowed_tools.clone()),
        instruction: instruction.unwrap_or_else(|| entry.body.clone()),
    };
    check_edit_size(&edit)?;
    let registry = &tier_load(&loads, tier)?.registry;
    if let Err(error) =
        skill::update_owned_skill(registry, &entry.skill_id, &entry.generation, &target, &edit)
    {
        let unchanged = context
            .load(tier)
            .ok()
            .and_then(|load| load.find(&entry.name).map(|now| now.generation == entry.generation))
            .unwrap_or(false);
        if unchanged {
            if renamed && name_taken(&root, &target) {
                return Err(exists(&target));
            }
            return Err(io_error(format!("the skill could not be saved: {error}")));
        }
        if holds_edit(&context, tier, [&entry.name, &target], &edit) {
            return Err(unverified(format!(
                "the skill text was saved but the update did not finish: {error}"
            ))
            .written());
        }
        return Err(stale());
    }
    let (skill, propagation) = written(&context, tier, &target)?;
    to_value(&UpdateResult {
        skill,
        renamed,
        propagation,
    })
}

// ── skills.install ─────────────────────────────────────────────────────────

/// The package bytes from `data_b64` or `path`, size-checked before they are
/// decoded or read.
fn package_bytes(map: &Map<String, Value>) -> Result<Vec<u8>, RpcError> {
    let inline = map.get("data_b64");
    let path = map.get("path");
    match (inline, path) {
        (Some(_), Some(_)) | (None, None) => Err(RpcError::reason(
            Reason::ConflictingParams,
            "give exactly one of data_b64 and path",
        )),
        (Some(Value::String(data)), None) => {
            if data.len() > INLINE_B64_CHARS {
                return Err(too_large(format!(
                    "data_b64 exceeds {INLINE_B64_CHARS} characters; give a path instead"
                )));
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|_| invalid_package("data_b64 is not valid base64"))?;
            if bytes.len() > INLINE_ZIP_BYTES {
                return Err(too_large("the inline package exceeds 28 MiB; give a path instead"));
            }
            Ok(bytes)
        }
        (Some(_), None) => Err(out_of_range("data_b64 must be a string")),
        (None, Some(Value::String(path))) => read_package(Path::new(path)),
        (None, Some(_)) => Err(out_of_range("path must be a string")),
    }
}

/// Open the package without following a symbolic link, and read at most the
/// core's cap from a regular file.
fn read_package(path: &Path) -> Result<Vec<u8>, RpcError> {
    if !path.is_absolute() {
        return Err(out_of_range("path must be absolute"));
    }
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(out_of_range("path must not be a symbolic link"))
        }
        Ok(metadata) if !metadata.is_file() => {
            return Err(out_of_range("path must be a regular file"))
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(not_found("the package file does not exist"))
        }
        Err(e) => return Err(io_error(format!("the package could not be read: {e}"))),
    }
    let file = open_no_follow(path)
        .map_err(|e| io_error(format!("the package could not be opened: {e}")))?;
    let metadata = file
        .metadata()
        .map_err(|e| io_error(format!("the package could not be read: {e}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(out_of_range("path must be a regular file"));
    }
    if metadata.len() > ZIP_BYTES {
        return Err(too_large("the package exceeds 64 MiB"));
    }
    let mut bytes = Vec::new();
    file.take(ZIP_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| io_error(format!("the package could not be read: {e}")))?;
    if bytes.len() as u64 > ZIP_BYTES {
        return Err(too_large("the package exceeds 64 MiB"));
    }
    Ok(bytes)
}

#[cfg(unix)]
fn open_no_follow(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    // O_NONBLOCK keeps a FIFO swapped in after the check from blocking the open.
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
}

#[cfg(windows)]
fn open_no_follow(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(any(unix, windows)))]
fn open_no_follow(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::File::open(path)
}

/// What the package preflight learned.
struct Package {
    /// The single top-level directory, when the files sit under one.
    top: Option<String>,
}

/// Check the package against the core's rules with clear messages, without
/// decompressing it: a readable archive, no `__MACOSX/`, safe relative paths,
/// no links, the size caps, and `SKILL.md` at the root or under one top-level
/// directory.
fn preflight(bytes: &[u8]) -> Result<Package, RpcError> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| invalid_package("the file is not a valid zip archive"))?;
    if archive.len() > MAX_FILES + MAX_DEPTH + 1 {
        return Err(too_large("the package has too many entries"));
    }
    let mut paths: Vec<(PathBuf, bool)> = Vec::new();
    // `(index, name, limit)` of every file entry, measured below.
    let mut measured: Vec<(usize, String, u64)> = Vec::new();
    let mut files = 0usize;
    let mut total = 0u64;
    for index in 0..archive.len() {
        let entry = archive
            .by_index_raw(index)
            .map_err(|_| invalid_package("the zip archive is malformed"))?;
        let raw = entry.name().to_string();
        if raw == "__MACOSX" || raw.starts_with("__MACOSX/") || raw.contains("/__MACOSX/") {
            return Err(invalid_package(
                "the package holds a __MACOSX/ folder; zip the skill again without macOS metadata",
            ));
        }
        let path = entry
            .enclosed_name()
            .ok_or_else(|| invalid_package(format!("the package holds an unsafe path: {raw}")))?;
        if path.as_os_str().is_empty() {
            continue;
        }
        if let Some(mode) = entry.unix_mode() {
            let kind = mode & 0o170000;
            if kind != 0 && kind != 0o040000 && kind != 0o100000 {
                return Err(invalid_package(format!(
                    "the package holds a symbolic link or special file: {raw}"
                )));
            }
        }
        let is_dir = entry.is_dir();
        if !is_dir {
            files += 1;
            if files > MAX_FILES {
                return Err(too_large("the package exceeds 1000 files"));
            }
            let limit = if path.file_name().is_some_and(|f| f == "SKILL.md") {
                ENTRY_BYTES
            } else {
                COMPANION_BYTES
            };
            if entry.size() > limit {
                return Err(too_large(format!("{raw} exceeds its size limit")));
            }
            total = total.saturating_add(entry.size());
            if total > ZIP_BYTES {
                return Err(too_large("the package exceeds 64 MiB uncompressed"));
            }
            measured.push((index, raw.clone(), limit));
        }
        paths.push((path, is_dir));
    }
    // The sizes in the headers can understate the data: decompress each file
    // as the installer does, up to one byte past its cap, and count the bytes.
    let mut total = 0u64;
    for (index, raw, limit) in measured {
        let mut entry = archive
            .by_index(index)
            .map_err(|_| invalid_package("the zip archive is malformed"))?;
        let size = std::io::copy(&mut entry.by_ref().take(limit + 1), &mut std::io::sink())
            .map_err(|_| invalid_package(format!("the package entry {raw} cannot be read")))?;
        if size > limit {
            return Err(too_large(format!("{raw} exceeds its size limit")));
        }
        total = total.saturating_add(size);
        if total > ZIP_BYTES {
            return Err(too_large("the package exceeds 64 MiB uncompressed"));
        }
    }
    let entry_file = |prefix: Option<&Path>, exact: bool| {
        paths.iter().any(|(path, is_dir)| {
            let relative = match prefix {
                Some(prefix) => match path.strip_prefix(prefix) {
                    Ok(relative) => relative,
                    Err(_) => return false,
                },
                None => path.as_path(),
            };
            let name = relative.to_string_lossy();
            !is_dir
                && relative.components().count() == 1
                && if exact {
                    name == "SKILL.md"
                } else {
                    name.eq_ignore_ascii_case("SKILL.md")
                }
        })
    };
    let lowercase = || {
        invalid_package("the skill file is named skill.md; rename it SKILL.md and zip it again")
    };
    let top = if entry_file(None, true) {
        None
    } else {
        let mut tops = paths.iter().filter_map(|(path, _)| match path.components().next() {
            Some(Component::Normal(part)) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        });
        let first = tops
            .next()
            .ok_or_else(|| invalid_package("the package is empty"))?;
        if tops.any(|part| part != first) {
            if entry_file(None, false) {
                return Err(lowercase());
            }
            return Err(invalid_package(
                "the package must hold SKILL.md at its root or under one top-level folder",
            ));
        }
        let prefix = PathBuf::from(&first);
        if !entry_file(Some(&prefix), true) {
            if entry_file(Some(&prefix), false) || entry_file(None, false) {
                return Err(lowercase());
            }
            return Err(invalid_package(
                "the package must hold SKILL.md at its root or under one top-level folder",
            ));
        }
        Some(first)
    };
    let prefix_depth = usize::from(top.is_some());
    let too_deep = paths.iter().any(|(path, is_dir)| {
        let parts = path.components().count().saturating_sub(prefix_depth);
        let depth = if *is_dir { parts } else { parts.saturating_sub(1) };
        depth > MAX_DEPTH
    });
    if too_deep {
        return Err(invalid_package("the package is nested deeper than 16 folders"));
    }
    Ok(Package { top })
}

/// Whether the package's `SKILL.md` parses as a skill, checked by koma's
/// own loader in a scratch directory.
fn entry_parses(bytes: &[u8], top: Option<&str>, name: &str) -> bool {
    let Ok(mut archive) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) else {
        return false;
    };
    let wanted = match top {
        Some(top) => format!("{top}/SKILL.md"),
        None => "SKILL.md".to_string(),
    };
    let mut text = String::new();
    let read = match archive.by_name(&wanted) {
        Ok(entry) => entry.take(ENTRY_BYTES + 1).read_to_string(&mut text).is_ok(),
        Err(_) => false,
    };
    if !read {
        return false;
    }
    let scratch = std::env::temp_dir().join(format!(
        "koma-skill-check-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let dir = scratch.join(name);
    let parsed = std::fs::create_dir_all(&dir).is_ok()
        && std::fs::write(dir.join("SKILL.md"), text).is_ok()
        && skill::load_root(&scratch, SkillSource::ProjectAgents)
            .iter()
            .any(|def| def.name == name);
    let _ = std::fs::remove_dir_all(&scratch);
    parsed
}

fn install(context: Context, map: Map<String, Value>) -> Result<Value, RpcError> {
    let tier = new_skill_tier(&map)?;
    context.require(tier)?;
    let given = new_name_param(&map, "name")?;
    let bytes = package_bytes(&map)?;
    let package = preflight(&bytes)?;
    let name = match given {
        Some(name) => name,
        None => {
            let top = package.top.as_deref().ok_or_else(|| {
                invalid_name("name is required when SKILL.md sits at the package root")
            })?;
            check_new_name(top)?
        }
    };
    context.writable(tier)?;
    let root = context.root(tier)?;
    if name_taken(&root, &name) {
        return Err(exists(&name));
    }
    let (workdir, target) = context.owned_target(tier)?;
    if let Err(error) = skill::install_owned_skill_zip(workdir.as_deref(), target, &name, &bytes) {
        if name_taken(&root, &name) {
            return Err(exists(&name));
        }
        if !entry_parses(&bytes, package.top.as_deref(), &name) {
            return Err(invalid_package(
                "SKILL.md is not a valid skill file; it needs a frontmatter description",
            ));
        }
        return Err(io_error(format!("the skill could not be installed: {error}")));
    }
    let (skill, propagation) = written(&context, tier, &name)?;
    to_value(&WriteResult { skill, propagation })
}

// ── skills.remove ──────────────────────────────────────────────────────────

/// Whether a skill tree can be deleted safely: no symbolic link or special
/// file anywhere, within the core's file and depth caps.
fn safe_tree(path: &Path, depth: usize, files: &mut usize) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if metadata.file_type().is_symlink() {
        return false;
    }
    if metadata.is_file() {
        *files += 1;
        return *files <= MAX_FILES;
    }
    if !metadata.is_dir() || depth > MAX_DEPTH {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return false;
    };
    entries
        .into_iter()
        .all(|entry| entry.is_ok_and(|entry| safe_tree(&entry.path(), depth + 1, files)))
}

fn remove(context: Context, map: Map<String, Value>) -> Result<Value, RpcError> {
    let tier = required_tier(&map)?;
    if !tier.editable() {
        return Err(read_only(tier));
    }
    context.require(tier)?;
    let name = address_name(&map)?;
    let generation = generation_param(&map)?;
    context.writable(tier)?;
    let loads = context.load_all()?;
    let entry = find_in(&loads, tier, &name)?.clone();
    check_generation(&entry, generation.as_deref())?;
    let tree = entry.skill_dir.clone().unwrap_or_else(|| entry.file_path.clone());
    if !safe_tree(&tree, 0, &mut 0) {
        return Err(RpcError::reason(Reason::ReadOnly, "the skill holds a symbolic link, a special file or too many files; remove it by hand"));
    }
    let registry = &tier_load(&loads, tier)?.registry;
    if let Err(error) = skill::delete_owned_skill(registry, &entry.skill_id, &entry.generation) {
        let now = context.load(tier).ok();
        let current = now.as_ref().and_then(|load| load.find(&entry.name));
        return Err(match current {
            Some(now)
                if now.generation == entry.generation
                    && now.companion_files == entry.companion_files =>
            {
                io_error(format!("the skill could not be removed: {error}"))
            }
            Some(now) if now.generation != entry.generation => stale(),
            _ => RpcError::reason(Reason::PartialDelete, format!("the skill was removed only in part: {error}")).in_process()
                .written(),
        });
    }
    let now_effective = context.load_all().ok().and_then(|loads| {
        let winners = winners(&loads);
        loads
            .iter()
            .rev()
            .find_map(|load| load.find(&name))
            .map(|entry| row(entry, &winners))
    });
    to_value(&RemoveResult {
        removed: true,
        now_effective,
        propagation: propagation(context.state),
    })
}
