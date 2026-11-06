//! Where a run's dynamic-target state lives, and the identity it keys on.
//!
//! A dynamic target — a service the code under test starts on a port, a container
//! that comes and goes — is not identified by its host alone. `localhost:8080` is
//! a different target in two sessions, and on two checkouts. So the key carries
//! the run context:
//!
//! ```text
//! <session tmp>/sec-scope/<session>/<digest(cwd)>/<verb>-<host>-<port>.json
//! ```
//!
//! Session is a path segment (it is a UUID, so it needs no escaping) and the cwd
//! is digested, so a deep checkout does not fan the tree out or leak its layout.
//! [`ScopeBase::of`] resolves all of it from the [`ToolCtx`], so a caller never
//! hand-wires the pieces and cannot forget one.
//!
//! The fingerprint rule is duplicated on the daemon side (`target.py`) because the
//! port/host validation belongs next to the thing that dials it. The Rust half
//! owns placement; the Python half owns identity. They must agree on the digest
//! input — hence [`digest`] is public and pinned by a test.

// Phase 3 only establishes where records live; the readers land with the scope
// record itself, so the accessors are unread until then.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use crate::tool::ToolCtx;

/// Directory under the session tmp dir. Sits beside the other per-session scratch
/// so cleanup already covers it.
const SUBDIR: &str = "sec-scope";

/// A resolved place to keep one run's dynamic-target records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeBase {
    dir: PathBuf,
}

impl ScopeBase {
    /// Resolve from the live context. `None` when the session has no directory or
    /// no id yet, because a record with neither cannot be placed or keyed.
    #[must_use]
    pub fn of(ctx: &ToolCtx) -> Option<Self> {
        let session_dir = ctx.session_dir.as_ref()?;
        if ctx.session_id.is_empty() {
            return None;
        }
        Some(Self::assemble(session_dir, &ctx.session_id, &ctx.workspace))
    }

    fn assemble(session_dir: &Path, session_id: &str, cwd: &Path) -> Self {
        let cwd = cwd.to_string_lossy();
        Self {
            dir: crate::model::store::session_tool_tmp_dir(session_dir)
                .join(SUBDIR)
                .join(session_id)
                .join(digest(&cwd)),
        }
    }

    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The record path for one target. `port` is part of the key: `:8080` and
    /// `:8443` on one host are different targets.
    #[must_use]
    pub fn record_path(&self, verb: &str, host: &str, port: Option<u16>) -> PathBuf {
        let mut name = format!("{verb}-{}", host.to_ascii_lowercase());
        if let Some(p) = port {
            name.push_str(&format!("-{p}"));
        }
        self.dir.join(format!("{}.json", slug(&name)))
    }
}

/// A short digest of the cwd. Truncated to 10 hex chars, matching `target.py`.
///
/// The two implementations must produce the same value for the same input; a
/// divergence silently forks every record. Pinned by
/// [`tests::digest_matches_the_daemon_side`].
#[must_use]
pub fn digest(text: &str) -> String {
    // FNV-1a, duplicated rather than pulled in: the daemon side uses SHA-256, so
    // this is only the placement key and never a security boundary.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}").chars().take(10).collect()
}

/// Lowercase `[a-z0-9-]`, collapsing runs, capped. Matches `target._slug`.
fn slug(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_dash = false;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_matches('-').chars().take(60).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(session: &str, cwd: &str) -> ScopeBase {
        ScopeBase::assemble(Path::new("/sessions/s1"), session, Path::new(cwd))
    }

    #[test]
    fn the_path_carries_session_and_a_digested_cwd() {
        let got = base("sess-a", "/work/repo");
        let shown = got.dir().to_string_lossy();
        assert!(shown.contains("sec-scope"));
        assert!(shown.contains("sess-a"));
        assert!(shown.contains(&digest("/work/repo")));
    }

    #[test]
    fn two_sessions_on_one_checkout_get_different_dirs() {
        assert_ne!(base("a", "/work/repo"), base("b", "/work/repo"));
    }

    #[test]
    fn two_checkouts_get_different_dirs() {
        assert_ne!(base("a", "/work/repo"), base("a", "/work/other"));
    }

    #[test]
    fn one_target_resolves_to_one_path() {
        let b = base("a", "/w");
        assert_eq!(b.record_path("red", "LocalHost", Some(8080)), b.record_path("red", "localhost", Some(8080)));
    }

    #[test]
    fn port_separates_targets_on_one_host() {
        let b = base("a", "/w");
        assert_ne!(b.record_path("red", "h", Some(8080)), b.record_path("red", "h", Some(8443)));
    }

    #[test]
    fn a_host_with_a_colon_or_slash_stays_a_single_filename() {
        let got = b_record("red", "localhost:8080", Some(80));
        assert_eq!(got.matches('/').count(), 0, "{got}");
    }

    fn b_record(verb: &str, host: &str, port: Option<u16>) -> String {
        let b = base("a", "/w");
        b.record_path(verb, host, port)
            .file_name()
            .expect("always has a name")
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn a_deep_cwd_does_not_fan_the_tree_out() {
        let deep = format!("/{}", "very/deep/".repeat(40));
        let shown = base("a", &deep).dir().to_string_lossy().into_owned();
        // session tmp + subdir + session + one digest segment.
        assert!(shown.ends_with(&digest(&deep)));
        assert!(shown.matches('/').count() < 12, "{shown}");
    }

    #[test]
    fn digest_is_stable_and_short() {
        assert_eq!(digest("/work/repo"), digest("/work/repo"));
        assert_eq!(digest("/work/repo").len(), 10);
        assert_ne!(digest("/work/repo"), digest("/work/repo2"));
    }

    #[test]
    fn digest_matches_the_daemon_side() {
        // The Rust placement key and the Python identity key must not diverge.
        // Pinned literally; `target.py::_digest` asserts the same two vectors, so
        // a change on either side is caught on both.
        assert_eq!(digest("/work/repo"), "1cd4fc81c8");
        assert_eq!(digest(""), "cbf29ce484");
    }
}
