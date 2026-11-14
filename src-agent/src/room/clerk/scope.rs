//! Is a host covered by the scope records on the shelf?
//!
//! The matching is deliberately strict, because a false "in scope" hides a real
//! violation while a false "out of scope" only costs the Chief a glance:
//!
//! - an entry matches that exact host;
//! - `*.example.com` matches any subdomain of `example.com` at any depth, but
//!   not `example.com` itself;
//! - `10.0.0.0/24` matches an IPv4 address inside the range;
//! - a deny entry always wins over an allow entry.
//!
//! A scope record's `host` counts as an allow entry. Nothing else is implied:
//! allowing `example.com` does not allow `app.example.com`.

use serde_json::Value;

use crate::room::shelf::ShelfEntry;

/// One scope record from the shelf.
#[derive(Debug, Clone, PartialEq)]
pub struct Scope {
    pub key: String,
    pub allow: Vec<String>,
    pub deny: Vec<String>,
}

/// Read the `scope/` records. Rows that are not scope records are skipped.
pub fn parse_scopes(entries: &[ShelfEntry]) -> Vec<Scope> {
    entries
        .iter()
        .filter_map(|entry| {
            let body: Value = serde_json::from_str(entry.content.as_deref()?).ok()?;
            if body.get("kind").and_then(Value::as_str) != Some("scope") {
                return None;
            }
            let list = |name: &str| -> Vec<String> {
                body.get(name)
                    .and_then(Value::as_array)
                    .map(|items| items.iter().filter_map(|v| v.as_str().map(normalize_host)).collect())
                    .unwrap_or_default()
            };
            let mut allow = list("allow");
            if let Some(host) = body.get("host").and_then(Value::as_str) {
                allow.push(normalize_host(host));
            }
            Some(Scope { key: entry.key.clone(), allow, deny: list("deny") })
        })
        .collect()
}

/// Reduce a URL or `host:port` to a bare lowercase host. Wildcards and CIDR
/// ranges pass through unchanged.
pub fn normalize_host(raw: &str) -> String {
    let mut s = raw.trim().to_lowercase();
    if let Some(i) = s.find("://") {
        s = s[i + 3..].to_string();
    }
    if let Some(i) = s.find(['/', '?', '#']) {
        // keep a CIDR suffix (`10.0.0.0/24`); a path starts with a non-digit
        let rest = &s[i + 1..];
        let is_cidr = s.as_bytes()[i] == b'/' && !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit());
        if !is_cidr {
            s.truncate(i);
        }
    }
    if let Some(i) = s.rfind('@') {
        s = s[i + 1..].to_string();
    }
    if !s.starts_with('[') {
        if let Some(i) = s.rfind(':') {
            if s[i + 1..].bytes().all(|b| b.is_ascii_digit()) && !s[..i].contains(':') {
                s.truncate(i);
            }
        }
    }
    s.trim_end_matches('.').to_string()
}

fn ipv4(s: &str) -> Option<u32> {
    let mut parts = s.split('.');
    let mut value: u32 = 0;
    for _ in 0..4 {
        let octet: u8 = parts.next()?.parse().ok()?;
        value = (value << 8) | u32::from(octet);
    }
    parts.next().is_none().then_some(value)
}

fn matches(pattern: &str, host: &str) -> bool {
    if pattern == host {
        return true;
    }
    if let Some(apex) = pattern.strip_prefix("*.") {
        return host.len() > apex.len() + 1 && host.ends_with(apex) && host[..host.len() - apex.len()].ends_with('.');
    }
    if let Some((net, bits)) = pattern.split_once('/') {
        if let (Some(base), Some(addr), Ok(bits)) = (ipv4(net), ipv4(host), bits.parse::<u32>()) {
            if bits <= 32 {
                let mask = if bits == 0 { 0 } else { u32::MAX << (32 - bits) };
                return base & mask == addr & mask;
            }
        }
    }
    false
}

/// True when `host` is allowed by some scope record and denied by none.
/// With no records at all nothing is in scope.
pub fn in_scope(host: &str, scopes: &[Scope]) -> bool {
    let host = normalize_host(host);
    if host.is_empty() {
        return false;
    }
    let denied = scopes.iter().flat_map(|s| &s.deny).any(|p| matches(p, &host));
    let allowed = scopes.iter().flat_map(|s| &s.allow).any(|p| matches(p, &host));
    allowed && !denied
}
