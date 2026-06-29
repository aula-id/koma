//! Session store: list, create, rename sessions in the pwd-keyed layout.
//!
//! Sessions are bucketed by the working directory they were opened from. Every
//! session opened from the same canonical workdir shares one `pwd_hash` bucket,
//! which holds a shared `settings.json` (the per-dir model catalogue) plus one
//! sub-directory per session UUID. Which sessions belong to which bucket — and
//! their display names + timestamps — is tracked in the SQLite registry
//! (`session_registry`), NOT by scanning the filesystem.
//!
//! ```text
//! ~/.simple-coder/
//!     session.sqlite                               ← registry (uuid → pwd_hash, name, …)
//!     sessions/
//!         <pwd_hash>/                              ← one bucket per working dir
//!             settings.json                        ← shared LocalConfig (session_models)
//!             550e8400-e29b-41d4-a716-446655440000/  ← one dir per session UUID
//!                 settings.json                    ← per-session behavioural settings
//!                 messages.json
//!                 messages.sqlite
//!                 memory/
//!                     MEMORY.md
//! ```
//!
//! **Key operations:**
//! - `list_sessions` — registry rows for the CURRENT dir's `pwd_hash`, newest first.
//! - `create_session` — allocate a UUID dir under the cwd's bucket, register, save.
//! - `rename_session` — update the registry `name` only (no filesystem move).
//!
//! Pre-swap `sessions/<name>/` directories from the old layout are never
//! registered, so they are simply not listed (and never crash the list).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use anyhow::{anyhow, Result};
use uuid::Uuid;
use crate::config::APP_DIR_NAME;
use crate::dto::chat::{ChatMessage, Role};
use crate::model::conversation::Conversation;
use crate::model::session::Session;
use crate::model::session_registry;
use crate::model::settings::Settings;

pub use crate::model::session_lock::{is_locked, remove_lock, write_lock};

/// Lightweight metadata about a session used in the session-list UI.
///
/// Loaded without deserialising the full message history — only `settings.json`
/// and the message count are read, keeping the list fast even for large histories.
#[derive(Debug, Clone)]
pub struct SessionMeta {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub modified: SystemTime,
    /// Number of non-System messages, counted best-effort (0 on read failure).
    pub message_count: usize,
    /// `true` when the session is currently open in a LIVE process (a fresh
    /// `session.lock` holding a still-running PID). Computed via [`is_locked`],
    /// so a stale lock from a crashed instance reads as unlocked. The picker
    /// shows a lock marker and refuses to enter a locked session.
    pub locked: bool,
}

/// Returns `~/.koma/` (the application data root).
pub fn base_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("cannot resolve home directory"))?;
    Ok(home.join(APP_DIR_NAME))
}

/// Root of koma's throwaway scratch space (`<temp>/koma`). Bash + file tools
/// are permitted to read/write anywhere under here.
pub fn scratch_root() -> PathBuf {
    std::env::temp_dir().join("koma")
}

/// Per-session scratch dir (`<temp>/koma/<session_id>`).
pub fn scratch_dir(session_id: &str) -> PathBuf {
    scratch_root().join(session_id)
}

/// One-time, non-destructive migration: rename `~/.simple-coder` to `~/.koma`
/// if the new dir does not yet exist and the old one does.
///
/// Must be called ONCE at startup before any code reads `base_dir()`.
/// Never panics — any error is printed to stderr and silently ignored so the
/// app can proceed (it will create a fresh `~/.koma` on first use).
pub fn migrate_legacy_dir() {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            eprintln!("koma: warning: cannot resolve home directory; skipping config migration");
            return;
        }
    };
    let old_dir = home.join(".simple-coder");
    let new_dir = home.join(APP_DIR_NAME); // ".koma"
    if new_dir.exists() {
        // New dir already exists — nothing to do.
        return;
    }
    if !old_dir.exists() {
        // Neither dir exists yet — fresh install, nothing to migrate.
        return;
    }
    match std::fs::rename(&old_dir, &new_dir) {
        Ok(()) => eprintln!("migrated config: ~/.simple-coder -> ~/.koma"),
        Err(e) => eprintln!("koma: warning: could not migrate ~/.simple-coder to ~/.koma: {e}"),
    }
}

