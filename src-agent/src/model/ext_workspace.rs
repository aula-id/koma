//! Extension workspace roots — validate + inject an extension's declared
//! `workspace_dir` (see [`koma_extension::protocol::ExtensionManifest::workspace_dir`])
//! into a session's workspace roots so agent writes into that directory pass the
//! safety harness.
//!
//! # Why this lives in `settings.workdir`
//!
//! Enforcement has TWO gates and they read DIFFERENT lists:
//! - the turn-level workspace check ([`crate::app::harness::workspace_allowed`]) reads
//!   BOTH `settings.workdir` AND `settings.allowed_folders`, but
//! - the hard per-path containment in [`crate::tool::resolve`] only honours the
//!   `settings.workdir` roots (a `[N]`-indexed list).
//!
//! So an extension directory must land in `settings.workdir` to be writable — putting
//! it in `allowed_folders` would pass the turn gate yet still get every individual
//! write rejected by `tool::resolve`. [`inject_extension_workspaces`] therefore pushes
//! the validated canonical path onto `settings.workdir`.
//!
//! # Lifecycle (in-memory, re-derived every boot)
//!
//! Injection is IN-MEMORY only (the caller does not persist it): the root set is
//! re-derived from the CURRENTLY-ENABLED extension set on every daemon/TUI start, so a
//! disabled or uninstalled extension's root simply isn't re-added next start — no stale
//! entry accrues in `settings.json`. Injection is idempotent (canonical-equality dedupe
//! against the existing roots), so repeated boots after the first are a no-op.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};

use koma_extension::protocol::ExtensionManifest;

use crate::model::app_config::InstalledExtension;
use crate::model::store;

/// Validate an extension's declared `workspace_dir`, create it if missing, and return
/// its canonical path. This is NET-NEW security code — a manifest is only as trusted as
/// the store that signed it, and this dir becomes a writable workspace root, so the
/// policy is deliberately strict and enforced on the CANONICAL (symlink-/`..`-resolved)
/// path, never a string prefix.
///
/// Steps:
/// 1. **Tilde-expand** via [`dirs::home_dir`] — `~` → `$HOME`, `~/x` → `$HOME/x`. A
///    `~user` form (anything starting with `~` that is neither `~` nor `~/…`) is
///    REJECTED (koma does not resolve other users' homes). Non-tilde paths are taken
///    verbatim (an absolute or relative path still has to survive the rules below).
/// 2. **Create** the directory (`create_dir_all` — extensions expect their state dir to
///    exist) then **canonicalize** it.
/// 3. **Reject** (returned as an `Err` the caller logs to `~/.koma/error.log`, never
///    `eprintln!`) any path that, after canonicalization, is:
///    - not STRICTLY under `$HOME` (i.e. `$HOME` itself, or anything outside `$HOME`);
///    - under koma's own state tree [`store::base_dir`] (`~/.koma`);
///    - under a credential/secret store — `~/.ssh`, `~/.aws`, `~/.gnupg` — or any of
///      their subtrees;
///    - exactly `~/.config` (its ROOT is off-limits, but XDG-style SUBdirs such as
///      `~/.config/my-ext` ARE allowed).
///
///    Everything else under `$HOME` — including a bare dotdir like `~/.babalic-extension`
///    — is allowed.
///
/// On Windows `$HOME` is `%USERPROFILE%` (both via [`dirs::home_dir`]); the identical
/// rules apply, and comparisons stay on canonicalized [`Path`]s (never string prefixes).
///
/// The obvious rejections are also checked BEFORE the `create_dir_all` so koma never
/// creates a sensitive directory it is about to reject; the post-canonicalization pass
/// is the authoritative gate that also catches a symlink or `..` escape.
pub fn validate_workspace_dir(raw: &str) -> Result<PathBuf> {
    let raw = raw.trim();
    if raw.is_empty() {
        bail!("workspace_dir is empty");
    }
    let home = dirs::home_dir().ok_or_else(|| anyhow!("cannot resolve home directory"))?;

    let expanded = expand_tilde(raw, &home)?;

    // Phase 1: lexical pre-check so a sensitive / $HOME / ~.koma path is never even
    // created on disk. Best-effort (paths not yet canonical); phase 2 is authoritative.
    if let Some(reason) = policy_violation(&expanded, &home) {
        bail!("workspace_dir '{raw}' {reason}");
    }

    std::fs::create_dir_all(&expanded)
        .map_err(|e| anyhow!("create workspace_dir '{}': {e}", expanded.display()))?;
    let canon = std::fs::canonicalize(&expanded)
        .map_err(|e| anyhow!("canonicalize workspace_dir '{}': {e}", expanded.display()))?;

    // Phase 2: authoritative check on the CANONICAL path (symlinks + `..` resolved).
    let home_canon = norm(&home);
    if let Some(reason) = policy_violation(&canon, &home_canon) {
        bail!("workspace_dir '{raw}' {reason} (after canonicalization)");
    }
    Ok(canon)
}

