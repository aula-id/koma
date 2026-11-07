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
    pub fn record_path(
        &self,
        verb: &str,
        host: &str,
        port: Option<u16>,
        path: Option<&str>,
    ) -> PathBuf {
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
    pub fn read(
        &self,
        verb: &str,
        host: &str,
        port: Option<u16>,
        path: Option<&str>,
    ) -> Option<Binding> {
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
        if self.read(
            &binding.verb,
            &binding.host,
            binding.port,
            binding.path.as_deref(),
        ) == Some(binding.clone())
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
        return Some(
            crate::app::sec::scope::Refusal::NoScope
                .reason()
                .to_string(),
        );
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
/// `None` means the call may proceed. A `sec_` call with no lane is refused.
/// Ordinary tools stay ungated.
pub fn lane_refusal(ctx: &ToolCtx, tool: &str, args: &mut serde_json::Value) -> Option<String> {
    use crate::app::sec::lane::Lane;
    if crate::security::is_sec_tool(tool) && ctx.cyber_lane.is_none() {
        return Some(
            ctx.sec_block
                .clone()
                .unwrap_or_else(|| "no cyber lane is selected for this session".to_string()),
        );
    }
    if ctx.sec_inactive.iter().any(|n| n == tool) {
        return Some(format!("{tool} is disabled for this session"));
    }
    let lane = ctx.cyber_lane?;
    if crate::security::is_sec_tool(tool) && !crate::app::sec::lane::surface(lane).contains(&tool) {
        return Some(format!("{} lane: {tool} is not on this lane", lane.label()));
    }
    if lane == Lane::Blue && Lane::blue_blocks(tool) {
        return Some(format!("blue lane: {tool} is refused"));
    }
    let cap = Lane::cap_of_tool(tool);
    if !lane.permits(cap) {
        return Some(format!("{} lane: {}", lane.label(), lane.reason_for(cap)));
    }
    if lane == Lane::Blue && tool == "sec_http" {
        return blue_http(args);
    }
    None
}

/// A Blue `sec_http` call is a bare `GET` or `HEAD`. Anything that can carry a
/// payload is refused, and redirects are forced off before the call runs.
fn blue_http(args: &mut serde_json::Value) -> Option<String> {
    let Some(obj) = args.as_object() else {
        return Some("blue lane: sec_http arguments must be an object".into());
    };
    for key in obj.keys() {
        if !matches!(
            key.as_str(),
            "url" | "method" | "headers" | "body" | "follow_redirects"
        ) {
            return Some(format!("blue lane: sec_http refuses {key}"));
        }
    }
    match obj.get("url").and_then(|v| v.as_str()) {
        Some(url) if !url.is_empty() && !url.contains(['?', '#', '@']) => {}
        _ => return Some("blue lane: sec_http requires a bare url".into()),
    }
    if let Some(method) = obj.get("method") {
        match method.as_str() {
            Some(method)
                if method.eq_ignore_ascii_case("GET") || method.eq_ignore_ascii_case("HEAD") => {}
            _ => return Some("blue lane: sec_http method must be GET or HEAD".into()),
        }
    }
    if let Some(headers) = obj.get("headers") {
        if !headers.as_object().is_some_and(|h| h.is_empty()) {
            return Some("blue lane: sec_http refuses headers".into());
        }
    }
    if let Some(body) = obj.get("body") {
        let empty = body.is_null() || body.as_str().is_some_and(|s| s.is_empty());
        if !empty {
            return Some("blue lane: sec_http refuses a body".into());
        }
    }
    if obj
        .get("follow_redirects")
        .is_some_and(|v| v.as_bool() != Some(false))
    {
        return Some("blue lane: sec_http does not follow redirects".into());
    }
    if let Some(obj) = args.as_object_mut() {
        obj.insert(
            "follow_redirects".to_string(),
            serde_json::Value::Bool(false),
        );
    }
    None
}

/// Settle a target's binding against the verdict the call returned.
///
/// No timestamps: the verdict alone decides. A confirmation keeps the record
/// addressable; anything else drops it, so a dead endpoint does not stay in scope
/// and quietly re-admit a later call.
pub fn settle(
    ctx: &ToolCtx,
    tool: &str,
    args: &serde_json::Value,
    verdict: crate::tool::verdict::Verdict,
) {
    if verdict.establishes_finding() {
        bind_from_call(ctx, tool, args);
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
fn destination_of(
    tool: &str,
    args: &serde_json::Value,
) -> Option<(String, Option<u16>, Option<String>)> {
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

    let (scheme, without_scheme) = raw
        .split_once("://")
        .map_or((None, raw), |(s, r)| (Some(s), r));
    let without_scheme = without_scheme
        .split(['?', '#'])
        .next()
        .unwrap_or(without_scheme);
    let (hostport, path) = match without_scheme.split_once('/') {
        // `https://h/` and `https://h` are the same target: a bare `/` is the
        // default path, so it must not fork the key.
        // An empty remainder is the bare `/`, the default path.
        Some((h, "")) => (h, None),
        Some((h, p)) => (h, Some(format!("/{p}"))),
        None => (without_scheme, None),
    };
    let (host, port) = if hostport.matches(':').count() > 1 && !hostport.starts_with('[') {
        (hostport, None)
    } else if let Some(rest) = hostport.strip_prefix('[') {
        let (host, tail) = rest.split_once(']')?;
        let port = tail.strip_prefix(':').and_then(|p| p.parse().ok());
        (host, port)
    } else {
        match hostport.rsplit_once(':') {
            Some((h, p)) if p.parse::<u16>().is_ok() => (h, p.parse().ok()),
            _ => (hostport, None),
        }
    };
    // Default the port the same way `Scope::endpoint` does, so the binding and
    // the admission cannot disagree about which port the same URL means.
    let port = port.or_else(|| {
        let transport = match scheme {
            Some("http") => Transport::Http,
            Some("tcp") => Transport::Tcp,
            None if host.parse::<std::net::IpAddr>().is_ok() => Transport::Http,
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
#[path = "scope_test.rs"]
mod tests;


/// Phase 1: the gate must not depend on the model telling the truth.
#[cfg(test)]
#[path = "scope_forged_test.rs"]
mod forged;

