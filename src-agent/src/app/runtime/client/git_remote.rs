//! Host-side GIT REMOTE SYNC (fetch/pull/push) + the per-repo key-assignment
//! store for the GUI Source Control panel — wave 4b, split out of
//! [`super::git`] purely for file size (that file was already near the
//! 600-line budget; this is pure code motion + new functionality, no
//! behaviour change to anything [`super::git`] already did). Mirrors
//! [`super::git`]'s exact host-relay pattern (a `git_cmd_env` choke point,
//! `GitOpResult { ok, op, error, message }` replies) and [`super::keys`]'s
//! vault reasoning (host-local only, never the daemon — this is a GUI-only
//! convenience over the model's own git credential machinery in
//! `git_cred.rs`/`git_operator.rs`, which is untouched by any of this).
//!
//! ## Key assignment
//!
//! Which SSH key (a name in [`super::keys`]'s vault) a repo uses for its
//! remote ops is persisted in `<~/.koma>/git_keys.json` — a flat `{
//! "<repoRoot>": "<keyName>" }` map, keyed by the repo's absolute root path
//! (as [`super::git::repo_root_for`] resolves it). [`assigned_key`] /
//! [`set_assigned_key`] are the read/write chokepoints; corruption-tolerant
//! (a parse failure reads as an empty map, never panics) and best-effort on
//! write (a persist failure is logged, never propagated — the in-memory
//! GUI state isn't touched by a failed save, so at worst the assignment
//! doesn't survive a restart).
//!
//! ## Remote ops
//!
//! [`git_fetch`]/[`git_pull`]/[`git_push`] each resolve the repo root, look up
//! its assigned key, and — when one is assigned AND its private-key file
//! still exists in the vault ([`super::keys::key_private_path`]) — inject a
//! `GIT_SSH_COMMAND` override via [`super::git::git_cmd_env`] pinning `ssh` to
//! that one key (`-o IdentitiesOnly=yes`, so a loaded ssh-agent identity can't
//! shadow it) and auto-accepting a first-time host key (`-o
//! StrictHostKeyChecking=accept-new` — avoids a non-tty hang on the very
//! first connect to a host, while still rejecting a CHANGED host key, unlike
//! `=no`). No key assigned (or a since-deleted one) runs with no override —
//! the system default ssh-agent/keys, exactly as a plain `git fetch` would
//! from a terminal.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::git::{git_cmd_env, git_failure, repo_root_for, GitOpResult};
use super::keys::key_private_path;

/// Process-lifetime monotonic counter folded into [`atomic_write`]'s temp
/// filename alongside the PID — the PID alone is shared by every thread in
/// this process, so two concurrent writers (the two host loops, or a fast
/// double-click on the key picker) would otherwise race on the SAME temp
/// path and lose one write to a `rename` ENOENT.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

fn op_ok(op: &str) -> GitOpResult {
    GitOpResult { ok: true, op: op.to_string(), error: None, message: None }
}

fn op_ok_msg(op: &str, message: impl Into<String>) -> GitOpResult {
    GitOpResult { ok: true, op: op.to_string(), error: None, message: Some(message.into()) }
}

fn op_err(op: &str, error: impl Into<String>) -> GitOpResult {
    GitOpResult { ok: false, op: op.to_string(), error: Some(error.into()), message: None }
}

/// Resolve `<~/.koma>/git_keys.json`'s path (does NOT create it — an absent file
/// just means an empty map, handled by [`load_map`]). Reuses the SAME
/// home/`~/.koma` resolver [`super::keys`]'s vault uses, never a hand-rolled
/// home lookup.
fn git_keys_path() -> Result<PathBuf, String> {
    let base = crate::model::store::base_dir().map_err(|e| e.to_string())?;
    Ok(base.join("git_keys.json"))
}

