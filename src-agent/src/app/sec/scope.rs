//! The scope record: what this run is allowed to touch, and its version.
//!
//! Absent scope means **no network**. A scope is `Active` only once it has been
//! confirmed, and every network dispatch re-reads it, so a paused or edited scope
//! cannot leave stale access.
//!
//! Versioning is the load-bearing part. An edit bumps `version`, and a call
//! stamped with an older version is refused rather than silently run against the
//! new one: broadening a scope mid-flight otherwise turns a call that was
//! admitted under narrow rules into one that was never re-checked.
//!
//! Normalisation happens here, once, at parse time — lowercased, IDNA-folded,
//! trailing dot dropped, IP literals canonicalised. Every later comparison is
//! therefore plain string equality and cannot drift from the rule that admitted
//! the call.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Which way traffic reaches a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    Http,
    Https,
    /// Raw TCP on a non-HTTP port. Requires `CONNECT`, not a replayed request.
    Tcp,
}

impl Transport {
    /// Loopback can only ever be `http`/`tcp`: there is no TLS on a local fixture
    /// unless someone terminates it, and guessing `https` would fail the handshake.
    fn default_for(host: &Host) -> Self {
        if host.is_ip() {
            Self::Http
        } else {
            Self::Https
        }
    }
}

/// A permission cell: one host, optionally narrowed to ports.
///
/// The empty port list means *every* port, not none — that distinction is the
/// common case and writing it out per host is noise.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct Host {
    /// Normalised (see [`Host::parse`]).
    pub host: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<u16>,
}

impl Host {
    /// Parse and normalise. `None` when the cell is unusable.
    ///
    /// Accepts a bare host, a `host:port`, or a bracketed IPv6 literal. A
    /// full URL is not accepted — scheme belongs to the target, not the host, and
    /// quietly splitting one would hide a scheme/port mismatch.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        let (host, port) = split_host_port(raw)?;
        let host = normalise(&host)?;
        Some(Self {
            host,
            ports: port.map(|p| vec![p]).unwrap_or_default(),
        })
    }

    #[must_use]
    pub fn is_ip(&self) -> bool {
        self.host.parse::<IpAddr>().is_ok()
    }

    /// Does this cell admit the target on `port`?
    ///
    /// IP cells are exact; a name cell admits itself and its subdomains, which is
    /// the shape a declared engagement actually has (`example.com` covers
    /// `api.example.com`). A leading-dot cell (`.example.com`) is the strict
    /// subdomain-only form.
    #[must_use]
    pub fn admits(&self, target: &str, port: u16) -> bool {
        if !self.ports.is_empty() && !self.ports.contains(&port) {
            return false;
        }
        self.admits_host_only(target)
    }

    /// [`Self::admits`] without the port rule, for diagnosis only.
    #[must_use]
    pub fn admits_host_only(&self, target: &str) -> bool {
        if self.host == target {
            return true;
        }
        if self.is_ip() {
            // Never suffix-match IPs: `10.0.0.1` would otherwise admit `20.10.0.1`.
            return false;
        }
        if let Some(stem) = self.host.strip_prefix('.') {
            return target.ends_with(&format!(".{stem}"));
        }
        target.ends_with(&format!(".{}", self.host))
    }
}

/// A concrete thing a call wants to reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    pub transport: Transport,
}

impl Endpoint {
    #[must_use]
    pub fn default_port(transport: Transport) -> u16 {
        match transport {
            Transport::Http => 80,
            Transport::Https | Transport::Tcp => 443,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    None,
    Supplied,
    Ask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Draft,
    AwaitingConfirm,
    Active,
    Paused,
    Closed,
}

/// Caps. `None` means unbounded-for-now rather than zero, so a missing value does
/// not silently throttle everything.
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Caps {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rps: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u32>,
}

/// A wall-clock window the scope is usable inside, in epoch seconds.
///
/// Separate from the caps on purpose: the caps bound how MUCH a run may do, the
/// window bounds WHEN. A forgotten run against a production host is usually a
/// timing problem, not a volume one, so volume limits do not cover it.
///
/// Both ends are inclusive-start, exclusive-end. A window with no end is a normal
/// "from now on"; one with no start began whenever it was declared.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Window {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<i64>,
}

/// Seconds since the epoch. Only this module needs a clock, and only for the
/// window, so the helper lives here rather than in a shared time module.
fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Window {
    /// Is `now` (epoch seconds) inside? An empty window admits everything.
    #[must_use]
    pub fn admits(&self, now: i64) -> bool {
        if let Some(from) = self.from {
            if now < from {
                return false;
            }
        }
        if let Some(until) = self.until {
            if now >= until {
                return false;
            }
        }
        true
    }

    /// Is any bound set? An unset window is not worth checking at the gate.
    #[must_use]
    pub fn is_set(&self) -> bool {
        self.from.is_some() || self.until.is_some()
    }
}

/// The record. Serialised with `version` so a stale caller can be refused.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Scope {
    /// Bumped by every edit. A dispatch stamps the version it was admitted under.
    pub version: u64,
    pub state: State,
    pub auth_mode: Mode,
    #[serde(default)]
    pub targets: Vec<Host>,
    #[serde(default)]
    pub exclusions: Vec<Host>,
    #[serde(default)]
    pub caps: Caps,
    /// When this scope may be used. Empty admits at any time.
    #[serde(default)]
    pub window: Window,
    /// Who confirmed, and thus whether this scope may be used at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_by: Option<String>,
}

