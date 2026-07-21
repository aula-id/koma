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
use std::sync::{Mutex, OnceLock};

use super::git::{git_cmd_env, git_failure, repo_root_for, with_git_transaction, GitOpResult};
use super::keys::key_private_path;

/// Repeated closure signature for host-side git command relays.
type GitCmdFn<'a> =
    &'a dyn Fn(&Path, &[&str], Option<(&str, &str)>) -> Option<std::process::Output>;

/// Process-lifetime monotonic counter folded into [`atomic_write`]'s temp
/// filename alongside the PID — the PID alone is shared by every thread in
/// this process, so two concurrent writers (the two host loops, or a fast
/// double-click on the key picker) would otherwise race on the SAME temp
/// path and lose one write to a `rename` ENOENT.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

fn op_ok(op: &str) -> GitOpResult {
    GitOpResult {
        ok: true,
        op: op.to_string(),
        error: None,
        message: None,
    }
}

fn op_ok_msg(op: &str, message: impl Into<String>) -> GitOpResult {
    GitOpResult {
        ok: true,
        op: op.to_string(),
        error: None,
        message: Some(message.into()),
    }
}

fn op_err(op: &str, error: impl Into<String>) -> GitOpResult {
    GitOpResult {
        ok: false,
        op: op.to_string(),
        error: Some(error.into()),
        message: None,
    }
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
    let file_name = path.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no file name")
    })?;
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
    load_map()
        .get(&root.to_string_lossy().into_owned())
        .cloned()
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

/// Push behavior requested by the GUI. `Automatic` delegates the decision to the
/// authoritative host planner; the other values are deliberate menu choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum GitPushMode {
    Automatic,
    Plain,
    SetUpstream,
    ForceWithLease,
}

#[derive(Clone, Debug)]
struct RebaseProof {
    old_tip: String,
    new_tip: String,
}

static REBASE_PROOFS: OnceLock<Mutex<HashMap<(PathBuf, String), RebaseProof>>> = OnceLock::new();

fn proofs() -> &'static Mutex<HashMap<(PathBuf, String), RebaseProof>> {
    REBASE_PROOFS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn begin_rebase(root: &Path, branch: &str, old_tip: &str) {
    let mut tracker = proofs().lock().unwrap_or_else(|e| e.into_inner());
    // Starting a new GUI rewrite invalidates every older proof in this repository.
    tracker.retain(|(proof_root, _), _| proof_root != root);
    tracker.insert(
        (root.to_path_buf(), branch.to_string()),
        RebaseProof {
            old_tip: old_tip.to_string(),
            new_tip: String::new(),
        },
    );
}

pub(super) fn finish_rebase(root: &Path, branch: &str, new_tip: &str) -> bool {
    let key = (root.to_path_buf(), branch.to_string());
    let mut tracker = proofs().lock().unwrap_or_else(|e| e.into_inner());
    let Some(proof) = tracker
        .get_mut(&key)
        .filter(|proof| proof.new_tip.is_empty())
    else {
        return false;
    };
    if proof.old_tip == new_tip {
        tracker.remove(&key);
    } else {
        proof.new_tip = new_tip.to_string();
    }
    true
}

pub(super) fn clear_pending_rebase(root: &Path) {
    proofs()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|(proof_root, _), proof| proof_root != root || !proof.new_tip.is_empty());
}

pub(super) fn invalidate_rebase_proofs(root: &Path) {
    proofs()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|(proof_root, _), _| proof_root != root);
}

pub(super) fn has_pending_rebase(root: &Path, branch: &str, old_tip: &str) -> bool {
    proofs()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&(root.to_path_buf(), branch.to_string()))
        .is_some_and(|proof| proof.new_tip.is_empty() && proof.old_tip == old_tip)
}

/// Record only GUI-initiated, successfully completed rebases.  A later automatic
/// force push must additionally prove ancestry and an exact remote lease.
pub(super) fn record_rebase(root: &Path, branch: &str, old_tip: &str, new_tip: &str) {
    let key = (root.to_path_buf(), branch.to_string());
    let mut tracker = proofs().lock().unwrap_or_else(|e| e.into_inner());
    if old_tip == new_tip {
        tracker.remove(&key);
    } else {
        tracker.insert(
            key,
            RebaseProof {
                old_tip: old_tip.to_string(),
                new_tip: new_tip.to_string(),
            },
        );
    }
}

#[derive(Debug)]
struct PushTarget {
    branch: String,
    remote: String,
    remote_branch: String,
    upstream_oid: Option<String>,
    has_upstream: bool,
}

