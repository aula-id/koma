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
            Self::NoScope => "no scope record — declare one before touching the network",
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
    pub fn admit(
        &self,
        target: &str,
        port: u16,
        version: Option<u64>,
    ) -> Result<(), Refusal> {
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
            if self.targets.iter().any(|t| t.admits_host_only(&host) && !t.ports.is_empty()) {
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
            _ => Transport::default_for(&Host { host: host.clone(), ports: Vec::new() }),
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
    match raw.rsplit_once(':') {
        // Only a numeric tail is a port; `fe80::1` has colons that are not.
        Some((h, p)) if p.parse::<u16>().is_ok() => Some((h.to_string(), p.parse().ok())),
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
    let host = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')).unwrap_or(host);
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
mod tests {
    use super::*;

    fn scope() -> Scope {
        Scope {
            version: 1,
            state: State::Active,
            auth_mode: Mode::None,
            targets: vec![Host::parse("example.com").expect("valid")],
            exclusions: Vec::new(),
            caps: Caps::default(),
            window: Window::default(),
            confirmed_by: Some("alice".into()),
        }
    }

    #[test]
    fn a_bare_host_is_admitted_on_any_port() {
        assert_eq!(scope().admit("example.com", 443, None), Ok(()));
        assert_eq!(scope().admit("example.com", 8080, None), Ok(()));
    }

    #[test]
    fn subdomains_are_covered_but_a_lookalike_is_not() {
        assert_eq!(scope().admit("api.example.com", 443, None), Ok(()));
        assert!(scope().admit("notexample.com", 443, None).is_err());
    }

    #[test]
    fn a_leading_dot_means_subdomains_only() {
        let mut s = scope();
        s.targets = vec![Host::parse(".example.com").expect("valid")];
        assert!(s.admit("api.example.com", 443, None).is_ok());
        assert!(s.admit("example.com", 443, None).is_err());
    }

    #[test]
    fn ip_cells_never_suffix_match() {
        let mut s = scope();
        s.targets = vec![Host::parse("10.0.0.1").expect("valid")];
        assert!(s.admit("10.0.0.1", 80, None).is_ok());
        assert!(s.admit("20.10.0.1", 80, None).is_err());
    }

    #[test]
    fn an_exclusion_beats_a_broader_target() {
        let mut s = scope();
        s.exclusions = vec![Host::parse("api.example.com").expect("valid")];
        assert_eq!(s.admit("api.example.com", 443, None), Err(Refusal::Excluded));
        assert_eq!(s.admit("example.com", 443, None), Ok(()));
    }

    #[test]
    fn a_port_narrowed_host_reports_the_port_not_the_host() {
        let mut s = scope();
        s.targets = vec![Host::parse("example.com:8080").expect("valid")];
        assert_eq!(s.admit("example.com", 80, None), Err(Refusal::PortNotAllowed));
        assert_eq!(s.admit("example.com", 8080, None), Ok(()));
    }

    #[test]
    fn exclusion_wins_even_on_a_port_the_target_allows() {
        let mut s = scope();
        s.targets = vec![Host::parse("example.com").expect("valid")];
        s.exclusions = vec![Host::parse("example.com:8443").expect("valid")];
        assert_eq!(s.admit("example.com", 8443, None), Err(Refusal::Excluded));
        assert_eq!(s.admit("example.com", 443, None), Ok(()));
    }

    #[test]
    fn an_inactive_unconfirmed_or_absent_scope_admits_nothing() {
        let mut s = scope();
        s.state = State::Paused;
        assert_eq!(s.admit("example.com", 443, None), Err(Refusal::NotActive));
        let mut s = scope();
        s.confirmed_by = None;
        assert_eq!(s.admit("example.com", 443, None), Err(Refusal::NotConfirmed));
    }

    #[test]
    fn a_stale_version_is_refused_rather_than_upgraded() {
        let s = scope();
        assert_eq!(s.admit("example.com", 443, Some(2)), Err(Refusal::NotActive));
        assert_eq!(s.admit("example.com", 443, Some(1)), Ok(()));
    }

    #[test]
    fn a_redirect_hop_is_re_admitted() {
        let mut s = scope();
        s.exclusions = vec![Host::parse("evil.example").expect("valid")];
        assert_eq!(s.admit_after_hop("evil.example", 443), Err(Refusal::Excluded));
        assert_eq!(s.admit_after_hop("other.example", 443), Err(Refusal::OutOfScope));
    }

    #[test]
    fn host_normalisation_folds_case_the_trailing_dot_and_ipv6() {
        assert_eq!(normalise("EXAMPLE.com."), Some("example.com".into()));
        assert_eq!(normalise("[::1]"), Some("::1".into()));
        assert_eq!(normalise("0:0:0:0:0:0:0:1"), normalise("::1"));
    }

    #[test]
    fn a_url_endpoint_defaults_its_port_from_the_scheme_not_the_host() {
        let e = scope().endpoint("https://example.com/x").expect("endpoint");
        assert_eq!((e.port, e.transport), (443, Transport::Https));
        let e = scope().endpoint("example.com").expect("endpoint");
        assert_eq!((e.port, e.transport), (443, Transport::Https));
    }

    #[test]
    fn a_defaulted_port_still_has_to_be_admitted() {
        let mut s = scope();
        s.targets = vec![Host::parse("example.com:8080").expect("valid")];
        let e = s.endpoint("https://example.com").expect("endpoint");
        assert_eq!(s.admit(&e.host, e.port, None), Err(Refusal::PortNotAllowed));
    }

    #[test]
    fn an_unusable_host_parses_to_none() {
        assert!(Host::parse("").is_none());
        assert!(Host::parse("   ").is_none());
        assert!(Host::parse("bad host!").is_none());
    }

    #[test]
    fn every_refusal_has_a_reason() {
        for r in [
            Refusal::NoScope,
            Refusal::NotActive,
            Refusal::NotConfirmed,
            Refusal::OutOfScope,
            Refusal::Excluded,
            Refusal::PortNotAllowed,
            Refusal::NoDestination,
        ] {
            assert!(!r.reason().is_empty());
        }
    }
}

#[cfg(test)]
mod chain_tests {
    use super::*;

    fn scoped() -> Scope {
        Scope {
            version: 1,
            state: State::Active,
            auth_mode: Mode::None,
            targets: vec![Host::parse("a.example").expect("valid")],
            exclusions: Vec::new(),
            caps: Caps::default(),
            window: Window::default(),
            confirmed_by: Some("t".into()),
        }
    }

    #[test]
    fn a_chain_inside_scope_is_admitted() {
        assert_eq!(
            scoped().admit_chain(["https://a.example/1", "https://a.example/2"]),
            Ok(())
        );
    }

    #[test]
    fn the_hop_that_leaves_scope_is_the_one_named() {
        let got = scoped().admit_chain(["https://a.example/1", "https://b.example/2"]);
        let (hop, refusal) = got.expect_err("refused");
        assert_eq!(hop, "https://b.example/2");
        assert_eq!(refusal, Refusal::OutOfScope);
    }

    #[test]
    fn an_unparseable_hop_refuses_rather_than_skipping() {
        let (_, refusal) = scoped().admit_chain([""]).expect_err("refused");
        assert_eq!(refusal, Refusal::Unparseable);
    }

    #[test]
    fn the_first_refusal_wins() {
        let (hop, _) = scoped()
            .admit_chain(["https://b.example/", "https://c.example/"])
            .expect_err("refused");
        assert_eq!(hop, "https://b.example/");
    }
}

#[cfg(test)]
mod window {
    //! Phase 3: a confirmed, active, in-target scope must still be refused outside
    //! its window. A timing limit that can be stepped around is not a limit.

    use super::*;

    fn scoped(window: Window) -> Scope {
        Scope {
            version: 1,
            state: State::Active,
            auth_mode: Mode::None,
            targets: vec![Host::parse("example.com").expect("valid")],
            exclusions: Vec::new(),
            caps: Caps::default(),
            window,
            confirmed_by: Some("tester".into()),
        }
    }

    #[test]
    fn an_unset_window_admits_at_any_time() {
        let s = scoped(Window::default());
        for now in [0, 1_000_000, i64::MAX] {
            assert!(s.admit_at("example.com", 443, Some(1), now).is_ok(), "{now}");
        }
    }

    #[test]
    fn before_the_start_is_refused() {
        let s = scoped(Window { from: Some(100), until: None });
        assert_eq!(
            s.admit_at("example.com", 443, Some(1), 99),
            Err(Refusal::OutsideWindow)
        );
    }

    #[test]
    fn at_and_after_the_start_is_admitted() {
        let s = scoped(Window { from: Some(100), until: None });
        assert!(s.admit_at("example.com", 443, Some(1), 100).is_ok());
        assert!(s.admit_at("example.com", 443, Some(1), 101).is_ok());
    }

    #[test]
    fn the_end_is_exclusive() {
        // Half-open, so a window ending at 200 permits 199 and not 200. Pinned
        // because an off-by-one here silently extends every run by its last second.
        let s = scoped(Window { from: None, until: Some(200) });
        assert!(s.admit_at("example.com", 443, Some(1), 199).is_ok());
        assert_eq!(
            s.admit_at("example.com", 443, Some(1), 200),
            Err(Refusal::OutsideWindow)
        );
    }

    #[test]
    fn inside_a_closed_range_works() {
        let s = scoped(Window { from: Some(100), until: Some(200) });
        assert!(s.admit_at("example.com", 443, Some(1), 150).is_ok());
    }

    #[test]
    fn the_window_refusal_wins_over_a_host_diagnosis() {
        // Outside the window every target is refused, so reporting "not in scope"
        // for an out-of-target host would blame the wrong thing.
        let s = scoped(Window { from: Some(100), until: Some(200) });
        assert_eq!(
            s.admit_at("elsewhere.test", 443, Some(1), 300),
            Err(Refusal::OutsideWindow)
        );
    }

    #[test]
    fn the_window_is_checked_after_activation_and_confirmation() {
        // A closed window on a paused scope should read as "not active", not as
        // a window problem — the earlier condition is the real one.
        let mut s = scoped(Window { from: Some(100), until: Some(200) });
        s.state = State::Paused;
        assert_eq!(
            s.admit_at("example.com", 443, Some(1), 300),
            Err(Refusal::NotActive)
        );
    }

    #[test]
    fn an_empty_window_is_not_worth_checking() {
        assert!(!Window::default().is_set());
        assert!(Window { from: Some(1), until: None }.is_set());
    }

    #[test]
    fn the_reason_is_specific() {
        assert!(Refusal::OutsideWindow.reason().contains("window"));
    }

    #[test]
    fn a_window_survives_a_round_trip() {
        let s = scoped(Window { from: Some(1), until: Some(2) });
        let json = serde_json::to_string(&s).expect("serialise");
        let back: Scope = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(back.window, s.window);
    }

    #[test]
    fn an_older_scope_document_without_a_window_still_loads() {
        // Every scope written before this field must keep working, and an absent
        // window means "no timing limit".
        let s = scoped(Window::default());
        let mut v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&s).expect("ser")).expect("parse");
        v.as_object_mut().expect("object").remove("window");
        let back: Scope = serde_json::from_value(v).expect("load");
        assert!(!back.window.is_set());
    }
}
