//! [`AgentRegistry`], directory helpers, built-in agents, and the directory
//! loader that merges tiers into the registry.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::model::store::base_dir;

use super::def::{AgentDef, AgentSource};
use super::parse::{load_agent_file, validate_agent_name};

// ---------------------------------------------------------------------------
// Directories
// ---------------------------------------------------------------------------

/// Returns `~/.koma/agents/` (the global agent registry directory).
pub fn global_agents_dir() -> Result<PathBuf> {
    Ok(base_dir()?.join("agents"))
}

/// Returns `<session_dir>/agents/` (session-specific agents).
pub fn session_agents_dir(session_dir: &Path) -> PathBuf {
    session_dir.join("agents")
}

/// Scope an agent operation targets: the global registry or a session directory.
#[derive(Debug, Clone, Copy)]
pub enum AgentScope<'a> {
    /// `~/.koma/agents/`.
    Global,
    /// `<session_dir>/agents/`.
    Session(&'a Path),
}

/// Resolve the on-disk agents directory for a scope.
pub fn agents_dir(scope: AgentScope) -> Result<PathBuf> {
    match scope {
        AgentScope::Global => global_agents_dir(),
        AgentScope::Session(session_dir) => Ok(session_agents_dir(session_dir)),
    }
}

// ---------------------------------------------------------------------------
// Built-in agents
// ---------------------------------------------------------------------------

/// Construct the set of built-in agents compiled into the binary.
///
/// Built-ins have `model: None` (they inherit the session model). Their prompts
/// are embedded from `src-misc/` at compile time.
pub(crate) fn builtin_agents() -> Vec<AgentDef> {
    let tools = |names: &[&str]| names.iter().map(|s| s.to_string()).collect::<Vec<_>>();

    vec![
        AgentDef {
            steps: Some(80),
            conditions: "When you need to locate where something is defined, used, or how code is structured across the codebase.".to_string(),
            ..AgentDef::builtin(
                "explore",
                "Read-only code locator: find where things are defined and used",
                include_str!("../../../../src-misc/agent-explore-prompt.txt"),
                tools(&["read", "grep", "glob", "dir_list", "web_search", "web_fetch"]),
            )
        },
        AgentDef {
            steps: Some(25),
            conditions: "When you have a scoped, self-contained task to complete end-to-end (read + edit + run), not just locating code.".to_string(),
            ..AgentDef::builtin(
                "general",
                "General-purpose subagent for a scoped task",
                include_str!("../../../../src-misc/agent-general-prompt.txt"),
                // NO "task" — recursion guard.
                tools(&["read", "grep", "glob", "dir_list", "edit", "write", "bash"]),
            )
        },
    ]
}

// ---------------------------------------------------------------------------
// Registry + loader
// ---------------------------------------------------------------------------

/// The in-memory agent registry: lowercased name → [`AgentDef`].
#[derive(Debug, Clone, Default)]
pub struct AgentRegistry {
    agents: HashMap<String, AgentDef>,
}

impl AgentRegistry {
    /// Load the full registry for a session (or `None` for built-in + global only).
    ///
    /// Merge order, later overriding earlier by lowercased name: built-in, then
    /// global, then EXTENSION-contributed sub-agents, then session (so a session
    /// agent always wins over an extension's). After merging, `disable: true`
    /// agents are removed from the registry.
    ///
    /// The extension tier reads `AppConfig::installed_extensions` + each enabled
    /// entry's on-disk `manifest.json` fresh on every call (like the global/session
    /// tiers re-scan their directories every call) — so installing, uninstalling,
    /// enabling, or disabling an extension is reflected on the very next `load()`
    /// with no separate cache-invalidation/reload trigger needed.
    pub fn load(session_dir: Option<&Path>) -> Self {
        let mut agents: HashMap<String, AgentDef> = HashMap::new();

        // Tier 1: built-ins.
        for agent in builtin_agents() {
            agents.insert(agent.name.clone(), agent);
        }

        // Tier 2: global.
        if let Ok(dir) = global_agents_dir() {
            load_agents_from_dir(&dir, AgentSource::Global, &mut agents);
        }

        // Tier 3: extension-contributed sub-agents (see `app::ext::register` for
        // the sibling `contributes.tools` half of this wiring).
        if let Ok(ext_root) = crate::model::store::extensions_dir() {
            let config = crate::model::app_config::AppConfig::load();
            merge_extension_sub_agents(&config, &ext_root, &mut agents);
        }

        // Tier 4: session.
        if let Some(session_path) = session_dir {
            let dir = session_agents_dir(session_path);
            load_agents_from_dir(&dir, AgentSource::Session, &mut agents);
        }

        // Post-merge: drop disabled agents (a disabling file overrode any prior
        // tier of the same name above, so this removes the name entirely).
        agents.retain(|_, a| !a.disable);

        Self { agents }
    }