fn output_text(out: Option<std::process::Output>) -> Option<String> {
    let out = out?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn valid_remote_component(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && !value.bytes().any(|b| b == 0 || b.is_ascii_whitespace())
}

fn parse_status_headers(text: &str) -> (Option<String>, Option<String>) {
    let mut branch = None;
    let mut upstream = None;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("# branch.head ") {
            if v != "(detached)" {
                branch = Some(v.to_string());
            }
        } else if let Some(v) = line.strip_prefix("# branch.upstream ") {
            upstream = Some(v.to_string());
        }
    }
    (branch, upstream)
}

fn config_value(
    git: GitCmdFn<'_>,
    root: &Path,
    key: &str,
) -> Option<String> {
    output_text(git(root, &["config", "--get", key], None)).filter(|s| !s.is_empty())
}

fn plan_target(
    git: GitCmdFn<'_>,
    root: &Path,
) -> Result<PushTarget, String> {
    let status = output_text(git(root, &["status", "--porcelain=v2", "--branch"], None))
        .ok_or_else(|| "git status failed".to_string())?;
    let (branch, upstream) = parse_status_headers(&status);
    let branch = branch.ok_or_else(|| "cannot push detached HEAD".to_string())?;
    if !valid_remote_component(&branch) {
        return Err("invalid current branch".to_string());
    }
    if let Some(upstream) = upstream {
        let branch_remote_key = format!("branch.{branch}.remote");
        let merge_key = format!("branch.{branch}.merge");
        let tracking_remote = config_value(git, root, &branch_remote_key)
            .filter(|r| r != ".")
            .ok_or_else(|| "unsupported upstream configuration".to_string())?;
        let merge_ref = config_value(git, root, &merge_key)
            .ok_or_else(|| "unsupported upstream configuration".to_string())?;
        let merge_branch = merge_ref
            .strip_prefix("refs/heads/")
            .ok_or_else(|| "nonstandard upstream mapping is not supported".to_string())?;
        // Porcelain's upstream name must have the standard remote/branch mapping;
        // otherwise deriving a push destination from a custom fetch refspec is unsafe.
        if upstream != format!("{tracking_remote}/{merge_branch}") {
            return Err("nonstandard upstream mapping is not supported".to_string());
        }

        let branch_push = format!("branch.{branch}.pushRemote");
        let remote = config_value(git, root, &branch_push)
            .or_else(|| config_value(git, root, "remote.pushDefault"))
            .unwrap_or_else(|| tracking_remote.clone());
        // An override remote has no independent merge mapping. It is safe only
        // for the standard same-name (`push.default=simple`) destination.
        let remote_branch = if remote == tracking_remote {
            merge_branch.to_string()
        } else if merge_branch == branch.as_str() {
            branch.clone()
        } else {
            return Err("push remote override has an ambiguous destination".to_string());
        };
        if !valid_remote_component(&remote) || !valid_remote_component(&remote_branch) {
            return Err("unsupported upstream configuration".to_string());
        }
        let upstream_oid = output_text(git(root, &["rev-parse", "--verify", &upstream], None));
        return Ok(PushTarget {
            branch,
            remote,
            remote_branch,
            upstream_oid,
            has_upstream: true,
        });
    }

    // Never guess among several remotes. Respect Git's explicit push settings,
    // then the branch fetch remote, and only finally a sole configured remote.
    let branch_push = format!("branch.{branch}.pushRemote");
    let branch_remote = format!("branch.{branch}.remote");
    let branch_remote_value = config_value(git, root, &branch_remote);
    let explicit = config_value(git, root, &branch_push)
        .or_else(|| config_value(git, root, "remote.pushDefault"));
    let remote = if let Some(remote) = explicit {
        Some(remote)
    } else if let Some(remote) = branch_remote_value {
        (remote != ".").then_some(remote)
    } else {
        let remotes = output_text(git(root, &["remote"], None)).unwrap_or_default();
        let mut lines = remotes.lines();
        let one = lines.next();
        match (one, lines.next()) {
            (Some(one), None) => Some(one.to_string()),
            _ => None,
        }
    }
    .ok_or_else(|| "no unambiguous remote is configured".to_string())?;
    if !valid_remote_component(&remote) {
        return Err("invalid push remote".to_string());
    }
    Ok(PushTarget {
        branch: branch.clone(),
        remote,
        remote_branch: branch,
        upstream_oid: None,
        has_upstream: false,
    })
}

fn is_ancestor(
    git: GitCmdFn<'_>,
    root: &Path,
    older: &str,
    newer: &str,
) -> bool {
    git(root, &["merge-base", "--is-ancestor", older, newer], None)
        .is_some_and(|o| o.status.success())
}