/// Returns `~/.simple-coder/sessions/`.
pub fn sessions_dir() -> Result<PathBuf> {
    Ok(base_dir()?.join("sessions"))
}

/// Create `~/.simple-coder/sessions/` (and its parents) if they do not exist.
pub fn ensure_dirs() -> Result<()> {
    let sessions = sessions_dir()?;
    std::fs::create_dir_all(&sessions)?;
    Ok(())
}

// --- pwd-keyed layout paths --------------------------------------------------
//
// These helpers compute the bucket hash and the on-disk paths for the pwd-keyed
// layout; the registry (`session_registry`) tracks which sessions belong to
// which bucket.

/// Deterministic hash of a working directory, stable across runs.
///
/// Canonicalises `workdir` (resolving symlinks / `..`); if canonicalisation
/// fails (e.g. the dir doesn't exist yet) the path is used as-is so the call is
/// infallible. The canonical path string is hashed with UUID v5 over the OID
/// namespace, and the simple (hyphenless) hex form is returned. Same directory
/// → same hash every time.
pub fn pwd_hash(workdir: &Path) -> String {
    let canonical = std::fs::canonicalize(workdir).unwrap_or_else(|_| workdir.to_path_buf());
    let path_str = canonical.to_string_lossy();
    Uuid::new_v5(&Uuid::NAMESPACE_OID, path_str.as_bytes())
        .simple()
        .to_string()
}

/// The bucket directory for a working dir: `~/.simple-coder/sessions/<pwd_hash>/`.
/// Shared by every session opened from that directory.
pub fn pwd_bucket_dir(pwd_hash: &str) -> Result<PathBuf> {
    Ok(sessions_dir()?.join(pwd_hash))
}

/// Shared per-dir settings path: `<pwd_bucket_dir>/settings.json`. Holds the
/// [`LocalConfig`](crate::model::settings::LocalConfig) (model setup) common to
/// all sessions in this working directory.
pub fn shared_settings_path(pwd_hash: &str) -> Result<PathBuf> {
    Ok(pwd_bucket_dir(pwd_hash)?.join("settings.json"))
}