/// For every ENABLED installed extension that declares a valid `workspace_dir`, inject
/// its canonical path into `workdir` (the session's [`crate::model::settings::Settings::workdir`]
/// list). Idempotent: a root already present (by canonical equality) is skipped, so
/// every boot after the first is a no-op. A `workspace_dir` that fails
/// [`validate_workspace_dir`] is logged to `~/.koma/error.log` and skipped — a bad path
/// never blocks the extension. Returns the list of newly-added canonical path strings
/// (empty ⇒ nothing changed) so the caller can decide whether to reindex the dir cache.
///
/// In-memory only — the caller does NOT persist `workdir` after this (see the module
/// docs on the re-derive-every-boot lifecycle).
pub fn inject_extension_workspaces(
    installed: &[InstalledExtension],
    workdir: &mut Vec<String>,
) -> Vec<String> {
    let mut added = Vec::new();
    for ext in installed.iter().filter(|e| e.enabled) {
        let Some(raw) = read_workspace_dir(&ext.id) else {
            continue;
        };
        let canon = match validate_workspace_dir(&raw) {
            Ok(p) => p,
            Err(e) => {
                store::append_global_error_log(
                    "ext-workspace",
                    &format!("extension '{}' workspace_dir rejected: {e:#}", ext.id),
                );
                continue;
            }
        };
        // Canonical-equality dedupe against the existing roots (idempotent per boot).
        if workdir.iter().any(|w| norm(Path::new(w.trim())) == canon) {
            continue;
        }
        let canon_str = canon.display().to_string();
        workdir.push(canon_str.clone());
        added.push(canon_str);
    }
    added
}

/// Read-only companion to [`inject_extension_workspaces`] for the system-prompt note:
/// return `(index_in_workdir, extension_id)` for each ENABLED extension whose declared
/// `workspace_dir` resolves to a root already present in `workdir`. No side effects — it
/// creates nothing and re-runs no security policy, because membership in `workdir`
/// already proves the root passed [`validate_workspace_dir`] at injection time. An
/// extension whose dir was rejected (or never injected) simply doesn't match, so it is
/// naturally excluded from the note.
pub fn active_extension_workspaces(
    installed: &[InstalledExtension],
    workdir: &[String],
) -> Vec<(usize, String)> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for ext in installed.iter().filter(|e| e.enabled) {
        let Some(raw) = read_workspace_dir(&ext.id) else {
            continue;
        };
        let Ok(expanded) = expand_tilde(&raw, &home) else {
            continue;
        };
        // Canonicalize-if-exists; an injected root exists on disk so this matches the
        // canonical string injection stored in `workdir`.
        let canon = norm(&expanded);
        if let Some(idx) = workdir.iter().position(|w| norm(Path::new(w.trim())) == canon) {
            out.push((idx, ext.id.clone()));
        }
    }
    out
}

/// Read `<extensions_dir>/<id>/manifest.json` and return a trimmed, non-empty
/// `workspace_dir`, or `None` when the field is absent/blank or the manifest is
/// missing/unparsable (best-effort — a bad manifest is simply "no workspace_dir").
fn read_workspace_dir(id: &str) -> Option<String> {
    let path = store::extensions_dir().ok()?.join(id).join("manifest.json");
    let bytes = std::fs::read(&path).ok()?;
    let manifest: ExtensionManifest = serde_json::from_slice(&bytes).ok()?;
    manifest
        .workspace_dir
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Expand a leading `~` against `home`: `~` → `home`, `~/rest` → `home/rest`. A `~user`
/// form (leading `~` that is neither `~` nor `~/…`) is rejected. Any other string is
/// returned verbatim as a [`PathBuf`].
fn expand_tilde(raw: &str, home: &Path) -> Result<PathBuf> {
    if raw == "~" {
        Ok(home.to_path_buf())
    } else if let Some(rest) = raw.strip_prefix("~/") {
        Ok(home.join(rest))
    } else if raw.starts_with('~') {
        bail!("the '~user' form is not supported: {raw}")
    } else {
        Ok(PathBuf::from(raw))
    }
}

/// The rejection policy shared by both validation phases. Returns `Some(reason)` when
/// `path` violates a rule (the reason is a human-readable clause the caller wraps into
/// the error). `path` and `home` must be in the SAME form (both raw in phase 1, both
/// canonical in phase 2); the koma/sensitive roots are canonicalized-if-existing so the
/// comparison holds against a canonical `path` in phase 2.
fn policy_violation(path: &Path, home: &Path) -> Option<String> {
    // Strictly under $HOME: not $HOME itself, not outside it.
    if path == home {
        return Some("resolves to $HOME itself".to_string());
    }
    if !path.starts_with(home) {
        return Some("is not under $HOME".to_string());
    }
    // Never koma's own state tree (~/.koma).
    if let Ok(base) = store::base_dir() {
        if path.starts_with(norm(&base)) {
            return Some("is under koma's own ~/.koma tree".to_string());
        }
    }
    // Never a credential/secret store — reject the whole subtree.
    for name in [".ssh", ".aws", ".gnupg"] {
        if path.starts_with(norm(&home.join(name))) {
            return Some(format!("is under the sensitive directory ~/{name}"));
        }
    }
    // ~/.config: reject the ROOT itself, but allow XDG-style subdirs (~/.config/<ext>).
    if path == norm(&home.join(".config")).as_path() {
        return Some("resolves to ~/.config itself".to_string());
    }
    None
}

/// Canonicalize `p`, falling back to `p` verbatim when it can't be canonicalized (e.g.
/// it doesn't exist). Mirrors [`crate::app::harness`]'s `norm` — a comparable form for
/// path-boundary containment checks.
fn norm(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}
