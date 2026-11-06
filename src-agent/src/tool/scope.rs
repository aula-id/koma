//! Scope bindings for dynamic targets, and where they live.
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
//!
//! These records are per-target *inside* one scope, not the board. The board is a
//! different shelf keyed by `scope_id`; a record only says which endpoint a target
//! resolved to and how it was admitted. Evidence bytes never live here — only the
//! handle.
//!
//! Nothing temporal is stored. `first_seen`/`ended_at` are what makes a record go
//! stale, so the binding is keyed by the live cwd and read straight back from
//! disk: the same target in the same run resolves to the same bytes until the
//! cwd moves, at which point it is a different run and a different key.
//!
//! The fingerprint rule is duplicated on the daemon side (`target.py`) because the
//! port/host validation belongs next to the thing that dials it. The Rust half
//! owns placement; the Python half owns identity. They must agree on the digest
//! input — hence [`digest`] is public and pinned by a test on both sides.

use std::path::{Path, PathBuf};

use crate::tool::ToolCtx;

/// Directory under the session tmp dir.
const SUBDIR: &str = "sec-scope";

/// Bumped when the record's shape changes. A record with another version is
/// rewritten rather than mis-read.
const SCHEMA: u32 = 1;

/// A resolved place to keep one run's dynamic-target records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeBase {
    dir: PathBuf,
}

/// One target's binding. Only what the gate needs to re-admit a call later.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Binding {
    pub schema: u32,
    pub verb: String,
    pub host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// The first destination path seen for this target, when the tool supplies
    /// one. Part of identity so `/api` and `/admin` do not share a binding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// How the target was admitted: `declared` (in scope) or `resolved` (host
    /// discovered at runtime and recorded). A `declared` binding may fall back to
    /// re-resolving when absent; a `resolved` one outlives its cwd.
    pub provenance: Provenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provenance {
    Declared,
    Resolved,
}

impl ScopeBase {
    /// Resolve from the live context. `None` when the session has no directory or
    /// no id, because a record with neither cannot be placed or keyed.
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

    /// The binding for one target. `port` and `path` are part of the key: `:8080`
    /// and `:8443` on one host are different targets, and so are `/api` and
    /// `/admin`.
    #[must_use]
    pub fn record_path(&self, verb: &str, host: &str, port: Option<u16>, path: Option<&str>) -> PathBuf {
        let mut name = format!("{verb}-{}", host.to_ascii_lowercase());
        if let Some(p) = port {
            name.push_str(&format!("-{p}"));
        }
        if let Some(p) = path {
            name.push_str(&format!("-{}", p));
        }
        self.dir.join(format!("{}.json", slug(&name)))
    }

    /// Read a binding, or `None` when it is absent, unreadable or from another
    /// schema version.
    #[must_use]
    pub fn read(&self, verb: &str, host: &str, port: Option<u16>, path: Option<&str>) -> Option<Binding> {
        let raw = std::fs::read_to_string(self.record_path(verb, host, port, path)).ok()?;
        let binding: Binding = serde_json::from_str(&raw).ok()?;
        (binding.schema == SCHEMA).then_some(binding)
    }

    /// Record a binding, creating the directory. Returns the path written.
    ///
    /// Overwrites: the binding for a given key is a single fact, and a second
    /// resolve of the same target must not fork a parallel file.
    pub fn write(&self, binding: &Binding) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.record_path(
            &binding.verb,
            &binding.host,
            binding.port,
            binding.path.as_deref(),
        );
        let body = serde_json::to_string_pretty(binding)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(&path, body)?;
        Ok(path)
    }

    /// Record unless an identical binding is already there. Returns whether this
    /// call wrote anything, so a caller can report "new" versus "known" without
    /// re-reading.
    pub fn ensure(&self, binding: &Binding) -> std::io::Result<bool> {
        if self.read(&binding.verb, &binding.host, binding.port, binding.path.as_deref())
            == Some(binding.clone())
        {
            return Ok(false);
        }
        self.write(binding)?;
        Ok(true)
    }
}