/// Load the `{ repoRoot: keyName }` map, corruption-tolerant: a missing file, an
/// unreadable file, OR a parse failure all read as an EMPTY map rather than
/// panicking or propagating an error — a corrupt `git_keys.json` degrades to
/// "no repo has an assigned key" instead of wedging the whole GIT panel.
fn load_map() -> HashMap<String, String> {
    let Ok(path) = git_keys_path() else {
        return HashMap::new();
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

/// PID+sequence-suffixed temp file + rename (mirrors the catalogue-overlay
/// cache's atomic write), so a crash mid-write never leaves a
/// truncated/partial `git_keys.json` for a concurrent reader ([`load_map`])
/// to observe. The sequence number (on top of the PID) makes the temp path
/// unique PER CALL, not just per-process — two concurrent [`set_assigned_key`]
/// calls (the two host loops, or a fast double-click on the key picker) each
/// get their own temp file instead of racing on write+rename.
fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no file name"))?;
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let mut tmp_name = file_name.to_owned();
    tmp_name.push(format!(".{}.{}.tmp", std::process::id(), seq));
    let tmp_path = parent.join(&tmp_name);
    if let Err(e) = std::fs::write(&tmp_path, bytes) {
        // Best-effort cleanup — the write itself failed, so there may be
        // nothing (or a partial file) to remove; either way, don't let a
        // failed remove mask the original write error.
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

/// The SSH key (by vault name) currently assigned to repo `root`, or `None` if
/// none is assigned. Read chokepoint for the assignment store — called by
/// [`super::git::compute_git_status`] to populate `GitStatusResult.key_name`
/// (the panel's key picker's current value) and by every remote op below to
/// decide whether to inject a `GIT_SSH_COMMAND` override.
pub(super) fn assigned_key(root: &Path) -> Option<String> {
    load_map().get(&root.to_string_lossy().into_owned()).cloned()
}

/// Assign (`Some(name)`) or clear (`None`) repo `root`'s key. Best-effort: a
/// resolve/serialize/write failure is logged to stderr and swallowed — the
/// caller (the GIT panel's key picker) always gets its follow-up `GitStatus`
/// re-push regardless, so the UI never hangs even if the persist silently
/// failed (the assignment just won't survive a restart).
pub(super) fn set_assigned_key(root: &Path, name: Option<String>) {
    let path = match git_keys_path() {
        Ok(p) => p,
        Err(e) => {
            crate::model::store::append_global_error_log(
                "gui",
                &format!("git key vault: could not resolve git_keys.json: {e}"),
            );
            return;
        }
    };
    let mut map = load_map();
    let key = root.to_string_lossy().into_owned();
    match name {
        Some(n) => {
            map.insert(key, n);
        }
        None => {
            map.remove(&key);
        }
    }
    let bytes = match serde_json::to_vec_pretty(&map) {
        Ok(b) => b,
        Err(e) => {
            crate::model::store::append_global_error_log(
                "gui",
                &format!("git key vault: failed to serialize git_keys.json: {e}"),
            );
            return;
        }
    };
    if let Err(e) = atomic_write(&path, &bytes) {
        crate::model::store::append_global_error_log(
            "gui",
            &format!("git key vault: failed to persist git_keys.json: {e}"),
        );
    }
}

/// Convenience wrapper for [`super::HostCtl::SetGitKey`]: resolve `session`'s
/// repo root and assign/clear its key in one call, so the host-relay loops
/// (`host.rs`/`push_loop.rs`) don't need to import [`repo_root_for`]
/// themselves just for this one mutation. A `session` that isn't inside a git
/// repository is a silent no-op (nothing to assign a key to).
pub(super) fn set_current_key(session: Option<&str>, name: Option<String>) {
    if let Some(root) = repo_root_for(session) {
        set_assigned_key(&root, name);
    }
}

/// Build this repo's `GIT_SSH_COMMAND` override string, or `None` when no key is
/// assigned OR the assigned key's private-key file no longer exists in the
/// vault (deleted after being assigned) — either way, the caller then runs
/// git with NO ssh override (the system default). `-o IdentitiesOnly=yes`
/// pins ssh to ONLY the given key (an ssh-agent identity can't override it);
/// `-o StrictHostKeyChecking=accept-new` auto-accepts a first-time host key
/// (avoiding a non-interactive hang on the very first connect) while still
/// rejecting a host key that CHANGED since (unlike `=no`, which accepts
/// blindly forever). The key path is POSIX single-quoted, with any embedded
/// single quote escaped via the standard `'\''` trick — the path includes an
/// environment-derived, unvalidated home-dir/`~/.koma` prefix, and
/// `GIT_SSH_COMMAND` is run through `/bin/sh -c`, so a bare `'` in the path
/// would otherwise break out of the quoting and inject shell syntax.
fn ssh_command_for(root: &Path) -> Option<String> {
    let name = assigned_key(root)?;
    let key_path = key_private_path(&name)?;
    let key_str = key_path.to_string_lossy();
    let quoted = format!("'{}'", key_str.replace('\'', r"'\''"));
    Some(format!(
        "ssh -i {quoted} -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new"
    ))
}

/// Cap on lines/chars kept from git's own success output before it's shipped
/// over IPC as a toast — a chatty `fetch --prune` or a multi-ref `push` can
/// produce a wall of text; nobody reads past a handful of lines in a toast
/// anyway.
const SUCCESS_MESSAGE_MAX_LINES: usize = 8;
const SUCCESS_MESSAGE_MAX_CHARS: usize = 400;

/// `git`'s own success summary (fetch/pull/push progress typically lands on
/// stderr; a fast-forward pull's "Updating a1b2c3..d4e5f6" style line can land
/// on stdout) — `None` when both are empty (nothing worth surfacing).
/// Deliberately reuses [`git_failure`]'s stderr-then-stdout preference with an
/// empty fallback, rather than duplicating that logic for the success path.
/// Capped to a handful of lines/chars (see [`SUCCESS_MESSAGE_MAX_LINES`]/
/// [`SUCCESS_MESSAGE_MAX_CHARS`]) so a chatty `fetch --prune` or multi-ref
/// `push` doesn't ship a giant blob over IPC on every call.
fn success_message(out: &std::process::Output) -> Option<String> {
    let raw = git_failure(out, "");
    if raw.is_empty() {
        return None;
    }
    let mut truncated = false;
    let mut capped: String = raw
        .lines()
        .take(SUCCESS_MESSAGE_MAX_LINES)
        .collect::<Vec<_>>()
        .join("\n");
    if raw.lines().count() > SUCCESS_MESSAGE_MAX_LINES {
        truncated = true;
    }
    if capped.chars().count() > SUCCESS_MESSAGE_MAX_CHARS {
        capped = capped.chars().take(SUCCESS_MESSAGE_MAX_CHARS).collect();
        truncated = true;
    }
    if truncated {
        capped.push('…');
    }
    if capped.is_empty() {
        None
    } else {
        Some(capped)
    }
}

/// `git fetch --prune` for `session`'s repo, answering a
/// [`super::HostCtl::GitFetch`]. Injects this repo's assigned-key
/// `GIT_SSH_COMMAND` (see [`ssh_command_for`]) when one resolves; otherwise runs
/// with the system default ssh. `--prune` drops remote-tracking branches whose
/// upstream was deleted, mirroring VSCode's default fetch behaviour. Failure
/// (auth, unreachable remote, no remote configured) surfaces git's own stderr.
pub(super) fn git_fetch(session: Option<&str>) -> GitOpResult {
    const OP: &str = "fetch";
    let Some(root) = repo_root_for(session) else {
        return op_err(OP, "not a git repository");
    };
    let ssh_cmd = ssh_command_for(&root);
    let extra = ssh_cmd.as_ref().map(|c| ("GIT_SSH_COMMAND", c.as_str()));
    match git_cmd_env(&root, &["fetch", "--prune"], extra) {
        Some(out) if out.status.success() => match success_message(&out) {
            Some(msg) => op_ok_msg(OP, msg),
            None => op_ok(OP),
        },
        Some(out) => op_err(OP, git_failure(&out, "git fetch failed")),
        None => op_err(OP, "failed to run git"),
    }
}

/// `git pull --ff-only` for `session`'s repo, answering a
/// [`super::HostCtl::GitPull`]. `--ff-only` is the safe choice for an
/// unattended GUI action: it FAILS LOUDLY (git's own stderr, e.g. "Not
/// possible to fast-forward") on any divergence rather than silently creating
/// a merge commit or leaving a half-merged/conflicted tree. Same
/// `GIT_SSH_COMMAND` injection as [`git_fetch`].
pub(super) fn git_pull(session: Option<&str>) -> GitOpResult {
    const OP: &str = "pull";
    let Some(root) = repo_root_for(session) else {
        return op_err(OP, "not a git repository");
    };
    let ssh_cmd = ssh_command_for(&root);
    let extra = ssh_cmd.as_ref().map(|c| ("GIT_SSH_COMMAND", c.as_str()));
    match git_cmd_env(&root, &["pull", "--ff-only"], extra) {
        Some(out) if out.status.success() => match success_message(&out) {
            Some(msg) => op_ok_msg(OP, msg),
            None => op_ok(OP),
        },
        Some(out) => op_err(OP, git_failure(&out, "git pull failed")),
        None => op_err(OP, "failed to run git"),
    }
}

/// `git push` for `session`'s repo, answering a [`super::HostCtl::GitPush`].
/// Same `GIT_SSH_COMMAND` injection as [`git_fetch`]/[`git_pull`]. Failure
/// (no upstream configured, non-fast-forward rejection, auth) surfaces git's
/// own stderr verbatim — never force-pushes or configures an upstream on the
/// GUI's behalf.
pub(super) fn git_push(session: Option<&str>) -> GitOpResult {
    const OP: &str = "push";
    let Some(root) = repo_root_for(session) else {
        return op_err(OP, "not a git repository");
    };
    let ssh_cmd = ssh_command_for(&root);
    let extra = ssh_cmd.as_ref().map(|c| ("GIT_SSH_COMMAND", c.as_str()));
    match git_cmd_env(&root, &["push"], extra) {
        Some(out) if out.status.success() => match success_message(&out) {
            Some(msg) => op_ok_msg(OP, msg),
            None => op_ok(OP),
        },
        Some(out) => op_err(OP, git_failure(&out, "git push failed")),
        None => op_err(OP, "failed to run git"),
    }
}
