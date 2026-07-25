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

use crate::config::APP_DIR_NAME;
use crate::dto::chat::{ChatMessage, Role};
use crate::model::conversation::Conversation;
use crate::model::session::Session;
use crate::model::session_registry;
use crate::model::settings::Settings;
use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use uuid::Uuid;

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
    /// The working directory this session was opened from (as stored in the registry).
    pub workdir: String,
    /// The pwd_hash bucket this session belongs to.
    pub pwd_hash: String,
}

/// Returns the application data root.
///
/// - **Unix/macOS:** `~/.koma/` (hidden dot-dir in home).
/// - **Windows:** `%LOCALAPPDATA%\koma` (`AppData\Local\koma`) — the standard
///   per-user machine-local data location. The old `~/.koma` path is migrated
///   on first launch by [`migrate_legacy_dir`].
pub fn base_dir() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        let data = dirs::data_local_dir()
            .ok_or_else(|| anyhow!("cannot resolve %LOCALAPPDATA%"))?;
        Ok(data.join("koma"))
    }
    #[cfg(not(windows))]
    {
        let home = dirs::home_dir()
            .ok_or_else(|| anyhow!("cannot resolve home directory"))?;
        Ok(home.join(APP_DIR_NAME))
    }
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

/// One-time, non-destructive migration to the canonical data directory.
///
/// - **Unix/macOS:** renames `~/.simple-coder` → `~/.koma` if needed.
/// - **Windows:** migrates from `~/.koma` (old location) or `~/.simple-coder`
///   into `%LOCALAPPDATA%\koma`. Cross-drive renames fall back to copy+remove.
///
/// Must be called ONCE at startup before any code reads `base_dir()`.
/// Never panics — any error is logged and silently ignored so the app can
/// proceed (it will create a fresh data dir on first use).
pub fn migrate_legacy_dir() {
    let new_dir = match base_dir() {
        Ok(d) => d,
        Err(e) => {
            append_global_error_log(
                "config migration skipped",
                &format!("cannot resolve base dir: {e}"),
            );
            return;
        }
    };

    // On Windows the canonical dir lives under %LOCALAPPDATA%, not ~.
    // We must also check the old home-relative locations as migration sources.
    #[cfg(windows)]
    let legacy_dirs: Vec<PathBuf> = {
        let mut v = Vec::new();
        if let Some(home) = dirs::home_dir() {
            // Old dot-dir in home (from early Windows port builds).
            let home_koma = home.join(APP_DIR_NAME); // ~/.koma
            if home_koma.exists() && home_koma != new_dir {
                v.push(home_koma);
            }
            // Pre-rename legacy name.
            let simple_coder = home.join(".simple-coder");
            if simple_coder.exists() && simple_coder != new_dir {
                v.push(simple_coder);
            }
        }
        v
    };

    // On Unix the canonical dir IS ~/.koma, so the only migration is
    // the old .simple-coder name.
    #[cfg(not(windows))]
    let legacy_dirs: Vec<PathBuf> = {
        let mut v = Vec::new();
        if let Some(home) = dirs::home_dir() {
            let simple_coder = home.join(".simple-coder");
            if simple_coder.exists() {
                v.push(simple_coder);
            }
        }
        v
    };

    if new_dir.exists() {
        // Target already exists — nothing to do (or merge not implemented).
        return;
    }
    if legacy_dirs.is_empty() {
        return; // fresh install
    }

    for old_dir in &legacy_dirs {
        // Try rename first (works within the same filesystem / drive).
        match std::fs::rename(old_dir, &new_dir) {
            Ok(()) => {
                append_global_error_log(
                    "config migrated",
                    &format!("{} -> {}", old_dir.display(), new_dir.display()),
                );
                return; // success — stop at the first one that moves
            }
            Err(e) => {
                // Cross-drive rename on Windows (e.g. C:\Users → C:\AppData\Local
                // can sometimes fail). Fall back to copy + remove.
                append_global_error_log(
                    "rename failed, trying copy",
                    &format!("{} → {}: {e}", old_dir.display(), new_dir.display()),
                );
                if copy_dir_all(old_dir, &new_dir).is_ok() {
                    let _ = std::fs::remove_dir_all(old_dir);
                    append_global_error_log(
                        "config migrated (copy)",
                        &format!("{} -> {}", old_dir.display(), new_dir.display()),
                    );
                    return;
                }
                append_global_error_log(
                    "config migration failed",
                    &format!("could not migrate {} to {}: {e}", old_dir.display(), new_dir.display()),
                );
            }
        }
    }
}