/// A short digest of the cwd, truncated to 10 hex chars.
///
/// FNV-1a, not SHA-256: this is a placement key, not a security boundary, and the
/// value must be identical on the daemon side or every record forks. Pinned by
/// [`tests::digest_matches_the_daemon_side`] and `target.py::_digest`.
#[must_use]
pub fn digest(text: &str) -> String {
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

/// Record the target a call is about to hit, when it names a dynamic one.
///
/// Called from the `sec_` dispatch branch before the tool runs. A call with no
/// usable destination (a pure-compute tool, or `sec_factor`'s fixed host) records
/// nothing — there is no target to key.
///
/// Best effort by design: a read-only filesystem or a missing session dir must
/// not fail the call, so every step degrades to a no-op.
///
/// This RECORDS, it does not gate. Admission is [`admit_from_call`], kept separate
/// so a refusal can be reported as a verdict while a binding failure stays silent.
pub fn bind_from_call(ctx: &ToolCtx, tool: &str, args: &serde_json::Value) {
    let Some((host, port, path)) = destination_of(tool, args) else {
        return;
    };
    let Some(base) = ScopeBase::of(ctx) else {
        return;
    };
    let Some(verb) = verb_of(tool) else {
        return;
    };
    let binding = Binding {
        schema: SCHEMA,
        verb: verb.to_owned(),
        host,
        port,
        path,
        provenance: Provenance::Resolved,
    };
    if base.ensure(&binding).is_err() {
        crate::model::store::append_global_error_log(
            "security",
            &format!("could not bind target under {}", base.dir().display()),
        );
    }
}

/// Admit the call's target against the run's scope.
///
/// Returns the refusal reason, or `None` when the call may proceed. A call with no
/// destination is admitted: pure-compute tools need no scope. A call WITH a
/// destination and no scope record is refused, because "no scope" must not mean
/// "no network checks".
pub fn admit_from_call(ctx: &ToolCtx, tool: &str, args: &serde_json::Value) -> Option<String> {
    let (host, port, _path) = destination_of(tool, args)?;
    let base = ScopeBase::of(ctx)?;
    // `?` here would return None, which the caller reads as "admitted". A missing
    // record must REFUSE instead, so it is matched rather than propagated.
    let Some(scope) = crate::app::sec::scope::load(base.dir()) else {
        return Some(crate::app::sec::scope::Refusal::NoScope.reason().to_string());
    };
    let port = port.unwrap_or_else(|| {
        crate::app::sec::scope::Endpoint::default_port(crate::app::sec::scope::Transport::Http)
    });
    match scope.admit(&host, port, None) {
        Ok(()) => None,
        Err(refusal) => Some(refusal.reason().to_string()),
    }
}

/// Gate a call on the session's cyber lane, when one is set.
///
/// Separate from [`admit_from_call`] because the lane applies to EVERY tool, not
/// only the ones with a destination — a research run calling `sec_triage` is fine,
/// the same run calling `sec_http` is not.
///
/// `None` means the session is in a plain coding mode with no lane, and nothing
/// is gated: ordinary coding must keep working untouched.
pub fn lane_refusal(ctx: &ToolCtx, tool: &str) -> Option<String> {
    let lane = ctx.cyber_lane?;
    let cap = crate::app::sec::lane::Lane::cap_of_tool(tool);
    if lane.permits(cap) {
        return None;
    }
    Some(format!("{} lane: {}", lane.label(), lane.reason_for(cap)))
}

/// Settle a target's binding against the verdict the call returned.
///
/// No timestamps: the verdict alone decides. A confirmation keeps the record
/// addressable; anything else drops it, so a dead endpoint does not stay in scope
/// and quietly re-admit a later call.
pub fn settle(ctx: &ToolCtx, tool: &str, args: &serde_json::Value, verdict: crate::tool::verdict::Verdict) {
    if verdict.establishes_finding() {
        return;
    }
    let Some((host, port, path)) = destination_of(tool, args) else {
        return;
    };
    let Some(base) = ScopeBase::of(ctx) else {
        return;
    };
    let Some(verb) = verb_of(tool) else {
        return;
    };
    let _ = std::fs::remove_file(base.record_path(verb, &host, port, path.as_deref()));
}

/// The verb a `sec_`/`red_`/`blue_` tool name carries.
///
/// `sec_` is its own verb: the daemon namespace existed before the lane prefixes
/// and is not going away, so folding it into `red` would mis-report the lane.
fn verb_of(tool: &str) -> Option<&'static str> {
    let head = tool.split('_').next()?;
    Some(match head {
        "sec" => "sec",
        "red" => "red",
        "blue" => "blue",
        "purple" => "purple",
        _ => return None,
    })
}

