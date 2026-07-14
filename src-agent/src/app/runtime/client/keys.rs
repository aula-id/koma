//! Host-side SSH KEY VAULT for the GUI Settings "SSH Keys" submenu — a
//! GUI-only, manual, user-owned key store. COMPLETELY SEPARATE from the
//! model's own git credential machinery (`git_cred.rs`/`git_operator.rs`):
//! the whole point of this vault is keys the USER owns and manages by hand,
//! distinct from whatever the agent uses. Everything here is host-local
//! (never the daemon), mirroring [`super::git`]'s exact host-relay pattern —
//! a `ssh_keygen_cmd` choke point (like `git_cmd`), `KeyOpResult { ok, op,
//! error }` (like `GitOpResult`), and [`sanitize_name`] as defense-in-depth
//! anchoring (like `safe_join`).
//!
//! Wave 4a scope: list / generate / import / reveal / delete. Remote
//! push/pull (wave 4b) is NOT implemented here.
//!
//! Keys live under `<~/.koma>/keys/` (resolved via
//! [`crate::model::store::base_dir`] — the SAME home/`~/.koma` resolver the
//! catalogue-overlay cache uses, never a hand-rolled home lookup). A keypair
//! is `<name>` (the private half, written 0600) + `<name>.pub` (the public
//! half). Every op shells `ssh-keygen` via `std::process::Command` (no shell
//! interpolation, ever), with `GIT_TERMINAL_PROMPT=0` so it can never block on
//! an interactive prompt.

use std::path::PathBuf;

/// One keypair entry in a [`list_keys`] reply.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct KeyInfo {
    pub name: String,
    pub fingerprint: String,
    pub comment: String,
    pub key_type: String,
}

/// The result of a host-side key MUTATION (generate/import/delete), pushed to
/// the GUI as a `KeyOp` envelope — mirrors [`super::git::GitOpResult`]
/// exactly. `op` is `"generate"`/`"import"`/`"delete"`; `error` (set only when
/// `ok` is `false`) is the failure reason so the Settings section can toast
/// it. Carries NO list data — always followed by a fresh [`list_keys`] push
/// so the vault list refreshes from authoritative state.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct KeyOpResult {
    pub ok: bool,
    pub op: String,
    pub error: Option<String>,
}

/// The result of a host-side [`reveal_key`], pushed as a `KeyReveal`
/// envelope. `private` echoes the request (`true` = the private half was
/// read) so React applies the reply to the right affordance ("Copy public
/// key" vs "Reveal private key"). `error` set means `content` is empty.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct KeyRevealResult {
    pub name: String,
    pub private: bool,
    pub content: String,
    pub error: Option<String>,
}

fn op_ok(op: &str) -> KeyOpResult {
    KeyOpResult { ok: true, op: op.to_string(), error: None }
}

fn op_err(op: &str, error: impl Into<String>) -> KeyOpResult {
    KeyOpResult { ok: false, op: op.to_string(), error: Some(error.into()) }
}

/// Resolve `<~/.koma>/keys/`, creating it if missing. Reuses the SAME
/// home/`~/.koma` resolver [`crate::model::store::base_dir`] the catalogue
/// overlay's `models.json` cache uses (`fetch.rs`) — never a hand-rolled home
/// lookup. `Err` (a string, ready to surface as an op error) only on a
/// resolution/create failure (no `$HOME`, or a permissions error).
fn keys_dir() -> Result<PathBuf, String> {
    let base = crate::model::store::base_dir().map_err(|e| e.to_string())?;
    let dir = base.join("keys");
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create key vault dir: {e}"))?;
    Ok(dir)
}

/// Sanitize an untrusted, wire-supplied key `name`: only ASCII
/// `[A-Za-z0-9._-]`, non-empty, never bare `.`/`..`, and never `.pub`-suffixed
/// (ASCII case-insensitive). Every op that takes a name MUST go through this
/// FIRST — defense-in-depth, mirroring [`super::git::safe_join`]'s
/// component-based path rejection (a name here can never contain a path
/// separator, since `/` and `\` aren't in the allowed char-set — the explicit
/// checks below are belt + suspenders against a future char-set widening). A
/// `.pub`-suffixed name is rejected outright: it would collide with the
/// public-key naming convention, making the resulting keypair invisible to
/// [`list_keys`] (which classifies entries by `!ends_with(".pub")`) and
/// un-deletable. `None` means reject.
pub(super) fn sanitize_name(name: &str) -> Option<String> {
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    if name.contains('/') || name.contains('\\') {
        return None;
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return None;
    }
    if name.len() >= 4 && name[name.len() - 4..].eq_ignore_ascii_case(".pub") {
        return None;
    }
    Some(name.to_string())
}