/// Recursively copy a directory tree. Used for cross-drive migration on Windows.
fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst.join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}

/// Returns `~/.simple-coder/sessions/`.
pub fn sessions_dir() -> Result<PathBuf> {
    Ok(base_dir()?.join("sessions"))
}

/// Returns `~/.koma/run/` — the per-session daemon runtime dir.
///
/// Daemon-per-session keys every daemon's unix socket + pidfile by the session UUID
/// it owns (`run/<session_id>.sock`, `run/<session_id>.pid`), so two `koma` in two
/// terminals get two fully independent daemons under here instead of contending for a
/// single global `daemon.sock`. Lives under the same [`base_dir`] (`~/.koma`) as every
/// other config path; created by [`ensure_dirs`].
pub fn run_dir() -> Result<PathBuf> {
    Ok(base_dir()?.join("run"))
}

/// Returns `~/.koma/extensions/` — the on-disk registry root for installed
/// extensions.
///
/// Each installed extension unpacks into `extensions/<id>/` (its `manifest.json`
/// plus `bin/<exec>`); the install path ([`crate::app::ext::install`]) writes here
/// and the [`ExtHostManager`](crate::app::ext::ExtHostManager) resolves an
/// extension's executable relative to `extensions/<id>/`. Created by [`ensure_dirs`].
pub fn extensions_dir() -> Result<PathBuf> {
    Ok(base_dir()?.join("extensions"))
}

/// Path to a per-extension host socket: `~/.koma/run/ext-<id>.sock` (unix) or the
/// named pipe `\\.\pipe\koma-ext-<id>` (windows).
///
/// koma binds this endpoint BEFORE spawning a daemon-kind extension, hands the path to
/// the child via `KOMA_EXT_SOCKET`, and accepts the child's inbound connection on it
/// (the child sends `Hello`, koma replies `Welcome`). On unix it lives beside the
/// per-session daemon sockets under [`run_dir`]; on windows a named pipe is not a
/// filesystem object, so it lives in the pipe namespace instead. The `id` is validated
/// at install time; the `/`→`_` (and `\`→`_`) fold here is belt-and-suspenders so a
/// stray separator can never escape the run dir / break the pipe name.
#[cfg(unix)]
pub fn ext_sock_path(id: &str) -> Result<PathBuf> {
    let safe: String = id
        .chars()
        .map(|c| if c == '/' || c == '\\' { '_' } else { c })
        .collect();
    Ok(run_dir()?.join(format!("ext-{safe}.sock")))
}

/// Windows twin of [`ext_sock_path`] — the per-extension host named pipe
/// `\\.\pipe\koma-ext-<id>`. See the unix variant for the contract; the same `id`
/// sanitization (fold `/` and `\` to `_`) keeps the pipe name well-formed.
#[cfg(windows)]
pub fn ext_sock_path(id: &str) -> Result<PathBuf> {
    let safe: String = id
        .chars()
        .map(|c| if c == '/' || c == '\\' { '_' } else { c })
        .collect();
    Ok(PathBuf::from(format!(r"\\.\pipe\koma-ext-{safe}")))
}