/// The shared per-PROJECT memory directory: `<pwd_bucket_dir>/memory/`. Every
/// session opened from the same working directory shares ONE memory store here
/// (mirrors [`shared_settings_path`]), so memories saved in one session are
/// visible from every other session in the same project.
///
/// The directory (and its bucket parent) is created on access so callers can
/// read/write under it without a separate `create_dir_all`.
pub fn memory_dir(pwd_hash: &str) -> Result<PathBuf> {
    let dir = pwd_bucket_dir(pwd_hash)?.join("memory");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// A single session's directory under its bucket:
/// `<pwd_bucket_dir>/<uuid>/`. Holds the per-session behavioural settings,
/// messages, memory, and agents.
pub fn session_dir(pwd_hash: &str, uuid: &str) -> Result<PathBuf> {
    Ok(pwd_bucket_dir(pwd_hash)?.join(uuid))
}

/// A session's image-attachment directory: `<session_dir>/images/`. Holds the
/// copied-in image bytes for every pasted/picked image attachment
/// (`images/NN-name.ext`). Lives INSIDE the session dir, so deleting a session
/// already removes its images — no separate cleanup is needed.
pub fn session_images_dir(pwd_hash: &str, uuid: &str) -> Result<PathBuf> {
    Ok(session_dir(pwd_hash, uuid)?.join("images"))
}

/// Create a session's `images/` dir (and parents) if absent. Best-effort, called
/// the same place the scratch dir is set up; a failure just means the first
/// ingest will retry the create.
pub fn ensure_session_images_dir(pwd_hash: &str, uuid: &str) {
    if let Ok(dir) = session_images_dir(pwd_hash, uuid) {
        let _ = std::fs::create_dir_all(&dir);
    }
}

/// Path to the SQLite session registry: `~/.simple-coder/session.sqlite`.
pub fn registry_path() -> Result<PathBuf> {
    Ok(base_dir()?.join("session.sqlite"))
}

/// Path to the daemon's unix-domain socket: `~/.koma/daemon.sock`.
///
/// This socket is the koma-daemon's liveness oracle (whoever binds it IS the live
/// daemon) and the rendezvous point the thin TUI client connects to. Resolved
/// from the same [`base_dir`] (`~/.koma`) as every other config path.
pub fn daemon_sock_path() -> Result<PathBuf> {
    Ok(base_dir()?.join("daemon.sock"))
}

/// Path to the daemon's PID file: `~/.koma/daemon.pid`.
///
/// Advisory only — recorded for diagnostics/`kill`. It is NOT the liveness oracle
/// (PIDs get reused, which would wedge spawn-or-attach); the bound socket at
/// [`daemon_sock_path`] is. Lives under the same [`base_dir`] (`~/.koma`).
pub fn daemon_pid_path() -> Result<PathBuf> {
    Ok(base_dir()?.join("daemon.pid"))
}

/// Write the running daemon's PID into [`daemon_pid_path`], overwriting any
/// stale one. Best-effort and advisory only (diagnostics / `kill`), so an IO
/// error is returned but callers treat it as non-fatal — the bound socket, not
/// this file, is the liveness oracle. The graceful-shutdown teardown unlinks it.
pub fn write_daemon_pid() -> Result<()> {
    std::fs::write(daemon_pid_path()?, std::process::id().to_string())?;
    Ok(())
}

/// A stable identity string for the CURRENTLY-RUNNING executable, used as the
/// daemon<->client build-skew handshake (task #142).
///
/// The koma daemon is long-lived and survives a rebuild: after `cargo build`
/// overwrites the on-disk binary, a freshly-built client attaching to the OLD
/// still-running daemon renders STALE behaviour (this already produced a phantom
/// `/agents` bug). The fingerprint lets a client detect that skew — the daemon
/// reports the value it computed AT STARTUP, and a client that computes a
/// DIFFERENT value now knows the binary changed since the daemon launched (a
/// rebuild) and can restart it instead of silently talking to stale code.
///
/// Identity = the running file's on-disk fingerprint, NOT a content hash (cheap +
/// std-only, yet flips on every rebuild because `cargo` rewrites the file):
/// `CARGO_PKG_VERSION` + the executable's byte length + its mtime. Any two
/// builds differ in length and/or mtime, so the string differs across every
/// rebuild while staying identical for a single running binary.
///
/// ROBUST BY CONTRACT — never panics and always returns *something*: if
/// [`std::env::current_exe`] or its [`std::fs::metadata`] can't be resolved (an
/// exotic platform, a deleted/replaced exe), it degrades to JUST the crate
/// version. That fallback is coarser (it won't catch a same-version rebuild) but
/// is strictly better than aborting the attach — a missing fingerprint must never
/// take the client down.
/// The compiled-in koma version (from Cargo.toml).
// Consumed by the version/update UI (next stage), which compares it against the
// fetched `latest_version` via `crate::app::version::is_newer`.
#[allow(dead_code)]
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub fn build_fingerprint() -> String {
    let version = env!("CARGO_PKG_VERSION");

    // Best-effort: the running file's length + mtime. Either step failing drops us
    // to the version-only fallback below (never a panic).
    let detail = std::env::current_exe()
        .ok()
        .and_then(|exe| std::fs::metadata(&exe).ok())
        .map(|meta| {
            let len = meta.len();
            // mtime as a stable string. `modified()` can be unsupported on some
            // platforms; fall back to a marker so two runs on such a platform still
            // compare equal (version+len then carry the signal).
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos().to_string())
                .unwrap_or_else(|| "no-mtime".to_string());
            format!("{len}:{mtime}")
        });

    match detail {
        Some(d) => format!("{version}+{d}"),
        None => version.to_string(),
    }
}

