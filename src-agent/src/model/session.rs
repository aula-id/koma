//! A single chat session: identity, filesystem path, settings, and conversation.
//!
//! A `Session` owns everything that belongs to one named conversation on disk:
//!
//! ```text
//! ~/.koma/sessions/<id>/
//!     settings.json   ← Settings (model, api_key, compaction…)
//!     messages.json   ← Vec<ChatMessage> (the full history)
//!     memory/
//!         MEMORY.md   ← optional long-term context (see model::memory)
//! ```
//!
//! **Load path:** `store::load` (or `Session::load` directly) reads both JSON
//! files, then immediately calls `rebuild_system()` so the live system prompt
//! (embedded binary + MEMORY.md) always overwrites any stale system message
//! that was stored in `messages.json`.
//!
//! **Save path:** `Session::save` writes `settings.json` and `messages.json`
//! atomically enough for a TUI — no WAL, no rename-over, just `write`.

use crate::dto::chat::ChatMessage;
use crate::model::agent_def::AgentRegistry;
use crate::model::conversation::Conversation;
use crate::model::memory::{load_agents, load_memory_index, migrate_legacy_memory};
use crate::model::session_registry;
use crate::model::settings::{LocalConfig, Settings};
use crate::model::skill::SkillRegistry;
use crate::model::store::shared_settings_path;
use crate::resources;
use anyhow::Result;
use std::path::{Path, PathBuf};

/// One named chat session.
///
/// `id` is the session UUID — the leaf directory name under the session's pwd
/// bucket (`sessions/<pwd_hash>/<id>/`). It is allocated once at creation and
/// never changes (rename only touches the registry `name`, never the path).
/// `pwd_hash` is the working-directory bucket this session lives in. `name` is
/// the human-readable label shown in the session list — it defaults to `id`
/// when `settings.name` is empty, and is sourced from the SQLite registry on
/// load.
pub struct Session {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    /// Working-directory bucket: the parent dir name of `path`
    /// (`sessions/<pwd_hash>/<id>`). Keys this session's per-project shared
    /// resources — the memory dir (`store::memory_dir`) and image-attachment dir
    /// (`store::session_images_dir`) — plus the one-time `session_models` migration
    /// lookup in `Session::load` (`store::shared_settings_path`, read only for
    /// pre-fix files). `session_models` itself is per-session now, not shared.
    pub pwd_hash: String,
    pub settings: Settings,
    pub conversation: Conversation,
    /// Whether the CURRENT `agent_mode` is `Plan`, mirrored in from
    /// `AppStateRest::set_agent_mode` before it calls `rebuild_system`. Not
    /// persisted (mode lives on `AppStateRest`, not the session on disk) — this
    /// is just the least-invasive way to get the mode into `rebuild_system`,
    /// which is a `Session` method with no access to `AppStateRest`. Read only
    /// by `rebuild_system` to decide whether to append the planning nudge.
    pub plan_mode_hint: bool,
    /// Whether the CURRENT `agent_mode` is `Sdlc`.
    pub sdlc_mode_hint: bool,
    /// Skill catalogue loaded during `rebuild_system`. Contains name→path mappings
    /// so the `skill` tool can load bodies on demand. Refreshed every rebuild.
    pub skills: SkillRegistry,
}

impl Session {
    /// Construct a `Session` from its parts.
    ///
    /// `name` is derived from `settings.name`, falling back to `id` when the
    /// name is blank. This is the only place that enforces the fallback.
    /// `pwd_hash` is the working-directory bucket the session lives in
    /// (`path`'s parent dir name).
    pub fn new(
        id: String,
        path: PathBuf,
        pwd_hash: String,
        settings: Settings,
        conversation: Conversation,
    ) -> Self {
        let name = if settings.name.is_empty() {
            id.clone()
        } else {
            settings.name.clone()
        };
        Self {
            id,
            name,
            path,
            pwd_hash,
            settings,
            conversation,
            plan_mode_hint: false,
            sdlc_mode_hint: false,
            skills: SkillRegistry::default(),
        }
    }

    fn settings_path(&self) -> PathBuf {
        self.path.join("settings.json")
    }

    fn messages_path(&self) -> PathBuf {
        self.path.join("messages.json")
    }