/// Create `~/.koma/`, `~/.simple-coder/sessions/`, `~/.koma/run/`, and
/// `~/.koma/extensions/` (and their parents) if they do not exist.
///
/// `~/.koma` and `~/.koma/run` are explicitly chmod'd `0700` on unix: `run/` holds
/// every session daemon's unix socket AND the extension host's per-extension
/// `ext-<id>.sock` files, none of which should ever be group/world-accessible on a
/// shared machine; `~/.koma` itself is the visible root for all of that plus session
/// creds, so it gets the same treatment. `create_dir_all`'s permissions otherwise
/// depend on the process umask, which is not something to rely on here.
pub fn ensure_dirs() -> Result<()> {
    let base = base_dir()?;
    std::fs::create_dir_all(&base)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700))?;
    }

    let sessions = sessions_dir()?;
    std::fs::create_dir_all(&sessions)?;
    // The per-session daemon socket/pid dir; the daemon binds `run/<id>.sock` here.
    let run = run_dir()?;
    std::fs::create_dir_all(&run)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o700))?;
    }
    // The installed-extension registry root; the install path unpacks `extensions/<id>/`.
    let extensions = extensions_dir()?;
    std::fs::create_dir_all(&extensions)?;
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

/// The bucket directory for a working dir: `~/.koma/sessions/<pwd_hash>/`.
/// Shared by every session opened from that directory.
pub fn pwd_bucket_dir(pwd_hash: &str) -> Result<PathBuf> {
    Ok(sessions_dir()?.join(pwd_hash))
}

/// The worktrees directory for a pwd bucket: `~/.koma/sessions/<pwd_hash>/worktrees/`.
pub fn worktrees_dir(pwd_hash: &str) -> Result<PathBuf> {
    Ok(pwd_bucket_dir(pwd_hash)?.join("worktrees"))
}

/// Shared per-dir settings path: `<pwd_bucket_dir>/settings.json`. Holds the
/// legacy [`LocalConfig`](crate::model::settings::LocalConfig) (per-dir model
/// catalogue). NO LONGER WRITTEN: `session_models` is persisted per-session now;
/// this path is only READ once by the one-time migration in `Session::load` that
/// seeds a pre-fix session's overrides from the old shared bucket.
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

/// Path to a session's append-only error log: `<session_dir>/error.log`.
pub fn error_log_path(session_dir: &Path) -> PathBuf {
    session_dir.join("error.log")
}

/// Best-effort append of one `"[unix:{ts}] {header}\n{body}\n\n"` entry to
/// `path` (creating `parent` first). Never panics, never propagates — shared
/// tail of [`append_error_log`] and [`append_global_error_log`].
fn append_log_entry(parent: &Path, path: &Path, header: &str, body: &str) {
    use std::io::Write;
    let _ = std::fs::create_dir_all(parent);
    // No `chrono` dependency in this crate — use a plain unix-seconds stamp.
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let entry = format!("[unix:{ts}] {header}\n{body}\n\n");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = f.write_all(entry.as_bytes());
    }
}

/// Best-effort append to the per-session error log. Never panics, never
/// propagates — logging must not break a request. `header` is a short label
/// (e.g. "HTTP 400 from <endpoint>"), `body` the detail (e.g. the raw
/// upstream response body).
pub fn append_error_log(session_dir: &Path, header: &str, body: &str) {
    append_log_entry(session_dir, &error_log_path(session_dir), header, body);
}

/// Path to the global (session-less) error log: `~/.koma/error.log`, for
/// startup/background diagnostics that have no session to log into.
// dead_code: no consumer needs the raw path yet — kept as the sibling of
// `error_log_path` for a future consumer (e.g. a `/errors` viewer).
#[allow(dead_code)]
pub fn global_error_log_path() -> Option<PathBuf> {
    base_dir().ok().map(|d| d.join("error.log"))
}

/// Best-effort append to the global error log; mirrors [`append_error_log`]
/// (never panics, never propagates).
pub fn append_global_error_log(header: &str, body: &str) {
    let Ok(dir) = base_dir() else { return };
    let path = dir.join("error.log");
    append_log_entry(&dir, &path, header, body);
}