fn automatic_mode(
    git: GitCmdFn<'_>,
    root: &Path,
    target: &PushTarget,
) -> Result<GitPushMode, String> {
    let Some(upstream) = target.upstream_oid.as_deref() else {
        return if target.has_upstream {
            Err("cannot resolve the configured upstream".to_string())
        } else {
            Ok(GitPushMode::SetUpstream)
        };
    };
    let head = output_text(git(root, &["rev-parse", "--verify", "HEAD"], None))
        .ok_or_else(|| "cannot resolve HEAD".to_string())?;
    if is_ancestor(git, root, upstream, &head) {
        return Ok(GitPushMode::Plain);
    }
    let proof = proofs()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&(root.to_path_buf(), target.branch.clone()))
        .cloned();
    match proof {
        Some(p) if p.new_tip == head && is_ancestor(git, root, upstream, &p.old_tip) => {
            Ok(GitPushMode::ForceWithLease)
        }
        Some(_) => {
            proofs()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&(root.to_path_buf(), target.branch.clone()));
            Err(
                "branch diverged from its upstream; no GUI rebase proof permits a force push"
                    .to_string(),
            )
        }
        None => Err(
            "branch diverged from its upstream; no GUI rebase proof permits a force push"
                .to_string(),
        ),
    }
}

/// Cheap local projection used by GitStatus to label the push menu. The actual
/// push always replans under the transaction and, for force, checks the server.
pub(super) fn push_mode_for(root: &Path) -> Option<GitPushMode> {
    with_git_transaction(|git| {
        let target = plan_target(git, root).ok()?;
        automatic_mode(git, root, &target).ok()
    })
}

