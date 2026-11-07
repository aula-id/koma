//! Sticky security working set — survives chat compaction.
//!
//! Fat `sec_catalog` payloads may be spilled or summarized away. This file is
//! rewritten from disk into the volatile prompt tail every turn so the model keeps
//! a small pin list (names + blurbs only, never full schemas).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Max pinned concrete tools in the sticky block.
pub const MAX_PINS: usize = 16;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Pin {
    pub name: String,
    #[serde(default)]
    pub blurb: String,
    #[serde(default)]
    pub effect: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct LastRun {
    pub name: String,
    #[serde(default)]
    pub verdict: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct WorkingSet {
    #[serde(default)]
    pub lane: String,
    #[serde(default)]
    pub pinned: Vec<Pin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run: Option<LastRun>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub last_categories: Vec<String>,
}

const FILE: &str = "sec_working_set.json";

fn path(session_dir: &Path) -> PathBuf {
    session_dir.join(FILE)
}

/// Load from the session directory, or empty if missing/corrupt.
pub fn load(session_dir: &Path) -> WorkingSet {
    let p = path(session_dir);
    let Ok(raw) = std::fs::read_to_string(&p) else {
        return WorkingSet::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

/// Atomic-ish write of the working set.
pub fn store(session_dir: &Path, ws: &WorkingSet) -> std::io::Result<()> {
    std::fs::create_dir_all(session_dir)?;
    let p = path(session_dir);
    let tmp = session_dir.join(format!("{FILE}.tmp"));
    let body = serde_json::to_vec_pretty(ws).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, p)?;
    Ok(())
}

impl WorkingSet {
    /// Pin or bump a tool (LRU: most recent at end). Caps at [`MAX_PINS`].
    pub fn pin(&mut self, name: &str, blurb: &str, effect: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        self.pinned.retain(|p| p.name != name);
        self.pinned.push(Pin {
            name: name.into(),
            blurb: blurb.chars().take(120).collect(),
            effect: effect.into(),
        });
        while self.pinned.len() > MAX_PINS {
            self.pinned.remove(0);
        }
    }

    pub fn set_last_run(&mut self, name: &str, verdict: &str) {
        self.last_run = Some(LastRun {
            name: name.into(),
            verdict: verdict.into(),
        });
    }

    pub fn set_lane(&mut self, lane: &str) {
        self.lane = lane.into();
    }

    pub fn remember_categories(&mut self, cats: &[String]) {
        self.last_categories = cats.iter().take(12).cloned().collect();
    }

    /// Clear pins (lane change / new engagement).
    pub fn clear_pins(&mut self) {
        self.pinned.clear();
        self.last_run = None;
    }
}

/// Volatile-tail block for the model. Empty string if nothing to show.
pub fn context_block(session_dir: &Path) -> Option<String> {
    let ws = load(session_dir);
    if ws.pinned.is_empty() && ws.last_run.is_none() && ws.last_categories.is_empty() {
        return None;
    }
    let mut out = String::from(
        "# Security working set\n\
Pins survive compaction; full schemas do not. Rediscover with sec_catalog; run with sec_run.\n",
    );
    if !ws.lane.is_empty() {
        out.push_str(&format!("lane: {}\n", ws.lane));
    }
    if !ws.pinned.is_empty() {
        out.push_str("pinned:\n");
        for p in &ws.pinned {
            let effect = if p.effect.is_empty() {
                String::new()
            } else {
                format!(" [{}]", p.effect)
            };
            let blurb = if p.blurb.is_empty() {
                String::new()
            } else {
                format!(" — {}", p.blurb)
            };
            out.push_str(&format!("- {}{}{}\n", p.name, effect, blurb));
        }
    }
    if let Some(lr) = &ws.last_run {
        out.push_str(&format!("last_run: {} → {}\n", lr.name, lr.verdict));
    }
    if !ws.last_categories.is_empty() {
        out.push_str(&format!("last_categories: {}\n", ws.last_categories.join(", ")));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_is_lru_and_capped() {
        let mut ws = WorkingSet::default();
        for i in 0..20 {
            ws.pin(&format!("t{i}"), "b", "static");
        }
        assert_eq!(ws.pinned.len(), MAX_PINS);
        assert_eq!(ws.pinned.first().unwrap().name, "t4");
        assert_eq!(ws.pinned.last().unwrap().name, "t19");
        ws.pin("t10", "updated", "remote");
        assert_eq!(ws.pinned.last().unwrap().name, "t10");
        assert_eq!(ws.pinned.last().unwrap().blurb, "updated");
        assert_eq!(ws.pinned.iter().filter(|p| p.name == "t10").count(), 1);
    }

    #[test]
    fn round_trip_file() {
        let dir = std::env::temp_dir().join(format!("koma-ws-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut ws = WorkingSet::default();
        ws.set_lane("red");
        ws.pin("sec_nuclei", "scanner", "remote");
        ws.set_last_run("sec_nuclei", "confirmed");
        store(&dir, &ws).unwrap();
        let back = load(&dir);
        assert_eq!(back.lane, "red");
        assert_eq!(back.pinned.len(), 1);
        assert_eq!(back.last_run.unwrap().verdict, "confirmed");
        let block = context_block(&dir).unwrap();
        assert!(block.contains("sec_nuclei"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