/// Physically delete a session: remove its on-disk directory tree AND its
/// registry row. `path` MUST be a session directory under [`sessions_dir`]
/// (e.g. a hub `HistoryEntry::path`); its final component is the session UUID.
/// Refuses any path outside the sessions root as a guard against a bad caller
/// nuking an unrelated directory. The registry delete is best-effort (a missing
/// row must not block the filesystem cleanup); a missing directory is a no-op.
pub fn delete_session(path: &Path) -> Result<()> {
    let root = sessions_dir()?;
    if !path.starts_with(&root) {
        return Err(anyhow!(
            "refusing to delete session path outside sessions dir: {}",
            path.display()
        ));
    }
    if let Some(uuid) = path.file_name().and_then(|n| n.to_str()) {
        let _ = session_registry::delete_by_uuid(uuid);
    }
    if path.exists() {
        std::fs::remove_dir_all(path)?;
    }
    Ok(())
}

/// A session's image-attachment directory: `<session_dir>/images/`. Holds the
/// copied-in image bytes for every pasted/picked image attachment
/// (`images/NN-name.ext`). Lives INSIDE the session dir, so deleting a session
/// already removes its images — no separate cleanup is needed.
pub fn session_images_dir(pwd_hash: &str, uuid: &str) -> Result<PathBuf> {
    Ok(session_dir(pwd_hash, uuid)?.join("images"))
}

/// A session's media directory: `<pwd_bucket_dir>/media/`. Holds downloaded
/// files from the `web_download` tool. Lives inside the pwd bucket so
/// downloads are shared across sessions in the same working directory.
pub fn session_media_dir(pwd_hash: &str) -> Result<PathBuf> {
    Ok(pwd_bucket_dir(pwd_hash)?.join("media"))
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

/// Path to a SESSION-daemon's unix-domain socket: `~/.koma/run/<session_id>.sock`.
///
/// Daemon-per-session: each `koma` owns its own daemon bound to a socket keyed by the
/// session UUID it serves, so two terminals get two independent daemons instead of
/// multiplexing one global `daemon.sock`. This socket is THAT daemon's liveness oracle
/// (whoever binds it IS the live daemon for `session_id`) and the rendezvous point its
/// thin TUI client connects to. The client mints `session_id` and passes it to the
/// daemon via `--session`, so both agree on the key. Lives under [`run_dir`].
#[cfg(unix)]
pub fn daemon_sock_path(session_id: &str) -> Result<PathBuf> {
    Ok(run_dir()?.join(format!("{session_id}.sock")))
}

/// Windows twin of [`daemon_sock_path`] — the per-session daemon named pipe
/// `\\.\pipe\koma-<session_id>`. A named pipe is not a filesystem object (no
/// [`run_dir`] file), and whoever creates the first instance IS the live daemon for
/// `session_id` (bind-as-oracle, same as the unix socket). Session ids are UUIDs, so no
/// sanitization is needed for the pipe name.
#[cfg(windows)]
pub fn daemon_sock_path(session_id: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(format!(r"\\.\pipe\koma-{session_id}")))
}

