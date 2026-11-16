//! The scrubber for room text. Every string of every room result and room
//! event passes through it: each configured secret value (provider keys, OAuth
//! tokens, sealed values) and the userinfo of every URL become `[redacted]`.
//! Nothing else is touched, so a credential a room found in its target and
//! quotes as evidence stays as written.
//!
//! The values come from `config.json`. Before each batch the file is stat-ed,
//! and the values are read again when its modification time, size or inode
//! changed, so a key added by another process after the bind is covered too.
use super::ring::RoomLine;
use crate::app::runtime::sdk::host::{config::ConfigStore, redact};
use serde_json::Value;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::SystemTime;

/// What identifies one version of `config.json` on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Stamp {
    modified: Option<SystemTime>,
    len: u64,
    inode: u64,
}

fn stamp_of(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    #[cfg(unix)]
    let inode = std::os::unix::fs::MetadataExt::ino(&meta);
    #[cfg(not(unix))]
    let inode = 0;
    Some(Stamp {
        modified: meta.modified().ok(),
        len: meta.len(),
        inode,
    })
}

#[derive(Debug, Default)]
pub struct Scrubber {
    /// `None` until the first successful read.
    read: Option<Option<Stamp>>,
    secrets: Vec<String>,
}

impl Scrubber {
    pub fn new() -> Self {
        Self::default()
    }

    /// Re-read the value set when `config.json` changed since the last read.
    /// An unreadable file keeps the previous set and is tried again next time.
    /// Blocking: it stats and may read the file.
    pub fn refresh(&mut self, store: &ConfigStore) {
        let Ok(path) = store.config_path() else {
            return;
        };
        let stamp = stamp_of(&path);
        if self.read.as_ref() == Some(&stamp) {
            return;
        }
        if let Ok(loaded) = store.read() {
            self.secrets = redact::secret_values(&loaded.config);
            // Reading may seal plain-text secrets and rewrite the file, so the
            // stamp is taken after the read.
            self.read = Some(stamp_of(&path));
        }
    }

    pub fn text(&self, text: &str) -> String {
        redact::scrub(text, &self.secrets)
    }

    /// Scrub every text field of a line.
    pub fn line(&self, line: &mut RoomLine) {
        for text in [&mut line.text, &mut line.args, &mut line.result]
            .into_iter()
            .flatten()
        {
            *text = self.text(text);
        }
        line.author = self.text(&line.author);
    }

    /// Scrub every string inside `value`, object keys included.
    pub fn value(&self, value: &mut Value) {
        match value {
            Value::String(text) => {
                let clean = self.text(text);
                if clean != *text {
                    *text = clean;
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|item| self.value(item)),
            Value::Object(map) => {
                let dirty = map.keys().any(|key| self.text(key) != *key);
                if dirty {
                    let taken = std::mem::take(map);
                    for (key, mut item) in taken {
                        self.value(&mut item);
                        map.insert(self.text(&key), item);
                    }
                } else {
                    map.values_mut().for_each(|item| self.value(item));
                }
            }
            _ => {}
        }
    }
}

/// One scrubber per adapter, shared by the room reader and the handlers.
pub type SharedScrubber = Arc<Mutex<Scrubber>>;

pub fn shared() -> SharedScrubber {
    Arc::new(Mutex::new(Scrubber::new()))
}

pub fn lock(scrub: &SharedScrubber) -> MutexGuard<'_, Scrubber> {
    scrub.lock().unwrap_or_else(|e| e.into_inner())
}

/// Refresh the value set on the blocking pool, then scrub `value` with it.
pub async fn scrub_value(scrub: &SharedScrubber, store: &ConfigStore, mut value: Value) -> Value {
    let shared = Arc::clone(scrub);
    let store = store.clone();
    let _ = tokio::task::spawn_blocking(move || lock(&shared).refresh(&store)).await;
    lock(scrub).value(&mut value);
    value
}

/// [`scrub_value`] for one string.
pub async fn scrub_text(scrub: &SharedScrubber, store: &ConfigStore, text: &str) -> String {
    match scrub_value(scrub, store, Value::String(text.to_string())).await {
        Value::String(text) => text,
        _ => String::new(),
    }
}