/// Resolve the PRIVATE-key file path for `name` (wave 4b — used by
/// [`super::git_remote`]'s remote ops to build a `GIT_SSH_COMMAND` override).
/// `name` is rejected via [`sanitize_name`] first; `None` is also returned
/// when the vault dir can't be resolved OR the named private-key file doesn't
/// exist (an assigned-but-since-deleted key), so a caller can safely treat
/// `None` as "fall back to no SSH override" without a separate existence
/// check of its own.
pub(super) fn key_private_path(name: &str) -> Option<PathBuf> {
    let name = sanitize_name(name)?;
    let dir = keys_dir().ok()?;
    let path = dir.join(&name);
    if path.is_file() {
        Some(path)
    } else {
        None
    }
}

/// Run `ssh-keygen <args>` with `GIT_TERMINAL_PROMPT=0` (never block on an
/// interactive passphrase/prompt, mirroring `git_cmd`'s guard), returning
/// `None` on any spawn failure rather than panicking.
fn ssh_keygen_cmd(args: &[&str]) -> Option<std::process::Output> {
    std::process::Command::new("ssh-keygen")
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .ok()
}

/// Extract ssh-keygen's own failure message from a non-zero `Output`: prefer
/// stderr (where ssh-keygen's errors land), falling back to stdout, then a
/// generic fallback if both are empty. Mirrors `git.rs`'s `git_failure`.
fn keygen_failure(out: &std::process::Output, fallback: &str) -> String {
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !stdout.is_empty() {
        return stdout;
    }
    fallback.to_string()
}

/// Parse one `ssh-keygen -lf <pub>` output line: `<bits> SHA256:<hash>
/// <comment> (<TYPE>)` — `<comment>` may itself contain spaces (or be empty,
/// e.g. a key generated with `-C ""`), so the key TYPE is located by the
/// LAST `(...)` group rather than naive whitespace splitting. `ssh-keygen`
/// itself emits the literal placeholder string `no comment` for a
/// comment-less key — that's normalized to an empty string here so the UI
/// shows nothing rather than the literal placeholder. Returns `None` if the
/// line doesn't have the expected `<bits> <fingerprint> ...` shape.
fn parse_keygen_lf(output: &str) -> Option<(String, String, String)> {
    let line = output.trim();
    let mut head = line.splitn(2, ' ');
    let _bits = head.next()?;
    let rest = head.next()?.trim();
    let mut rest_parts = rest.splitn(2, ' ');
    let fingerprint = rest_parts.next()?.to_string();
    let remainder = rest_parts.next().unwrap_or("").trim();
    let (comment, key_type) = match remainder.rfind('(') {
        Some(open) => {
            let comment = remainder[..open].trim().to_string();
            let ty = remainder[open + 1..].trim_end_matches(')').trim().to_string();
            (comment, ty)
        }
        None => (remainder.to_string(), String::new()),
    };
    let comment = if comment == "no comment" { String::new() } else { comment };
    Some((fingerprint, comment, key_type))
}