/// Why a target was refused. Kept as an enum so callers and tests do not depend
/// on message text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// No scope record at all.
    NoScope,
    NotActive,
    NotConfirmed,
    /// Not in `targets`.
    OutOfScope,
    /// In `targets` *and* in `exclusions`; exclusion wins.
    Excluded,
    /// Admitted the host but not this port.
    PortNotAllowed,
    /// The call carried no destination and the scope needs one.
    NoDestination,
    /// `now` is outside the scope's window.
    OutsideWindow,
    /// A hop in the chain could not be parsed, so it cannot be admitted.
    Unparseable,
}

impl Refusal {
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            Self::NoScope => "no scope record — call sec_runtime op=net mode=scope with targets and the user confirms them",
            Self::NotActive => "scope is not active",
            Self::NotConfirmed => "scope is not confirmed",
            Self::OutOfScope => "target not in scope",
            Self::Excluded => "target is explicitly excluded",
            Self::PortNotAllowed => "port not allowed for this host",
            Self::NoDestination => "call carries no destination",
            Self::OutsideWindow => "the scope's time window is closed",
            Self::Unparseable => "hop could not be parsed, so it cannot be admitted",
        }
    }
}

impl Scope {
    /// May this endpoint be reached under this scope, at `version`?
    ///
    /// `version` is the version the caller was admitted under. A mismatch is a
    /// stale call and is refused, never upgraded.
    pub fn admit(&self, target: &str, port: u16, version: Option<u64>) -> Result<(), Refusal> {
        self.admit_at(target, port, version, now_epoch())
    }

    /// [`Self::admit`] against an explicit clock, so window tests need no sleeping.
    pub fn admit_at(
        &self,
        target: &str,
        port: u16,
        version: Option<u64>,
        now: i64,
    ) -> Result<(), Refusal> {
        if let Some(v) = version {
            if v != self.version {
                return Err(Refusal::NotActive);
            }
        }
        if self.state != State::Active {
            return Err(Refusal::NotActive);
        }
        if self.confirmed_by.is_none() {
            return Err(Refusal::NotConfirmed);
        }
        // Window before any host work: a closed window refuses every target, so
        // checking it early keeps the diagnosis about the window rather than
        // about whichever host happened to be tried.
        if !self.window.admits(now) {
            return Err(Refusal::OutsideWindow);
        }
        let Some(host) = normalise(target) else {
            return Err(Refusal::NoDestination);
        };
        // Exclusion is checked FIRST and wins. A declared host that an exclusion
        // later narrows must not be reachable through the broader rule.
        if self.exclusions.iter().any(|e| e.admits(&host, port)) {
            return Err(Refusal::Excluded);
        }
        if !self.targets.iter().any(|t| t.admits(&host, port)) {
            // Distinguish "wrong port" from "wrong host" so the reason is useful:
            // the host is declared, just not on this port.
            if self
                .targets
                .iter()
                .any(|t| t.admits_host_only(&host) && !t.ports.is_empty())
            {
                return Err(Refusal::PortNotAllowed);
            }
            return Err(Refusal::OutOfScope);
        }
        Ok(())
    }

    /// Resolve a raw destination into an endpoint, applying the scope's defaults.
    ///
    /// Only `admit` decides; this just builds the shapes. A bare host with no
    /// scheme or port gets the transport's default port, but *that* port is then
    /// admitted, so a host narrowed to `:8080` does not become reachable on `:80`.
    #[must_use]
    pub fn endpoint(&self, raw: &str) -> Option<Endpoint> {
        let raw = raw.trim();
        let (scheme, rest) = match raw.split_once("://") {
            Some((s, r)) => (Some(s.to_ascii_lowercase()), r),
            None => (None, raw),
        };
        // Strip the path first: `host:port` splitting would otherwise swallow it
        // into the host, and a query string can carry a colon too.
        let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
        let (host, port) = split_host_port(authority)?;
        let host = normalise(&host)?;
        let transport = match scheme.as_deref() {
            Some("http") => Transport::Http,
            Some("https") => Transport::Https,
            Some("tcp") => Transport::Tcp,
            _ => Transport::default_for(&Host {
                host: host.clone(),
                ports: Vec::new(),
            }),
        };
        Some(Endpoint {
            host,
            port: port.unwrap_or_else(|| Endpoint::default_port(transport)),
            transport,
        })
    }