/// List the sessions for the CURRENT working directory, most-recently updated
/// first.
///
/// The list is driven by the SQLite registry (`session_registry`), NOT a
/// filesystem scan: only sessions whose `pwd_hash` matches `std::env::current_dir()`
/// are returned, already ordered by `updated_at` descending. Old pre-swap
/// `sessions/<name>/` directories are never registered, so they simply don't
/// appear — and an absent registry (first run) yields an empty list rather than
/// an error. The System message is excluded from `message_count`.
pub fn list_sessions() -> Result<Vec<SessionMeta>> {
    let workdir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let hash = pwd_hash(&workdir);
    list_sessions_for(&hash)
}

/// List the sessions for an EXPLICIT working-directory bucket (`pwd_hash`), most-
/// recently updated first.
///
/// The pwd-EXPLICIT twin of [`list_sessions`]: it takes the bucket hash directly
/// instead of reading `std::env::current_dir()`. The headless daemon needs this —
/// its own process cwd is the dir it was spawned in, NOT the attaching client's pwd,
/// so a `current_dir()`-based listing would enumerate the wrong directory's sessions.
/// pwd-aware attach (see `app::runtime::actions::session::attach_select_for_pwd`)
/// passes the CLIENT's `pwd_hash` here so it lists sessions for the client's dir.
/// [`list_sessions`] is the thin `current_dir()` wrapper over this.
pub fn list_sessions_for(pwd_hash: &str) -> Result<Vec<SessionMeta>> {
    let rows = session_registry::list_by_pwd(pwd_hash)?;
    let mut metas: Vec<SessionMeta> = Vec::with_capacity(rows.len());

    for row in rows {
        let path = session_dir(pwd_hash, &row.uuid)?;

        // Count non-System messages for the list view; 0 on any parse failure
        // (e.g. a session that's registered but never saved messages.json yet).
        let messages_path = path.join("messages.json");
        let message_count = match std::fs::read(&messages_path) {
            Ok(bytes) => serde_json::from_slice::<Vec<ChatMessage>>(&bytes)
                .map(|msgs| msgs.iter().filter(|m| m.role != Role::System).count())
                .unwrap_or(0),
            Err(_) => 0,
        };

        // The registry's updated_at (unix seconds) is the "modified" time; the
        // picker view formats it as an elapsed duration. Saturating add keeps a
        // garbage/negative timestamp from panicking.
        let modified = row
            .updated_at
            .try_into()
            .ok()
            .map(|secs| SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
            .unwrap_or(SystemTime::UNIX_EPOCH);

        // Lock state for the picker. `is_locked` treats a stale lock (dead PID)
        // as unlocked and opportunistically clears it, so this is also the place
        // crashed-instance leftovers get swept on the next listing.
        let locked = is_locked(&path);

        metas.push(SessionMeta {
            id: row.uuid,
            name: row.name,
            path,
            modified,
            message_count,
            locked,
        });
    }

    Ok(metas)
}

/// Create a brand-new session with a UUID id, bucketed by the current working
/// directory's `pwd_hash`.
///
/// Layout: the session lives at `sessions/<pwd_hash>/<uuid>/`. Also creates
/// `memory/` inside the session directory so `load_memory` can scan it without
/// an error, registers the session in the SQLite registry (the rename/list
/// source of truth), and calls `rebuild_system` before the first save so the
/// system prompt is set correctly.
pub fn create_session() -> Result<Session> {
    // The launch cwd determines both the bucket (pwd_hash) and the seeded
    // workdir. Fall back to "." if the cwd can't be resolved.
    let workdir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    create_session_in(&workdir)
}

/// Create a brand-new session bucketed by an EXPLICIT `workdir` (its `pwd_hash`),
/// seeding that same `workdir` as the session's first workspace root.
///
/// The pwd-EXPLICIT twin of [`create_session`]: it takes the target working
/// directory directly instead of reading `std::env::current_dir()`. The headless
/// daemon needs this because its own process cwd is the dir it was spawned in, not
/// the attaching client's pwd — so a `current_dir()`-based create would bucket the new
/// session under the WRONG directory. pwd-aware attach passes the CLIENT's launch dir
/// here so a relaunch from a new dir gets a session rooted at that new dir.
/// [`create_session`] is the thin `current_dir()` wrapper over this.
pub fn create_session_in(workdir: &Path) -> Result<Session> {
    let hash = pwd_hash(workdir);
    let uuid = Uuid::new_v4().to_string();
    let dir = session_dir(&hash, &uuid)?;
    // Pre-create memory/ so the user can drop MEMORY.md there immediately. This
    // also creates the session dir (and its bucket parent) as a side effect.
    std::fs::create_dir_all(dir.join("memory"))?;

    // Best-effort: create the per-session scratch dir so it is ready immediately.
    let scratch = scratch_dir(&uuid);
    if let Err(e) = std::fs::create_dir_all(&scratch) {
        eprintln!("koma: warning: could not create scratch dir {}: {e}", scratch.display());
    }

    // Pre-create the image-attachment dir so the first paste-ingest has a home.
    ensure_session_images_dir(&hash, &uuid);

    let workdir_str = workdir.display().to_string();
    let settings = Settings {
        name: uuid.clone(),
        // Seed the workdir path list with a single entry: the launch cwd.
        workdir: vec![workdir_str.clone()],
        ..Default::default()
    };
    let conversation = Conversation::from_messages(vec![]);
    let mut session = Session::new(uuid.clone(), dir, hash.clone(), settings, conversation);
    // Register before the first save so the row exists for list/rename. The
    // initial display name is the uuid (matches settings.name).
    session_registry::register(&uuid, &hash, &uuid, &workdir_str)?;
    session.rebuild_system();
    session.save()?;
    Ok(session)
}

/// Rename a session by updating its registry `name` only.
///
/// In the pwd-keyed layout the on-disk directory is the immutable session UUID;
/// the display name lives in the SQLite registry, so a rename is just a name
/// update there — NO filesystem move, NO collision handling. The session's
/// in-memory `name` / `settings.name` are updated to match (the `id`, `path`,
/// and `pwd_hash` are unchanged), then the session is saved.
pub fn rename_session(session: &mut Session, new_name: &str) -> Result<()> {
    let display = new_name.trim().to_string();
    session_registry::set_name(&session.id, &display)?;
    session.name = display.clone();
    session.settings.name = display;
    session.save()?;
    Ok(())
}

/// Convert an arbitrary string into a lowercase, hyphen-separated filesystem slug.
///
/// Algorithm:
/// 1. Walk each Unicode character of `name`.
/// 2. Alphanumeric characters are lowercased and kept.
/// 3. Every non-alphanumeric character becomes a space.
/// 4. The result is split on whitespace and joined with `'-'`, collapsing
///    consecutive non-alphanumeric runs into a single hyphen.
///
/// Returns `Err` if the result is empty (e.g. the input was all punctuation).
///
/// Examples: `"My Project!"` → `"my-project"`, `"  foo  bar  "` → `"foo-bar"`.
///
/// Retained for potential reuse / a friendlier on-disk layout; the pwd-keyed
/// rename no longer slugifies (directories are immutable UUIDs, the name lives
/// in the registry), so this is currently unused.
#[allow(dead_code)]
pub(crate) fn slugify(name: &str) -> Result<String> {
    let mut mapped = String::new();
    for c in name.chars() {
        if c.is_alphanumeric() {
            // to_lowercase() returns an iterator because some chars expand to
            // multiple code points (e.g. the German ß → ss).
            for lc in c.to_lowercase() {
                mapped.push(lc);
            }
        } else {
            // Treat any non-alphanumeric character as a word separator.
            mapped.push(' ');
        }
    }
    // split_whitespace collapses consecutive spaces, join reinserts hyphens.
    let slug = mapped.split_whitespace().collect::<Vec<_>>().join("-");
    if slug.is_empty() {
        Err(anyhow!("name contains no usable characters"))
    } else {
        Ok(slug)
    }
}