/// List every keypair in the vault, answering a [`super::HostCtl::KeyList`].
/// A "keypair" is any file under `<~/.koma>/keys/` whose name does NOT end in
/// `.pub` and which HAS a sibling `<name>.pub` (an orphaned private-only or
/// public-only file is skipped — it isn't a usable pair). Fingerprint/type/
/// comment come from `ssh-keygen -lf <name>.pub`; an entry whose `.pub` fails
/// to parse (corrupt file) is silently dropped rather than erroring the whole
/// list. ALWAYS returns (an empty `Vec` on any directory-level failure)
/// rather than panicking — creates the vault dir if missing (mirrors
/// `compute_git_status`'s always-reply rule, though there's no `error` field
/// here since an empty list is itself a valid "no keys yet" state).
pub(super) fn list_keys() -> Vec<KeyInfo> {
    let dir = match keys_dir() {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut names: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(fname) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if fname.ends_with(".pub") {
            continue;
        }
        if !dir.join(format!("{fname}.pub")).exists() {
            continue;
        }
        names.push(fname.to_string());
    }
    names.sort();

    names
        .into_iter()
        .filter_map(|name| {
            let pub_path = dir.join(format!("{name}.pub"));
            let pub_str = pub_path.to_str()?;
            let out = ssh_keygen_cmd(&["-lf", pub_str])?;
            if !out.status.success() {
                return None;
            }
            let text = String::from_utf8_lossy(&out.stdout);
            let (fingerprint, comment, key_type) = parse_keygen_lf(&text)?;
            Some(KeyInfo { name, fingerprint, comment, key_type })
        })
        .collect()
}

/// Generate a fresh PASSPHRASE-LESS ed25519 keypair named `name` (rejected via
/// [`sanitize_name`] first), answering a [`super::HostCtl::KeyGenerate`].
/// Fails outright if `<name>` already exists (never overwrites). `comment`
/// defaults to `"koma"` when blank/whitespace-only. Passphrase-less is
/// deliberate — these are for automated git operations (wave 4b), not
/// interactive login; `ssh-keygen -N ""` supplies the empty passphrase so the
/// call never blocks on a prompt.
pub(super) fn generate_key(name: &str, comment: &str) -> KeyOpResult {
    const OP: &str = "generate";
    let Some(name) = sanitize_name(name) else {
        return op_err(OP, "invalid key name");
    };
    let dir = match keys_dir() {
        Ok(d) => d,
        Err(e) => return op_err(OP, e),
    };
    let priv_path = dir.join(&name);
    if priv_path.exists() {
        return op_err(OP, format!("a key named \"{name}\" already exists"));
    }
    let comment = {
        let c = comment.trim();
        if c.is_empty() { "koma".to_string() } else { c.to_string() }
    };
    let Some(priv_str) = priv_path.to_str() else {
        return op_err(OP, "invalid path encoding");
    };
    match ssh_keygen_cmd(&["-t", "ed25519", "-f", priv_str, "-N", "", "-C", &comment]) {
        Some(out) if out.status.success() => op_ok(OP),
        Some(out) => op_err(OP, keygen_failure(&out, "ssh-keygen failed")),
        None => op_err(OP, "failed to run ssh-keygen"),
    }
}

/// Import an EXISTING private key `private_key` (raw PEM/OpenSSH text) under
/// `name` (rejected via [`sanitize_name`] first), answering a
/// [`super::HostCtl::KeyImport`]. Fails outright if `<name>` already exists.
/// Writes the private half FIRST, CREATED with 0600 permissions from the
/// start (`OpenOptions::mode(0o600)` + `create_new` — never briefly
/// group/other-readable at umask default, closing the TOCTOU window a
/// create-then-chmod sequence would leave open), then derives the public half
/// via `ssh-keygen -y -f <name>`; if that derivation fails (a
/// passphrase-protected key, malformed content, or any other `ssh-keygen`
/// rejection) the just-written private file is cleaned up and an error
/// surfaced — never leaves an orphaned/unusable private-only file behind.
pub(super) fn import_key(name: &str, private_key: &str) -> KeyOpResult {
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;

    const OP: &str = "import";
    let Some(name) = sanitize_name(name) else {
        return op_err(OP, "invalid key name");
    };
    let dir = match keys_dir() {
        Ok(d) => d,
        Err(e) => return op_err(OP, e),
    };
    let priv_path = dir.join(&name);
    if priv_path.exists() {
        return op_err(OP, format!("a key named \"{name}\" already exists"));
    }

    // ssh-keygen/OpenSSH key parsers expect a trailing newline; be forgiving of
    // pasted text that's missing one.
    let mut content = private_key.to_string();
    if !content.ends_with('\n') {
        content.push('\n');
    }

    {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        // Create with 0600 from the start (no create-then-chmod TOCTOU window).
        // TODO(windows-port, phase B3: restrict ACLs so the private key file
        // isn't inherited-permissive) — Windows has no unix permission bits;
        // a real port needs a security-descriptor/ACL equivalent at create time.
        #[cfg(unix)]
        opts.mode(0o600);
        let file = opts.open(&priv_path);
        match file {
            Ok(mut f) => {
                if let Err(e) = f.write_all(content.as_bytes()) {
                    let _ = std::fs::remove_file(&priv_path);
                    return op_err(OP, format!("could not write key: {e}"));
                }
            }
            Err(e) => return op_err(OP, format!("could not write key: {e}")),
        }
    }

    let Some(priv_str) = priv_path.to_str() else {
        let _ = std::fs::remove_file(&priv_path);
        return op_err(OP, "invalid path encoding");
    };

    match ssh_keygen_cmd(&["-y", "-f", priv_str]) {
        Some(out) if out.status.success() => {
            let pub_path = dir.join(format!("{name}.pub"));
            if let Err(e) = std::fs::write(&pub_path, &out.stdout) {
                let _ = std::fs::remove_file(&priv_path);
                return op_err(OP, format!("could not write public key: {e}"));
            }
            op_ok(OP)
        }
        Some(out) => {
            let _ = std::fs::remove_file(&priv_path);
            let failure = keygen_failure(&out, "ssh-keygen -y failed");
            if failure.to_lowercase().contains("passphrase") {
                op_err(
                    OP,
                    "passphrase-protected keys are not supported — remove the passphrase first (ssh-keygen -p)",
                )
            } else {
                op_err(OP, format!("could not import key: {failure}"))
            }
        }
        None => {
            let _ = std::fs::remove_file(&priv_path);
            op_err(OP, "could not import key: failed to run ssh-keygen")
        }
    }
}