    /// Get an agent by name (case-insensitive).
    pub fn get(&self, name: &str) -> Option<&AgentDef> {
        self.agents.get(&name.to_lowercase())
    }

    /// List agents, sorted by name. When `exclude_hidden` is true, hidden agents
    /// are omitted (they remain in the registry and are still resolvable by name).
    pub fn list(&self, exclude_hidden: bool) -> Vec<&AgentDef> {
        let mut out: Vec<&AgentDef> = self
            .agents
            .values()
            .filter(|a| !exclude_hidden || !a.hidden)
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// All agents as a map for advanced queries.
    pub fn all(&self) -> &HashMap<String, AgentDef> {
        &self.agents
    }

    /// Number of agents in the registry.
    pub fn len(&self) -> usize {
        self.agents.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.agents.is_empty()
    }
}

/// Load every `*.md` file in `dir`, parse it, and merge into `agents`.
///
/// A missing directory is fine (nothing to load). Per-file errors are logged to
/// stderr and skipped — one corrupt file never breaks the registry. Later files
/// override earlier entries of the same lowercased name.
fn load_agents_from_dir(
    dir: &Path,
    source: AgentSource,
    agents: &mut HashMap<String, AgentDef>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        // Missing/unreadable dir is not an error: nothing to merge.
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().is_none_or(|ext| ext != "md") {
            continue;
        }
        match load_agent_file(&path, source) {
            Ok(agent) => {
                agents.insert(agent.name.clone(), agent);
            }
            Err(e) => {
                crate::model::store::append_global_error_log(
                    "agent registry",
                    &format!("skipped agent {}: {e}", path.display()),
                );
            }
        }
    }
}

/// Merge extension-contributed sub-agents into `agents` (tier 3 of
/// [`AgentRegistry::load`], between global and session).
///
/// For every ENABLED [`InstalledExtension`](crate::model::app_config::InstalledExtension)
/// in `config`, reads `<ext_root>/<id>/manifest.json` and turns each
/// `contributes.sub_agents` entry into an [`AgentDef`] tagged
/// [`AgentSource::Extension`]: `description`/`conditions` come from the
/// manifest's `description`; `prompt` prefers the sub-agent's own (trimmed,
/// non-empty) `prompt` field, falling back to `description` when absent/blank
/// (an old-style manifest with no `prompt` field behaves exactly as before);
/// `model`/`effort` are copied through VERBATIM as the RAW slug/token the
/// manifest declares — resolution (turning that slug into a concrete route) is
/// a SPAWN-TIME concern (see [`crate::app::resolve::resolve_agent`]'s step 1c),
/// never done here. `tools` is seeded from the manifest sub-agent's own `tools`
/// list, filtered against [`crate::tool::agent_selectable_tools`] (unknown names
/// are dropped with a logged warning, not a hard failure) and de-duplicated
/// while preserving declaration order; an empty/absent manifest `tools` list
/// leaves `AgentDef::tools` empty too, which [`AgentDef::effective_tools`] then
/// falls back to the safe read-only default for — same as before this field
/// existed. This is a MANIFEST-SEEDED DEFAULT ONLY: it is recomputed fresh on
/// every `load()` call, so a user who saves an edited copy of this sub-agent
/// (which persists as a `Session`-scope override — see
/// `app::runtime::actions::agents::handle_save_agent`) has that override merged
/// in AFTER this extension tier and wins outright; the manifest's `tools` never
/// claws back a user's customization. Each merged def also carries
/// [`AgentDef::ext_id`] — the owning extension's manifest id — for a later
/// wave's ext-scoped model lookup.
///
/// `InstalledExtension` only carries a flat projection of the manifest (id,
/// version, tier, kind, exec, enabled) — NOT a cached `contributes` — so this
/// re-reads `manifest.json` from disk every call, same as `install::unpack`
/// does at install time. A missing/unparsable manifest, or an extension with no
/// `sub_agents`, contributes nothing (best-effort: one broken extension must
/// never break the whole registry load).
///
/// Split out with explicit `config`/`ext_root` parameters (rather than calling
/// `AppConfig::load()` / `store::extensions_dir()` directly) so it is
/// unit-testable against a temp dir instead of the real `~/.koma`; the only
/// production caller is [`AgentRegistry::load`], which supplies both from the
/// real global config + `store::extensions_dir()`.
pub(crate) fn merge_extension_sub_agents(
    config: &crate::model::app_config::AppConfig,
    ext_root: &Path,
    agents: &mut HashMap<String, AgentDef>,
) {
    for ext in &config.installed_extensions {
        if !ext.enabled {
            continue;
        }
        let manifest_path = ext_root.join(&ext.id).join("manifest.json");
        let bytes = match std::fs::read(&manifest_path) {
            Ok(b) => b,
            // Not installed on disk (or unreadable) — skip silently; a dangling
            // config entry with no unpacked dir contributes nothing.
            Err(_) => continue,
        };
        let manifest: koma_extension::protocol::ExtensionManifest =
            match serde_json::from_slice(&bytes) {
                Ok(m) => m,
                Err(e) => {
                    crate::model::store::append_global_error_log(
                        "agent registry",
                        &format!(
                            "extension '{}': skipped sub_agents, bad manifest.json: {e}",
                            ext.id
                        ),
                    );
                    continue;
                }
            };
        for sub in &manifest.contributes.sub_agents {
            let name = sub.name.trim().to_lowercase();
            if name.is_empty() {
                continue;
            }
            // Prefer the sub-agent's own prompt (trimmed, non-empty); fall back to
            // the description so an old-style manifest fragment (no `prompt` key)
            // keeps behaving exactly as before.
            let prompt = sub
                .prompt
                .as_deref()
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| sub.description.clone());
            let tools = validate_manifest_tools(&ext.id, &sub.name, &sub.tools);
            agents.insert(
                name.clone(),
                AgentDef {
                    name,
                    description: sub.description.clone(),
                    conditions: sub.description.clone(),
                    prompt,
                    model: sub.model.clone(),
                    effort: sub.effort.clone(),
                    tools,
                    source: AgentSource::Extension,
                    ext_id: Some(ext.id.clone()),
                    ..AgentDef::default()
                },
            );
        }
    }
}