    /// Path to this session's approved-plan file (`<session>/plan.md`), written
    /// by the `plan_ready` interception when a plan is presented for approval and
    /// re-read to seed a compacted conversation. Mirrors [`Self::settings_path`].
    pub fn plan_path(&self) -> PathBuf {
        self.path.join("plan.md")
    }

    /// Path to this session's mission contract (`<session>/mission.json`).
    #[allow(dead_code)] // used by mission load helpers / future UI
    pub fn mission_path(&self) -> std::path::PathBuf {
        self.path.join("mission.json")
    }

    /// Session-scoped plan-mode todo list (distinct from the per-directory
    /// working `TODO.md`). Lives beside `plan.md`; cleared when plan mode
    /// exits. Mirrors [`Self::plan_path`].
    pub fn plan_todos_path(&self) -> PathBuf {
        self.path.join("plan_todos.md")
    }

    /// Load a session from `dir` on disk.
    ///
    /// `dir` is the per-session directory `sessions/<pwd_hash>/<uuid>/`.
    ///
    /// Steps:
    /// 1. Derive `id` from `dir`'s file name (the session UUID) and `pwd_hash`
    ///    from `dir.parent()`'s file name (the working-directory bucket).
    /// 2. Read the per-session `settings.json` (or use defaults if absent).
    ///    `session_models` now round-trips through this file (`#[serde(default)]`).
    ///    ONE-TIME MIGRATION: a pre-fix file that lacks the `session_models` key is
    ///    seeded once from the legacy shared `LocalConfig` bucket so existing picks
    ///    survive the upgrade; a file that HAS the key (even an empty array) is
    ///    trusted verbatim and never re-seeded.
    /// 3. Source `name` from the SQLite registry (falling back to `id`).
    /// 4. Read `messages.json` verbatim. A missing or unparseable file yields
    ///    an empty vec; no placeholder system message is inserted here.
    /// 5. Call `rebuild_system()` to seed/overwrite `messages[0]` with the
    ///    embedded system prompt + live MEMORY.md. This ensures the stored
    ///    system message (which may be stale) is always replaced on resume.
    pub fn load(dir: &Path) -> Result<Self> {
        let id = dir
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        // The bucket this session lives in is the parent dir's name.
        let pwd_hash = dir
            .parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();

        let settings_path = dir.join("settings.json");
        let mut settings = if settings_path.exists() {
            Settings::load(&settings_path)?
        } else {
            Settings {
                name: id.clone(),
                ..Default::default()
            }
        };

        // session_models now persists in the per-session settings.json above
        // (#[serde(default)], so old files without the key load as empty). ONE-TIME
        // MIGRATION for pre-fix files: the field used to live only in the shared
        // per-dir LocalConfig bucket and was #[serde(skip)] here, so a file written
        // by the old code has NO `session_models` key. Detect that (probe the raw
        // JSON for the key) and, only then, seed from the shared bucket so the
        // user's existing picks survive the upgrade. The seeded value self-persists
        // on the next save(). If the key IS present (any post-fix file, even an
        // empty array) we trust the file verbatim and never re-seed — that is what
        // makes the migration one-time and lets a user legitimately clear their
        // overrides without them coming back. Best-effort throughout: a missing
        // shared file yields an empty catalogue.
        let has_session_models_key = std::fs::read(&settings_path)
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .map(|v| v.get("session_models").is_some())
            // Fail-safe direction: a probe re-read failure defaults to "key absent"
            // → migrate from the legacy shared bucket, which can only RESTORE old
            // values, never lose what Settings::load already produced in memory.
            .unwrap_or(false);
        if !has_session_models_key {
            if let Ok(shared) = shared_settings_path(&pwd_hash) {
                settings.session_models = LocalConfig::load(&shared)
                    .map(|c| c.session_models)
                    .unwrap_or_default();
            }
        }

        // Read messages.json verbatim. If missing OR the parsed vec is empty,
        // start with an empty conversation (no placeholder System seeding here).
        let messages_path = dir.join("messages.json");
        let messages: Vec<ChatMessage> = match std::fs::read(&messages_path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => Vec::new(),
        };

        let conversation = Conversation::from_messages(messages);

        // Display name comes from the registry (the rename source of truth), not
        // the per-session settings.json. Fall back to the id when there's no row.
        let name = match session_registry::get(&id) {
            Ok(Some(row)) if !row.name.trim().is_empty() => row.name,
            _ => id.clone(),
        };
        // Keep settings.name in sync so a later save() writes a consistent file.
        settings.name = name.clone();

        let mut session = Self {
            id,
            name,
            path: dir.to_path_buf(),
            pwd_hash,
            settings,
            conversation,
            plan_mode_hint: false,
            sdlc_mode_hint: false,
            skills: SkillRegistry::default(),
        };

        // Ensure the per-session scratch dir exists. Best-effort: a failure here
        // (read-only /tmp, unusual permissions) must never prevent the session
        // from loading.
        let scratch = crate::model::store::scratch_dir(&session.id);
        if let Err(e) = std::fs::create_dir_all(&scratch) {
            crate::model::store::append_global_error_log(
                "session",
                &format!(
                    "warning: could not create scratch dir {}: {e}",
                    scratch.display()
                ),
            );
        }

        // Ensure the image-attachment dir exists so resumed sessions can ingest
        // pastes immediately and re-read previously attached images. Best-effort.
        crate::model::store::ensure_session_images_dir(&session.pwd_hash, &session.id);

        // Overwrite the stored system message with the live one so that
        // changes to the embedded prompt or MEMORY.md take effect on resume.
        session.rebuild_system();
        Ok(session)
    }