/// Reveal `name`'s contents — the public half (`private: false`, for "Copy
/// public key") or the PRIVATE half (`private: true`, for "Reveal private
/// key"), answering a [`super::HostCtl::KeyReveal`]. Private reveal is
/// INTENTIONAL: the user owns these keys outright, and the React side gates
/// it behind a deliberate click + a warning (never surfaced passively).
/// `name` is rejected via [`sanitize_name`] first; a read failure (missing
/// file, permissions) sets `error` with empty `content` rather than
/// panicking.
pub(super) fn reveal_key(name: &str, private: bool) -> KeyRevealResult {
    let empty = |error: Option<String>| KeyRevealResult {
        name: name.to_string(),
        private,
        content: String::new(),
        error,
    };
    let Some(name) = sanitize_name(name) else {
        return empty(Some("invalid key name".to_string()));
    };
    let dir = match keys_dir() {
        Ok(d) => d,
        Err(e) => return empty(Some(e)),
    };
    let path = if private { dir.join(&name) } else { dir.join(format!("{name}.pub")) };
    match std::fs::read_to_string(&path) {
        Ok(content) => KeyRevealResult { name, private, content, error: None },
        Err(e) => empty(Some(format!("could not read key: {e}"))),
    }
}

/// Delete keypair `name` (both halves, best-effort), answering a
/// [`super::HostCtl::KeyDelete`]. `name` is rejected via [`sanitize_name`]
/// first. Each half is removed independently — a missing half is not an
/// error (deleting an already-half-gone pair still succeeds for the half
/// that's there); a REMOVE failure (permissions) on a half that exists is
/// collected and reported, mirroring `git_discard`'s per-path error
/// collection.
pub(super) fn delete_key(name: &str) -> KeyOpResult {
    const OP: &str = "delete";
    let Some(name) = sanitize_name(name) else {
        return op_err(OP, "invalid key name");
    };
    let dir = match keys_dir() {
        Ok(d) => d,
        Err(e) => return op_err(OP, e),
    };

    let mut errors: Vec<String> = Vec::new();

    let priv_path = dir.join(&name);
    if priv_path.exists() {
        if let Err(e) = std::fs::remove_file(&priv_path) {
            errors.push(format!("private key: {e}"));
        }
    }
    let pub_path = dir.join(format!("{name}.pub"));
    if pub_path.exists() {
        if let Err(e) = std::fs::remove_file(&pub_path) {
            errors.push(format!("public key: {e}"));
        }
    }

    if errors.is_empty() {
        op_ok(OP)
    } else {
        op_err(OP, errors.join("; "))
    }
}