/// Authoritative push planner/executor. Planning and execution are serialized as
/// one transaction. Force is accepted only with GUI-rebase proof, ancestry proof,
/// and a freshly queried exact server-side lease.
pub(super) fn git_push(
    mode: Option<GitPushMode>,
    expected_root: Option<&str>,
    session: Option<&str>,
) -> GitOpResult {
    const OP: &str = "push";
    let Some(root) = repo_root_for(session) else {
        return op_err(OP, "not a git repository");
    };
    if expected_root.is_some_and(|expected| root != Path::new(expected)) {
        return op_err(OP, "repository changed; refresh status and try again");
    }
    let ssh_cmd = ssh_command_for(&root);
    let extra = ssh_cmd.as_ref().map(|c| ("GIT_SSH_COMMAND", c.as_str()));
    with_git_transaction(|git| {
        let target = match plan_target(git, &root) {
            Ok(v) => v,
            Err(e) => return op_err(OP, e),
        };
        let requested = mode.unwrap_or(GitPushMode::Automatic);
        let selected = if requested == GitPushMode::Automatic {
            match automatic_mode(git, &root, &target) {
                Ok(GitPushMode::ForceWithLease) => {
                    return op_err(OP, "force push requires explicit inline confirmation")
                }
                Ok(v) => v,
                Err(e) => return op_err(OP, e),
            }
        } else {
            requested
        };

        let dst = format!("HEAD:refs/heads/{}", target.remote_branch);
        let out = match selected {
            GitPushMode::Automatic => unreachable!(),
            GitPushMode::Plain => git(&root, &["push", &target.remote, &dst], extra),
            GitPushMode::SetUpstream => {
                if target.has_upstream {
                    return op_err(OP, "branch already has an upstream");
                }
                git(
                    &root,
                    &["push", "--set-upstream", &target.remote, &dst],
                    extra,
                )
            }
            GitPushMode::ForceWithLease => {
                if automatic_mode(git, &root, &target) != Ok(GitPushMode::ForceWithLease) {
                    return op_err(OP, "force push is not backed by a current GUI rebase proof");
                }
                let expected = match target.upstream_oid.as_deref() {
                    Some(v) => v,
                    None => return op_err(OP, "force push requires an upstream"),
                };
                let remote_ref = format!("refs/heads/{}", target.remote_branch);
                let advertised = output_text(git(
                    &root,
                    &["ls-remote", &target.remote, &remote_ref],
                    extra,
                ))
                .and_then(|s| s.split_whitespace().next().map(str::to_string));
                if advertised.as_deref() != Some(expected) {
                    return op_err(OP, "remote changed since the last fetch; fetch and review before force pushing");
                }
                let lease = format!("--force-with-lease={remote_ref}:{expected}");
                git(&root, &["push", &lease, &target.remote, &dst], extra)
            }
        };
        match out {
            Some(out) if out.status.success() => {
                if selected == GitPushMode::ForceWithLease {
                    proofs()
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&(root.clone(), target.branch.clone()));
                }
                match success_message(&out) {
                    Some(msg) => op_ok_msg(OP, msg),
                    None => op_ok(OP),
                }
            }
            Some(out) => op_err(OP, git_failure(&out, "git push failed")),
            None => op_err(OP, "failed to run git"),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_branch_headers() {
        let (branch, upstream) = parse_status_headers(
            "# branch.oid abc\n# branch.head feature/x\n# branch.upstream origin/feature/x\n# branch.ab +2 -1\n",
        );
        assert_eq!(branch.as_deref(), Some("feature/x"));
        assert_eq!(upstream.as_deref(), Some("origin/feature/x"));
    }

    #[test]
    fn detached_and_missing_upstream_are_distinct() {
        assert_eq!(
            parse_status_headers("# branch.head (detached)\n"),
            (None, None)
        );
        assert_eq!(
            parse_status_headers("# branch.head topic\n"),
            (Some("topic".to_string()), None)
        );
    }

    #[test]
    fn remote_components_reject_option_and_whitespace_injection() {
        assert!(valid_remote_component("origin"));
        assert!(valid_remote_component("team/repo"));
        assert!(!valid_remote_component(""));
        assert!(!valid_remote_component("--upload-pack=evil"));
        assert!(!valid_remote_component("origin other"));
        assert!(!valid_remote_component("origin\nother"));
    }

    fn ok_output(text: &str) -> std::process::Output {
        std::process::Output {
            status: std::process::Command::new("true").status().unwrap(),
            stdout: text.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    #[test]
    fn existing_upstream_honors_push_remote_precedence() {
        let git = |_root: &Path, args: &[&str], _extra: Option<(&str, &str)>| {
            let value = match args {
                ["status", "--porcelain=v2", "--branch"] => {
                    "# branch.head topic\n# branch.upstream origin/topic\n"
                }
                ["config", "--get", "branch.topic.remote"] => "origin\n",
                ["config", "--get", "branch.topic.merge"] => "refs/heads/topic\n",
                ["config", "--get", "branch.topic.pushRemote"] => "publish\n",
                ["config", "--get", "remote.pushDefault"] => "fallback\n",
                ["rev-parse", "--verify", "origin/topic"] => "abc\n",
                _ => return None,
            };
            Some(ok_output(value))
        };
        let target = plan_target(&git, Path::new(".")).unwrap();
        assert_eq!(target.remote, "publish");
        assert_eq!(target.remote_branch, "topic");
    }

    #[test]
    fn existing_upstream_refuses_ambiguous_override_destination() {
        let git = |_root: &Path, args: &[&str], _extra: Option<(&str, &str)>| {
            let value = match args {
                ["status", "--porcelain=v2", "--branch"] => {
                    "# branch.head topic\n# branch.upstream origin/main\n"
                }
                ["config", "--get", "branch.topic.remote"] => "origin\n",
                ["config", "--get", "branch.topic.merge"] => "refs/heads/main\n",
                ["config", "--get", "branch.topic.pushRemote"] => "publish\n",
                _ => return None,
            };
            Some(ok_output(value))
        };
        assert!(plan_target(&git, Path::new(".")).is_err());
    }

    #[test]
    fn push_modes_have_stable_wire_names() {
        assert_eq!(
            serde_json::to_string(&GitPushMode::Automatic).unwrap(),
            "\"automatic\""
        );
        assert_eq!(
            serde_json::to_string(&GitPushMode::SetUpstream).unwrap(),
            "\"set-upstream\""
        );
        assert_eq!(
            serde_json::to_string(&GitPushMode::ForceWithLease).unwrap(),
            "\"force-with-lease\""
        );
        assert_eq!(
            serde_json::from_str::<GitPushMode>("\"force-with-lease\"").unwrap(),
            GitPushMode::ForceWithLease
        );
    }

    #[test]
    fn rebase_tracker_records_only_rewrites() {
        let root = Path::new("tracker-test-root");
        let key = (root.to_path_buf(), "topic".to_string());
        proofs()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&key);
        begin_rebase(root, "topic", "aaa");
        assert!(has_pending_rebase(root, "topic", "aaa"));
        assert!(!finish_rebase(root, "other", "bbb"));
        assert!(has_pending_rebase(root, "topic", "aaa"));
        record_rebase(root, "topic", "aaa", "aaa");
        assert!(proofs()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
            .is_none());
        record_rebase(root, "topic", "aaa", "bbb");
        let proof = proofs()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
            .cloned()
            .unwrap();
        assert_eq!(proof.old_tip, "aaa");
        assert_eq!(proof.new_tip, "bbb");
        proofs()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&key);
    }
}