    /// Persist the session to disk + registry.
    ///
    /// Writes, in order:
    /// 1. the per-session `settings.json` to `self.path`. This now INCLUDES
    ///    `session_models`, persisted via the normal `Settings` serialisation
    ///    (the field is `#[serde(default)]`, not `#[serde(skip)]`), so a session's
    ///    model overrides round-trip in its own file and never touch a sibling
    ///    session's;
    /// 2. `messages.json` to `self.path`;
    /// 3. a registry `touch` so the session sorts most-recent in its bucket.
    ///
    /// Creates `self.path` if it does not exist (needed for a brand-new
    /// session before its first save).
    pub fn save(&self) -> Result<()> {
        std::fs::create_dir_all(&self.path)?;
        // Writes the FULL Settings including session_models (now #[serde(default)],
        // no longer #[serde(skip)]). The legacy shared per-dir LocalConfig bucket is
        // deliberately NOT written anymore — that shared write was the cross-session
        // clobber source (every sibling session overwrote the one bucket with its
        // own in-memory copy). session_models is per-session now.
        self.settings.save(&self.settings_path())?;

        let json = serde_json::to_vec_pretty(self.conversation.messages())?;
        std::fs::write(self.messages_path(), json)?;

        // Best-effort: bump the registry's updated_at so /resume sorts this
        // session to the top. A missing row (e.g. an unregistered session) just
        // updates nothing; a DB hiccup must not fail the save.
        let _ = session_registry::touch(&self.id);
        Ok(())
    }

    /// The images attachment directory for this session: `<session.path>/images/`.
    ///
    /// Uses [`crate::model::store::session_images_dir`] as the canonical source,
    /// falling back to `self.path.join("images")` when the store helper fails
    /// (e.g. the bucket dir hasn't been created yet). All callers should use this
    /// method rather than constructing the path inline.
    pub fn images_dir(&self) -> std::path::PathBuf {
        crate::model::store::session_images_dir(&self.pwd_hash, &self.id)
            .unwrap_or_else(|_| self.path.join("images"))
    }

