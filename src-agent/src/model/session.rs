//! A single chat session: identity, filesystem path, settings, and conversation.
//!
//! A `Session` owns everything that belongs to one named conversation on disk:
//!
//! ```text
//! ~/.simple-coder/sessions/<id>/
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

use std::path::{Path, PathBuf};
use anyhow::Result;
use crate::dto::chat::ChatMessage;
use crate::model::agent_def::AgentRegistry;
use crate::model::conversation::Conversation;
use crate::model::memory::{load_agents, load_memory_index, migrate_legacy_memory};
use crate::model::session_registry;
use crate::model::settings::{LocalConfig, Settings};
use crate::model::store::shared_settings_path;
use crate::resources;

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
    /// (`sessions/<pwd_hash>/<id>`). Identifies the shared `LocalConfig` that
    /// holds this session's `session_models` (see `store::shared_settings_path`).
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

    /// Load a session from `dir` on disk.
    ///
    /// `dir` is the per-session directory `sessions/<pwd_hash>/<uuid>/`.
    ///
    /// Steps:
    /// 1. Derive `id` from `dir`'s file name (the session UUID) and `pwd_hash`
    ///    from `dir.parent()`'s file name (the working-directory bucket).
    /// 2. Read the per-session `settings.json` (or use defaults if absent), then
    ///    overlay `session_models` from the shared `LocalConfig` for this bucket
    ///    (it is `#[serde(skip)]` in the per-session file — see `settings.rs`).
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

        // session_models is no longer in the per-session settings.json; overlay
        // it from the shared per-dir LocalConfig so the in-memory Settings carry
        // the catalogue the resolver expects. Best-effort: a missing/blank shared
        // file yields an empty catalogue (LocalConfig::load handles that).
        if let Ok(shared) = shared_settings_path(&pwd_hash) {
            settings.session_models = LocalConfig::load(&shared)
                .map(|c| c.session_models)
                .unwrap_or_default();
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
        };

        // Ensure the per-session scratch dir exists. Best-effort: a failure here
        // (read-only /tmp, unusual permissions) must never prevent the session
        // from loading.
        let scratch = crate::model::store::scratch_dir(&session.id);
        if let Err(e) = std::fs::create_dir_all(&scratch) {
            eprintln!("koma: warning: could not create scratch dir {}: {e}", scratch.display());
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
    /// 1. the per-session `settings.json` to `self.path` (`session_models` is
    ///    `#[serde(skip)]`, so it is omitted here automatically);
    /// 2. the shared per-dir `LocalConfig` (carrying `session_models`) to this
    ///    bucket's `shared_settings_path`, creating the bucket dir if needed;
    /// 3. `messages.json` to `self.path`;
    /// 4. a registry `touch` so the session sorts most-recent in its bucket.
    ///
    /// Creates `self.path` if it does not exist (needed for a brand-new
    /// session before its first save).
    pub fn save(&self) -> Result<()> {
        std::fs::create_dir_all(&self.path)?;
        self.settings.save(&self.settings_path())?;

        // Persist the per-dir model catalogue to the SHARED bucket settings.json
        // (the only place session_models lives now). Create the bucket dir if the
        // session dir's parent doesn't exist yet.
        let shared = shared_settings_path(&self.pwd_hash)?;
        if let Some(parent) = shared.parent() {
            std::fs::create_dir_all(parent)?;
        }
        LocalConfig {
            session_models: self.settings.session_models.clone(),
        }
        .save(&shared)?;

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
        let dirs: Vec<std::path::PathBuf> = self.settings
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

    /// Rebuild the system prompt and push it into the conversation.
    ///
    /// Called on session load and after the user edits `MEMORY.md` at runtime,
    /// and after agent create/edit/delete so the sub-agent roster stays live.
    /// Reads the session's `memory/MEMORY.md` (via `load_memory`), passes the
    /// result to `resources::build_system_prompt` which stitches together the
    /// embedded base prompt and the optional memory section, then calls
    /// `Conversation::set_system` to insert or replace `messages[0]`.
    pub fn rebuild_system(&mut self) {
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
        let registry = AgentRegistry::load(Some(&self.path));
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
                    a.description.lines().next().unwrap_or("").trim().to_string()
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

        let mut sys = resources::build_system_prompt(
            mem.as_deref(),
            agents.as_deref(),
            subagents.as_deref(),
        );

        // Append the scratch space section so the model knows where it can
        // freely write temporary files and clone repositories.
        let scratch_path = crate::model::store::scratch_dir(&self.id);
        sys.push_str(&format!(
            "\n\n# Scratch space\nYou have a writable scratch directory at: {}\nUse it for temporary files, cloning repositories, and downloads. Both bash and the file tools may read and write under it. It is separate from the user's workspace — keep throwaway work here, not in the project.",
            scratch_path.display()
        ));

        // Plan mode: append a soft nudge (no MUST, no protocol walls — weak
        // models over-obey rigid instructions). `plan_mode_hint` is mirrored in
        // from `AppStateRest::set_agent_mode` right before it calls this method.
        if self.plan_mode_hint {
            sys.push_str(
                "\n\n# Plan mode\nPlan mode is active. Tools are read-only: explore the codebase and gather what you need. You can use the seqthink tool to structure your reasoning step by step. When your plan is solid, call plan_ready with a summary that names each file to edit and why, plus the full detailed plan (files, exact changes, reasoning). The user will approve it or discuss further."
            );
        }

        self.conversation.set_system(sys);
    }
}
