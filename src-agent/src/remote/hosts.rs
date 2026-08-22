//! Remote host persistence: saved SSH hosts in `~/.koma/remote-hosts.json`.
//!
//! Each [`RemoteHost`] is a user-defined SSH target (name, user, host, port,
//! optional key) that can be reused across sessions. The JSON file is
//! atomically written and loaded on demand.

use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// A saved remote SSH host.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteHost {
    /// UUID — generated on add, stable identity.
    pub id: String,
    /// User-friendly label (e.g. "prod-server").
    pub name: String,
    /// SSH user.
    pub user: String,
    /// Hostname or IP.
    pub host: String,
    /// SSH port (default 22).
    pub port: u16,
    /// Optional key file path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_path: Option<String>,
    /// Unix timestamp of last successful connection (seconds since epoch).
    /// `None` if never connected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_connected: Option<u64>,
    /// Optional grouping tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl RemoteHost {
    /// Build a `user@host:port` display string.
    pub fn address(&self) -> String {
        if self.port == 22 {
            format!("{}@{}", self.user, self.host)
        } else {
            format!("{}@{}:{}", self.user, self.host, self.port)
        }
    }

    /// Record the current time as the last-connected timestamp.
    pub fn touch_last_connected(&mut self) {
        self.last_connected = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs());
    }
}

/// Persisted collection of remote hosts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RemoteHosts {
    pub hosts: Vec<RemoteHost>,
}

/// Resolve the path to `~/.koma/remote-hosts.json`.
fn hosts_path() -> Result<PathBuf> {
    let base = crate::model::store::base_dir()?;
    Ok(base.join("remote-hosts.json"))
}

/// Load hosts from disk. Returns an empty list if the file doesn't exist or
/// is malformed.
pub fn load_hosts() -> RemoteHosts {
    let path = match hosts_path() {
        Ok(p) => p,
        Err(_) => return RemoteHosts::default(),
    };
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => return RemoteHosts::default(),
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Save hosts to disk atomically (write-to-temp then rename).
pub fn save_hosts(hosts: &RemoteHosts) -> Result<()> {
    let path = hosts_path()?;
    let json = serde_json::to_vec_pretty(hosts).context("serialise remote hosts")?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &json).context("write remote hosts tmp")?;
    std::fs::rename(&tmp, &path).context("rename remote hosts tmp")?;
    Ok(())
}

/// Find a host by its UUID.
pub fn host_by_id<'a>(hosts: &'a RemoteHosts, id: &str) -> Option<&'a RemoteHost> {
    hosts.hosts.iter().find(|h| h.id == id)
}

/// Insert a new host or update an existing one (matched by `id`).
///
/// Returns the index of the host in the list after upsert.
pub fn upsert_host(hosts: &mut RemoteHosts, host: RemoteHost) -> usize {
    if let Some(idx) = hosts.hosts.iter().position(|h| h.id == host.id) {
        hosts.hosts[idx] = host;
        idx
    } else {
        hosts.hosts.push(host);
        hosts.hosts.len() - 1
    }
}

/// Remove a host by its UUID. Returns `true` if found and removed.
pub fn delete_host(hosts: &mut RemoteHosts, id: &str) -> bool {
    let before = hosts.hosts.len();
    hosts.hosts.retain(|h| h.id != id);
    hosts.hosts.len() < before
}

/// Flush a parsed SSH config block into an imported host entry.
/// Pending host data being parsed from SSH config.
struct PendingHost {
    alias: Option<String>,
    user: Option<String>,
    hostname: Option<String>,
    port: u16,
    key_path: Option<String>,
}

impl PendingHost {
    fn new() -> Self {
        Self {
            alias: None,
            user: None,
            hostname: None,
            port: 22,
            key_path: None,
        }
    }
}

