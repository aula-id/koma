//! Remote host persistence: saved SSH hosts in `~/.koma/remote-hosts.json`.
//!
//! Each [`RemoteHost`] is a user-defined SSH target (name, user, host, port,
//! optional key) that can be reused across sessions. The JSON file is
//! atomically written and loaded on demand.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

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

    /// Whether this host was connected to recently (within `threshold`).
    #[allow(dead_code)]
    pub fn is_recently_connected(&self, threshold: Duration) -> bool {
        let Some(ts) = self.last_connected else {
            return false;
        };
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        now.saturating_sub(ts) <= threshold.as_secs()
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

/// Find a host by its label (case-insensitive).
#[allow(dead_code)]
pub fn host_by_name<'a>(hosts: &'a RemoteHosts, name: &str) -> Option<&'a RemoteHost> {
    let name_lc = name.to_lowercase();
    hosts
        .hosts
        .iter()
        .find(|h| h.name.to_lowercase() == name_lc)
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

    let existing_hosts: std::collections::HashSet<&str> =
        hosts.hosts.iter().map(|h| h.host.as_str()).collect();
    let existing_names: std::collections::HashSet<String> =
        hosts.hosts.iter().map(|h| h.name.to_lowercase()).collect();

    let mut imported = Vec::new();
    let mut current_host: Option<String> = None;
    let mut current_user: Option<String> = None;
    let mut current_hostname: Option<String> = None;
    let mut current_port: u16 = 22;

    for line in text.lines() {
        let trimmed = line.trim();

        // Blank line or comment → flush the current block.
        if trimmed.is_empty() || trimmed.starts_with('#') {
            if let (Some(alias), Some(user), Some(hostname)) =
                (current_host.take(), current_user.take(), current_hostname.take())
            {
                if !existing_hosts.contains(hostname.as_str())
                    && !existing_names.contains(&alias.to_lowercase())
                {
                    imported.push(RemoteHost {
                        id: crate::model::app_config::new_uuid(),
                        name: alias,
                        user,
                        host: hostname,
                        port: current_port,
                        key_path: None,
                        last_connected: None,
                        tags: vec!["imported".into()],
                    });
                }
                current_port = 22;
            }
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
                if let (Some(alias), Some(user), Some(hostname)) =
                    (current_host.take(), current_user.take(), current_hostname.take())
                {
                    if !existing_hosts.contains(hostname.as_str())
                        && !existing_names.contains(&alias.to_lowercase())
                    {
                        imported.push(RemoteHost {
                            id: crate::model::app_config::new_uuid(),
                            name: alias,
                            user,
                            host: hostname,
                            port: current_port,
                            key_path: None,
                            last_connected: None,
                            tags: vec!["imported".into()],
                        });
                    }
                    current_port = 22;
                }
                // Skip wildcard/negated patterns.
                if value.starts_with('*') || value.starts_with('!') {
                    current_host = None;
                } else {
                    current_host = Some(value);
                }
            }
            "user" => current_user = Some(value),
            "hostname" => current_hostname = Some(value),
            "port" => {
                if let Ok(p) = value.parse::<u16>() {
                    current_port = p;
                }
            }
            "identityfile" if current_host.is_some() => {
                // Store for the CURRENT block (best-effort — if no block
                // is active, ignore).
                // We can't set key_path here because the RemoteHost
                // isn't built yet; stash it alongside. For simplicity
                // in v1 we'll handle this in the flush above.
                // TODO: carry key_path through the block parser
            }
            _ => {}
        }
    }

    // Flush the final block.
    if let (Some(alias), Some(user), Some(hostname)) =
        (current_host, current_user, current_hostname)
    {
        if !existing_hosts.contains(hostname.as_str())
            && !existing_names.contains(&alias.to_lowercase())
        {
            imported.push(RemoteHost {
                id: crate::model::app_config::new_uuid(),
                name: alias,
                user,
                host: hostname,
                port: current_port,
                key_path: None,
                last_connected: None,
                tags: vec!["imported".into()],
            });
        }
    }

    imported
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn upsert_inserts_new_host() {
        let mut hosts = RemoteHosts::default();
        let host = RemoteHost {
            id: "test-id".into(),
            name: "test".into(),
            user: "root".into(),
            host: "10.0.0.1".into(),
            port: 22,
            key_path: None,
            last_connected: None,
            tags: vec![],
        };
        let idx = upsert_host(&mut hosts, host);
        assert_eq!(idx, 0);
        assert_eq!(hosts.hosts.len(), 1);
    }

    #[test]
    fn upsert_updates_existing() {
        let mut hosts = RemoteHosts::default();
        let host = RemoteHost {
            id: "same-id".into(),
            name: "v1".into(),
            user: "root".into(),
            host: "10.0.0.1".into(),
            port: 22,
            key_path: None,
            last_connected: None,
            tags: vec![],
        };
        upsert_host(&mut hosts, host);

        let updated = RemoteHost {
            id: "same-id".into(),
            name: "v2".into(),
            user: "admin".into(),
            host: "10.0.0.2".into(),
            port: 2222,
            key_path: None,
            last_connected: None,
            tags: vec![],
        };
        let idx = upsert_host(&mut hosts, updated);
        assert_eq!(idx, 0);
        assert_eq!(hosts.hosts.len(), 1);
        assert_eq!(hosts.hosts[0].name, "v2");
        assert_eq!(hosts.hosts[0].port, 2222);
    }

    #[test]
    fn delete_removes_host() {
        let mut hosts = RemoteHosts::default();
        hosts.hosts.push(RemoteHost {
            id: "to-delete".into(),
            name: "del".into(),
            user: "root".into(),
            host: "10.0.0.1".into(),
            port: 22,
            key_path: None,
            last_connected: None,
            tags: vec![],
        });
        assert!(delete_host(&mut hosts, "to-delete"));
        assert!(hosts.hosts.is_empty());
        assert!(!delete_host(&mut hosts, "nonexistent"));
    }

    #[test]
    fn host_by_id_finds() {
        let mut hosts = RemoteHosts::default();
        hosts.hosts.push(RemoteHost {
            id: "find-me".into(),
            name: "found".into(),
            user: "root".into(),
            host: "10.0.0.1".into(),
            port: 22,
            key_path: None,
            last_connected: None,
            tags: vec![],
        });
        assert!(host_by_id(&hosts, "find-me").is_some());
        assert!(host_by_id(&hosts, "nope").is_none());
    }

    #[test]
    fn host_address_format() {
        let host = RemoteHost {
            id: "x".into(),
            name: "x".into(),
            user: "ubuntu".into(),
            host: "example.com".into(),
            port: 22,
            key_path: None,
            last_connected: None,
            tags: vec![],
        };
        assert_eq!(host.address(), "ubuntu@example.com");

        let host2 = RemoteHost {
            port: 2222,
            ..host
        };
        assert_eq!(host2.address(), "ubuntu@example.com:2222");
    }

    #[test]
    fn serde_roundtrip() {
        let hosts = RemoteHosts {
            hosts: vec![RemoteHost {
                id: "rt".into(),
                name: "test".into(),
                user: "root".into(),
                host: "10.0.0.1".into(),
                port: 22,
                key_path: Some("/home/user/.ssh/id_ed25519".into()),
                last_connected: Some(1700000000),
                tags: vec!["prod".into()],
            }],
        };
        let json = serde_json::to_vec_pretty(&hosts).unwrap();
        let back: RemoteHosts = serde_json::from_slice(&json).unwrap();
        assert_eq!(back.hosts.len(), 1);
        assert_eq!(back.hosts[0].id, "rt");
        assert_eq!(back.hosts[0].key_path, Some("/home/user/.ssh/id_ed25519".into()));
    }
}