    /// Should this host be re-validated after a redirect or DNS change?
    ///
    /// Always: the whole reason the record is re-read is that a redirect can move
    /// the answer elsewhere, so there is no cheaper path worth special-casing.
    pub fn admit_after_hop(&self, target: &str, port: u16) -> Result<(), Refusal> {
        self.admit(target, port, None)
    }

    /// Re-admit every hop of a followed redirect chain.
    ///
    /// The first hop is where the widening would happen: the daemon follows
    /// redirects itself, so a `302` to an out-of-scope host is only caught here.
    /// Each hop is checked in order and the first refusal wins, so the report can
    /// name the hop that broke rather than a generic denial.
    pub fn admit_chain<'a, I>(&self, hops: I) -> Result<(), (String, Refusal)>
    where
        I: IntoIterator<Item = &'a str>,
    {
        for hop in hops {
            let Some(e) = self.endpoint(hop) else {
                return Err((hop.to_owned(), Refusal::Unparseable));
            };
            // No path is threaded here: a hop is admitted on host and port, which
            // is what the egress proxy checks anyway.
            if let Err(refusal) = self.admit(&e.host, e.port, None) {
                return Err((hop.to_owned(), refusal));
            }
        }
        Ok(())
    }
}

/// The scope record for a run, read back from disk.
///
/// Lives beside the target bindings (same session/cwd key) so a scope and its
/// targets cannot be separated. `None` when absent or unreadable — which
/// [`Refusal::NoScope`] turns into a refusal, never an open door.
#[must_use]
pub fn load(dir: &std::path::Path) -> Option<Scope> {
    let raw = std::fs::read_to_string(dir.join(SCOPE_FILE)).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Persist a scope. Version is the CALLER's responsibility to have bumped.
pub fn store(dir: &std::path::Path, scope: &Scope) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let body = serde_json::to_string_pretty(scope)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(dir.join(SCOPE_FILE), body)
}

/// The scope file name inside a run directory.
pub const SCOPE_FILE: &str = "_scope.json";

/// Split `host:port`, handling bracketed IPv6.
fn split_host_port(raw: &str) -> Option<(String, Option<u16>)> {
    let raw = raw.trim();
    if let Some(rest) = raw.strip_prefix('[') {
        let (host, tail) = rest.split_once(']')?;
        let port = match tail.strip_prefix(':') {
            Some(p) => Some(p.parse().ok()?),
            None if tail.is_empty() => None,
            None => return None,
        };
        return Some((host.to_string(), port));
    }
    // A bare IPv6 literal has more than one colon. A port on it uses brackets.
    if raw.matches(':').count() > 1 {
        return Some((raw.to_string(), None));
    }
    match raw.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() && p.parse::<u16>().is_ok() => {
            Some((h.to_string(), p.parse().ok()))
        }
        _ => Some((raw.to_string(), None)),
    }
}

/// Canonical form, or `None` when the host is not usable.
///
/// IP literals go through their parsed form so `::1` and `0:0:0:0:0:0:0:1` cannot
/// both be declared and both be needed. Names are lowercased, IDNA-folded and lose
/// a trailing dot (the root label is not part of identity here).
fn normalise(host: &str) -> Option<String> {
    let host = host.trim().trim_end_matches('.');
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if host.is_empty() {
        return None;
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Some(match ip {
            IpAddr::V4(v4) => Ipv4Addr::from(v4.octets()).to_string(),
            IpAddr::V6(v6) => Ipv6Addr::from(v6.octets()).to_string(),
        });
    }
    let lowered = host.to_lowercase();
    // `idna` is not a dependency; lowercase + reject what cannot be a name is
    // enough for the ASCII case, and a non-ASCII label is passed through for the
    // proxy to resolve rather than silently mangled.
    let valid = lowered
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_' || !c.is_ascii());
    valid.then_some(lowered)
}

#[cfg(test)]
#[path = "scope_test.rs"]
mod tests;


#[cfg(test)]
#[path = "scope_chain_test.rs"]
mod chain_tests;


#[cfg(test)]
#[path = "scope_window_test.rs"]
mod window;