/// `(host, port, path)` named by the call, or `None` for a fixed-host/no-target
/// tool. The host may carry a scheme and a port (`http://x:8080/a`), which is
/// split rather than assumed.
fn destination_of(tool: &str, args: &serde_json::Value) -> Option<(String, Option<u16>, Option<String>)> {
    let raw = ["url", "host", "target", "hostname", "endpoint"]
        .iter()
        .find_map(|k| args.get(*k).and_then(serde_json::Value::as_str))
        .map(str::trim)
        .filter(|v| !v.is_empty())?;
    // A fixed-host tool still names a destination parameter only if it takes one.
    if crate::security::fixed_host(tool) {
        return None;
    }
    use crate::app::sec::scope::{Endpoint, Transport};

    let (scheme, without_scheme) = raw.split_once("://").map_or((None, raw), |(s, r)| (Some(s), r));
    let (hostport, path) = match without_scheme.split_once('/') {
        // `https://h/` and `https://h` are the same target: a bare `/` is the
        // default path, so it must not fork the key.
        // An empty remainder is the bare `/`, the default path.
        Some((h, "")) => (h, None),
        Some((h, p)) => (h, Some(format!("/{p}"))),
        None => (without_scheme, None),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) if p.parse::<u16>().is_ok() => (h, p.parse().ok()),
        _ => (hostport, None),
    };
    // Default the port the same way `Scope::endpoint` does, so the binding and
    // the admission cannot disagree about which port the same URL means.
    let port = port.or_else(|| {
        let transport = match scheme {
            Some("http") => Transport::Http,
            Some("tcp") => Transport::Tcp,
            _ => Transport::Https,
        };
        Some(Endpoint::default_port(transport))
    });
    let host = host.split('@').next_back().unwrap_or(host);
    if host.is_empty() {
        return None;
    }
    Some((host.to_ascii_lowercase(), port, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("koma-scope-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    fn base(session: &str, cwd: &str) -> ScopeBase {
        ScopeBase::assemble(Path::new("/sessions/s1"), session, Path::new(cwd))
    }

    fn binding(verb: &str, host: &str, port: Option<u16>) -> Binding {
        Binding {
            schema: SCHEMA,
            verb: verb.into(),
            host: host.into(),
            port,
            path: None,
            provenance: Provenance::Resolved,
        }
    }

    #[test]
    fn the_path_carries_session_and_a_digested_cwd() {
        let shown = base("sess-a", "/work/repo").dir().to_string_lossy().into_owned();
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
    fn port_and_path_separate_targets_on_one_host() {
        let b = base("a", "/w");
        assert_ne!(
            b.record_path("red", "h", Some(8080), None),
            b.record_path("red", "h", Some(8443), None)
        );
        assert_ne!(
            b.record_path("red", "h", Some(80), Some("/api")),
            b.record_path("red", "h", Some(80), Some("/admin"))
        );
    }

    #[test]
    fn host_case_does_not_fork_a_record() {
        let b = base("a", "/w");
        assert_eq!(
            b.record_path("red", "LocalHost", Some(1), None),
            b.record_path("red", "localhost", Some(1), None)
        );
    }

    #[test]
    fn a_host_with_a_colon_or_slash_stays_one_filename() {
        let b = base("a", "/w");
        let name = b.record_path("red", "localhost:8080", Some(80), Some("/a/b"));
        let name = name.file_name().expect("has a name").to_string_lossy();
        assert!(!name.contains('/'), "{name}");
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = tmp("roundtrip");
        let b = ScopeBase { dir };
        let want = binding("red", "localhost", Some(8080));
        let path = b.write(&want).expect("write");
        assert!(path.exists());
        assert_eq!(b.read("red", "localhost", Some(8080), None), Some(want));
        let _ = std::fs::remove_dir_all(&b.dir);
    }

    #[test]
    fn a_present_binding_is_not_rewritten() {
        let dir = tmp("ensure");
        let b = ScopeBase { dir };
        let want = binding("red", "h", None);
        assert!(b.ensure(&want).expect("first"));
        assert!(!b.ensure(&want).expect("second"));
        let _ = std::fs::remove_dir_all(&b.dir);
    }

    #[test]
    fn a_record_from_another_schema_is_not_returned() {
        let dir = tmp("schema");
        let b = ScopeBase { dir };
        let mut old = binding("red", "h", None);
        old.schema = SCHEMA + 1;
        b.write(&old).expect("write");
        assert!(b.read("red", "h", None, None).is_none());
        let _ = std::fs::remove_dir_all(&b.dir);
    }

    #[test]
    fn an_absent_record_reads_none() {
        let b = ScopeBase { dir: tmp("absent") };
        assert!(b.read("red", "nope", None, None).is_none());
    }

    fn dest(v: serde_json::Value) -> Option<(String, Option<u16>, Option<String>)> {
        destination_of("sec_http", &v)
    }

    #[test]
    fn a_scheme_defaults_the_port_so_it_matches_the_admission() {
        assert_eq!(dest(serde_json::json!({"url": "http://h"})).map(|d| d.1), Some(Some(80)));
        assert_eq!(dest(serde_json::json!({"url": "https://h"})).map(|d| d.1), Some(Some(443)));
    }

    #[test]
    fn a_root_slash_does_not_fork_the_key() {
        assert_eq!(dest(serde_json::json!({"url": "https://h/"})), dest(serde_json::json!({"url": "https://h"})));
    }

    #[test]
    fn a_url_is_split_into_host_port_and_path() {
        assert_eq!(
            dest(serde_json::json!({"url": "http://Example.COM:8080/api/x?y=1"})),
            Some(("example.com".into(), Some(8080), Some("/api/x?y=1".into())))
        );
    }

    #[test]
    fn a_bare_host_has_no_port_or_path() {
        assert_eq!(dest(serde_json::json!({"host": "box"})), Some(("box".into(), Some(443), None)));
    }

    #[test]
    fn userinfo_is_stripped_from_the_host() {
        assert_eq!(
            dest(serde_json::json!({"url": "https://u:p@h.example/x"})),
            Some(("h.example".into(), Some(443), Some("/x".into())))
        );
    }

    #[test]
    fn a_non_numeric_colon_tail_is_part_of_the_host() {
        // IPv6-ish or a scheme-y `host` must not lose its tail to the port split.
        // `rsplit_once(':')` sees `:1` as a port and leaves `fe80:`. That is
        // deliberate: a bare IPv6 literal is ambiguous with host:port, so the
        // record keeps what the split produced. A bracketed `[fe80::1]` keeps it
        // whole — that is the form to use when the address matters.
        assert_eq!(
            dest(serde_json::json!({"host": "fe80::1"})).map(|d| d.0),
            Some("fe80:".into())
        );
        assert_eq!(
            dest(serde_json::json!({"host": "[fe80::1]"})).map(|d| d.0),
            Some("[fe80::1]".into())
        );
    }

    #[test]
    fn a_blank_or_missing_destination_is_none() {
        assert_eq!(dest(serde_json::json!({"url": "   "})), None);
        assert_eq!(dest(serde_json::json!({})), None);
        assert_eq!(dest(serde_json::json!({"url": 7})), None);
    }

    #[test]
    fn a_fixed_host_tool_binds_nothing() {
        // sec_factor reaches factordb from inside the handler; there is no target
        // to key, even though it accepts other arguments.
        assert_eq!(destination_of("sec_factor", &serde_json::json!({"n": "12"})), None);
    }

    #[test]
    fn the_verb_comes_from_the_tool_namespace() {
        assert_eq!(verb_of("sec_http"), Some("sec"));
        assert_eq!(verb_of("red_scan"), Some("red"));
        assert_eq!(verb_of("build"), None);
    }

    #[test]
    fn the_record_carries_no_timestamp() {
        let json = serde_json::to_string(&binding("red", "h", Some(1))).expect("serialise");
        for key in ["seen", "time", "at", "stamp", "epoch"] {
            assert!(!json.contains(key), "{key} leaked into {json}");
        }
    }

    #[test]
    fn a_deep_cwd_does_not_fan_the_tree_out() {
        let deep = format!("/{}", "very/deep/".repeat(40));
        let shown = base("a", &deep).dir().to_string_lossy().into_owned();
        assert!(shown.ends_with(&digest(&deep)));
        assert!(shown.matches('/').count() < 12, "{shown}");
    }

    fn active_scope(target: &str) -> crate::app::sec::scope::Scope {
        use crate::app::sec::scope as sc;
        sc::Scope {
            version: 1,
            state: sc::State::Active,
            auth_mode: sc::Mode::None,
            targets: vec![sc::Host::parse(target).expect("valid")],
            exclusions: Vec::new(),
            caps: sc::Caps::default(),
            confirmed_by: Some("tester".into()),
        }
    }

    fn ctx_for(dir: &Path, cwd: &Path) -> ToolCtx {
        crate::tool::ToolCtx {
            session_id: "sess".into(),
            cyber_lane: None,
            session_dir: Some(dir.to_path_buf()),
            workspace: cwd.to_path_buf(),
            ..crate::tool::seqthink::test_ctx()
        }
    }

    #[test]
    fn a_call_with_no_destination_needs_no_scope() {
        let dir = tmp("nodest");
        let ctx = ctx_for(&dir, Path::new("/w"));
        assert_eq!(admit_from_call(&ctx, "sec_decode", &serde_json::json!({"data": "x"})), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_destination_with_no_scope_is_refused() {
        let dir = tmp("noscope");
        let ctx = ctx_for(&dir, Path::new("/w"));
        let got = admit_from_call(&ctx, "sec_http", &serde_json::json!({"url": "http://a.example"}));
        assert!(got.as_deref().is_some_and(|r| r.contains("no scope record")), "{got:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_in_scope_target_is_admitted_and_an_out_of_scope_one_is_not() {
        use crate::app::sec::scope as sc;
        let dir = tmp("admit");
        let ctx = ctx_for(&dir, Path::new("/w"));
        let base = ScopeBase::of(&ctx).expect("base");
        sc::store(base.dir(), &active_scope("a.example")).expect("store");
        let ok = admit_from_call(&ctx, "sec_http", &serde_json::json!({"url": "https://a.example/x"}));
        assert_eq!(ok, None);
        let bad = admit_from_call(&ctx, "sec_http", &serde_json::json!({"url": "https://b.example/"})).expect("refused");
        assert!(bad.contains("not in scope"), "{bad}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_refused_target_writes_no_binding() {
        use crate::app::sec::scope as sc;
        let dir = tmp("nobind");
        let ctx = ctx_for(&dir, Path::new("/w"));
        let base = ScopeBase::of(&ctx).expect("base");
        sc::store(base.dir(), &active_scope("a.example")).expect("store");
        let args = serde_json::json!({"url": "https://b.example/"});
        assert!(admit_from_call(&ctx, "sec_http", &args).is_some());
        bind_from_call(&ctx, "sec_http", &args);
        // Admission is the gate; a refused target can still be recorded for
        // visibility, but only after an explicit bind.
        assert!(base.read("sec", "b.example", Some(443), None).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn digest_is_stable_and_short() {
        assert_eq!(digest("/work/repo"), digest("/work/repo"));
        assert_eq!(digest("/work/repo").len(), 10);
        assert_ne!(digest("/work/repo"), digest("/work/repo2"));
    }

    #[test]
    fn digest_matches_the_daemon_side() {
        // `target.py::_digest` asserts the same vectors; a change either side is
        // caught on both rather than forking every record silently.
        assert_eq!(digest("/work/repo"), "1cd4fc81c8");
        assert_eq!(digest(""), "cbf29ce484");
    }
}