/// Enumerate this user's koma named-pipe SESSION endpoints from the Windows pipe
/// namespace, returning each pipe's `<session_id>` suffix — the same id
/// [`daemon_sock_path`] embeds after `koma-`.
///
/// A named pipe is not a filesystem object, so [`run_dir`] has nothing to scan on
/// Windows — every unix discovery/cleanup site walks `run/*.sock` FILES, which
/// simply don't exist here. The pipe namespace itself IS enumerable instead:
/// `read_dir(r"\\.\pipe\")` yields one entry per pipe with a live server instance,
/// and each entry's `file_name()` is the pipe's bare name (no `\\.\pipe\` prefix).
/// This is the Windows twin of that `run_dir` scan for every site that needs
/// session ids: [`super::super::app::runtime::manage`]'s `list_session_sockets`,
/// the MCP daemon's idle reaper, and `cmd_clean`'s orphan-pidfile sweep.
///
/// Filters out every RESERVED (non-session) `koma-*` pipe this codebase also
/// binds, so callers never mistake one for a session: the singleton MCP daemon
/// (`koma-mcp`, [`mcp_daemon_sock_path`]), any per-extension host pipe
/// (`koma-ext-<id>`, [`ext_sock_path`]), and the two self-test pipes
/// (`koma-ipc-selftest`, `koma-daemon-selftest`). Any OTHER `koma-`-prefixed name
/// is assumed to be a session id minted by [`daemon_sock_path`].
///
/// Best-effort: enumerating the pipe namespace can transiently fail; an error
/// degrades to an empty `Vec` rather than propagating, mirroring the "unreadable
/// dir ⇒ nothing found" contract the unix `run_dir` scans already have.
#[cfg(windows)]
pub fn list_koma_session_pipes() -> Vec<String> {
    const PREFIX: &str = "koma-";
    // Exact non-session pipe names (checked with the shared `koma-` prefix still on).
    const RESERVED_EXACT: &[&str] = &["koma-mcp", "koma-oauth", "koma-ipc-selftest", "koma-daemon-selftest"];
    // Non-session pipe name PREFIXES (also checked before stripping `koma-`), so a
    // whole family — every extension host — is excluded without listing each id.
    const RESERVED_PREFIX: &[&str] = &["koma-ext-"];

    let entries = match std::fs::read_dir(r"\\.\pipe\") {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(PREFIX) {
            continue; // not a koma pipe at all
        }
        if RESERVED_EXACT.contains(&name) || RESERVED_PREFIX.iter().any(|p| name.starts_with(p)) {
            continue; // a known non-session koma pipe
        }
        out.push(name[PREFIX.len()..].to_string());
    }
    out
}

/// Path to a SESSION-daemon's PID file: `~/.koma/run/<session_id>.pid`.
///
/// Advisory only — recorded for diagnostics/`kill`. It is NOT the liveness oracle
/// (PIDs get reused, which would wedge spawn-or-attach); the bound socket at
/// [`daemon_sock_path`] is. Keyed by the same `session_id` as the socket, under
/// [`run_dir`].
pub fn daemon_pid_path(session_id: &str) -> Result<PathBuf> {
    Ok(run_dir()?.join(format!("{session_id}.pid")))
}

/// Write the running daemon's PID into [`daemon_pid_path`] for `session_id`,
/// overwriting any stale one. Best-effort and advisory only (diagnostics / `kill`), so
/// an IO error is returned but callers treat it as non-fatal — the bound socket, not
/// this file, is the liveness oracle. The graceful-shutdown teardown unlinks it.
pub fn write_daemon_pid(session_id: &str) -> Result<()> {
    std::fs::write(daemon_pid_path(session_id)?, std::process::id().to_string())?;
    Ok(())
}

/// Path to the GLOBAL MCP daemon's unix-domain socket: `~/.koma/mcp.sock`.
///
/// UNLIKE the per-SESSION daemon sockets under [`run_dir`] (`run/<id>.sock`, one per
/// session), the MCP daemon is a SINGLETON: exactly one process owns every configured
/// MCP server connection so N session-daemons proxy to it instead of each spawning
/// their own copies of a heavyweight server (e.g. `serena`). It therefore lives
/// directly under [`base_dir`] (`~/.koma`), not keyed by any session. Whoever binds
/// this socket IS the live MCP daemon (bind-as-oracle, same rule as the session
/// sockets); the session-daemon MCP proxy (next commit) connects here.
#[cfg(unix)]
pub fn mcp_daemon_sock_path() -> Result<PathBuf> {
    Ok(base_dir()?.join("mcp.sock"))
}

/// Windows twin of [`mcp_daemon_sock_path`] — the singleton MCP daemon named pipe
/// `\\.\pipe\koma-mcp`. Not a filesystem object, so it is not under [`base_dir`]; whoever
/// creates the first instance IS the live MCP daemon (bind-as-oracle).
#[cfg(windows)]
pub fn mcp_daemon_sock_path() -> Result<PathBuf> {
    Ok(PathBuf::from(r"\\.\pipe\koma-mcp"))
}