/// Filter+dedupe a manifest sub-agent's declared `tools` list against koma's
/// selectable tool universe ([`crate::tool::agent_selectable_tools`]).
///
/// Unknown names are dropped (best-effort — a typo'd tool name in one
/// extension's manifest must never break that extension's whole sub-agent, let
/// alone the registry load) and logged via [`crate::model::store::append_global_error_log`].
/// Order is preserved and duplicates collapsed to their first occurrence, so
/// the resulting `AgentDef::tools` is deterministic across reloads.
fn validate_manifest_tools(ext_id: &str, agent_name: &str, declared: &[String]) -> Vec<String> {
    if declared.is_empty() {
        return Vec::new();
    }
    let known = crate::tool::agent_selectable_tools();
    let mut out = Vec::with_capacity(declared.len());
    let mut unknown = Vec::new();
    for name in declared {
        if !known.iter().any(|k| k == name) {
            unknown.push(name.clone());
            continue;
        }
        if !out.contains(name) {
            out.push(name.clone());
        }
    }
    if !unknown.is_empty() {
        crate::model::store::append_global_error_log(
            "agent registry",
            &format!(
                "extension '{ext_id}': sub-agent '{agent_name}' declared unknown tool(s) {unknown:?}, dropped"
            ),
        );
    }
    out
}

// ---------------------------------------------------------------------------
// Public API: registry load + agent save/delete
// ---------------------------------------------------------------------------

/// Load the registry (built-in + global + optional session agents).
pub fn load_registry(session_dir: Option<&Path>) -> AgentRegistry {
    AgentRegistry::load(session_dir)
}

/// Persist an agent definition into a scope, creating the directory if needed.
///
/// The file is `<scope_dir>/<name>.md` (overwritten if it exists). The agent's
/// `name` is re-validated to keep the filename safe. Built-in prompts can be
/// saved out to disk this way (which then shadows the built-in on next load).
pub fn save_agent(scope: AgentScope, agent: &AgentDef) -> Result<PathBuf> {
    let name = validate_agent_name(&agent.name)?;
    let dir = agents_dir(scope)?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{name}.md"));
    std::fs::write(&path, agent.to_markdown())?;
    Ok(path)
}

/// Delete an agent file `<scope_dir>/<name>.md`.
///
/// Returns an error if the name is invalid. A missing file is treated as success
/// (idempotent delete) so a double-delete from the UI does not error.
pub fn delete_agent(scope: AgentScope, name: &str) -> Result<()> {
    let name = validate_agent_name(name)?;
    let dir = agents_dir(scope)?;
    let path = dir.join(format!("{name}.md"));
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