    /// The session's working directory: the FIRST non-empty entry of the
    /// `workdir` path list (trimmed), else the process's current dir.
    ///
    /// The setting is a managed list; only the first usable entry is the
    /// effective workdir. The remaining entries still feed the harness
    /// workspace allow-set (see `harness::workspace_allowed`).
    pub fn workdir(&self) -> std::path::PathBuf {
        self.settings
            .workdir
            .iter()
            .map(|s| s.trim())
            .find(|s| !s.is_empty())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
            })
    }

    /// All configured workdirs (trimmed, non-empty), falling back to the
    /// process cwd when the list is empty. Used by `DirCacheUpdate` and the
    /// `@` autocomplete to index every workspace root.
    pub fn workdirs(&self) -> Vec<std::path::PathBuf> {
        let dirs: Vec<std::path::PathBuf> = self
            .settings
            .workdir
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(std::path::PathBuf::from)
            .collect();
        if dirs.is_empty() {
            vec![std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))]
        } else {
            dirs
        }
    }

    /// Multi-root legend for the system prompt. Returns `None` when there is
    /// only a single workspace root (bare relatives already unambiguous).
    fn format_workspaces_block(workdirs: &[std::path::PathBuf]) -> Option<String> {
        if workdirs.len() <= 1 {
            return None;
        }
        let mut s = String::from(
            "\n\n# Workspaces\n\
Multiple workspace roots are configured. Paths written as [N]… (for example from \
@ mentions) refer to these roots. Bare relative tool paths target [0] (primary):",
        );
        for (i, p) in workdirs.iter().enumerate() {
            let path = p.display().to_string().replace('\\', "/");
            if i == 0 {
                s.push_str(&format!("\n[{i}] {path}  (primary)"));
            } else {
                s.push_str(&format!("\n[{i}] {path}"));
            }
        }
        Some(s)
    }

    /// Rebuild the system prompt and push it into the conversation.
    ///
    /// Called on session load and after the user edits `MEMORY.md` at runtime,
    /// and after agent create/edit/delete so the sub-agent roster stays live.
    /// Reads the session's `memory/MEMORY.md` (via `load_memory`), passes the
    /// result to `resources::build_system_prompt` which stitches together the
    /// embedded base prompt and the optional memory section, then calls
    /// `Conversation::set_system` to insert or replace `messages[0]`.
    ///
    /// Rebuild the system prompt from on-disk sources (the hot path: memory,
    /// agent roster, skills, config). Convenience wrapper that loads
    /// `AgentRegistry` and `AppConfig` from disk before delegating to
    /// [`Self::rebuild_system_with`].
    pub fn rebuild_system(&mut self) {
        let config = crate::model::app_config::AppConfig::load();
        let registry = AgentRegistry::load(Some(&self.path));
        self.rebuild_system_with(&registry, &config);
    }

    /// Rebuild the system prompt from pre-loaded registry and config, avoiding
    /// redundant filesystem reads. The caller (e.g. `set_agent`, `delete_agent`)
    /// already has a fresh `AgentRegistry` + `AppConfig` in scope, so this skips
    /// the double/triple load that the no-arg `rebuild_system()` would perform.
    pub fn rebuild_system_with(
        &mut self,
        registry: &AgentRegistry,
        config: &crate::model::app_config::AppConfig,
    ) {
        // Memory is now per-PROJECT (shared across every session in this
        // working dir). Resolve the project memory dir, run the best-effort
        // legacy migration (flat per-session MEMORY.md -> index store), then load
        // ONLY the index (pointer bullets) for injection. Fail-open: any memory
        // error degrades to "no memory" and never blocks the rebuild.
        let mem = match crate::model::store::memory_dir(&self.pwd_hash) {
            Ok(dir) => {
                migrate_legacy_memory(&dir, &self.path);
                load_memory_index(&dir)
            }
            Err(_) => None,
        };
        let agents = load_agents(&self.workdir());

        // Build the sub-agent roster from the AgentRegistry (visible agents only).
        let visible = registry.list(true); // exclude_hidden = true
        let roster: String = visible
            .iter()
            .map(|a| {
                // The roster line describes WHEN to delegate: prefer `conditions`
                // (its first line), falling back to `description` when it's empty.
                // `description` alone is a human-facing label and never injected.
                let when = if !a.conditions.trim().is_empty() {
                    a.conditions.lines().next().unwrap_or("").trim().to_string()
                } else {
                    a.description
                        .lines()
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string()
                };
                format!("- {}: {}", a.name, when)
            })
            .collect::<Vec<_>>()
            .join("\n");

        let subagents = if roster.is_empty() {
            None
        } else {
            Some(roster)
        };

        // Load the skill catalogue from known discovery roots. The registry
        // snapshot is stored on `self.skills` so the `skill` tool can resolve
        // names to file paths at load/unload time.
        let skills_reg = SkillRegistry::load(Some(&self.workdir()));
        let skills_cat = {
            let t = skills_reg.catalogue_text();
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        };
        self.skills = skills_reg;

        let mut sys = resources::build_system_prompt(
            mem.as_deref(),
            agents.as_deref(),
            subagents.as_deref(),
            skills_cat.as_deref(),
        );

        // Append the scratch space section so the model knows where it can
        // freely write temporary files and clone repositories.
        let scratch_path = crate::model::store::scratch_dir(&self.id);
        sys.push_str(&format!(
            "\n\n# Scratch space\nYou have a writable scratch directory at: {}\nUse it for temporary files, cloning repositories, and downloads. Both bash and the file tools may read and write under it. It is separate from the user's workspace — keep throwaway work here, not in the project.",
            scratch_path.display()
        ));

        // Multi-root legend: when several workspace roots are configured, tell
        // the model which physical directory each [N] index maps to.
        if let Some(block) = Self::format_workspaces_block(&self.workdirs()) {
            sys.push_str(&block);
        }

        // Extension workspaces: when an enabled extension owns an injected workspace root
        // (see `ext_workspace::inject_extension_workspaces`), name it so the model knows it
        // may write there for that extension's tasks. Read-only: reads the live extension
        // registry + this session's current workdir roots (creates nothing, no side effects).
        let ext_ws = crate::model::ext_workspace::active_extension_workspaces(
            &config.installed_extensions,
            &self.settings.workdir,
        );
        if !ext_ws.is_empty() {
            sys.push_str("\n\n# Extension workspaces");
            for (idx, ext_id) in &ext_ws {
                sys.push_str(&format!(
                    "\nWorkspace [{idx}] is an extension-owned state dir for '{ext_id}' — write there when that extension's tasks require it."
                ));
            }
        }

        // Plan mode: append a soft nudge (no MUST, no protocol walls — weak
        // models over-obey rigid instructions). `plan_mode_hint` is mirrored in
        // from `AppStateRest::set_agent_mode` right before it calls this method.
        if self.plan_mode_hint {
            sys.push_str(
                "\n\n# Plan mode\nPlan mode is active. Tools are read-only: explore the codebase and gather what you need, and use the seqthink tool to structure your reasoning. Build the plan as a todo list with the checklist tool — one item per step (two locked rail items are managed for you). When the plan is complete, call plan_ready with `highlights` (the key changes, decisions, and risks the user needs to approve) and `plan` (the full detailed plan — files, exact changes, reasoning — saved to plan.md). The user will approve it or discuss further."
            );
        }

        // SDLC mode: comprehensive envelope instruction.
        if self.sdlc_mode_hint {
            sys.push_str(
                "\n\n# SDLC mode\n\
You are PM+tech lead inside an SDLC envelope. The harness (WC/PC/TAC) is fully intact — tool approval works like Auto mode, not Yolo.\n\n\
Phases: assess → prepare → execute → verify → integrate → done → assess.\n\n\
## Assess phase (current)\n\
Assess is runtime-enforced read-only for workspace mutations: write/edit/delete/bash and other \
mutating tools are denied until mission approval. Explore with read/search/web, build acceptance \
and a hierarchical graph of tasks (epic→story→task) via the checklist tool, then call mission_ready. \
Main agent is PM after approval — delegate OPEN leaves only.\n\
When ready, call `mission_ready` with:\n\
- `highlights`: the key things the user must know to approve (changes, decisions, risks)\n\
- `goal`: what this mission achieves\n\
- `non_goals`: what it explicitly does NOT do\n\
- `acceptance`: concrete criteria that must be met\n\
- `lane`: express|standard|full (how much verification; full needs a tree or ≥3 leaves)\n\
- `verify_plan`: steps to verify correctness\n\
- `human_gates`: checkpoints requiring human review\n\
- `risks`: known risks\n\
- `rationale`: why this approach\n\
- `graph_tasks`: array of task titles or {title, parent?} objects for the checklist tree\n\
- `target_branch` (optional): which branch the mission merges into on integrate (defaults to current branch at approval time)\n\n\
Only call mission_ready when your exploration is complete and you are confident in the contract.\n\
Amending an approved contract: call mission_ready again (sets needs_reapproval); never silently overwrite.\n\n\
## Prepare phase\n\
After mission approval the FSM enters prepare. During `sdlc:prepare`, the source branch and worktree \
are set up. You have full tool access (bash, git, write, edit, read) — this is the time to establish \
branch topology and worktree assignments for leaf tasks. You may create additional worktrees for \
parallel execution. Verify the worktree is bound and the branch is ready before transitioning.\n\
When setup is complete, call `mission_prepare` to transition to execute.\n\n\
## Post-prepare (execute phase)\n\
NO preference nags; research, decide, ship. Never invent APIs — read the code.\n\
- Execute inside the bound mission worktree (cwd is switched only after binding succeeds). Do not thrash the user's main tree.\n\
- Never force-push; plain push only the mission branch.\n\
- One OPEN leaf claim at a time (main: checklist in_progress or task.node_id). Second claim is denied until the active leaf is sealed.\n\
- Path ownership: if the claimed leaf has owned_paths, stay inside them. Write/edit/delete only inside the mission worktree during execute. Do not mutate the primary tree until integrate.\n\
- Do NOT call git_worktree enter/exit/create/remove during execute/integrate — binding is frozen.\n\
- Keep the checklist/graph honest: SEALED done nodes must not be re-implemented. Graph is the sole authority for SDLC tasks; TODO.md is for ordinary/project todos only, not SDLC checklist.\n\
- Delegate with `task` only to OPEN leaves and always pass `task.node_id`.\n\
- Seal only via `mission_verify` with leaf node_id + real evidence (tests/build) before treating a node as sealed. Done without verify is false-done — the keeper will reopen it (optional backstop). Parents roll up; verify is leaf-only.\n\
- No auto-commit. When OPEN is empty, acceptance is green, leaves verified, binding valid, and human gates approved, call `mission_integrate` (needs clean mission WT + commits ahead).\n\
- Integrate never force-pushes. Integration to main/master is blocked — use a feature/integration branch and merge via PR or manual merge. Dirty target → leave the mission branch ready (or PR); clean target may FF/merge into the frozen target_branch. Branch-only cannot bypass evidence gates. Destination is exclusively frozen target_worktree_path.\n\
- Human gates on the contract require explicit user y/n via mission_verify(human_gate=...) — the model cannot self-approve gates. Integrate stays gated on persisted approvals.\n\
- External shell/MCP is not OS-sandboxed — stay inside the mission tree by discipline.\n\
- Unsure: web_search → message_find → ask the user.\n\n\
## On confusion about mission/details\n\
Re-read mission.json and the OPEN/SEALED capsule. The contract is the source of truth.\n"
            );
            // Mission capsule: when an approved mission exists, inject OPEN+SEALED
            // so sealed work stays sealed across every turn/rebuild.
            if let Some(mission) = crate::model::sdlc::Mission::load(&self.path) {
                if mission.approved {
                    let (open, sealed, all, sealed_commit_shas) =
                        crate::model::msglog::open(&self.path)
                            .ok()
                            .map(|conn| {
                                let _ = crate::model::sdlc::graph::ensure_tables(&conn);
                                let open =
                                    crate::model::sdlc::graph::list_open(&conn).unwrap_or_default();
                                let sealed = crate::model::sdlc::graph::list_sealed(&conn)
                                    .unwrap_or_default();
                                let all =
                                    crate::model::sdlc::graph::list_all(&conn).unwrap_or_default();
                                let sealed_ids: Vec<String> =
                                    sealed.iter().map(|n| n.id.clone()).collect();
                                let sealed_commit_shas =
                                    crate::model::sdlc::graph::latest_verified_commit_shas(
                                        &conn,
                                        &sealed_ids,
                                    )
                                    .unwrap_or_default();
                                (open, sealed, all, sealed_commit_shas)
                            })
                            .unwrap_or_default();
                    sys.push('\n');
                    sys.push_str(&crate::model::sdlc::mission::build_seed_capsule_with_all(
                        &mission,
                        &open,
                        &sealed,
                        &all,
                        &sealed_commit_shas,
                    ));
                    if let Some(ref wt) = mission.worktree_name {
                        sys.push_str(&format!(
                            "\nWorktree intent: {} (branch: {})\n",
                            wt,
                            mission.branch.as_deref().unwrap_or("n/a")
                        ));
                    }
                    // Recent edit history rail: project what changed recently.
                    if let Ok(conn) = crate::model::msglog::open(&self.path) {
                        if crate::model::sdlc::graph::ensure_tables(&conn).is_ok() {
                            if let Ok(history) =
                                crate::model::sdlc::graph::recent_edit_history(&conn, 20)
                            {
                                let section =
                                    crate::model::sdlc::mission::format_edit_history_section(
                                        &history,
                                    );
                                if !section.is_empty() {
                                    sys.push_str(&section);
                                }
                            }
                        }
                    }
                }
            }
        }

        self.conversation.set_system(sys);
    }
}

#[cfg(test)]
#[path = "session_test.rs"]
mod tests;