/// Path to the GLOBAL MCP daemon's PID file: `~/.koma/mcp.pid`.
///
/// Advisory only — recorded for diagnostics / `koma daemon kill` — NOT the liveness
/// oracle (PIDs get reused; the bound [`mcp_daemon_sock_path`] socket is the oracle).
/// Singleton, so it lives directly under [`base_dir`] alongside the socket rather than
/// under [`run_dir`].
pub fn mcp_daemon_pid_path() -> Result<PathBuf> {
    Ok(base_dir()?.join("mcp.pid"))
}

/// Write the running MCP daemon's PID into [`mcp_daemon_pid_path`], overwriting any
/// stale one. Best-effort + advisory (diagnostics / `kill`); an IO error is returned
/// but callers treat it as non-fatal — the bound socket, not this file, is the
/// liveness oracle. The MCP daemon's graceful-shutdown teardown unlinks it.
pub fn write_mcp_daemon_pid() -> Result<()> {
    std::fs::write(mcp_daemon_pid_path()?, std::process::id().to_string())?;
    Ok(())
}

/// Path to the GLOBAL OAuth keep-alive daemon's unix-domain socket: `~/.koma/oauth.sock`.
///
/// Singleton like [`mcp_daemon_sock_path`]: exactly one process owns every configured
/// OAuth connection so session-daemons get proactive token refresh. Whoever binds this
/// socket IS the live OAuth daemon (bind-as-oracle).
#[cfg(unix)]
pub fn oauth_daemon_sock_path() -> Result<PathBuf> {
    Ok(base_dir()?.join("oauth.sock"))
}

/// Windows twin of [`oauth_daemon_sock_path`] — the singleton OAuth daemon named pipe
/// `\\.\pipe\koma-oauth`.
#[cfg(windows)]
pub fn oauth_daemon_sock_path() -> Result<PathBuf> {
    Ok(PathBuf::from(r"\\.\pipe\koma-oauth"))
}

/// Path to the GLOBAL OAuth daemon's PID file: `~/.koma/oauth.pid`.
///
/// Advisory only — recorded for diagnostics / `koma daemon kill` — NOT the liveness
/// oracle (PIDs get reused; the bound [`oauth_daemon_sock_path`] socket is the oracle).
pub fn oauth_daemon_pid_path() -> Result<PathBuf> {
    Ok(base_dir()?.join("oauth.pid"))
}

/// Write the running OAuth daemon's PID into [`oauth_daemon_pid_path`], overwriting any
/// stale one. Best-effort + advisory (diagnostics / `kill`); an IO error is returned
/// but callers treat it as non-fatal — the bound socket, not this file, is the
/// liveness oracle. The OAuth daemon's graceful-shutdown teardown unlinks it.
pub fn write_oauth_daemon_pid() -> Result<()> {
    std::fs::write(oauth_daemon_pid_path()?, std::process::id().to_string())?;
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

        // Count non-System messages for the list view. Prefer the msglog sqlite
        // (one indexed COUNT(*) query — see `message_count`'s docs for why a
        // plain count already excludes System rows); fall back to parsing the
        // legacy `messages.json` for pre-msglog sessions that have never had a
        // `messages.sqlite` written. 0 on any parse failure (e.g. a session
        // that's registered but never saved messages.json yet).
        let message_count = crate::model::msglog::message_count(&path).unwrap_or_else(|| {
            let messages_path = path.join("messages.json");
            match std::fs::read(&messages_path) {
                Ok(bytes) => serde_json::from_slice::<Vec<ChatMessage>>(&bytes)
                    .map(|msgs| msgs.iter().filter(|m| m.role != Role::System).count())
                    .unwrap_or(0),
                Err(_) => 0,
            }
        });

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
            workdir: row.workdir.clone(),
            pwd_hash: pwd_hash.to_string(),
        });
    }

    Ok(metas)
}