fn flush_host(
    pending: &mut PendingHost,
    existing_hosts: &std::collections::HashSet<&str>,
    existing_names: &std::collections::HashSet<String>,
    imported: &mut Vec<RemoteHost>,
) {
    let alias = pending.alias.take();
    let user = pending.user.take();
    let hostname = pending.hostname.take();
    let port = pending.port;
    let key_path = pending.key_path.take();
    pending.port = 22;

    if let (Some(alias), Some(user), Some(hostname)) = (alias, user, hostname) {
        if !existing_hosts.contains(hostname.as_str())
            && !existing_names.contains(&alias.to_lowercase())
        {
            imported.push(RemoteHost {
                id: crate::model::app_config::new_uuid(),
                name: alias,
                user,
                host: hostname,
                port,
                key_path,
                last_connected: None,
                tags: vec!["imported".into()],
            });
        }
    }
}

/// Parse `~/.ssh/config` and return any host entries not already present in
/// the saved hosts (by hostname match). This is read-only — imported hosts
/// are suggestions, not synced back.
///
/// Each SSH config `Host` block with `HostName` + `User` produces one entry.
/// Blocks without `User` are skipped (we need a user to connect).
pub fn import_ssh_config(hosts: &RemoteHosts) -> Vec<RemoteHost> {
    let ssh_config = match dirs::home_dir() {
        Some(home) => home.join(".ssh").join("config"),
        None => return Vec::new(),
    };
    let text = match std::fs::read_to_string(&ssh_config) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };

    parse_ssh_config_text(&text, hosts)
}

/// Parse SSH config text and return imported hosts not already present.
fn parse_ssh_config_text(text: &str, hosts: &RemoteHosts) -> Vec<RemoteHost> {
    let existing_hosts: std::collections::HashSet<&str> =
        hosts.hosts.iter().map(|h| h.host.as_str()).collect();
    let existing_names: std::collections::HashSet<String> =
        hosts.hosts.iter().map(|h| h.name.to_lowercase()).collect();

    let mut imported = Vec::new();
    let mut pending = PendingHost::new();

    for line in text.lines() {
        let trimmed = line.trim();

        // Blank line or comment → flush the current block.
        if trimmed.is_empty() || trimmed.starts_with('#') {
            flush_host(
                &mut pending,
                &existing_hosts,
                &existing_names,
                &mut imported,
            );
            continue;
        }

        // Parse keyword lines (case-insensitive keyword, value after first whitespace).
        let (key, value) = match trimmed.split_once(char::is_whitespace) {
            Some((k, v)) => (k.to_lowercase(), v.trim().to_string()),
            None => continue,
        };

        match key.as_str() {
            "host" => {
                // Flush previous block.
                flush_host(
                    &mut pending,
                    &existing_hosts,
                    &existing_names,
                    &mut imported,
                );

                // Skip wildcard/negated patterns.
                if value.starts_with('*') || value.starts_with('!') {
                    pending.alias = None;
                } else {
                    pending.alias = Some(value);
                }
            }
            "user" => pending.user = Some(value),
            "hostname" => pending.hostname = Some(value),
            "port" => {
                if let Ok(p) = value.parse::<u16>() {
                    pending.port = p;
                }
            }
            "identityfile" if pending.alias.is_some() && pending.key_path.is_none() => {
                let path = value.trim_matches(|c| c == '"' || c == '\'');
                // Expand leading ~/ with home_dir
                let expanded = if let Some(rest) = path.strip_prefix("~/") {
                    if let Some(home) = dirs::home_dir() {
                        home.join(rest).to_string_lossy().into_owned()
                    } else {
                        path.to_string()
                    }
                } else {
                    path.to_string()
                };
                pending.key_path = Some(expanded);
            }
            _ => {}
        }
    }

    // Flush the final block.
    flush_host(
        &mut pending,
        &existing_hosts,
        &existing_names,
        &mut imported,
    );

    imported
}

#[cfg(test)]
#[path = "hosts_test.rs"]
mod tests;