/// List ALL sessions across every working-directory bucket, most-recently updated first.
///
/// Like [`list_sessions_for`] but without a pwd filter: every session in the registry
/// is returned regardless of which directory it was opened from. Each `SessionMeta`
/// carries `workdir` and `pwd_hash` so callers can label and sort by directory.
pub fn list_all_sessions() -> Result<Vec<SessionMeta>> {
    let rows = session_registry::list_all()?;
    let mut metas: Vec<SessionMeta> = Vec::with_capacity(rows.len());

    for row in rows {
        // Use the stored pwd_hash directly — do NOT re-canonicalize/re-hash the
        // workdir, which could differ on a machine where the path no longer exists.
        let path = session_dir(&row.pwd_hash, &row.uuid)?;

        // See the parallel block in `list_sessions_for` for why msglog is tried
        // first and what the `messages.json` fallback covers.
        let message_count = crate::model::msglog::message_count(&path).unwrap_or_else(|| {
            let messages_path = path.join("messages.json");
            match std::fs::read(&messages_path) {
                Ok(bytes) => serde_json::from_slice::<Vec<crate::dto::chat::ChatMessage>>(&bytes)
                    .map(|msgs| {
                        msgs.iter()
                            .filter(|m| m.role != crate::dto::chat::Role::System)
                            .count()
                    })
                    .unwrap_or(0),
                Err(_) => 0,
            }
        });

        let modified = row
            .updated_at
            .try_into()
            .ok()
            .map(|secs| SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
            .unwrap_or(SystemTime::UNIX_EPOCH);

        let locked = is_locked(&path);

        metas.push(SessionMeta {
            id: row.uuid,
            name: row.name,
            path,
            modified,
            message_count,
            locked,
            workdir: row.workdir,
            pwd_hash: row.pwd_hash,
        });
    }

    Ok(metas)
}

/// Return the last path segment (basename) of `workdir` as a display label.
/// Returns an empty string if the path has no filename component.
pub(crate) fn dir_basename(workdir: &str) -> String {
    std::path::Path::new(workdir)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string()
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
    create_session_in_with_id(workdir, &Uuid::new_v4().to_string())
}

/// Create a brand-new session with a CALLER-PROVIDED id, bucketed by an EXPLICIT
/// `workdir` (its `pwd_hash`).
///
/// The id-EXPLICIT twin of [`create_session_in`]: instead of minting a fresh
/// `Uuid::new_v4()` it uses the `id` the caller supplies. Daemon-per-session needs this
/// — the client mints the session UUID and hands it to the daemon via `--session`, and
/// the daemon must create THAT exact session (its socket key == its session id) rather
/// than a different random one. ALL other logic is identical to [`create_session_in`]
/// (session dir, `memory/`, scratch dir, images dir, `Settings { name = id }`,
/// `Session::new`, registry `register`, `rebuild_system` + first `save`).
///
/// The caller is responsible for ensuring `id` is unique (a fresh v4 UUID); reusing an
/// existing id would re-`register` (UPSERT) the same row and overwrite its on-disk dir.
pub fn create_session_in_with_id(workdir: &Path, id: &str) -> Result<Session> {
    let hash = pwd_hash(workdir);
    let uuid = id.to_string();
    let dir = session_dir(&hash, &uuid)?;
    // Pre-create memory/ so the user can drop MEMORY.md there immediately. This
    // also creates the session dir (and its bucket parent) as a side effect.
    std::fs::create_dir_all(dir.join("memory"))?;

    // Best-effort: create the per-session scratch dir so it is ready immediately.
    let scratch = scratch_dir(&uuid);
    if let Err(e) = std::fs::create_dir_all(&scratch) {
        append_global_error_log(
            "session",
            &format!(
                "warning: could not create scratch dir {}: {e}",
                scratch.display()
            ),
        );
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
